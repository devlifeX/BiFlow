mod canonical;
mod cloud;
mod google_search;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ipnet::IpNet;
use iran_split_config::{ClientId, EgressKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

pub use canonical::{canonical_target, domain_matches_pin, domain_specificity, registrable_domain};
pub use cloud::{
    bundled_snapshot_is_complete, ensure_bundled_snapshot, provider_entry_count,
    resolve_provider_path, CloudRuleSetStatus, CloudRuleStore, CloudRulesStatus, CloudSyncError,
    RuleFetcher,
};
pub use google_search::{
    expands_google_search_companions, rebind_hosts_for_pin, GOOGLE_SEARCH_COMPANION_DOMAINS,
};

#[derive(Debug, Error)]
pub enum RuleError {
    #[error("invalid direct rule: {0}")]
    InvalidRule(String),
    #[error("rule I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("rule data is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("rule publication failed: {0}")]
    Persist(#[from] tempfile::PersistError),
    #[error("DNS resolution failed: {0}")]
    Resolve(String),
    #[error("rule revision conflict: expected {expected}, found {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DirectTarget {
    Domain(String),
    Ip(IpAddr),
}

impl DirectTarget {
    /// Parses an exact domain or IP address into a direct-routing target.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError::InvalidRule`] when the input is not a valid IP
    /// address or normalized domain.
    pub fn parse(input: &str) -> Result<Self, RuleError> {
        canonical_target(input)
    }

    #[must_use]
    pub fn display_value(&self) -> String {
        match self {
            Self::Domain(domain) => domain.clone(),
            Self::Ip(address) => address.to_string(),
        }
    }
}

/// Normalizes and validates an exact domain name for direct routing.
///
/// # Errors
///
/// Returns [`RuleError::InvalidRule`] for URLs, paths, wildcards, user info,
/// invalid IDNA, or malformed DNS labels.
pub fn normalize_domain(input: &str) -> Result<String, RuleError> {
    let candidate = input.trim().trim_end_matches('.').to_lowercase();
    if candidate.is_empty()
        || candidate.len() > 253
        || candidate.contains("://")
        || candidate.contains('/')
        || candidate.contains('*')
        || candidate.contains('@')
    {
        return Err(RuleError::InvalidRule(
            "enter an exact domain without a URL, path, wildcard, or user info".into(),
        ));
    }
    let ascii = idna::domain_to_ascii(&candidate)
        .map_err(|_| RuleError::InvalidRule("domain cannot be converted to IDNA ASCII".into()))?;
    let labels: Vec<_> = ascii.split('.').collect();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(RuleError::InvalidRule(
            "domain must contain valid DNS labels and a suffix".into(),
        ));
    }
    Ok(ascii)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectRule {
    pub target: DirectTarget,
    pub resolved_ips: Vec<IpAddr>,
    pub created_at: DateTime<Utc>,
    pub refreshed_at: Option<DateTime<Utc>>,
}

/// One user pin: a host lives in exactly one outbound and one named list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedRoute {
    pub target: DirectTarget,
    pub outbound: Outbound,
    #[serde(default)]
    pub list_id: Option<Uuid>,
    pub resolved_ips: Vec<IpAddr>,
    pub created_at: DateTime<Utc>,
    pub refreshed_at: Option<DateTime<Utc>>,
}

/// A named bundle of pins with exactly one outbound. The list is the
/// user-facing model; generation still reads the flat pins by outbound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleListMeta {
    pub id: Uuid,
    pub name: String,
    pub outbound: Outbound,
}

/// User route pins. Schema 3 is a single list; older `rules` / `vpn_rules`
/// documents are migrated on load when a legacy client id is supplied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RoutePinsDocument {
    pub revision: u64,
    #[serde(default)]
    pub pins: Vec<PinnedRoute>,
    #[serde(default)]
    pub lists: Vec<RuleListMeta>,
}

/// Compatibility name used by older call sites and the desktop IPC.
pub type DirectRulesDocument = RoutePinsDocument;

/// Client ids that receive leftover `vpn_rules` / `openvpn_rules` lists.
#[derive(Debug, Clone, Copy, Default)]
pub struct LegacyPinClients {
    pub vpn: Option<ClientId>,
    pub openvpn: Option<ClientId>,
}

/// How strictly a pin may target private or loopback addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinPolicy {
    Direct,
    LocalProxy,
    OwnedSideTunnel,
}

impl PinPolicy {
    #[must_use]
    pub const fn for_kind(kind: EgressKind) -> Self {
        match kind {
            EgressKind::LocalProxy | EgressKind::Unsupported => Self::LocalProxy,
            EgressKind::OwnedSideTunnel => Self::OwnedSideTunnel,
        }
    }
}

impl RoutePinsDocument {
    #[must_use]
    pub fn pins_for<'a>(&'a self, outbound: &'a Outbound) -> Vec<&'a PinnedRoute> {
        self.pins
            .iter()
            .filter(|pin| pin.outbound == *outbound)
            .collect()
    }

    #[must_use]
    pub fn count_for_client(&self, id: ClientId) -> usize {
        self.pins
            .iter()
            .filter(|pin| pin.outbound == Outbound::client(id))
            .count()
    }

    pub fn delete_client_pins(&mut self, id: ClientId) -> usize {
        let before = self.pins.len();
        self.pins.retain(|pin| pin.outbound != Outbound::client(id));
        self.lists
            .retain(|list| list.outbound != Outbound::client(id));
        before.saturating_sub(self.pins.len())
    }

    pub fn move_client_pins(&mut self, from: ClientId, to: Outbound) -> usize {
        let mut moved = 0;
        for pin in &mut self.pins {
            if pin.outbound == Outbound::client(from) {
                pin.outbound = to;
                moved += 1;
            }
        }
        for list in &mut self.lists {
            if list.outbound == Outbound::client(from) {
                list.outbound = to;
            }
        }
        self.pins.sort_by_key(|pin| pin.target.display_value());
        ensure_list_membership(self);
        moved
    }

    #[must_use]
    pub fn list_meta(&self, id: Uuid) -> Option<&RuleListMeta> {
        self.lists.iter().find(|list| list.id == id)
    }

    #[must_use]
    pub fn pins_in_list(&self, id: Uuid) -> Vec<&PinnedRoute> {
        self.pins
            .iter()
            .filter(|pin| pin.list_id == Some(id))
            .collect()
    }

    /// First list bound to `outbound`, creating a default-named one if none
    /// exists. Returns its id.
    pub fn default_list_for(&mut self, outbound: Outbound) -> Uuid {
        if let Some(list) = self.lists.iter().find(|list| list.outbound == outbound) {
            return list.id;
        }
        let id = Uuid::new_v4();
        self.lists.push(RuleListMeta {
            id,
            name: default_list_name(outbound),
            outbound,
        });
        id
    }
}

fn default_list_name(outbound: Outbound) -> String {
    match outbound {
        Outbound::Direct => "Direct".into(),
        Outbound::Client { .. } => "Client pins".into(),
    }
}

/// Repairs the pin <-> list relationship: every pin belongs to a list whose
/// outbound matches; orphan pins are attached to (or get) a default list.
fn ensure_list_membership(document: &mut RoutePinsDocument) {
    let outbounds: Vec<Outbound> = document.pins.iter().map(|pin| pin.outbound).collect();
    for outbound in outbounds {
        if !document.lists.iter().any(|list| list.outbound == outbound) {
            document.lists.push(RuleListMeta {
                id: Uuid::new_v4(),
                name: default_list_name(outbound),
                outbound,
            });
        }
    }
    let lists = document.lists.clone();
    for pin in &mut document.pins {
        let valid = pin.list_id.is_some_and(|id| {
            lists
                .iter()
                .any(|list| list.id == id && list.outbound == pin.outbound)
        });
        if !valid {
            pin.list_id = lists
                .iter()
                .find(|list| list.outbound == pin.outbound)
                .map(|list| list.id);
        }
    }
}

