#![allow(clippy::module_name_repetitions)]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{info, warn};

const BIFLOW_REPOSITORY: &str = "devlifeX/BiFlow";
const BIFLOW_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/devlifeX/BiFlow/main/resources/rules/manifest.json";
const BIFLOW_RAW_PREFIX: &str = "https://raw.githubusercontent.com/devlifeX/BiFlow/";
/// `manifest.commit` records the **upstream** `Chocolate4U` revision the rules
/// were taken from, not a `BiFlow` commit, so it cannot address a file in this
/// repository — using it here 404s. The snapshot files sit beside the manifest
/// on the same branch, and every file is still pinned by the `SHA-256` recorded
/// in the manifest, so integrity does not depend on the ref.
const BIFLOW_SNAPSHOT_REF: &str = "main";
const META_FILE: &str = "sync-meta.json";
const STAGING_DIR: &str = ".staging";
const MAX_BYTES: usize = 20 * 1024 * 1024;
const FETCH_ATTEMPTS: u32 = 3;
const FETCH_BACKOFF: Duration = Duration::from_millis(400);

#[derive(Debug, Error)]
pub enum CloudSyncError {
    #[error("rule I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("rule metadata is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("rule publication failed: {0}")]
    Persist(#[from] tempfile::PersistError),
    #[error("cloud rule download failed: {0}")]
    Fetch(String),
    #[error("downloaded rule set {0} is empty or truncated")]
    InvalidSet(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Domain,
    IpCidr,
}

#[derive(Debug, Clone, Copy)]
struct CatalogEntry {
    local_name: &'static str,
    kind: ProviderKind,
    min_entries: u64,
}

const CATALOG: [CatalogEntry; 3] = [
    CatalogEntry {
        local_name: "iran-domains.txt",
        kind: ProviderKind::Domain,
        min_entries: 1_000,
    },
    CatalogEntry {
        local_name: "iran-networks.txt",
        kind: ProviderKind::IpCidr,
        min_entries: 100,
    },
    CatalogEntry {
        local_name: "private.txt",
        kind: ProviderKind::IpCidr,
        min_entries: 5,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteManifest {
    schema_version: u32,
    commit: String,
    rules: Vec<RemoteManifestRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteManifestRule {
    file: String,
    kind: String,
    entry_count: u64,
    sha256: String,
}

fn manifest_fetch_url() -> &'static str {
    BIFLOW_MANIFEST_URL
}

fn snapshot_file_url(_commit: &str, file: &str) -> String {
    format!("{BIFLOW_RAW_PREFIX}{BIFLOW_SNAPSHOT_REF}/resources/rules/{file}")
}

#[must_use]
pub fn provider_entry_count(text: &str) -> u64 {
    u64::try_from(provider_lines(text).count()).unwrap_or(u64::MAX)
}

#[must_use]
pub fn resolve_provider_path(cache_dir: &Path, bundled_dir: &Path, name: &str) -> PathBuf {
    let cached = cache_dir.join(name);
    if cached.is_file() {
        cached
    } else {
        bundled_dir.join(name)
    }
}

fn provider_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("payload:"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn manifest_kind(value: &str) -> Option<ProviderKind> {
    match value {
        "domain" => Some(ProviderKind::Domain),
        "ip_cidr" => Some(ProviderKind::IpCidr),
        _ => None,
    }
}

fn validate_manifest(manifest: &RemoteManifest) -> Result<(), CloudSyncError> {
    if manifest.schema_version != 1 {
        return Err(CloudSyncError::Fetch(format!(
            "unsupported manifest schema version {}",
            manifest.schema_version
        )));
    }
    if manifest.commit.len() != 40
        || !manifest
            .commit
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(CloudSyncError::Fetch(
            "manifest commit is not a 40-character hex SHA".into(),
        ));
    }
    if !CATALOG.iter().all(|entry| {
        manifest
            .rules
            .iter()
            .any(|rule| rule.file == entry.local_name)
    }) {
        return Err(CloudSyncError::Fetch(
            "manifest is missing an upstream provider file".into(),
        ));
    }
    Ok(())
}

#[async_trait]
pub trait RuleFetcher: Send + Sync {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, CloudSyncError>;
}

#[derive(Debug)]
pub struct ReqwestFetcher {
    client: reqwest::Client,
    direct: reqwest::Client,
}

impl ReqwestFetcher {
    fn new() -> Self {
        Self {
            client: Self::build_client(false),
            direct: Self::build_client(true),
        }
    }

    fn build_client(no_proxy: bool) -> reqwest::Client {
        let mut builder = reqwest::Client::builder()
            .user_agent("BiFlow/0.1.0")
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::limited(8));
        if no_proxy {
            builder = builder.no_proxy();
        }
        builder.build().unwrap_or_else(|_| reqwest::Client::new())
    }

    async fn fetch_with(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, CloudSyncError> {
        let response = client
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| CloudSyncError::Fetch(error.to_string()))?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| CloudSyncError::Fetch(error.to_string()))?;
        if bytes.len() > MAX_BYTES {
            return Err(CloudSyncError::Fetch(format!(
                "response exceeded {MAX_BYTES} bytes"
            )));
        }
        Ok(bytes.to_vec())
    }
}

impl Default for ReqwestFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RuleFetcher for ReqwestFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, CloudSyncError> {
        match Self::fetch_with(&self.client, url).await {
            Ok(bytes) => Ok(bytes),
            Err(proxy_error) => match Self::fetch_with(&self.direct, url).await {
                Ok(bytes) => Ok(bytes),
                Err(_) => Err(proxy_error),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRuleSetStatus {
    pub id: String,
    pub kind: ProviderKind,
    pub entry_count: u64,
    pub source: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRulesStatus {
    pub domain_count: u64,
    pub ip_count: u64,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub source: String,
    pub snapshot_revision: Option<String>,
    pub sets: Vec<CloudRuleSetStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SyncMeta {
    last_synced_at: Option<DateTime<Utc>>,
    source: String,
    snapshot_revision: Option<String>,
    sets: Vec<CloudRuleSetStatus>,
}

#[derive(Clone)]
pub struct CloudRuleStore {
    bundled_dir: PathBuf,
    cache_dir: PathBuf,
    fetcher: Arc<dyn RuleFetcher>,
    sync_lock: Arc<Mutex<()>>,
}

impl std::fmt::Debug for CloudRuleStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudRuleStore")
            .field("bundled_dir", &self.bundled_dir)
            .field("cache_dir", &self.cache_dir)
            .finish_non_exhaustive()
    }
}

impl CloudRuleStore {
    #[must_use]
    pub fn load(bundled_dir: impl Into<PathBuf>, cache_dir: impl Into<PathBuf>) -> Self {
        Self::with_fetcher(bundled_dir, cache_dir, Arc::new(ReqwestFetcher::new()))
    }

    #[must_use]
    pub fn with_fetcher(
        bundled_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        fetcher: Arc<dyn RuleFetcher>,
    ) -> Self {
        Self {
            bundled_dir: bundled_dir.into(),
            cache_dir: cache_dir.into(),
            fetcher,
            sync_lock: Arc::new(Mutex::new(())),
        }
    }

    #[must_use]
    pub fn resolve(&self, name: &str) -> PathBuf {
        resolve_provider_path(&self.cache_dir, &self.bundled_dir, name)
    }

    /// Returns status from a complete cache, falling back to bundled rules.
    ///
    /// # Errors
    ///
    /// Returns [`CloudSyncError`] when cached metadata cannot be read or decoded.
    pub fn status(&self) -> Result<CloudRulesStatus, CloudSyncError> {
        let meta = self.read_meta()?;
        let cache_complete = CATALOG
            .iter()
            .all(|entry| self.cache_dir.join(entry.local_name).is_file());
        if cache_complete {
            if let Some(status) = status_from_meta(&meta) {
                return Ok(status);
            }
        }
        self.bundled_status()
    }

    /// Returns the last published snapshot revision, if the cache has one.
    #[must_use]
    pub fn cached_revision(&self) -> Option<String> {
        self.read_meta()
            .ok()
            .and_then(|meta| meta.snapshot_revision)
    }

    /// Fetches the `BiFlow` manifest and returns its snapshot revision.
    ///
    /// # Errors
    ///
    /// Returns [`CloudSyncError`] when the manifest cannot be downloaded or decoded.
    pub async fn peek_remote_revision(&self) -> Result<String, CloudSyncError> {
        Ok(self.fetch_manifest().await?.commit)
    }

    /// Downloads, validates, and atomically publishes every cloud rule set.
    ///
    /// # Errors
    ///
    /// Returns [`CloudSyncError`] for download, encoding, validation, metadata,
    /// or atomic persistence failures. Existing cached rules remain available
    /// when a replacement cannot be published.
    pub async fn sync(&self) -> Result<CloudRulesStatus, CloudSyncError> {
        let _guard = self.sync_lock.lock().await;
        info!(
            event = "cloud_rules.sync_started",
            section = "cloud_rules",
            initiator = "cloud_rule_store",
            cause = "user_request",
            trace_route = "tauri_command->cloud_rule_store->biflow_manifest",
            provider_count = CATALOG.len(),
            "cloud rule synchronization started"
        );
        fs::create_dir_all(&self.cache_dir)?;
        let manifest = self.fetch_manifest().await?;
        validate_manifest(&manifest)?;
        let (pending, sets) = self.download_manifest_generation(&manifest).await?;
        let meta = SyncMeta {
            last_synced_at: Some(Utc::now()),
            source: BIFLOW_REPOSITORY.into(),
            snapshot_revision: Some(manifest.commit),
            sets,
        };
        self.publish_generation(&pending, &meta)?;
        let status = status_from_meta(&meta).unwrap_or(CloudRulesStatus {
            domain_count: 0,
            ip_count: 0,
            last_synced_at: meta.last_synced_at,
            source: meta.source,
            snapshot_revision: meta.snapshot_revision,
            sets: meta.sets,
        });
        info!(
            event = "cloud_rules.sync_completed",
            section = "cloud_rules",
            initiator = "cloud_rule_store",
            cause = "none",
            trace_route = "tauri_command->cloud_rule_store->atomic_cache",
            domain_count = status.domain_count,
            ip_count = status.ip_count,
            source = status.source,
            "cloud rule synchronization completed"
        );
        Ok(status)
    }

    fn publish_generation(
        &self,
        pending: &[(&'static str, Vec<u8>)],
        meta: &SyncMeta,
    ) -> Result<(), CloudSyncError> {
        let staging = self.cache_dir.join(STAGING_DIR);
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(&staging)?;
        let publish = (|| -> Result<(), CloudSyncError> {
            for (name, bytes) in pending {
                write_atomic(&staging.join(name), bytes)?;
            }
            write_atomic(&staging.join(META_FILE), &serde_json::to_vec_pretty(meta)?)?;
            for (name, _) in pending {
                write_atomic(&self.cache_dir.join(name), &fs::read(staging.join(name))?)?;
            }
            write_atomic(
                &self.cache_dir.join(META_FILE),
                &fs::read(staging.join(META_FILE))?,
            )?;
            Ok(())
        })();
        if let Err(cause) = fs::remove_dir_all(&staging) {
            warn!(
                event = "cloud_rules.staging_cleanup_failed",
                section = "cloud_rules",
                initiator = "cloud_rule_store",
                cause = %cause,
                trace_route = "cloud_rule_store->staging_dir",
                "cloud rule staging directory could not be removed"
            );
        }
        publish
    }

    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, CloudSyncError> {
        let mut last_error = CloudSyncError::Fetch("cloud fetch failed".into());
        for attempt in 0..FETCH_ATTEMPTS {
            match self.fetcher.fetch(url).await {
                Ok(bytes) => return Ok(bytes),
                Err(error) => {
                    last_error = error;
                    warn!(
                        event = "cloud_rules.fetch_attempt_failed",
                        section = "cloud_rules",
                        initiator = "cloud_rule_store",
                        cause = %last_error,
                        attempt = attempt + 1,
                        attempts = FETCH_ATTEMPTS,
                        "cloud rule fetch attempt failed"
                    );
                }
            }
            if attempt + 1 < FETCH_ATTEMPTS {
                tokio::time::sleep(FETCH_BACKOFF.saturating_mul(1 << attempt.min(3))).await;
            }
        }
        Err(last_error)
    }

    async fn download_manifest_generation(
        &self,
        manifest: &RemoteManifest,
    ) -> Result<(Vec<(&'static str, Vec<u8>)>, Vec<CloudRuleSetStatus>), CloudSyncError> {
        let mut pending = Vec::with_capacity(CATALOG.len());
        let mut sets = Vec::with_capacity(CATALOG.len());
        for entry in CATALOG {
            let rule = manifest
                .rules
                .iter()
                .find(|rule| rule.file == entry.local_name)
                .ok_or_else(|| {
                    CloudSyncError::Fetch(format!(
                        "manifest is missing provider file {}",
                        entry.local_name
                    ))
                })?;
            let Some(kind) = manifest_kind(&rule.kind) else {
                return Err(CloudSyncError::Fetch(format!(
                    "manifest has unsupported kind for {}",
                    entry.local_name
                )));
            };
            if kind != entry.kind {
                return Err(CloudSyncError::Fetch(format!(
                    "manifest kind mismatch for {}",
                    entry.local_name
                )));
            }
            let url = snapshot_file_url(&manifest.commit, &rule.file);
            let bytes = self.fetch_bytes(&url).await?;
            if bytes.len() > MAX_BYTES {
                return Err(CloudSyncError::Fetch(format!(
                    "response exceeded {MAX_BYTES} bytes"
                )));
            }
            let digest = sha256_hex(&bytes);
            if digest != rule.sha256 {
                return Err(CloudSyncError::InvalidSet(format!(
                    "{} sha256 mismatch",
                    entry.local_name
                )));
            }
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| CloudSyncError::InvalidSet(entry.local_name.into()))?;
            let count = provider_entry_count(text);
            if count < entry.min_entries || count != rule.entry_count {
                return Err(CloudSyncError::InvalidSet(entry.local_name.into()));
            }
            pending.push((entry.local_name, bytes));
            sets.push(CloudRuleSetStatus {
                id: entry.local_name.trim_end_matches(".txt").to_owned(),
                kind: entry.kind,
                entry_count: count,
                source: BIFLOW_REPOSITORY.into(),
                sha256: Some(digest),
            });
        }
        Ok((pending, sets))
    }

    async fn fetch_manifest(&self) -> Result<RemoteManifest, CloudSyncError> {
        let bytes = self.fetch_bytes(manifest_fetch_url()).await?;
        serde_json::from_slice(&bytes).map_err(CloudSyncError::from)
    }

    fn bundled_status(&self) -> Result<CloudRulesStatus, CloudSyncError> {
        let mut sets = Vec::new();
        for entry in CATALOG {
            let path = self.bundled_dir.join(entry.local_name);
            let bytes = fs::read(&path)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| CloudSyncError::InvalidSet(entry.local_name.into()))?;
            let count = provider_entry_count(text);
            if count < entry.min_entries {
                return Err(CloudSyncError::InvalidSet(entry.local_name.into()));
            }
            sets.push(CloudRuleSetStatus {
                id: entry.local_name.trim_end_matches(".txt").to_owned(),
                kind: entry.kind,
                entry_count: count,
                source: "bundled".into(),
                sha256: Some(sha256_hex(&bytes)),
            });
        }
        Ok(summarize(sets, None, "bundled", None))
    }

    fn read_meta(&self) -> Result<SyncMeta, CloudSyncError> {
        let path = self.cache_dir.join(META_FILE);
        if !path.is_file() {
            return Ok(SyncMeta::default());
        }
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }
}

fn status_from_meta(meta: &SyncMeta) -> Option<CloudRulesStatus> {
    if meta.sets.is_empty() {
        return None;
    }
    Some(summarize(
        meta.sets.clone(),
        meta.last_synced_at,
        &meta.source,
        meta.snapshot_revision.clone(),
    ))
}

fn summarize(
    sets: Vec<CloudRuleSetStatus>,
    last_synced_at: Option<DateTime<Utc>>,
    source: &str,
    snapshot_revision: Option<String>,
) -> CloudRulesStatus {
    let domain_count = sets
        .iter()
        .filter(|set| set.kind == ProviderKind::Domain)
        .map(|set| set.entry_count)
        .sum();
    let ip_count = sets
        .iter()
        .filter(|set| set.kind == ProviderKind::IpCidr)
        .map(|set| set.entry_count)
        .sum();
    CloudRulesStatus {
        domain_count,
        ip_count,
        last_synced_at,
        source: source.to_owned(),
        snapshot_revision,
        sets,
    }
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), CloudSyncError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(content)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

const EMBEDDED_BUNDLED_RULES: &[(&str, &str)] = &[
    (
        "iran-domains.txt",
        include_str!("../../../resources/rules/iran-domains.txt"),
    ),
    (
        "iran-networks.txt",
        include_str!("../../../resources/rules/iran-networks.txt"),
    ),
    (
        "private.txt",
        include_str!("../../../resources/rules/private.txt"),
    ),
    (
        "iran-business-domains.txt",
        include_str!("../../../resources/rules/iran-business-domains.txt"),
    ),
    (
        "iran-cdn-networks.txt",
        include_str!("../../../resources/rules/iran-cdn-networks.txt"),
    ),
];

const CURATED_FILES: [&str; 2] = ["iran-business-domains.txt", "iran-cdn-networks.txt"];

/// Returns whether `dir` contains every bundled Iran/private/curated rule file.
#[must_use]
pub fn bundled_snapshot_is_complete(dir: &Path) -> bool {
    CATALOG
        .iter()
        .all(|entry| dir.join(entry.local_name).is_file())
        && CURATED_FILES.iter().all(|name| dir.join(name).is_file())
}

/// Uses `packaged` when it already has the snapshot; otherwise writes the
/// compile-time snapshot into `fallback`.
///
/// # Errors
///
/// Returns [`CloudSyncError`] when the fallback directory cannot be created or
/// the embedded files cannot be written.
pub fn ensure_bundled_snapshot(
    packaged: &Path,
    fallback: &Path,
) -> Result<PathBuf, CloudSyncError> {
    if bundled_snapshot_is_complete(packaged) {
        return Ok(packaged.to_owned());
    }
    fs::create_dir_all(fallback)?;
    for (name, contents) in EMBEDDED_BUNDLED_RULES {
        write_atomic(&fallback.join(name), contents.as_bytes())?;
    }
    if bundled_snapshot_is_complete(fallback) {
        Ok(fallback.to_owned())
    } else {
        Err(CloudSyncError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "embedded Iran rule snapshot could not be materialized",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fmt::Write as _;

    struct MapFetcher {
        responses: HashMap<String, Result<Vec<u8>, String>>,
    }

    #[async_trait]
    impl RuleFetcher for MapFetcher {
        async fn fetch(&self, url: &str) -> Result<Vec<u8>, CloudSyncError> {
            match self.responses.get(url) {
                Some(Ok(bytes)) => Ok(bytes.clone()),
                Some(Err(message)) => Err(CloudSyncError::Fetch(message.clone())),
                None => Err(CloudSyncError::Fetch("missing".into())),
            }
        }
    }

    fn domain_payload(count: usize) -> Vec<u8> {
        (0..count)
            .fold(String::new(), |mut payload, index| {
                writeln!(payload, "+.example{index}.ir").expect("write to string");
                payload
            })
            .into_bytes()
    }

    fn cidr_payload(count: usize) -> Vec<u8> {
        (0..count)
            .fold(String::new(), |mut payload, index| {
                writeln!(payload, "203.0.{index}.0/24").expect("write to string");
                payload
            })
            .into_bytes()
    }

    fn manifest_for(commit: &str, rules: &[(&str, ProviderKind, u64, &str)]) -> RemoteManifest {
        RemoteManifest {
            schema_version: 1,
            commit: commit.to_owned(),
            rules: rules
                .iter()
                .map(|(file, kind, count, digest)| RemoteManifestRule {
                    file: (*file).into(),
                    kind: match kind {
                        ProviderKind::Domain => "domain",
                        ProviderKind::IpCidr => "ip_cidr",
                    }
                    .into(),
                    entry_count: *count,
                    sha256: (*digest).into(),
                })
                .collect(),
        }
    }

    fn seed_bundled(bundled: &Path) {
        fs::create_dir_all(bundled).expect("bundled");
        fs::write(bundled.join("iran-domains.txt"), "+.old.ir\n").expect("domains");
        fs::write(bundled.join("iran-networks.txt"), "1.2.3.0/24\n").expect("networks");
        fs::write(bundled.join("private.txt"), "10.0.0.0/8\n").expect("private");
        fs::write(
            bundled.join("iran-business-domains.txt"),
            "+.technolife.com\n",
        )
        .expect("business");
        fs::write(bundled.join("iran-cdn-networks.txt"), "185.163.216.0/22\n").expect("cdn");
    }

    #[test]
    fn runtime_urls_use_only_biflow_repository() {
        assert!(manifest_fetch_url().starts_with(BIFLOW_RAW_PREFIX));
        let file_url = snapshot_file_url(
            "abc123def4567890abc123def4567890abc123de",
            "iran-domains.txt",
        );
        assert!(file_url.starts_with(BIFLOW_RAW_PREFIX));
        assert!(file_url.ends_with("/resources/rules/iran-domains.txt"));
    }

    #[test]
    fn runtime_source_has_no_third_party_rule_hosts() {
        const FORBIDDEN: &[&str] = &["Chocolate4U", "jsdelivr", "Iran-clash-rules"];
        for line in include_str!("cloud.rs").lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("const ") && trimmed.contains("https://") {
                for forbidden in FORBIDDEN {
                    assert!(
                        !trimmed.contains(forbidden),
                        "runtime URL constant must not reference {forbidden}: {trimmed}"
                    );
                }
            }
        }
    }

    #[test]
    fn counts_ignore_comments_and_payload_headers() {
        let text = "# comment\npayload:\n+.digikala.com\n\n5.22.0.0/16\n";
        assert_eq!(provider_entry_count(text), 2);
    }

    #[test]
    fn incomplete_bundled_snapshot_is_rejected() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = CloudRuleStore::load(directory.path(), directory.path().join("cache"));

        assert!(matches!(store.status(), Err(CloudSyncError::Io(_))));
    }

    #[test]
    fn ensure_bundled_snapshot_keeps_a_complete_packaged_dir() {
        let directory = tempfile::tempdir().expect("tempdir");
        let packaged = directory.path().join("packaged");
        seed_bundled(&packaged);
        let fallback = directory.path().join("fallback");

        let resolved = ensure_bundled_snapshot(&packaged, &fallback).expect("packaged snapshot");

        assert_eq!(resolved, packaged);
        assert!(!fallback.exists());
    }

    #[test]
    fn ensure_bundled_snapshot_writes_embedded_files_when_packaged_is_missing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let packaged = directory.path().join("missing");
        let fallback = directory.path().join("fallback");

        let resolved = ensure_bundled_snapshot(&packaged, &fallback).expect("embedded snapshot");
        let store = CloudRuleStore::load(resolved.clone(), directory.path().join("cache"));
        let status = store.status().expect("bundled status");

        assert_eq!(resolved, fallback);
        assert!(bundled_snapshot_is_complete(&fallback));
        assert!(status.domain_count >= 1_000);
        assert!(status.ip_count >= 105);
        assert_eq!(status.source, "bundled");
    }

    #[tokio::test]
    async fn sync_fetches_manifest_then_files_and_keeps_last_good_on_failure() {
        let directory = tempfile::tempdir().expect("tempdir");
        let bundled = directory.path().join("bundled");
        let cache = directory.path().join("cache");
        seed_bundled(&bundled);

        let commit = "a".repeat(40);
        let domain_bytes = domain_payload(1_000);
        let network_bytes = cidr_payload(100);
        let private_bytes = cidr_payload(8);
        let manifest = manifest_for(
            &commit,
            &[
                (
                    "iran-domains.txt",
                    ProviderKind::Domain,
                    1_000,
                    &sha256_hex(&domain_bytes),
                ),
                (
                    "iran-networks.txt",
                    ProviderKind::IpCidr,
                    100,
                    &sha256_hex(&network_bytes),
                ),
                (
                    "private.txt",
                    ProviderKind::IpCidr,
                    8,
                    &sha256_hex(&private_bytes),
                ),
            ],
        );
        let mut responses = HashMap::new();
        responses.insert(
            manifest_fetch_url().to_owned(),
            Ok(serde_json::to_vec(&manifest).expect("manifest json")),
        );
        responses.insert(
            snapshot_file_url(&commit, "iran-domains.txt"),
            Ok(domain_bytes),
        );
        responses.insert(
            snapshot_file_url(&commit, "iran-networks.txt"),
            Ok(network_bytes),
        );
        responses.insert(snapshot_file_url(&commit, "private.txt"), Ok(private_bytes));

        let store = CloudRuleStore::with_fetcher(
            bundled,
            cache.clone(),
            Arc::new(MapFetcher { responses }),
        );
        let status = store.sync().await.expect("sync");
        assert_eq!(status.domain_count, 1_000);
        assert_eq!(status.ip_count, 108);
        assert_eq!(status.source, BIFLOW_REPOSITORY);
        assert_eq!(status.snapshot_revision, Some(commit.clone()));
        assert!(status.last_synced_at.is_some());
        assert!(cache.join("iran-domains.txt").is_file());

        let failing = CloudRuleStore::with_fetcher(
            directory.path().join("bundled"),
            cache.clone(),
            Arc::new(MapFetcher {
                responses: HashMap::new(),
            }),
        );
        assert!(failing.sync().await.is_err());
        let kept = failing.status().expect("status");
        assert_eq!(kept.domain_count, 1_000);
        assert_eq!(kept.source, BIFLOW_REPOSITORY);
        assert_eq!(kept.snapshot_revision, Some(commit.clone()));
        assert_eq!(failing.cached_revision(), Some(commit.clone()));
        assert_eq!(store.peek_remote_revision().await.expect("peek"), commit);
    }

    #[tokio::test]
    async fn sync_does_not_touch_direct_rules_or_overwrite_curated_catalog() {
        let directory = tempfile::tempdir().expect("tempdir");
        let bundled = directory.path().join("bundled");
        let cache = directory.path().join("cache");
        seed_bundled(&bundled);
        let pins = directory.path().join("direct-rules.json");
        fs::write(&pins, br#"{"revision":3,"rules":[],"vpn_rules":[]}"#).expect("pins");
        let curated_before = fs::read(bundled.join("iran-business-domains.txt")).expect("curated");
        let curated_cidrs_before =
            fs::read(bundled.join("iran-cdn-networks.txt")).expect("curated cidrs");
        let pins_before = fs::read(&pins).expect("pins bytes");

        let commit = "c".repeat(40);
        let domain_bytes = domain_payload(1_000);
        let network_bytes = cidr_payload(100);
        let private_bytes = cidr_payload(8);
        let manifest = manifest_for(
            &commit,
            &[
                (
                    "iran-domains.txt",
                    ProviderKind::Domain,
                    1_000,
                    &sha256_hex(&domain_bytes),
                ),
                (
                    "iran-networks.txt",
                    ProviderKind::IpCidr,
                    100,
                    &sha256_hex(&network_bytes),
                ),
                (
                    "private.txt",
                    ProviderKind::IpCidr,
                    8,
                    &sha256_hex(&private_bytes),
                ),
            ],
        );
        let mut responses = HashMap::new();
        responses.insert(
            manifest_fetch_url().to_owned(),
            Ok(serde_json::to_vec(&manifest).expect("manifest json")),
        );
        responses.insert(
            snapshot_file_url(&commit, "iran-domains.txt"),
            Ok(domain_bytes),
        );
        responses.insert(
            snapshot_file_url(&commit, "iran-networks.txt"),
            Ok(network_bytes),
        );
        responses.insert(snapshot_file_url(&commit, "private.txt"), Ok(private_bytes));

        let store = CloudRuleStore::with_fetcher(
            bundled.clone(),
            cache.clone(),
            Arc::new(MapFetcher { responses }),
        );
        store.sync().await.expect("sync");
        assert_eq!(fs::read(&pins).expect("pins after"), pins_before);
        assert_eq!(
            fs::read(bundled.join("iran-business-domains.txt")).expect("curated after"),
            curated_before
        );
        assert_eq!(
            fs::read(bundled.join("iran-cdn-networks.txt")).expect("curated cidrs after"),
            curated_cidrs_before
        );
        assert!(!cache.join("iran-business-domains.txt").is_file());
        assert!(!cache.join("iran-cdn-networks.txt").is_file());
        assert!(!cache.join("direct-rules.json").is_file());
    }

    #[tokio::test]
    async fn sync_rejects_sha256_mismatch_before_publishing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let bundled = directory.path().join("bundled");
        let cache = directory.path().join("cache");
        seed_bundled(&bundled);

        let commit = "b".repeat(40);
        let domain_bytes = domain_payload(1_000);
        let manifest = manifest_for(
            &commit,
            &[
                ("iran-domains.txt", ProviderKind::Domain, 1_000, "deadbeef"),
                (
                    "iran-networks.txt",
                    ProviderKind::IpCidr,
                    100,
                    &sha256_hex(&cidr_payload(100)),
                ),
                (
                    "private.txt",
                    ProviderKind::IpCidr,
                    8,
                    &sha256_hex(&cidr_payload(8)),
                ),
            ],
        );
        let mut responses = HashMap::new();
        responses.insert(
            manifest_fetch_url().to_owned(),
            Ok(serde_json::to_vec(&manifest).expect("manifest json")),
        );
        responses.insert(
            snapshot_file_url(&commit, "iran-domains.txt"),
            Ok(domain_bytes),
        );

        let store = CloudRuleStore::with_fetcher(
            bundled,
            cache.clone(),
            Arc::new(MapFetcher { responses }),
        );
        assert!(store.sync().await.is_err());
        assert!(!cache.join("iran-domains.txt").is_file());
        assert!(!cache.join(META_FILE).is_file());
    }

    struct FlakyFetcher {
        inner: MapFetcher,
        remaining: std::sync::atomic::AtomicU32,
    }

    #[async_trait]
    impl RuleFetcher for FlakyFetcher {
        async fn fetch(&self, url: &str) -> Result<Vec<u8>, CloudSyncError> {
            let previous = self
                .remaining
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if previous > 0 {
                return Err(CloudSyncError::Fetch("flaky".into()));
            }
            self.remaining
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.fetch(url).await
        }
    }

    #[tokio::test]
    async fn sync_retries_transient_fetches_and_clears_staging() {
        let directory = tempfile::tempdir().expect("tempdir");
        let bundled = directory.path().join("bundled");
        let cache = directory.path().join("cache");
        seed_bundled(&bundled);
        fs::create_dir_all(cache.join(STAGING_DIR)).expect("stale staging");
        fs::write(cache.join(STAGING_DIR).join("junk.txt"), b"stale").expect("junk");

        let commit = "c".repeat(40);
        let domain_bytes = domain_payload(1_000);
        let network_bytes = cidr_payload(100);
        let private_bytes = cidr_payload(8);
        let manifest = manifest_for(
            &commit,
            &[
                (
                    "iran-domains.txt",
                    ProviderKind::Domain,
                    1_000,
                    &sha256_hex(&domain_bytes),
                ),
                (
                    "iran-networks.txt",
                    ProviderKind::IpCidr,
                    100,
                    &sha256_hex(&network_bytes),
                ),
                (
                    "private.txt",
                    ProviderKind::IpCidr,
                    8,
                    &sha256_hex(&private_bytes),
                ),
            ],
        );
        let mut responses = HashMap::new();
        responses.insert(
            manifest_fetch_url().to_owned(),
            Ok(serde_json::to_vec(&manifest).expect("manifest json")),
        );
        responses.insert(
            snapshot_file_url(&commit, "iran-domains.txt"),
            Ok(domain_bytes),
        );
        responses.insert(
            snapshot_file_url(&commit, "iran-networks.txt"),
            Ok(network_bytes),
        );
        responses.insert(snapshot_file_url(&commit, "private.txt"), Ok(private_bytes));

        let store = CloudRuleStore::with_fetcher(
            bundled,
            cache.clone(),
            Arc::new(FlakyFetcher {
                inner: MapFetcher { responses },
                remaining: std::sync::atomic::AtomicU32::new(2),
            }),
        );
        let status = store.sync().await.expect("sync after retries");
        assert_eq!(status.snapshot_revision, Some(commit));
        assert!(!cache.join(STAGING_DIR).exists());
        assert!(cache.join("iran-domains.txt").is_file());
    }
}