#[async_trait]
pub trait Resolver: Send + Sync {
    async fn resolve(&self, domain: &str) -> Result<Vec<IpAddr>, RuleError>;
}

#[derive(Debug, Default)]
pub struct SystemResolver;

#[async_trait]
impl Resolver for SystemResolver {
    async fn resolve(&self, domain: &str) -> Result<Vec<IpAddr>, RuleError> {
        let addresses = tokio::net::lookup_host((domain, 0))
            .await
            .map_err(|error| RuleError::Resolve(error.to_string()))?;
        Ok(unique_addresses(addresses.map(|address| address.ip())))
    }
}

#[derive(Debug, Clone)]
pub struct DohResolver {
    client: reqwest::Client,
    endpoint: &'static str,
}

impl Default for DohResolver {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: "https://cloudflare-dns.com/dns-query",
        }
    }
}

#[derive(Debug, Deserialize)]
struct DohResponse {
    #[serde(default, rename = "Answer")]
    answers: Vec<DohAnswer>,
}

#[derive(Debug, Deserialize)]
struct DohAnswer {
    data: String,
}

#[async_trait]
impl Resolver for DohResolver {
    async fn resolve(&self, domain: &str) -> Result<Vec<IpAddr>, RuleError> {
        let mut values = Vec::new();
        for record_type in ["A", "AAAA"] {
            let response = self
                .client
                .get(self.endpoint)
                .header(reqwest::header::ACCEPT, "application/dns-json")
                .query(&[("name", domain), ("type", record_type)])
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|error| RuleError::Resolve(error.to_string()))?
                .json::<DohResponse>()
                .await
                .map_err(|error| RuleError::Resolve(error.to_string()))?;
            values.extend(
                response
                    .answers
                    .into_iter()
                    .filter_map(|answer| answer.data.parse::<IpAddr>().ok()),
            );
        }
        Ok(unique_addresses(values))
    }
}

fn unique_addresses(values: impl IntoIterator<Item = IpAddr>) -> Vec<IpAddr> {
    let mut values: Vec<_> = values
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    values.sort_unstable();
    values
}

#[derive(Clone)]
pub struct RuleManager {
    path: PathBuf,
    resolver: Arc<dyn Resolver>,
    document: Arc<Mutex<DirectRulesDocument>>,
}

impl std::fmt::Debug for RuleManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuleManager")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl RuleManager {
    /// Loads the pin document at `path`, or starts empty when absent.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] when the document cannot be read or decoded.
    pub fn load(path: impl Into<PathBuf>, resolver: Arc<dyn Resolver>) -> Result<Self, RuleError> {
        Self::load_with_legacy(path, resolver, LegacyPinClients::default())
    }

    /// Loads pins and remaps leftover `vpn_rules` / `openvpn_rules` lists.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] when the document cannot be read or decoded.
    pub fn load_with_legacy(
        path: impl Into<PathBuf>,
        resolver: Arc<dyn Resolver>,
        legacy: LegacyPinClients,
    ) -> Result<Self, RuleError> {
        let path = path.into();
        let document = if path.exists() {
            let bytes = fs::read(&path)?;
            match decode_pins_document(&bytes, legacy) {
                Ok(original) => {
                    let migrated = canonicalize_document(original.clone());
                    if migrated != original {
                        backup_last_good(&path)?;
                        publish(&path, &migrated)?;
                    }
                    migrated
                }
                Err(RuleError::Json(cause)) => {
                    // A document another (older or newer) build wrote must
                    // never keep this build from starting: quarantine the
                    // file and begin empty. The copy stays for recovery.
                    tracing::error!(
                        event = "rules.document_quarantined",
                        section = "rules",
                        initiator = "rule_manager",
                        cause = %cause,
                        trace_route = "rule_manager->load->decode",
                        "route pins document is unreadable; starting empty and keeping a .corrupt copy"
                    );
                    let quarantine = path.with_extension("json.corrupt");
                    fs::copy(&path, quarantine)?;
                    let empty = RoutePinsDocument::default();
                    publish(&path, &empty)?;
                    empty
                }
                Err(error) => return Err(error),
            }
        } else {
            RoutePinsDocument::default()
        };
        Ok(Self {
            path,
            resolver,
            document: Arc::new(Mutex::new(document)),
        })
    }

    pub async fn list(&self) -> DirectRulesDocument {
        self.document.lock().await.clone()
    }

    /// Adds an exact domain or IP rule to the DIRECT list.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for invalid input, DNS resolution failure, a
    /// revision conflict, or an atomic persistence failure.
    pub async fn add(
        &self,
        input: &str,
        expected_revision: u64,
    ) -> Result<DirectRulesDocument, RuleError> {
        self.pin(input, Outbound::Direct, expected_revision).await
    }

    /// Pins an exact domain or IP to one outbound.
    ///
    /// A host belongs to at most one user list, so pinning it to an outbound
    /// drops any pin it had on the other one.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for invalid input, a private or local address
    /// pinned to the VPN, DNS resolution failure, a revision conflict, or an
    /// atomic persistence failure.
    pub async fn pin(
        &self,
        input: &str,
        outbound: Outbound,
        expected_revision: u64,
    ) -> Result<RoutePinsDocument, RuleError> {
        let policy = match outbound {
            Outbound::Direct => PinPolicy::Direct,
            Outbound::Client { .. } => PinPolicy::LocalProxy,
        };
        self.pin_with_policy(input, outbound, policy, expected_revision)
            .await
    }

    /// Pins a host to one outbound using the egress-kind policy for that client.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for invalid input, a rejected private address, a
    /// revision conflict, or an atomic persistence failure.
    pub async fn pin_with_policy(
        &self,
        input: &str,
        outbound: Outbound,
        policy: PinPolicy,
        expected_revision: u64,
    ) -> Result<RoutePinsDocument, RuleError> {
        self.pin_into(input, outbound, None, policy, expected_revision)
            .await
    }

    /// Pins a host into one specific named list.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for invalid input, an unknown list, a rejected
    /// private address, a revision conflict, or a persistence failure.
    pub async fn pin_to_list(
        &self,
        input: &str,
        list_id: Uuid,
        policy: PinPolicy,
        expected_revision: u64,
    ) -> Result<RoutePinsDocument, RuleError> {
        let outbound = {
            let document = self.document.lock().await;
            document
                .list_meta(list_id)
                .map(|list| list.outbound)
                .ok_or_else(|| RuleError::InvalidRule("unknown list".into()))?
        };
        self.pin_into(input, outbound, Some(list_id), policy, expected_revision)
            .await
    }

    async fn pin_into(
        &self,
        input: &str,
        outbound: Outbound,
        list_id: Option<Uuid>,
        policy: PinPolicy,
        expected_revision: u64,
    ) -> Result<RoutePinsDocument, RuleError> {
        let target = DirectTarget::parse(input)?;
        if let DirectTarget::Ip(address) = &target {
            reject_pin_address(*address, policy)?;
        }
        let resolved_ips = match &target {
            DirectTarget::Domain(_) => Vec::new(),
            DirectTarget::Ip(address) => vec![*address],
        };
        let mut document = self.document.lock().await;
        ensure_revision(&document, expected_revision)?;
        let list_id = list_id.unwrap_or_else(|| document.default_list_for(outbound));

        let already_pinned = document
            .pins
            .iter()
            .any(|pin| pin.target == target && pin.outbound == outbound);
        let other_count = document
            .pins
            .iter()
            .filter(|pin| pin.target == target && pin.outbound != outbound)
            .count();
        document
            .pins
            .retain(|pin| pin.target != target || pin.outbound == outbound);
        if already_pinned {
            // Keep the pin but move it into the requested list if needed.
            let mut moved = false;
            for pin in &mut document.pins {
                if pin.target == target && pin.list_id != Some(list_id) {
                    pin.list_id = Some(list_id);
                    moved = true;
                }
            }
            if other_count == 0 {
                let companions =
                    attach_google_search_companions(&mut document, &target, outbound, list_id);
                if moved || companions {
                    document.revision = document.revision.saturating_add(1);
                    publish(&self.path, &document)?;
                }
                return Ok(document.clone());
            }
            attach_google_search_companions(&mut document, &target, outbound, list_id);
        } else {
            let now = Utc::now();
            document.pins.push(PinnedRoute {
                target: target.clone(),
                outbound,
                list_id: Some(list_id),
                resolved_ips,
                created_at: now,
                refreshed_at: Some(now),
            });
            attach_google_search_companions(&mut document, &target, outbound, list_id);
            document.pins.sort_by_key(|pin| pin.target.display_value());
        }
        document.revision = document.revision.saturating_add(1);
        publish(&self.path, &document)?;
        Ok(document.clone())
    }

    /// Creates a named list bound to one outbound.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for an empty name, a revision conflict, or a
    /// persistence failure.
    pub async fn create_list(
        &self,
        name: &str,
        outbound: Outbound,
        expected_revision: u64,
    ) -> Result<RoutePinsDocument, RuleError> {
        let name = validate_list_name(name)?;
        let mut document = self.document.lock().await;
        ensure_revision(&document, expected_revision)?;
        document.lists.push(RuleListMeta {
            id: Uuid::new_v4(),
            name,
            outbound,
        });
        document.revision = document.revision.saturating_add(1);
        publish(&self.path, &document)?;
        Ok(document.clone())
    }

    /// Renames a list.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for an empty name, an unknown list, a revision
    /// conflict, or a persistence failure.
    pub async fn rename_list(
        &self,
        list_id: Uuid,
        name: &str,
        expected_revision: u64,
    ) -> Result<RoutePinsDocument, RuleError> {
        let name = validate_list_name(name)?;
        let mut document = self.document.lock().await;
        ensure_revision(&document, expected_revision)?;
        let list = document
            .lists
            .iter_mut()
            .find(|list| list.id == list_id)
            .ok_or_else(|| RuleError::InvalidRule("unknown list".into()))?;
        list.name = name;
        document.revision = document.revision.saturating_add(1);
        publish(&self.path, &document)?;
        Ok(document.clone())
    }

    /// Deletes a list and every pin in it.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for an unknown list, a revision conflict, or a
    /// persistence failure.
    pub async fn delete_list(
        &self,
        list_id: Uuid,
        expected_revision: u64,
    ) -> Result<RoutePinsDocument, RuleError> {
        let mut document = self.document.lock().await;
        ensure_revision(&document, expected_revision)?;
        if document.list_meta(list_id).is_none() {
            return Err(RuleError::InvalidRule("unknown list".into()));
        }
        document.lists.retain(|list| list.id != list_id);
        document.pins.retain(|pin| pin.list_id != Some(list_id));
        document.revision = document.revision.saturating_add(1);
        publish(&self.path, &document)?;
        Ok(document.clone())
    }

    /// Re-binds a list (and every pin in it) to another outbound.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for an unknown list, a pin that the target
    /// policy rejects, a revision conflict, or a persistence failure.
    pub async fn set_list_outbound(
        &self,
        list_id: Uuid,
        outbound: Outbound,
        policy: PinPolicy,
        expected_revision: u64,
    ) -> Result<RoutePinsDocument, RuleError> {
        let mut document = self.document.lock().await;
        ensure_revision(&document, expected_revision)?;
        if document.list_meta(list_id).is_none() {
            return Err(RuleError::InvalidRule("unknown list".into()));
        }
        for pin in document
            .pins
            .iter()
            .filter(|pin| pin.list_id == Some(list_id))
        {
            if let DirectTarget::Ip(address) = &pin.target {
                reject_pin_address(*address, policy)?;
            }
        }
        for list in &mut document.lists {
            if list.id == list_id {
                list.outbound = outbound;
            }
        }
        for pin in &mut document.pins {
            if pin.list_id == Some(list_id) {
                pin.outbound = outbound;
            }
        }
        document.revision = document.revision.saturating_add(1);
        publish(&self.path, &document)?;
        Ok(document.clone())
    }

    /// Removes an exact domain or IP pin from whichever list holds it.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] for invalid input, a revision conflict, or an
    /// atomic persistence failure.
    pub async fn remove(
        &self,
        input: &str,
        expected_revision: u64,
    ) -> Result<DirectRulesDocument, RuleError> {
        let target = DirectTarget::parse(input)?;
        let mut document = self.document.lock().await;
        ensure_revision(&document, expected_revision)?;
        let before = document.pins.len();
        document.pins.retain(|pin| pin.target != target);
        if document.pins.len() != before {
            document.revision = document.revision.saturating_add(1);
            publish(&self.path, &document)?;
        }
        Ok(document.clone())
    }

    /// Refreshes resolved IP addresses for every stored domain rule.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] when DNS resolution or atomic persistence fails.
    pub async fn refresh(&self) -> Result<DirectRulesDocument, RuleError> {
        let domains = {
            let document = self.document.lock().await;
            document
                .pins
                .iter()
                .filter_map(|rule| match &rule.target {
                    DirectTarget::Domain(domain) => Some(domain.clone()),
                    DirectTarget::Ip(_) => None,
                })
                .collect::<Vec<_>>()
        };
        let mut resolved = Vec::with_capacity(domains.len());
        for domain in domains {
            resolved.push((domain.clone(), self.resolver.resolve(&domain).await?));
        }
        let now = Utc::now();
        let mut document = self.document.lock().await;
        for rule in &mut document.pins {
            if let DirectTarget::Domain(domain) = &rule.target {
                if let Some((_, addresses)) = resolved.iter().find(|(name, _)| name == domain) {
                    rule.resolved_ips.clone_from(addresses);
                    rule.refreshed_at = Some(now);
                }
            }
        }
        document.revision = document.revision.saturating_add(1);
        publish(&self.path, &document)?;
        Ok(document.clone())
    }

    /// Replaces the in-memory document and publishes it. Used to roll a failed
    /// live apply back to the last-good pins.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError`] when the document cannot be written atomically.
    pub async fn restore(
        &self,
        document: DirectRulesDocument,
    ) -> Result<DirectRulesDocument, RuleError> {
        let mut current = self.document.lock().await;
        *current = document;
        publish(&self.path, &current)?;
        Ok(current.clone())
    }
}

fn validate_list_name(name: &str) -> Result<String, RuleError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(RuleError::InvalidRule(
            "list name must be 1-60 characters".into(),
        ));
    }
    Ok(name.to_owned())
}

fn canonicalize_document(mut document: RoutePinsDocument) -> RoutePinsDocument {
    let before = document.clone();
    document.pins = merge_canonical_pins(document.pins);
    ensure_list_membership(&mut document);
    fill_google_search_companions(&mut document);
    if document.pins != before.pins || document.lists != before.lists {
        document.revision = document.revision.saturating_add(1);
    }
    document
}

fn fill_google_search_companions(document: &mut RoutePinsDocument) {
    let mut found = None;
    for pin in &document.pins {
        let DirectTarget::Domain(domain) = &pin.target else {
            continue;
        };
        if !expands_google_search_companions(domain) {
            continue;
        }
        if !matches!(pin.outbound, Outbound::Client { .. }) {
            continue;
        }
        found = Some((pin.outbound, pin.list_id));
        break;
    }
    let Some((outbound, existing_list)) = found else {
        return;
    };
    let list_id = existing_list.unwrap_or_else(|| document.default_list_for(outbound));
    attach_google_search_companions(
        document,
        &DirectTarget::Domain("google.com".into()),
        outbound,
        list_id,
    );
}

fn attach_google_search_companions(
    document: &mut RoutePinsDocument,
    target: &DirectTarget,
    outbound: Outbound,
    list_id: Uuid,
) -> bool {
    if !matches!(outbound, Outbound::Client { .. }) {
        return false;
    }
    let DirectTarget::Domain(domain) = target else {
        return false;
    };
    if !expands_google_search_companions(domain) {
        return false;
    }
    let now = Utc::now();
    let mut added = false;
    for companion in GOOGLE_SEARCH_COMPANION_DOMAINS {
        let companion_target = DirectTarget::Domain((*companion).into());
        if document
            .pins
            .iter()
            .any(|pin| pin.target == companion_target)
        {
            continue;
        }
        document.pins.push(PinnedRoute {
            target: companion_target,
            outbound,
            list_id: Some(list_id),
            resolved_ips: Vec::new(),
            created_at: now,
            refreshed_at: Some(now),
        });
        added = true;
    }
    if added {
        document.pins.sort_by_key(|pin| pin.target.display_value());
    }
    added
}

fn merge_canonical_pins(pins: Vec<PinnedRoute>) -> Vec<PinnedRoute> {
    let mut merged: Vec<PinnedRoute> = Vec::new();
    for pin in pins {
        let target = match &pin.target {
            DirectTarget::Domain(domain) => {
                canonical_target(domain).unwrap_or_else(|_| pin.target.clone())
            }
            DirectTarget::Ip(_) => pin.target.clone(),
        };
        if let Some(existing) = merged.iter_mut().find(|item| item.target == target) {
            existing.outbound = pin.outbound;
            existing.list_id = pin.list_id;
            if pin.created_at < existing.created_at {
                existing.created_at = pin.created_at;
            }
            continue;
        }
        merged.push(PinnedRoute {
            target,
            outbound: pin.outbound,
            list_id: pin.list_id,
            resolved_ips: Vec::new(),
            created_at: pin.created_at,
            refreshed_at: pin.refreshed_at,
        });
    }
    merged.sort_by_key(|pin| pin.target.display_value());
    merged
}

#[derive(Debug, Deserialize)]
struct RawPinsDocument {
    revision: u64,
    #[serde(default)]
    pins: Vec<PinnedRoute>,
    #[serde(default)]
    lists: Vec<RuleListMeta>,
    #[serde(default)]
    rules: Vec<DirectRule>,
    #[serde(default)]
    vpn_rules: Vec<DirectRule>,
    #[serde(default)]
    openvpn_rules: Vec<DirectRule>,
}

fn decode_pins_document(
    bytes: &[u8],
    legacy: LegacyPinClients,
) -> Result<RoutePinsDocument, RuleError> {
    let raw: RawPinsDocument = serde_json::from_slice(bytes)?;
    if !raw.pins.is_empty()
        || (raw.rules.is_empty() && raw.vpn_rules.is_empty() && raw.openvpn_rules.is_empty())
    {
        return Ok(RoutePinsDocument {
            revision: raw.revision,
            pins: raw.pins,
            lists: raw.lists,
        });
    }
    let mut pins = Vec::new();
    for rule in raw.rules {
        pins.push(pin_from_legacy(rule, Outbound::Direct));
    }
    if let Some(id) = legacy.vpn {
        for rule in raw.vpn_rules {
            pins.push(pin_from_legacy(rule, Outbound::client(id)));
        }
    }
    if let Some(id) = legacy.openvpn {
        for rule in raw.openvpn_rules {
            pins.push(pin_from_legacy(rule, Outbound::client(id)));
        }
    }
    Ok(RoutePinsDocument {
        revision: raw.revision,
        pins,
        lists: raw.lists,
    })
}

fn pin_from_legacy(rule: DirectRule, outbound: Outbound) -> PinnedRoute {
    PinnedRoute {
        target: rule.target,
        outbound,
        list_id: None,
        resolved_ips: rule.resolved_ips,
        created_at: rule.created_at,
        refreshed_at: rule.refreshed_at,
    }
}

fn reject_pin_address(address: IpAddr, policy: PinPolicy) -> Result<(), RuleError> {
    if matches!(policy, PinPolicy::LocalProxy) && is_private_or_local(address) {
        return Err(RuleError::InvalidRule(
            "private, loopback, and carrier-grade NAT addresses cannot be sent through a local proxy".into(),
        ));
    }
    if matches!(policy, PinPolicy::OwnedSideTunnel) && is_loopback_only(address) {
        return Err(RuleError::InvalidRule(
            "loopback addresses cannot be sent through a side tunnel".into(),
        ));
    }
    Ok(())
}

fn is_loopback_only(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_loopback(),
        IpAddr::V6(address) => address.is_loopback(),
    }
}

fn backup_last_good(path: &Path) -> Result<(), RuleError> {
    let backup = path.with_extension("json.last-good");
    fs::copy(path, backup)?;
    Ok(())
}

fn ensure_revision(
    document: &DirectRulesDocument,
    expected_revision: u64,
) -> Result<(), RuleError> {
    if document.revision != expected_revision {
        return Err(RuleError::RevisionConflict {
            expected: expected_revision,
            actual: document.revision,
        });
    }
    Ok(())
}

fn publish(path: &Path, document: &DirectRulesDocument) -> Result<(), RuleError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(document)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outbound {
    Direct,
    Client { client_id: ClientId },
}

impl Outbound {
    #[must_use]
    pub const fn client(client_id: ClientId) -> Self {
        Self::Client { client_id }
    }

    #[must_use]
    pub const fn client_id(self) -> Option<ClientId> {
        match self {
            Self::Direct => None,
            Self::Client { client_id } => Some(client_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    VpnRule,
    CustomRule,
    PrivateOrLocal,
    IranDomain,
    IranCidr,
    DefaultProxy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecision {
    pub outbound: Outbound,
    pub reason: DecisionReason,
    pub matched_rule: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    client_domains: HashMap<ClientId, HashSet<String>>,
    client_ips: HashMap<ClientId, HashSet<IpAddr>>,
    custom_domains: HashSet<String>,
    custom_ips: HashSet<IpAddr>,
    iran_domains: HashSet<String>,
    business_domains: HashSet<String>,
    iran_cidrs: Vec<IpNet>,
    default_outbound: Outbound,
}

impl Default for Outbound {
    fn default() -> Self {
        Self::Direct
    }
}

impl RuleSet {
    pub fn from_sources(
        custom: &RoutePinsDocument,
        iran_domains: impl IntoIterator<Item = String>,
        iran_cidrs: impl IntoIterator<Item = IpNet>,
        business_domains: impl IntoIterator<Item = String>,
        default_outbound: Outbound,
        enabled_clients: &HashSet<ClientId>,
    ) -> Self {
        let mut set = Self {
            iran_domains: iran_domains.into_iter().collect(),
            business_domains: business_domains.into_iter().collect(),
            iran_cidrs: iran_cidrs.into_iter().collect(),
            default_outbound,
            ..Self::default()
        };
        for pin in &custom.pins {
            match pin.outbound {
                Outbound::Direct => match &pin.target {
                    DirectTarget::Domain(domain) => {
                        set.custom_domains.insert(domain.clone());
                    }
                    DirectTarget::Ip(address) => {
                        set.custom_ips.insert(*address);
                    }
                },
                Outbound::Client { client_id } if enabled_clients.contains(&client_id) => {
                    match &pin.target {
                        DirectTarget::Domain(domain) => {
                            set.client_domains
                                .entry(client_id)
                                .or_default()
                                .insert(domain.clone());
                        }
                        DirectTarget::Ip(address) => {
                            set.client_ips
                                .entry(client_id)
                                .or_default()
                                .insert(*address);
                        }
                    }
                }
                Outbound::Client { .. } => {}
            }
        }
        set
    }

    /// Chooses the outbound route for an exact domain or IP target.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError::InvalidRule`] when `target` is neither a valid IP
    /// address nor a normalized domain.
    pub fn decide(&self, target: &str) -> Result<RouteDecision, RuleError> {
        if let Ok(address) = target.parse::<IpAddr>() {
            return Ok(self.decide_ip(address));
        }
        let domain = normalize_domain(target)?;
        let canonical = registrable_domain(&domain).unwrap_or_else(|_| domain.clone());
        // The most specific matching pin wins across every list, so a
        // `developer.google.com` pin overrides a `google.com` pin even when
        // they route to different outbounds.
        if let Some((outbound, pin)) = self.best_pin_match(&domain) {
            let reason = match outbound {
                Outbound::Direct => DecisionReason::CustomRule,
                Outbound::Client { .. } => DecisionReason::VpnRule,
            };
            return Ok(RouteDecision {
                outbound,
                reason,
                matched_rule: Some(pin),
            });
        }
        if let Some(rule) = self
            .iran_domains
            .iter()
            .find(|rule| domain == **rule || domain.ends_with(&format!(".{rule}")))
        {
            return Ok(direct(DecisionReason::IranDomain, Some(rule.clone())));
        }
        if let Some(rule) = self
            .business_domains
            .iter()
            .find(|pin| domain_matches_pin(&domain, pin) || *pin == &canonical)
        {
            return Ok(direct(DecisionReason::IranDomain, Some(rule.clone())));
        }
        Ok(RouteDecision {
            outbound: self.default_outbound,
            reason: DecisionReason::DefaultProxy,
            matched_rule: Some("MATCH".into()),
        })
    }

    fn best_pin_match(&self, domain: &str) -> Option<(Outbound, String)> {
        let mut best: Option<(usize, Outbound, String)> = None;
        let mut consider = |outbound: Outbound, pin: &str| {
            if domain_matches_pin(domain, pin) {
                let score = domain_specificity(pin);
                if best.as_ref().is_none_or(|(current, _, _)| score > *current) {
                    best = Some((score, outbound, pin.to_owned()));
                }
            }
        };
        for (client_id, pins) in &self.client_domains {
            for pin in pins {
                consider(Outbound::client(*client_id), pin);
            }
        }
        for pin in &self.custom_domains {
            consider(Outbound::Direct, pin);
        }
        best.map(|(_, outbound, pin)| (outbound, pin))
    }

    fn decide_ip(&self, address: IpAddr) -> RouteDecision {
        // Loopback and LAN stay direct even under an exclusion; the generated
        // config keeps private-networks ahead of the client rule sets too.
        if is_private_or_local(address) {
            return direct(DecisionReason::PrivateOrLocal, Some(address.to_string()));
        }
        if let Some(client_id) = self
            .client_ips
            .iter()
            .find_map(|(id, pins)| pins.contains(&address).then_some(*id))
        {
            return RouteDecision {
                outbound: Outbound::client(client_id),
                reason: DecisionReason::VpnRule,
                matched_rule: Some(address.to_string()),
            };
        }
        if self.custom_ips.contains(&address) {
            return direct(DecisionReason::CustomRule, Some(address.to_string()));
        }
        if let Some(network) = self
            .iran_cidrs
            .iter()
            .find(|network| network.contains(&address))
        {
            return direct(DecisionReason::IranCidr, Some(network.to_string()));
        }
        RouteDecision {
            outbound: self.default_outbound,
            reason: DecisionReason::DefaultProxy,
            matched_rule: Some("MATCH".into()),
        }
    }
}

fn direct(reason: DecisionReason, matched_rule: Option<String>) -> RouteDecision {
    RouteDecision {
        outbound: Outbound::Direct,
        reason,
        matched_rule,
    }
}

fn is_private_or_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || u32::from(address) & 0xffc0_0000 == u32::from_be_bytes([100, 64, 0, 0])
        }
        IpAddr::V6(address) => {
            address.is_loopback() || address.is_unique_local() || address.is_unicast_link_local()
        }
    }
}

#[allow(dead_code)]
fn _socket_address_example(address: SocketAddr) -> IpAddr {
    address.ip()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FixedResolver;

    #[async_trait]
    impl Resolver for FixedResolver {
        async fn resolve(&self, _domain: &str) -> Result<Vec<IpAddr>, RuleError> {
            Ok(vec!["203.0.113.9".parse().expect("IP")])
        }
    }

    #[test]
    fn normalizes_unicode_and_rejects_urls() {
        assert_eq!(
            normalize_domain("Example.COM.").expect("domain"),
            "example.com"
        );
        assert!(normalize_domain("https://example.com/path").is_err());
        assert!(normalize_domain("*.example.com").is_err());
    }

    fn test_client() -> ClientId {
        ClientId::parse("11111111-1111-1111-1111-111111111111").expect("uuid")
    }

    fn test_outbound() -> Outbound {
        Outbound::client(test_client())
    }

    fn enabled_clients() -> HashSet<ClientId> {
        HashSet::from([test_client()])
    }

    fn iran_rule_set(custom: &RoutePinsDocument) -> RuleSet {
        RuleSet::from_sources(
            custom,
            ["ir".to_owned()],
            [],
            [],
            test_outbound(),
            &enabled_clients(),
        )
    }

    #[tokio::test]
    async fn an_unreadable_document_is_quarantined_instead_of_blocking_startup() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("direct-rules.json");
        fs::write(&path, b"{ not json at all").expect("write");
        let manager = RuleManager::load(&path, Arc::new(FixedResolver)).expect("load starts empty");
        let document = manager.list().await;
        assert!(document.pins.is_empty());
        assert!(directory.path().join("direct-rules.json.corrupt").exists());
        // The published replacement parses cleanly on the next load.
        RuleManager::load(&path, Arc::new(FixedResolver)).expect("reload");
    }

    #[tokio::test]
    async fn named_lists_cover_create_pin_rebind_and_delete() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");

        let document = manager
            .create_list("Office", test_outbound(), 0)
            .await
            .expect("create");
        let list_id = document.lists[0].id;
        assert_eq!(document.lists[0].name, "Office");

        let document = manager
            .pin_to_list(
                "office.example",
                list_id,
                PinPolicy::LocalProxy,
                document.revision,
            )
            .await
            .expect("pin to list");
        assert_eq!(document.pins_in_list(list_id).len(), 1);
        assert_eq!(document.pins[0].outbound, test_outbound());

        // Re-binding the list moves its pins with it.
        let document = manager
            .set_list_outbound(
                list_id,
                Outbound::Direct,
                PinPolicy::Direct,
                document.revision,
            )
            .await
            .expect("rebind");
        assert_eq!(document.pins[0].outbound, Outbound::Direct);
        assert_eq!(
            document.list_meta(list_id).expect("list").outbound,
            Outbound::Direct
        );

        // Unnamed pins land in an auto-created default list per outbound.
        let document = manager
            .pin_with_policy(
                "auto.example",
                test_outbound(),
                PinPolicy::LocalProxy,
                document.revision,
            )
            .await
            .expect("pin default");
        let auto = document
            .pins
            .iter()
            .find(|pin| pin.target.display_value() == "auto.example")
            .expect("pin");
        let auto_list = auto.list_id.expect("list id");
        assert_ne!(auto_list, list_id);
        assert_eq!(
            document.list_meta(auto_list).expect("auto list").outbound,
            test_outbound()
        );

        // Deleting a list drops its pins only.
        let document = manager
            .delete_list(list_id, document.revision)
            .await
            .expect("delete");
        assert!(document.list_meta(list_id).is_none());
        assert_eq!(document.pins.len(), 1);
        assert_eq!(document.pins[0].target.display_value(), "auto.example");

        // Empty and oversized names are rejected.
        assert!(manager
            .create_list("   ", Outbound::Direct, document.revision)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_vpn_pin_overrides_the_bundled_iran_list_and_survives_removal() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");

        // iran.ir is DIRECT only because the bundled list carries `ir`.
        let decision = iran_rule_set(&manager.list().await)
            .decide("iran.ir")
            .expect("decide");
        assert_eq!(decision.outbound, Outbound::Direct);
        assert_eq!(decision.reason, DecisionReason::IranDomain);

        let pinned = manager
            .pin("iran.ir", test_outbound(), 0)
            .await
            .expect("pin vpn");
        assert_eq!(pinned.count_for_client(test_client()), 1);
        let decision = iran_rule_set(&pinned).decide("iran.ir").expect("decide");
        assert_eq!(decision.outbound, test_outbound());
        assert_eq!(decision.reason, DecisionReason::VpnRule);

        // Removing the pin must restore the bundled decision, not delete `ir`.
        let cleared = manager
            .remove("iran.ir", pinned.revision)
            .await
            .expect("remove");
        assert_eq!(cleared.count_for_client(test_client()), 0);
        assert_eq!(
            iran_rule_set(&cleared)
                .decide("iran.ir")
                .expect("decide")
                .reason,
            DecisionReason::IranDomain
        );
    }

    #[tokio::test]
    async fn a_host_lives_in_exactly_one_user_list() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");

        let direct = manager.add("example.com", 0).await.expect("direct");
        assert_eq!(direct.pins_for(&Outbound::Direct).len(), 1);
        assert_eq!(
            iran_rule_set(&direct)
                .decide("example.com")
                .expect("decide")
                .reason,
            DecisionReason::CustomRule
        );

        let moved = manager
            .pin("example.com", test_outbound(), direct.revision)
            .await
            .expect("vpn");
        assert!(
            moved.pins_for(&Outbound::Direct).is_empty(),
            "the direct pin must be dropped"
        );
        assert_eq!(moved.count_for_client(test_client()), 1);

        let back = manager
            .pin("example.com", Outbound::Direct, moved.revision)
            .await
            .expect("direct again");
        assert_eq!(back.pins_for(&Outbound::Direct).len(), 1);
        assert_eq!(back.count_for_client(test_client()), 0);
    }

    #[tokio::test]
    async fn private_and_loopback_addresses_cannot_be_pinned_to_the_vpn() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");

        // The rejection happens before the revision check, so revision 0 holds
        // for every address here.
        for address in ["192.168.1.1", "127.0.0.1", "100.64.0.1", "::1"] {
            let error = manager
                .pin(address, test_outbound(), 0)
                .await
                .expect_err(address);
            assert!(matches!(error, RuleError::InvalidRule(_)), "{address}");
        }
        // The same address is still allowed on the direct list.
        let pinned = manager
            .pin("192.168.1.1", Outbound::Direct, 0)
            .await
            .expect("direct");
        assert_eq!(pinned.pins_for(&Outbound::Direct).len(), 1);
    }

    #[tokio::test]
    async fn refresh_resolves_domains_on_both_lists() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        let direct = manager.add("direct.example", 0).await.expect("direct");
        let pinned = manager
            .pin("vpn.example", test_outbound(), direct.revision)
            .await
            .expect("vpn");

        let refreshed = manager.refresh().await.expect("refresh");
        assert_eq!(refreshed.pins_for(&Outbound::Direct).len(), 1);
        assert_eq!(refreshed.count_for_client(test_client()), 1);
        for rule in &refreshed.pins {
            assert!(rule.refreshed_at.is_some());
            assert_eq!(
                rule.resolved_ips,
                vec!["203.0.113.9".parse::<IpAddr>().expect("ip")]
            );
        }
        let _ = pinned;
    }

    #[tokio::test]
    async fn mutations_are_revisioned_and_atomic() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        let added = manager.add("example.com", 0).await.expect("add");
        assert_eq!(added.revision, 1);
        assert!(added.pins[0].resolved_ips.is_empty());
        assert!(manager.remove("example.com", 0).await.is_err());
        let removed = manager.remove("example.com", 1).await.expect("remove");
        assert!(removed.pins.is_empty());
    }

    #[test]
    fn precedence_is_custom_then_private_then_iran_then_proxy() {
        let custom = RoutePinsDocument {
            revision: 1,
            pins: vec![PinnedRoute {
                target: DirectTarget::Domain("example.com".into()),
                outbound: Outbound::Direct,
                list_id: None,
                resolved_ips: vec![],
                created_at: Utc::now(),
                refreshed_at: None,
            }],
            lists: vec![],
        };
        let set = RuleSet::from_sources(
            &custom,
            ["digikala.com".into()],
            ["5.22.0.0/16".parse().expect("CIDR")],
            ["technolife.com".into()],
            test_outbound(),
            &enabled_clients(),
        );
        assert_eq!(
            set.decide("www.technolife.com").expect("catalog").reason,
            DecisionReason::IranDomain
        );
        assert_eq!(
            set.decide("www.technolife.com")
                .expect("catalog")
                .matched_rule
                .as_deref(),
            Some("technolife.com")
        );
        assert_eq!(
            set.decide("example.com").expect("decision").reason,
            DecisionReason::CustomRule
        );
        assert_eq!(
            set.decide("192.168.1.1").expect("decision").reason,
            DecisionReason::PrivateOrLocal
        );
        assert_eq!(
            set.decide("100.64.1.1").expect("decision").reason,
            DecisionReason::PrivateOrLocal
        );
        assert_eq!(
            set.decide("cdn.digikala.com").expect("decision").reason,
            DecisionReason::IranDomain
        );
        assert_eq!(
            set.decide("5.22.12.1").expect("decision").reason,
            DecisionReason::IranCidr
        );
        assert_eq!(
            set.decide("openai.com").expect("decision").outbound,
            test_outbound()
        );
    }

    #[test]
    fn bundled_catalog_sends_kavenegar_console_direct() {
        let catalog = include_str!("../../../resources/rules/iran-business-domains.txt")
            .lines()
            .filter_map(|line| line.strip_prefix("+.").map(str::to_owned));
        let set = RuleSet::from_sources(
            &RoutePinsDocument::default(),
            [],
            [],
            catalog,
            test_outbound(),
            &enabled_clients(),
        );
        let decision = set.decide("console.kavenegar.com").expect("decide");
        assert_eq!(decision.outbound, Outbound::Direct);
        assert_eq!(decision.reason, DecisionReason::IranDomain);
        assert_eq!(decision.matched_rule.as_deref(), Some("kavenegar.com"));
        let arzinja = set.decide("www.arzinja.info").expect("decide");
        assert_eq!(arzinja.outbound, Outbound::Direct);
        assert_eq!(arzinja.reason, DecisionReason::IranDomain);
        assert_eq!(arzinja.matched_rule.as_deref(), Some("arzinja.info"));
        let ketabrah = set.decide("www.ketabrah.com").expect("decide");
        assert_eq!(ketabrah.outbound, Outbound::Direct);
        assert_eq!(ketabrah.reason, DecisionReason::IranDomain);
        assert_eq!(ketabrah.matched_rule.as_deref(), Some("ketabrah.com"));
    }

    #[test]
    fn curated_cdn_cidrs_are_direct_and_client_ip_pins_win() {
        let cidrs = include_str!("../../../resources/rules/iran-cdn-networks.txt")
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::parse)
            .collect::<Result<Vec<_>, _>>()
            .expect("cidrs");
        let custom = RoutePinsDocument {
            revision: 1,
            pins: vec![PinnedRoute {
                target: DirectTarget::Ip("185.163.216.9".parse().expect("ip")),
                outbound: test_outbound(),
                list_id: None,
                resolved_ips: vec![],
                created_at: Utc::now(),
                refreshed_at: None,
            }],
            lists: vec![],
        };
        let set =
            RuleSet::from_sources(&custom, [], cidrs, [], test_outbound(), &enabled_clients());
        let covered = set.decide("185.172.72.10").expect("decide");
        assert_eq!(covered.outbound, Outbound::Direct);
        assert_eq!(covered.reason, DecisionReason::IranCidr);
        let pinned = set.decide("185.163.216.9").expect("pin");
        assert_eq!(pinned.outbound, test_outbound());
        assert_eq!(pinned.reason, DecisionReason::VpnRule);
    }

    #[tokio::test]
    async fn pinning_google_com_to_a_client_adds_search_companion_roots() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        let pinned = manager
            .pin("google.com", test_outbound(), 0)
            .await
            .expect("pin");
        let names: Vec<_> = pinned
            .pins
            .iter()
            .filter_map(|pin| match &pin.target {
                DirectTarget::Domain(domain) => Some(domain.as_str()),
                DirectTarget::Ip(_) => None,
            })
            .collect();
        assert!(names.contains(&"google.com"));
        for companion in GOOGLE_SEARCH_COMPANION_DOMAINS {
            assert!(names.contains(companion), "{companion}");
        }
        let set = iran_rule_set(&pinned);
        for host in [
            "www.gstatic.com",
            "ssl.gstatic.com",
            "www.googleapis.com",
            "lh3.googleusercontent.com",
            "www.googletagmanager.com",
        ] {
            let decision = set.decide(host).expect("decide");
            assert_eq!(decision.outbound, test_outbound(), "{host}");
        }
    }

    #[tokio::test]
    async fn pinning_google_com_direct_does_not_add_search_companions() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        let pinned = manager.add("google.com", 0).await.expect("add");
        assert_eq!(pinned.pins.len(), 1);
        assert_eq!(
            pinned.pins[0].target,
            DirectTarget::Domain("google.com".into())
        );
    }

    #[tokio::test]
    async fn google_search_companions_do_not_steal_an_existing_direct_pin() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        let added = manager.add("gstatic.com", 0).await.expect("direct gstatic");
        let pinned = manager
            .pin("google.com", test_outbound(), added.revision)
            .await
            .expect("pin google");
        let gstatic = pinned
            .pins
            .iter()
            .find(|pin| pin.target == DirectTarget::Domain("gstatic.com".into()))
            .expect("gstatic");
        assert_eq!(gstatic.outbound, Outbound::Direct);
        let set = iran_rule_set(&pinned);
        assert_eq!(
            set.decide("www.gstatic.com").expect("decide").outbound,
            Outbound::Direct
        );
    }

    #[tokio::test]
    async fn load_fills_google_search_companions_for_a_client_google_pin() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("direct-rules.json");
        let client = test_outbound();
        let document = RoutePinsDocument {
            revision: 1,
            pins: vec![PinnedRoute {
                target: DirectTarget::Domain("google.com".into()),
                outbound: client,
                list_id: None,
                resolved_ips: vec![],
                created_at: Utc::now(),
                refreshed_at: None,
            }],
            lists: vec![],
        };
        fs::write(&path, serde_json::to_vec_pretty(&document).expect("json")).expect("write");
        let manager = RuleManager::load(path, Arc::new(FixedResolver)).expect("load");
        let loaded = manager.list().await;
        for companion in GOOGLE_SEARCH_COMPANION_DOMAINS {
            assert!(
                loaded.pins.iter().any(|pin| {
                    pin.target == DirectTarget::Domain((*companion).into())
                        && pin.outbound == client
                }),
                "{companion}"
            );
        }
    }

    #[test]
    fn a_client_google_com_pin_covers_every_subdomain() {
        let custom = RoutePinsDocument {
            revision: 1,
            pins: vec![PinnedRoute {
                target: DirectTarget::Domain("google.com".into()),
                outbound: test_outbound(),
                list_id: None,
                resolved_ips: vec![],
                created_at: chrono::Utc::now(),
                refreshed_at: None,
            }],
            lists: vec![],
        };
        let set = iran_rule_set(&custom);
        for host in [
            "google.com",
            "www.google.com",
            "gemini.google.com",
            "accounts.google.com",
        ] {
            let decision = set.decide(host).expect("decide");
            assert_eq!(decision.outbound, test_outbound(), "{host}");
            assert_eq!(decision.matched_rule.as_deref(), Some("google.com"));
        }
        assert_eq!(
            set.decide("notgoogle.com")
                .expect("sibling")
                .matched_rule
                .as_deref(),
            Some("MATCH")
        );
    }

    #[tokio::test]
    async fn a_subdomain_pin_beats_a_root_pin_in_another_list() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        // Root pin to DIRECT: covers itself and every subdomain.
        let added = manager.add("google.com", 0).await.expect("add root");
        // A more specific subdomain pin routed to the client.
        let added = manager
            .pin("developer.google.com", test_outbound(), added.revision)
            .await
            .expect("pin subdomain");
        assert_eq!(added.pins.len(), 2);
        let set = iran_rule_set(&added);
        // The wildcard covers unpinned subdomains…
        assert_eq!(
            set.decide("gemini.google.com").expect("sub").outbound,
            Outbound::Direct
        );
        // …but the longer pin wins for its own subtree.
        assert_eq!(
            set.decide("developer.google.com").expect("exact").outbound,
            test_outbound()
        );
        assert_eq!(
            set.decide("api.developer.google.com")
                .expect("nested")
                .outbound,
            test_outbound()
        );
        assert_eq!(
            set.decide("notgoogle.com").expect("sibling").outbound,
            test_outbound()
        );
    }

    #[tokio::test]
    async fn a_subdomain_pin_stays_exact_and_covers_its_own_subtree() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        let added = manager.add("api.shop.example.com", 0).await.expect("add");
        assert_eq!(added.pins.len(), 1);
        assert_eq!(
            added.pins[0].target,
            DirectTarget::Domain("api.shop.example.com".into())
        );
        let set = iran_rule_set(&added);
        assert_eq!(
            set.decide("api.shop.example.com").expect("exact").reason,
            DecisionReason::CustomRule
        );
        assert_eq!(
            set.decide("v2.api.shop.example.com")
                .expect("nested")
                .reason,
            DecisionReason::CustomRule
        );
        // Siblings and the bare root are NOT covered by a subdomain pin.
        assert_eq!(
            set.decide("www.example.com").expect("sibling").outbound,
            test_outbound()
        );
        let moved = manager
            .pin("api.shop.example.com", test_outbound(), added.revision)
            .await
            .expect("move");
        assert!(moved.pins_for(&Outbound::Direct).is_empty());
        assert_eq!(moved.count_for_client(test_client()), 1);
        assert_eq!(
            iran_rule_set(&moved)
                .decide("v2.api.shop.example.com")
                .expect("cdn")
                .outbound,
            test_outbound()
        );
    }

    #[tokio::test]
    async fn a_private_suffix_tenant_does_not_pin_the_whole_suffix() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        let added = manager.add("user.github.io", 0).await.expect("pages");
        let set = iran_rule_set(&added);
        assert_eq!(
            set.decide("user.github.io").expect("self").reason,
            DecisionReason::CustomRule
        );
        assert_eq!(
            set.decide("other.github.io").expect("other").outbound,
            test_outbound()
        );
    }

    #[tokio::test]
    async fn pin_does_not_wait_for_dns() {
        struct SlowResolver;
        #[async_trait]
        impl Resolver for SlowResolver {
            async fn resolve(&self, _domain: &str) -> Result<Vec<IpAddr>, RuleError> {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Ok(Vec::new())
            }
        }
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(SlowResolver),
        )
        .expect("manager");
        let started = std::time::Instant::now();
        manager.add("example.com", 0).await.expect("add");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "pin must not wait on DNS"
        );
    }

    #[tokio::test]
    async fn load_keeps_exact_hosts_and_assigns_list_membership() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("direct-rules.json");
        let original = serde_json::json!({
            "revision": 4,
            "rules": [
                {
                    "target": { "kind": "domain", "value": "www.example.com" },
                    "resolved_ips": ["203.0.113.9"],
                    "created_at": Utc::now(),
                    "refreshed_at": null
                },
                {
                    "target": { "kind": "domain", "value": "api.example.com" },
                    "resolved_ips": [],
                    "created_at": Utc::now(),
                    "refreshed_at": null
                }
            ],
            "vpn_rules": []
        });
        fs::write(&path, serde_json::to_vec_pretty(&original).expect("json")).expect("write");
        let manager = RuleManager::load(&path, Arc::new(FixedResolver)).expect("load");
        let loaded = manager.list().await;
        assert_eq!(loaded.pins.len(), 2);
        assert_eq!(
            loaded.pins[0].target,
            DirectTarget::Domain("api.example.com".into())
        );
        assert_eq!(
            loaded.pins[1].target,
            DirectTarget::Domain("www.example.com".into())
        );
        assert!(loaded.pins.iter().all(
            |pin| pin.list_id.is_some() && loaded.list_meta(pin.list_id.expect("id")).is_some()
        ));
        assert_eq!(loaded.revision, 5);
        assert!(path.with_extension("json.last-good").is_file());
    }

    #[tokio::test]
    async fn legacy_vpn_rules_migrate_onto_the_supplied_client() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("direct-rules.json");
        let original = serde_json::json!({
            "revision": 2,
            "rules": [{
                "target": { "kind": "domain", "value": "direct.example" },
                "resolved_ips": [],
                "created_at": Utc::now(),
                "refreshed_at": null
            }],
            "vpn_rules": [{
                "target": { "kind": "domain", "value": "vpn.example" },
                "resolved_ips": [],
                "created_at": Utc::now(),
                "refreshed_at": null
            }]
        });
        fs::write(&path, serde_json::to_vec_pretty(&original).expect("json")).expect("write");
        let manager = RuleManager::load_with_legacy(
            &path,
            Arc::new(FixedResolver),
            LegacyPinClients {
                vpn: Some(test_client()),
                openvpn: None,
            },
        )
        .expect("load");
        let loaded = manager.list().await;
        assert_eq!(loaded.pins_for(&Outbound::Direct).len(), 1);
        assert_eq!(loaded.count_for_client(test_client()), 1);
    }

    #[test]
    fn match_follows_a_direct_default_route() {
        let set = RuleSet::from_sources(
            &RoutePinsDocument::default(),
            [],
            [],
            [],
            Outbound::Direct,
            &HashSet::new(),
        );
        assert_eq!(
            set.decide("openai.com").expect("decide").outbound,
            Outbound::Direct
        );
    }

    #[test]
    fn disabled_client_pins_are_not_decided() {
        let other = ClientId::parse("22222222-2222-2222-2222-222222222222").expect("uuid");
        let custom = RoutePinsDocument {
            revision: 1,
            pins: vec![PinnedRoute {
                target: DirectTarget::Domain("office.example".into()),
                outbound: Outbound::client(other),
                list_id: None,
                resolved_ips: vec![],
                created_at: Utc::now(),
                refreshed_at: None,
            }],
            lists: vec![],
        };
        let set = RuleSet::from_sources(&custom, [], [], [], test_outbound(), &enabled_clients());
        assert_eq!(
            set.decide("office.example").expect("decide").outbound,
            test_outbound()
        );
    }

    #[test]
    fn pin_moves_across_clients_and_delete_drops_that_clients_pins() {
        let mut document = RoutePinsDocument {
            revision: 1,
            pins: vec![PinnedRoute {
                target: DirectTarget::Domain("office.example".into()),
                outbound: Outbound::client(test_client()),
                list_id: None,
                resolved_ips: vec![],
                created_at: Utc::now(),
                refreshed_at: None,
            }],
            lists: vec![],
        };
        let other = ClientId::parse("22222222-2222-2222-2222-222222222222").expect("uuid");
        assert_eq!(
            document.move_client_pins(test_client(), Outbound::client(other)),
            1
        );
        assert_eq!(document.count_for_client(test_client()), 0);
        assert_eq!(document.count_for_client(other), 1);
        assert_eq!(document.delete_client_pins(other), 1);
        assert!(document.pins.is_empty());
    }

    #[tokio::test]
    async fn rfc1918_may_go_to_a_side_tunnel_but_not_a_local_proxy() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = RuleManager::load(
            directory.path().join("direct-rules.json"),
            Arc::new(FixedResolver),
        )
        .expect("manager");
        manager
            .pin_with_policy("192.168.10.5", test_outbound(), PinPolicy::LocalProxy, 0)
            .await
            .expect_err("proxy");
        let pinned = manager
            .pin_with_policy(
                "192.168.10.5",
                test_outbound(),
                PinPolicy::OwnedSideTunnel,
                0,
            )
            .await
            .expect("tunnel");
        assert_eq!(pinned.count_for_client(test_client()), 1);
        manager
            .pin_with_policy(
                "127.0.0.1",
                test_outbound(),
                PinPolicy::OwnedSideTunnel,
                pinned.revision,
            )
            .await
            .expect_err("loopback");
    }
}
