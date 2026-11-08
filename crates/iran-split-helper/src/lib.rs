mod openvpn;

use iran_split_ipc::{CleanupReport, ProcessStatus, ServiceLogEntry};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs, io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, LazyLock,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};
use uuid::Uuid;

const PROCESS_STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// Mihomo removes its own routes, nftables, and TUN on `SIGTERM`. Killing it
/// immediately leaves those rules in place and the network stays black-holed.
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(2);
/// The desktop health loop talks to the helper every 10 seconds. A longer
/// silence means the app crashed or was killed, so the capture must come down.
const DESKTOP_ABSENCE_LIMIT: Duration = Duration::from_secs(90);
const MIHOMO_STAY_ALIVE: Duration = Duration::from_millis(400);
const MAX_LOG_ENTRIES: usize = 2_000;
const FIXED_GENERATION_FILES: [&str; 8] = [
    "config.yaml",
    "private.txt",
    "iran-domains.txt",
    "iran-business-domains.txt",
    "iran-networks.txt",
    "iran-cdn-networks.txt",
    "custom-direct-domains.txt",
    "custom-direct-ips.txt",
];

fn is_allowed_generation_file(name: &str) -> bool {
    FIXED_GENERATION_FILES.contains(&name)
        || name == "custom-vpn-domains.txt"
        || name == "custom-vpn-ips.txt"
        || iran_split_config::is_custom_client_generation_file(name)
}

#[derive(Debug, Error)]
pub enum HelperServiceError {
    #[error("helper I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("helper configuration is invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("helper configuration is unsafe: {0}")]
    UnsafeConfig(String),
    #[error("runtime generation is invalid: {0}")]
    InvalidGeneration(String),
    #[error("Mihomo binary integrity check failed")]
    BinaryIntegrity,
    #[error("Mihomo process failed: {0}")]
    Process(String),
    /// Shown verbatim in the install dialog, so it carries no prefix of its
    /// own: the desktop already frames it as an installation failure, and
    /// `Process` would blame Mihomo for a Task Scheduler problem.
    #[error("{0}")]
    Install(String),
    #[error("IPC failed: {0}")]
    Protocol(#[from] iran_split_ipc::ProtocolError),
    #[error("side tunnel failed: {0}")]
    SideTunnel(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelperSettings {
    pub authorized_uid: u32,
    #[serde(default)]
    pub authorized_gid: u32,
    pub socket_path: PathBuf,
    pub staging_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub mihomo_binary: PathBuf,
    pub mihomo_sha256: String,
    pub tun_name: String,
}

impl HelperSettings {
    /// Loads and validates a root-owned helper configuration file.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is unsafe, its contents cannot be read or
    /// decoded, or a setting violates the helper's security constraints.
    pub fn load(path: &Path) -> Result<Self, HelperServiceError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(HelperServiceError::UnsafeConfig(
                "helper config must be a regular, non-symlink file".into(),
            ));
        }
        #[cfg(unix)]
        validate_root_owned_private_file(&metadata)?;
        let settings: Self = toml::from_str(&fs::read_to_string(path)?)?;
        settings.validate()?;
        Ok(settings)
    }

    pub(crate) fn validate(&self) -> Result<(), HelperServiceError> {
        for (name, path) in [
            ("socket_path", &self.socket_path),
            ("staging_dir", &self.staging_dir),
            ("runtime_dir", &self.runtime_dir),
            ("mihomo_binary", &self.mihomo_binary),
        ] {
            if !path.is_absolute() || path.components().any(|part| part.as_os_str() == "..") {
                return Err(HelperServiceError::UnsafeConfig(format!(
                    "{name} must be an absolute normalized path"
                )));
            }
        }
        if self.staging_dir.starts_with(&self.runtime_dir)
            || self.runtime_dir.starts_with(&self.staging_dir)
        {
            return Err(HelperServiceError::UnsafeConfig(
                "staging and system runtime directories must not contain one another".into(),
            ));
        }
        if !valid_sha256(&self.mihomo_sha256) {
            return Err(HelperServiceError::UnsafeConfig(
                "mihomo_sha256 must be lowercase SHA-256".into(),
            ));
        }
        if self.tun_name.is_empty()
            || self.tun_name.len() > 15
            || !self
                .tun_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(HelperServiceError::UnsafeConfig(
                "tun_name must be 1-15 safe interface-name characters".into(),
            ));
        }
        Ok(())
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(unix)]
fn validate_root_owned_private_file(metadata: &fs::Metadata) -> Result<(), HelperServiceError> {
    use std::os::unix::fs::MetadataExt;
    if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(HelperServiceError::UnsafeConfig(
            "helper config must be root-owned and not group/world writable".into(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct ManagedChild {
    child: Child,
    generation_id: Uuid,
    started_at: String,
}

#[derive(Debug)]
pub struct Supervisor {
    settings: HelperSettings,
    child: Mutex<Option<ManagedChild>>,
    lifecycle: Mutex<()>,
    registered: Mutex<HashMap<Uuid, String>>,
    logs: Arc<Mutex<VecDeque<ServiceLogEntry>>>,
    pub(crate) side_tunnels: Mutex<HashMap<Uuid, openvpn::RunningSideTunnel>>,
    desktop_contact: Mutex<Option<Instant>>,
    commands_in_flight: AtomicUsize,
    network_release_pending: AtomicBool,
    started_at: Instant,
}

impl Supervisor {
    #[must_use]
    pub fn new(settings: HelperSettings) -> Self {
        Self {
            settings,
            child: Mutex::new(None),
            lifecycle: Mutex::new(()),
            registered: Mutex::new(HashMap::new()),
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES))),
            side_tunnels: Mutex::new(HashMap::new()),
            desktop_contact: Mutex::new(None),
            commands_in_flight: AtomicUsize::new(0),
            network_release_pending: AtomicBool::new(false),
            started_at: Instant::now(),
        }
    }

    pub(crate) async fn note_desktop_contact(&self) {
        *self.desktop_contact.lock().await = Some(Instant::now());
    }

    async fn lock_lifecycle(&self, operation: &'static str) -> tokio::sync::MutexGuard<'_, ()> {
        let trace_id = Uuid::new_v4();
        tracing::info!(event = "helper.lifecycle_waiting", section = "helper_lifecycle",
            initiator = "helper_command", cause = "serialize_native_network_changes", %trace_id, operation,
            trace_route = "helper_command->lifecycle_lock->native_network", "waiting for owned network lifecycle work");
        let guard = self.lifecycle.lock().await;
        tracing::info!(event = "helper.lifecycle_acquired", section = "helper_lifecycle",
            initiator = "helper_command", cause = "previous_native_work_finished", %trace_id, operation,
            trace_route = "helper_command->lifecycle_lock->native_network", "acquired owned network lifecycle work");
        guard
    }

    pub(crate) fn begin_command(&self) -> CommandInFlight<'_> {
        self.commands_in_flight.fetch_add(1, Ordering::SeqCst);
        CommandInFlight { supervisor: self }
    }

    #[must_use]
    pub const fn settings(&self) -> &HelperSettings {
        &self.settings
    }

    /// Validates and publishes an immutable runtime generation.
    ///
    /// # Errors
    ///
    /// Returns an error when the generation path, files, or hash are invalid,
    /// or publishing the generation fails.
    pub async fn register_generation(
        &self,
        generation_id: Uuid,
        expected_sha256: &str,
    ) -> Result<(), HelperServiceError> {
        if !valid_sha256(expected_sha256) {
            return Err(HelperServiceError::InvalidGeneration(
                "invalid config SHA-256".into(),
            ));
        }
        let source_root = self.settings.staging_dir.join(generation_id.to_string());
        // A bare `?` here reports only "cannot find the file specified", which
        // hides the one fact that matters: which staging root the helper was
        // configured with. That path is recorded in helper.toml at install
        // time, so it is wrong for every later run whenever the installing
        // process saw a different profile than the app does now.
        let source_root_canonical = source_root.canonicalize().map_err(|error| {
            HelperServiceError::InvalidGeneration(format!(
                "staged generation {} is unreadable at {}: {error}",
                generation_id,
                source_root.display()
            ))
        })?;
        let staging_canonical = self.settings.staging_dir.canonicalize().map_err(|error| {
            HelperServiceError::InvalidGeneration(format!(
                "configured staging directory {} is unreadable: {error}",
                self.settings.staging_dir.display()
            ))
        })?;
        if !source_root_canonical.starts_with(&staging_canonical) {
            return Err(HelperServiceError::InvalidGeneration(
                "generation escaped staging directory".into(),
            ));
        }
        let config_source = checked_generation_file(&source_root_canonical, "config.yaml")?;
        let actual_hash = sha256_file(&config_source)?;
        if actual_hash != expected_sha256 {
            return Err(HelperServiceError::InvalidGeneration(
                "config hash does not match request".into(),
            ));
        }

        let generations_root = self.settings.runtime_dir.join("generations");
        fs::create_dir_all(&generations_root)?;
        let temporary_root = generations_root.join(format!(".{generation_id}.staging"));
        if temporary_root.exists() {
            fs::remove_dir_all(&temporary_root)?;
        }
        fs::create_dir(&temporary_root)?;
        #[cfg(unix)]
        set_directory_permissions(&temporary_root)?;
        #[cfg(not(unix))]
        set_directory_permissions(&temporary_root);
        for name in collect_generation_files(&source_root_canonical)? {
            let source = checked_generation_file(&source_root_canonical, &name)?;
            let destination = temporary_root.join(&name);
            copy_new_file(&source, &destination)?;
        }
        let destination_root = generations_root.join(generation_id.to_string());
        if destination_root.exists() {
            fs::remove_dir_all(&destination_root)?;
        }
        fs::rename(&temporary_root, &destination_root)?;
        self.registered
            .lock()
            .await
            .insert(generation_id, expected_sha256.into());
        self.push_log("info", "runtime_generation_registered", BTreeMap::new())
            .await;
        Ok(())
    }

    /// Starts Mihomo from a previously registered runtime generation.
    ///
    /// # Errors
    ///
    /// Returns an error when the binary fails integrity checks, the generation
    /// is not registered, or the child process cannot be managed.
    pub async fn start(
        &self,
        generation_id: Uuid,
        expected_sha256: &str,
    ) -> Result<ProcessStatus, HelperServiceError> {
        let _lifecycle = self.lock_lifecycle("start").await;
        self.start_inner(generation_id, expected_sha256).await
    }

    async fn start_inner(
        &self,
        generation_id: Uuid,
        expected_sha256: &str,
    ) -> Result<ProcessStatus, HelperServiceError> {
        self.verify_binary()?;
        let registered = self.registered.lock().await;
        if registered.get(&generation_id).map(String::as_str) != Some(expected_sha256) {
            return Err(HelperServiceError::InvalidGeneration(
                "generation must be registered with the same hash before start".into(),
            ));
        }
        drop(registered);

        let mut current = self.child.lock().await;
        if let Some(managed) = current.as_mut() {
            if managed.child.try_wait()?.is_none() && managed.generation_id == generation_id {
                return Ok(process_status(Some(managed)));
            }
            stop_managed_child(managed).await?;
            *current = None;
        }

        let generation_root = self
            .settings
            .runtime_dir
            .join("generations")
            .join(generation_id.to_string());
        let config_path = checked_generation_file(&generation_root, "config.yaml")?;
        if sha256_file(&config_path)? != expected_sha256 {
            return Err(HelperServiceError::InvalidGeneration(
                "published config changed after registration".into(),
            ));
        }
        orphan::cleanup(&self.settings, generation_id).await?;
        #[cfg(target_os = "macos")]
        if macos_dns::recovery_pending(&self.settings.runtime_dir)
            || macos_tun::saved(&self.settings.runtime_dir).map_or(true, |device| device.is_some())
        {
            self.release_macos_network().await?;
        }
        self.network_release_pending.store(true, Ordering::SeqCst);
        #[cfg(windows)]
        windows_network::prepare(&self.settings.tun_name).await?;
        // Persist DNS before the core can change native resolver state.
        #[cfg(target_os = "macos")]
        macos_dns::prepare(&self.settings.runtime_dir)?;
        let started_at = now_string();
        let launch_id = Uuid::new_v4();
        let mut child = spawn_mihomo(&self.settings, &generation_root, &config_path)?;
        if let Some(stdout) = child.stdout.take() {
            capture_lines(stdout, Arc::clone(&self.logs), "info", launch_id);
        }
        if let Some(stderr) = child.stderr.take() {
            capture_lines(stderr, Arc::clone(&self.logs), "warn", launch_id);
        }
        tokio::time::sleep(MIHOMO_STAY_ALIVE).await;
        if let Some(status) = child.try_wait()? {
            let detail = last_mihomo_output(&self.logs).await;
            return Err(HelperServiceError::Process(match detail {
                Some(message) => format!("Mihomo exited immediately ({status}): {message}"),
                None => format!("Mihomo exited immediately ({status})"),
            }));
        }
        let managed = ManagedChild {
            child,
            generation_id,
            started_at,
        };
        let status = process_status(Some(&managed));
        *current = Some(managed);
        drop(current);
        self.push_log("info", "mihomo_started", BTreeMap::new())
            .await;
        // On macOS the system resolver queries the configured nameserver, and
        // the LAN/router DNS bypasses the TUN so Mihomo's `dns-hijack` never
        // sees it. Point every network service at `127.0.0.1` (Mihomo listens
        // on port 53 as root) so DNS flows through the TUN.
        #[cfg(target_os = "macos")]
        {
            macos_tun::remember_started_device(
                &self.settings.runtime_dir,
                &self.logs,
                launch_id,
                Duration::from_secs(4),
            )
            .await?;
            macos_dns::apply(&self.settings.runtime_dir)?;
        }
        Ok(status)
    }

    /// Copies a registered generation into the running Mihomo workdir.
    ///
    /// Mihomo Meta resolves rule-provider paths against `-d`, so a pin apply
    /// cannot point the controller at a sibling generation directory. Overlay
    /// keeps that workdir and the process; the desktop then `PUT /configs`.
    ///
    /// # Errors
    ///
    /// Returns an error when the generation is not registered, the process is
    /// missing, or the copy fails.
    pub async fn overlay_running(
        &self,
        generation_id: Uuid,
        expected_sha256: &str,
    ) -> Result<ProcessStatus, HelperServiceError> {
        let _lifecycle = self.lock_lifecycle("overlay").await;
        let registered = self.registered.lock().await;
        if registered.get(&generation_id).map(String::as_str) != Some(expected_sha256) {
            return Err(HelperServiceError::InvalidGeneration(
                "generation must be registered with the same hash before overlay".into(),
            ));
        }
        drop(registered);

        let mut current = self.child.lock().await;
        let Some(managed) = current.as_mut() else {
            return Ok(ProcessStatus {
                running: false,
                pid: None,
                generation_id: None,
                started_at: None,
            });
        };
        if managed.child.try_wait()?.is_some() {
            *current = None;
            return Ok(ProcessStatus {
                running: false,
                pid: None,
                generation_id: None,
                started_at: None,
            });
        }

        let generations_root = self.settings.runtime_dir.join("generations");
        let source_root = generations_root.join(generation_id.to_string());
        let destination_root = generations_root.join(managed.generation_id.to_string());
        overlay_generation_files(&source_root, &destination_root)?;
        let config_path = checked_generation_file(&destination_root, "config.yaml")?;
        if sha256_file(&config_path)? != expected_sha256 {
            return Err(HelperServiceError::InvalidGeneration(
                "overlaid config hash does not match request".into(),
            ));
        }
        // The live process home on Windows is the runtime root, not the
        // generation directory. A nested absolute path is rejected, so the
        // same files must also sit directly in that home for a payload reload.
        overlay_generation_files(&source_root, &self.settings.runtime_dir)?;
        let status = process_status(Some(managed));
        drop(current);
        self.push_log("info", "runtime_generation_overlaid", BTreeMap::new())
            .await;
        Ok(status)
    }

    /// Stops the helper-owned Mihomo process.
    ///
    /// # Errors
    ///
    /// Returns an error when the process cannot be inspected or stopped.
    pub async fn stop(&self) -> Result<ProcessStatus, HelperServiceError> {
        let _lifecycle = self.lock_lifecycle("stop").await;
        self.stop_inner().await
    }

    async fn stop_inner(&self) -> Result<ProcessStatus, HelperServiceError> {
        let mut current = self.child.lock().await;
        if let Some(managed) = current.as_mut() {
            stop_managed_child(managed).await?;
        }
        *current = None;
        // Hold the child lock across the orphan kill so a concurrent start
        // cannot spawn a new Mihomo while this stop is still tearing the
        // previous one down.
        if let Err(cause) = orphan::cleanup(&self.settings, Uuid::new_v4()).await {
            tracing::warn!(
                event = "helper.orphan_after_stop_failed",
                section = "helper_process",
                initiator = "helper_process",
                cause = %cause,
                trace_route = "helper_process->stop->owned_orphan_cleanup",
                "scoped Mihomo orphan remained after stop"
            );
        }
        drop(current);
        self.push_log("info", "mihomo_stopped", BTreeMap::new())
            .await;
        // DNS restore talks to networksetup for every service. Keep it off
        // the child lock so a status read is not stuck behind that wait.
        // Start takes the same lock before it spawns, and the desktop does
        // not overlap stop with start.
        #[cfg(target_os = "macos")]
        self.release_macos_network().await?;
        #[cfg(windows)]
        if windows_network::captured(&self.settings.tun_name, true).await? {
            return Err(HelperServiceError::Process(
                "Windows tunnel capture remains after stop".into(),
            ));
        }
        Ok(ProcessStatus {
            running: false,
            pid: None,
            generation_id: None,
            started_at: None,
        })
    }

    pub(crate) async fn restart(
        &self,
        generation_id: Uuid,
        expected_sha256: &str,
    ) -> Result<ProcessStatus, HelperServiceError> {
        let _lifecycle = self.lock_lifecycle("restart").await;
        self.stop_inner().await?;
        self.start_inner(generation_id, expected_sha256).await
    }

    /// Drops macOS tunnel routes and points system DNS back at the network
    /// the machine had before Connect.
    #[cfg(target_os = "macos")]
    async fn release_macos_network(&self) -> Result<(), HelperServiceError> {
        let routes = async {
            let device = macos_tun::saved(&self.settings.runtime_dir)?;
            let interface = device.as_deref().unwrap_or(&self.settings.tun_name);
            if interface_exists(interface)? {
                // A utun name can be reused after our process exits. Leave a live
                // device alone and retain recovery instead of touching a new owner.
                return Err(HelperServiceError::Process(
                    "macOS native tunnel is still present; release remains pending".into(),
                ));
            }
            clear_owned_routes(interface).await
        }
        .await;
        let dns = macos_dns::restore(&self.settings.runtime_dir);
        if let Err(ref cause) = dns {
            tracing::warn!(
                event = "helper.dns_restore_after_stop_failed",
                section = "helper_dns",
                initiator = "helper_process",
                cause = %cause,
                trace_route = "helper_process->stop->macos_dns",
                "macOS DNS was not restored when Mihomo stopped"
            );
        }
        let cache = flush_dns_cache();
        routes?;
        dns?;
        cache?;
        macos_tun::retire(&self.settings.runtime_dir)
    }

    /// Returns the current helper-owned process status.
    ///
    /// # Errors
    ///
    /// Returns an error when the child process status cannot be inspected.
    pub async fn status(&self) -> Result<ProcessStatus, HelperServiceError> {
        let mut current = self.child.lock().await;
        if let Some(managed) = current.as_mut() {
            if managed.child.try_wait()?.is_some() {
                *current = None;
            }
        }
        Ok(process_status(current.as_ref()))
    }

    /// Stops Mihomo and removes the helper-owned network interface.
    ///
    /// # Errors
    ///
    /// Returns an error when the process or network cleanup fails.
    pub async fn cleanup(&self) -> Result<CleanupReport, HelperServiceError> {
        let _lifecycle = self.lock_lifecycle("cleanup").await;
        self.cleanup_inner().await
    }

    async fn cleanup_inner(&self) -> Result<CleanupReport, HelperServiceError> {
        self.network_release_pending.store(true, Ordering::SeqCst);
        self.stop_all_side_tunnels().await;
        // A DNS/route failure must not skip the remaining cleanup. Stop may
        // have already reaped the process before network restoration failed.
        let stop_result = self.stop_inner().await;
        let process_stopped = !self.status().await?.running;
        #[cfg(target_os = "linux")]
        let was_active = interface_exists(&self.settings.tun_name);
        #[cfg(target_os = "linux")]
        if was_active {
            delete_owned_interface(&self.settings.tun_name).await?;
        }
        #[cfg(target_os = "macos")]
        let native_release = if stop_result.is_ok() {
            Ok(())
        } else {
            self.release_macos_network().await
        };
        #[cfg(target_os = "macos")]
        let tun_removed = native_release.is_ok();
        #[cfg(target_os = "linux")]
        let tun_removed = !interface_exists(&self.settings.tun_name);
        #[cfg(windows)]
        let tun_removed = stop_result.is_ok()
            || !windows_network::captured(&self.settings.tun_name, true).await?;
        let warnings = if tun_removed {
            Vec::new()
        } else {
            vec![format!(
                "owned interface {} remains after cleanup",
                self.settings.tun_name
            )]
        };
        #[cfg(not(target_os = "macos"))]
        let cache_result = flush_dns_cache();
        #[cfg(target_os = "macos")]
        native_release?;
        #[cfg(not(target_os = "macos"))]
        cache_result?;
        // A second pass may repair a transient network failure from Stop. A
        // process-stop failure still cannot be reported as successful cleanup.
        if !process_stopped {
            stop_result?;
        }
        let report = CleanupReport {
            process_stopped,
            tun_removed,
            routes_removed: 0,
            dns_restored: true,
            warnings,
        };
        self.push_log("info", "network_cleanup_finished", BTreeMap::new())
            .await;
        if report.process_stopped
            && report.tun_removed
            && report.dns_restored
            && report.warnings.is_empty()
        {
            self.network_release_pending.store(false, Ordering::SeqCst);
        }
        Ok(report)
    }

    pub async fn logs(&self, maximum: usize) -> Vec<ServiceLogEntry> {
        let logs = self.logs.lock().await;
        logs.iter()
            .rev()
            .take(maximum.min(MAX_LOG_ENTRIES))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    fn verify_binary(&self) -> Result<(), HelperServiceError> {
        let metadata = fs::symlink_metadata(&self.settings.mihomo_binary)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(HelperServiceError::BinaryIntegrity);
        }
        if sha256_file(&self.settings.mihomo_binary)? != self.settings.mihomo_sha256 {
            return Err(HelperServiceError::BinaryIntegrity);
        }
        Ok(())
    }

    pub(crate) async fn push_log(
        &self,
        level: &str,
        event: &str,
        fields: BTreeMap<String, String>,
    ) {
        let mut logs = self.logs.lock().await;
        if logs.len() == MAX_LOG_ENTRIES {
            logs.pop_front();
        }
        logs.push_back(ServiceLogEntry {
            timestamp: now_string(),
            level: level.into(),
            event: event.into(),
            fields,
        });
    }
}

fn spawn_mihomo(
    settings: &HelperSettings,
    generation_root: &Path,
    config_path: &Path,
) -> Result<Child, HelperServiceError> {
    let mut command = Command::new(&settings.mihomo_binary);
    command
        .arg("-d")
        .arg(generation_root)
        .arg("-f")
        .arg(config_path)
        .current_dir(generation_root)
        .env_clear()
        .env("PATH", mihomo_search_path(&settings.mihomo_binary))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    apply_windows_mihomo_spawn(&mut command);
    command
        .spawn()
        .map_err(|error| HelperServiceError::Process(error.to_string()))
}

fn mihomo_search_path(binary: &Path) -> String {
    let system = if cfg!(windows) {
        r"C:\Windows\System32;C:\Windows"
    } else {
        "/usr/sbin:/usr/bin:/sbin:/bin"
    };
    match binary.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            if cfg!(windows) {
                format!("{};{system}", parent.display())
            } else {
                format!("{}:{system}", parent.display())
            }
        }
        _ => system.to_owned(),
    }
}

#[cfg(windows)]
fn apply_windows_mihomo_spawn(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let system_root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| r"C:\Windows".into());
    let system_drive = std::env::var("SYSTEMDRIVE").unwrap_or_else(|_| r"C:".into());
    command
        .env("SYSTEMROOT", &system_root)
        .env("SystemRoot", &system_root)
        .env("WINDIR", &system_root)
        .env("SYSTEMDRIVE", system_drive)
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn apply_windows_mihomo_spawn(_command: &mut Command) {}

async fn last_mihomo_output(logs: &Mutex<VecDeque<ServiceLogEntry>>) -> Option<String> {
    logs.lock().await.iter().rev().find_map(|entry| {
        if entry.event == "mihomo_output" {
            entry.fields.get("message").cloned()
        } else {
            None
        }
    })
}

fn process_status(managed: Option<&ManagedChild>) -> ProcessStatus {
    managed.map_or(
        ProcessStatus {
            running: false,
            pid: None,
            generation_id: None,
            started_at: None,
        },
        |managed| ProcessStatus {
            running: true,
            pid: managed.child.id(),
            generation_id: Some(managed.generation_id),
            started_at: Some(managed.started_at.clone()),
        },
    )
}

async fn stop_managed_child(managed: &mut ManagedChild) -> Result<(), HelperServiceError> {
    if managed.child.try_wait()?.is_some() {
        return Ok(());
    }
    request_graceful_stop(&managed.child);
    match tokio::time::timeout(GRACEFUL_STOP_TIMEOUT, managed.child.wait()).await {
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(error)) => return Err(HelperServiceError::Process(error.to_string())),
        Err(_) => {}
    }
    if managed.child.try_wait()?.is_some() {
        return Ok(());
    }
    managed
        .child
        .start_kill()
        .map_err(|error| HelperServiceError::Process(error.to_string()))?;
    tokio::time::timeout(PROCESS_STOP_TIMEOUT, managed.child.wait())
        .await
        .map_err(|_| HelperServiceError::Process("process stop timed out".into()))?
        .map_err(|error| HelperServiceError::Process(error.to_string()))?;
    Ok(())
}

#[cfg(unix)]
fn request_graceful_stop(child: &Child) {
    let Some(pid) = child.id() else {
        return;
    };
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    if let Err(cause) = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    ) {
        // The child can exit between try_wait and kill; no signal is then needed.
        if cause != nix::errno::Errno::ESRCH {
            tracing::warn!(
                event = "helper.graceful_stop_failed",
                section = "helper_process",
                initiator = "helper_process",
                cause = %cause,
                trace_route = "helper_process->stop->sigterm",
                "graceful stop failed; the bounded force-stop fallback remains active"
            );
        }
    }
}

#[cfg(windows)]
fn request_graceful_stop(child: &Child) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Some(pid) = child.id() else {
        return;
    };
    let result = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let cause = match result {
        Ok(status) if status.success() => return,
        Ok(_) => "taskkill_failed",
        Err(_) => "taskkill_spawn_failed",
    };
    tracing::warn!(
        event = "helper.graceful_stop_failed",
        section = "helper_process",
        initiator = "helper_process",
        cause,
        trace_route = "helper_process->stop->taskkill",
        "graceful stop failed; the bounded force-stop fallback remains active"
    );
}

/// Drops cached fake-ip answers (`198.18.0.0/16`) so lookups work again after
/// the TUN is gone. Windows/macOS cache failures keep cleanup retryable.
fn flush_dns_cache() -> Result<(), HelperServiceError> {
    let mut flushed = false;
    let mut failed = false;
    for (program, args) in dns_flush_commands() {
        if !Path::new(program).is_file() {
            failed = true;
            tracing::warn!(
                event = "helper.dns_cache_flush_unavailable",
                section = "helper_dns",
                initiator = "helper_process",
                cause = "native_tool_missing",
                trace_route = "helper_process->cleanup->dns_cache",
                "native DNS cache tool is unavailable"
            );
            continue;
        }
        match dns_flush_status(program, args) {
            Ok(status) if status.success() => {
                flushed = true;
                tracing::info!(
                    event = "helper.dns_cache_flushed",
                    section = "helper_dns",
                    initiator = "helper_process",
                    cause = "network_cleanup",
                    trace_route = "helper_process->cleanup->dns_cache",
                    "flushed the system DNS cache after releasing the tunnel"
                );
            }
            Ok(status) => {
                failed = true;
                tracing::warn!(
                    event = "helper.dns_cache_flush_failed",
                    section = "helper_dns",
                    initiator = "helper_process",
                    cause = "flush_command_failed",
                    trace_route = "helper_process->cleanup->dns_cache",
                    status = %status,
                    "system DNS cache was not flushed"
                );
            }
            Err(cause) => {
                failed = true;
                tracing::warn!(
                    event = "helper.dns_cache_flush_failed",
                    section = "helper_dns",
                    initiator = "helper_process",
                    cause = %cause,
                    trace_route = "helper_process->cleanup->dns_cache",
                    "system DNS cache flush could not start"
                );
            }
        }
    }
    // Linux resolver tools are optional alternatives and may be installed
    // while systemd-resolved is disabled. Preserve that platform's policy.
    if !cfg!(target_os = "linux") && (!flushed || failed) {
        return Err(HelperServiceError::Process(
            "system DNS cache cleanup remains pending".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn dns_flush_commands() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("/usr/bin/dscacheutil", vec!["-flushcache"]),
        ("/usr/bin/killall", vec!["-HUP", "mDNSResponder"]),
    ]
}

#[cfg(target_os = "linux")]
fn dns_flush_commands() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("/usr/bin/resolvectl", vec!["flush-caches"]),
        ("/usr/bin/systemd-resolve", vec!["--flush-caches"]),
    ]
}

#[cfg(windows)]
fn dns_flush_commands() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![(r"C:\Windows\System32\ipconfig.exe", vec!["/flushdns"])]
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn dns_flush_commands() -> Vec<(&'static str, Vec<&'static str>)> {
    Vec::new()
}

fn dns_flush_status(program: &str, args: Vec<&str>) -> io::Result<std::process::ExitStatus> {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.status()
}

struct CommandInFlight<'a> {
    supervisor: &'a Supervisor,
}

impl Drop for CommandInFlight<'_> {
    fn drop(&mut self) {
        self.supervisor
            .commands_in_flight
            .fetch_sub(1, Ordering::SeqCst);
    }
}

/// `true` when the desktop has not been seen for `limit` and a capture remains.
#[must_use]
fn abandoned_capture(
    contact: Option<Instant>,
    started_at: Instant,
    now: Instant,
    in_flight: bool,
    captured: bool,
    limit: Duration,
) -> bool {
    if in_flight || !captured {
        return false;
    }
    let age = match contact {
        Some(seen) => now.saturating_duration_since(seen),
        None => now.saturating_duration_since(started_at),
    };
    age >= limit
}

impl Supervisor {
    /// Stops Mihomo and restores DNS when the desktop process has disappeared.
    pub(crate) async fn release_if_desktop_absent(&self) {
        let Ok(_lifecycle) = self.lifecycle.try_lock() else {
            return;
        };
        let in_flight = self.commands_in_flight.load(Ordering::SeqCst) > 0;
        let contact = *self.desktop_contact.lock().await;
        if !abandoned_capture(
            contact,
            self.started_at,
            Instant::now(),
            in_flight,
            true,
            DESKTOP_ABSENCE_LIMIT,
        ) {
            return;
        }
        if !self.network_captured().await {
            return;
        }
        tracing::warn!(
            event = "helper.desktop_absent",
            section = "helper_lifecycle",
            initiator = "helper_process",
            cause = "desktop_contact_expired",
            trace_route = "helper_process->absence_watch->cleanup",
            "desktop contact expired while the tunnel was still up; releasing the network"
        );
        if let Err(cause) = self.cleanup_inner().await {
            tracing::warn!(
                event = "helper.desktop_absent_cleanup_failed",
                section = "helper_lifecycle",
                initiator = "helper_process",
                cause = %cause,
                trace_route = "helper_process->absence_watch->cleanup",
                "network release after a missing desktop did not finish"
            );
        }
        if !self.network_captured().await {
            *self.desktop_contact.lock().await = None;
        }
    }

    async fn network_captured(&self) -> bool {
        let running = if let Ok(status) = self.status().await {
            status.running
        } else {
            tracing::warn!(
                event = "helper.capture_status_unknown",
                section = "helper_network",
                initiator = "absence_watch",
                cause = "process_inspection_failed",
                trace_route = "helper_process->absence_watch->native_state",
                "process inspection failed; keeping network release pending"
            );
            true
        };
        #[cfg(windows)]
        let captured = windows_network::captured(&self.settings.tun_name, false)
            .await
            .unwrap_or(true);
        #[cfg(target_os = "linux")]
        let captured = interface_exists(&self.settings.tun_name);
        #[cfg(target_os = "macos")]
        let captured = if let (Ok(active), Ok(device)) = (
            interface_exists(&self.settings.tun_name),
            macos_tun::saved(&self.settings.runtime_dir),
        ) {
            active || device.is_some() || macos_dns::recovery_pending(&self.settings.runtime_dir)
        } else {
            tracing::warn!(
                event = "helper.capture_status_unknown",
                section = "helper_network",
                initiator = "absence_watch",
                cause = "native_inspection_failed",
                trace_route = "helper_process->absence_watch->native_state",
                "native inspection failed; keeping network release pending"
            );
            true
        };
        running || captured || self.network_release_pending.load(Ordering::SeqCst)
    }
}

pub(crate) fn spawn_absence_watch(supervisor: Arc<Supervisor>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            supervisor.release_if_desktop_absent().await;
        }
    });
}

fn collect_generation_files(root: &Path) -> Result<Vec<String>, HelperServiceError> {
    let mut names: Vec<String> = FIXED_GENERATION_FILES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let entries = fs::read_dir(root)?;
    for entry in entries {
        let name = entry?.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            continue;
        }
        if is_allowed_generation_file(name) && !names.iter().any(|existing| existing == name) {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

fn checked_generation_file(root: &Path, name: &str) -> Result<PathBuf, HelperServiceError> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(HelperServiceError::InvalidGeneration(
            "file is not in the runtime allowlist".into(),
        ));
    }
    if !is_allowed_generation_file(name) {
        return Err(HelperServiceError::InvalidGeneration(
            "file is not in the runtime allowlist".into(),
        ));
    }
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(HelperServiceError::InvalidGeneration(format!(
            "{name} must be a regular non-symlink file"
        )));
    }
    let canonical = path.canonicalize()?;
    let canonical_root = root.canonicalize()?;
    if !canonical.starts_with(canonical_root) {
        return Err(HelperServiceError::InvalidGeneration(format!(
            "{name} escaped the generation directory"
        )));
    }
    if metadata.len() > 4 * 1024 * 1024 {
        return Err(HelperServiceError::InvalidGeneration(format!(
            "{name} exceeds the 4 MiB per-file limit"
        )));
    }
    Ok(canonical)
}

fn sha256_file(path: &Path) -> Result<String, HelperServiceError> {
    let bytes = fs::read(path)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// Copies `source` onto `destination` unless they resolve to the same file or
/// contain the same bytes. Windows locks a running executable against writes,
/// so reinstall must not overwrite an identical Mihomo/helper payload.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn copy_file_unless_same(
    source: &Path,
    destination: &Path,
) -> Result<(), HelperServiceError> {
    if paths_refer_to_same_file(source, destination)
        || files_have_same_contents(source, destination)?
    {
        return Ok(());
    }
    fs::copy(source, destination)?;
    Ok(())
}

#[cfg_attr(not(windows), allow(dead_code))]
fn files_have_same_contents(source: &Path, destination: &Path) -> Result<bool, HelperServiceError> {
    let source_metadata = fs::metadata(source)?;
    let destination_metadata = match fs::metadata(destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !source_metadata.is_file()
        || !destination_metadata.is_file()
        || source_metadata.len() != destination_metadata.len()
    {
        return Ok(false);
    }
    Ok(sha256_file(source)? == sha256_file(destination)?)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Task Scheduler XML with separate `Command` and `Arguments`.
///
/// `schtasks /TR "\"exe\" --config \"file\""` stores one string that `/Create`
/// and `/Run` both accept without ever starting a process, which is what left
/// the named pipe missing (`os error 2`) while the install reported success.
///
/// `ExecutionTimeLimit` is `PT0S` so the scheduler never treats the helper as
/// an overdue job, and `AllowHardTerminate` stays `true` so `schtasks /End` can
/// actually stop it — a reinstall has to release the running helper's own image
/// before it can copy over it.
#[cfg(any(windows, test))]
#[must_use]
pub(crate) fn scheduled_task_xml(helper: &Path, config: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n\
         <Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
           <Triggers><BootTrigger><Enabled>true</Enabled></BootTrigger></Triggers>\n\
           <Principals><Principal id=\"Author\"><UserId>S-1-5-18</UserId>\
           <RunLevel>HighestAvailable</RunLevel></Principal></Principals>\n\
           <Settings>\n\
             <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n\
             <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n\
             <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n\
             <AllowHardTerminate>true</AllowHardTerminate>\n\
             <StartWhenAvailable>true</StartWhenAvailable>\n\
             <AllowStartOnDemand>true</AllowStartOnDemand>\n\
             <Enabled>true</Enabled>\n\
             <Hidden>true</Hidden>\n\
             <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n\
           </Settings>\n\
           <Actions Context=\"Author\"><Exec>\n\
             <Command>{}</Command>\n\
             <Arguments>--config \"{}\"</Arguments>\n\
           </Exec></Actions>\n\
         </Task>\n",
        xml_escape(&helper.to_string_lossy()),
        xml_escape(&config.to_string_lossy()),
    )
}

#[cfg(any(windows, test))]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Reads back one `Exec` field from `schtasks /Query /XML`. The element names
/// are the same on every locale, unlike the `/FO LIST` field labels.
#[cfg(any(windows, test))]
pub(crate) fn xml_element(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_owned())
}

/// `schtasks` writes UTF-16 LE when its output is a pipe rather than a console,
/// and not always with a BOM. Interleaved NULs are the giveaway: they are valid
/// UTF-8, so a plain lossy decode returns a string that no element name can be
/// found in — which would read as "the task stored no action at all".
#[cfg(any(windows, test))]
pub(crate) fn decode_console_output(bytes: &[u8]) -> String {
    let payload = bytes.strip_prefix(&[0xFF, 0xFE]);
    if payload.is_none() && !bytes.contains(&0) {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let units: Vec<u16> = payload
        .unwrap_or(bytes)
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// `install.log` is read back one line at a time, so a message that kept its
/// newlines would be reported as whichever fragment happened to land last.
#[cfg(any(windows, test))]
pub(crate) fn single_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn overlay_generation_files(
    source_root: &Path,
    destination_root: &Path,
) -> Result<(), HelperServiceError> {
    if !destination_root.is_dir() {
        return Err(HelperServiceError::InvalidGeneration(
            "running generation directory is missing".into(),
        ));
    }
    let names = collect_generation_files(source_root)?;
    let mut config = Vec::new();
    let mut others = Vec::new();
    for name in names {
        if name == "config.yaml" {
            config.push(name);
        } else {
            others.push(name);
        }
    }
    for name in others.into_iter().chain(config) {
        if !source_root.join(&name).is_file() {
            continue;
        }
        let source = checked_generation_file(source_root, &name)?;
        copy_over_file(&source, &destination_root.join(&name))?;
    }
    Ok(())
}

fn copy_new_file(source: &Path, destination: &Path) -> Result<(), HelperServiceError> {
    copy_file_with_options(source, destination, true)
}

fn copy_over_file(source: &Path, destination: &Path) -> Result<(), HelperServiceError> {
    copy_file_with_options(source, destination, false)
}

fn copy_file_with_options(
    source: &Path,
    destination: &Path,
    create_new: bool,
) -> Result<(), HelperServiceError> {
    let mut options = fs::OpenOptions::new();
    options.write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true).truncate(true);
    }
    let mut output = options.open(destination)?;
    let mut input = fs::File::open(source)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    #[cfg(unix)]
    set_file_permissions(destination)?;
    #[cfg(not(unix))]
    set_file_permissions(destination);
    Ok(())
}

#[cfg(unix)]
fn set_file_permissions(path: &Path) -> Result<(), HelperServiceError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_file_permissions(_path: &Path) {}

#[cfg(unix)]
fn set_directory_permissions(path: &Path) -> Result<(), HelperServiceError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_directory_permissions(_path: &Path) {}

fn now_string() -> String {
    // UTC RFC3339 without pulling wall-clock parsing into the IPC contract.
    format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| value.as_secs())
    )
}

fn capture_lines<R>(
    reader: R,
    logs: Arc<Mutex<VecDeque<ServiceLogEntry>>>,
    level: &'static str,
    launch_id: Uuid,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut entries = logs.lock().await;
            if entries.len() == MAX_LOG_ENTRIES {
                entries.pop_front();
            }
            entries.push_back(ServiceLogEntry {
                timestamp: now_string(),
                level: level.into(),
                event: "mihomo_output".into(),
                fields: BTreeMap::from([
                    ("message".into(), redact(&line)),
                    ("launch_id".into(), launch_id.to_string()),
                ]),
            });
        }
    });
}

static SENSITIVE_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(authorization|bearer|secret|token|password)(\s*[:=]\s*|\s+)[^\s,;]+")
        .expect("redaction regex is valid")
});
static SUBSCRIPTION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://[^\s]+(?:sub|subscription|token)[^\s]*")
        .expect("subscription regex is valid")
});

#[must_use]
pub fn redact(input: &str) -> String {
    let bounded: String = input.chars().take(8_192).collect();
    let value = SENSITIVE_VALUE.replace_all(&bounded, "$1=[REDACTED]");
    SUBSCRIPTION_URL
        .replace_all(&value, "[REDACTED_URL]")
        .into_owned()
}

#[cfg(target_os = "linux")]
async fn delete_owned_interface(name: &str) -> Result<(), HelperServiceError> {
    let binary = [Path::new("/usr/sbin/ip"), Path::new("/usr/bin/ip")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| HelperServiceError::Process("ip utility was not found".into()))?;
    let output = Command::new(binary)
        .args(["link", "delete", "dev", name])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if !output.status.success() {
        return Err(HelperServiceError::Process(
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1_024)
                .collect(),
        ));
    }
    Ok(())
}

/// A route row from `netstat -rn` whose interface column is the helper TUN.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, PartialEq, Eq)]
struct OwnedRoute {
    destination: String,
    gateway: String,
    host_route: bool,
}

/// `netstat` puts the interface in the fourth column. An optional expire
/// value comes after it, so the interface is not always the last field.
#[cfg(any(target_os = "macos", test))]
fn owned_routes(table: &str, interface: &str) -> Vec<OwnedRoute> {
    table
        .lines()
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.len() < 4 || columns[3] != interface {
                return None;
            }
            let destination = columns[0];
            if destination.eq_ignore_ascii_case("destination") {
                return None;
            }
            Some(OwnedRoute {
                destination: destination.to_owned(),
                gateway: columns[1].to_owned(),
                host_route: columns[2].contains('H'),
            })
        })
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn route_delete_args(route: &OwnedRoute) -> Vec<String> {
    let mut args = vec!["-n".into(), "delete".into()];
    if route.gateway.contains(':') || route.destination.contains(':') {
        args.push("-inet6".into());
    }
    if route.destination != "default" {
        args.push(if route.host_route { "-host" } else { "-net" }.into());
    }
    args.push(route.destination.clone());
    args.push(route.gateway.clone());
    args
}

#[cfg(any(target_os = "macos", test))]
fn verify_owned_routes_absent(table: &str, interface: &str) -> Result<(), HelperServiceError> {
    if !owned_routes(table, interface).is_empty() {
        return Err(HelperServiceError::Process(
            "macOS owned routes remain after cleanup".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn route_command(
    program: &str,
    args: &[String],
) -> Result<std::process::Output, HelperServiceError> {
    tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(program)
            .args(args)
            .env_clear()
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| HelperServiceError::Process("macOS route command timed out".into()))?
    .map_err(HelperServiceError::Io)
}

#[cfg(target_os = "macos")]
async fn clear_owned_routes(interface: &str) -> Result<(), HelperServiceError> {
    let trace_id = Uuid::new_v4();
    let mut removed = 0usize;
    let mut failed = 0usize;
    for family in ["inet", "inet6"] {
        let args = ["-rn".into(), "-f".into(), family.into()];
        let output = route_command("/usr/sbin/netstat", &args).await;
        let Ok(output) = output else {
            failed += 1;
            continue;
        };
        if !output.status.success() {
            failed += 1;
            continue;
        }
        for route in owned_routes(&String::from_utf8_lossy(&output.stdout), interface) {
            match route_command("/sbin/route", &route_delete_args(&route)).await {
                Ok(result) if result.status.success() => removed += 1,
                _ => failed += 1,
            }
        }
        match route_command("/usr/sbin/netstat", &args).await {
            Ok(result)
                if result.status.success()
                    && verify_owned_routes_absent(
                        &String::from_utf8_lossy(&result.stdout),
                        interface,
                    )
                    .is_ok() => {}
            _ => failed += 1,
        }
    }
    tracing::info!(event = "helper.tun_routes_cleared", section = "helper_network",
        initiator = "helper_process", cause = "network_cleanup", %trace_id, removed, failed,
        trace_route = "helper_process->stop->route", "verified owned tunnel route release");
    if failed > 0 {
        return Err(HelperServiceError::Process(
            "macOS owned route cleanup could not be verified".into(),
        ));
    }
    Ok(())
}

/// Returns `true` when a network interface with `name` is present.
#[cfg(target_os = "linux")]
fn interface_exists(name: &str) -> bool {
    Path::new("/sys/class/net").join(name).exists()
}

/// Returns `true` when a network interface with `name` is present. macOS has no
/// `/sys/class/net`; enumerate exact names and propagate inspection failures.
#[cfg(target_os = "macos")]
fn interface_exists(name: &str) -> Result<bool, HelperServiceError> {
    let output = std::process::Command::new("/sbin/ifconfig")
        .arg("-l")
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(HelperServiceError::Process(
            "macOS interface enumeration failed".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .any(|interface| interface == name))
}

mod commands;
mod orphan;

#[cfg(any(windows, test))]
mod windows_network;

#[cfg(any(target_os = "macos", test))]
mod macos_dns;
#[cfg(any(target_os = "macos", test))]
mod macos_tun;

#[cfg(unix)]
mod unix_peer;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::run_linux;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::run_macos;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{install, persist_install_error, run_named_pipe, uninstall};

/// Safe `install.log` text for a clap parse failure. Omits argv and paths —
/// those are user content. Help and version are not install failures.
#[must_use]
pub fn clap_install_error_message(error: &clap::Error) -> Option<String> {
    use clap::error::ErrorKind;
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => None,
        ErrorKind::UnknownArgument => Some("unexpected argument".into()),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::InvalidValue
        | ErrorKind::ValueValidation => Some("missing or invalid install argument".into()),
        _ => Some("helper argument error".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mutating_commands_wait_until_previous_native_release_finishes() {
        let root = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(HelperSettings {
            authorized_uid: 1000,
            authorized_gid: 1000,
            socket_path: root.path().join("helper.sock"),
            staging_dir: root.path().join("staging"),
            runtime_dir: root.path().join("runtime"),
            mihomo_binary: root.path().join("missing-core"),
            mihomo_sha256: "a".repeat(64),
            tun_name: "biflow-test".into(),
        });
        let release = supervisor.lifecycle.lock().await;
        let budget = Duration::from_millis(20);
        assert!(
            tokio::time::timeout(budget, supervisor.start(Uuid::new_v4(), &"a".repeat(64)))
                .await
                .is_err()
        );
        assert!(tokio::time::timeout(budget, supervisor.stop())
            .await
            .is_err());
        assert!(tokio::time::timeout(budget, supervisor.cleanup())
            .await
            .is_err());
        assert!(
            tokio::time::timeout(budget, supervisor.restart(Uuid::new_v4(), &"a".repeat(64)))
                .await
                .is_err()
        );
        assert!(tokio::time::timeout(
            budget,
            supervisor.overlay_running(Uuid::new_v4(), &"a".repeat(64))
        )
        .await
        .is_err());
        // Skip busy work rather than queue a stale absence cleanup after Connect.
        tokio::time::timeout(
            Duration::from_secs(1),
            supervisor.release_if_desktop_absent(),
        )
        .await
        .unwrap();
        drop(release);
        assert!(matches!(
            supervisor.start(Uuid::new_v4(), &"a".repeat(64)).await,
            Err(HelperServiceError::Io(_))
        ));
    }

    #[test]
    fn absence_releases_only_an_idle_capture() {
        let now = Instant::now();
        let ago = |age: Duration| now.checked_sub(age).expect("instant stays in range");
        let started = ago(Duration::from_secs(120));
        assert!(abandoned_capture(
            Some(ago(Duration::from_secs(90))),
            started,
            now,
            false,
            true,
            DESKTOP_ABSENCE_LIMIT
        ));
        assert!(!abandoned_capture(
            Some(ago(Duration::from_secs(10))),
            started,
            now,
            false,
            true,
            DESKTOP_ABSENCE_LIMIT
        ));
        assert!(!abandoned_capture(
            Some(ago(Duration::from_secs(120))),
            started,
            now,
            true,
            true,
            DESKTOP_ABSENCE_LIMIT
        ));
        assert!(!abandoned_capture(
            None,
            started,
            now,
            false,
            false,
            DESKTOP_ABSENCE_LIMIT
        ));
        assert!(abandoned_capture(
            None,
            ago(DESKTOP_ABSENCE_LIMIT),
            now,
            false,
            true,
            DESKTOP_ABSENCE_LIMIT
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_waits_for_a_graceful_exit_before_killing() {
        // Perl installs the ignore handler before it sleeps. A shell `trap`
        // loses the race: stop's SIGTERM arrives before the trap is parsed.
        let child = Command::new("perl")
            .args(["-e", "$SIG{TERM} = 'IGNORE'; sleep 30;"])
            .kill_on_drop(true)
            .spawn()
            .expect("process that ignores SIGTERM");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut managed = ManagedChild {
            child,
            generation_id: Uuid::new_v4(),
            started_at: String::new(),
        };
        let started = Instant::now();
        stop_managed_child(&mut managed)
            .await
            .expect("process stopped");
        let elapsed = started.elapsed();
        assert!(
            elapsed >= GRACEFUL_STOP_TIMEOUT,
            "stop killed the process before the graceful wait: {elapsed:?}"
        );
        assert!(
            elapsed < PROCESS_STOP_TIMEOUT,
            "stop waited for the force-kill timeout: {elapsed:?}"
        );
    }

    #[test]
    fn successful_route_command_is_not_proof_of_route_release() {
        let table = "default 192.0.2.1 UGSc en0\n128.0/1 198.18.0.1 UGSc utun9\n";
        assert!(verify_owned_routes_absent(table, "utun9").is_err());
        assert!(verify_owned_routes_absent("default 192.0.2.1 UGSc en0\n", "utun9").is_ok());
        assert!(verify_owned_routes_absent(table, "utun8").is_ok());
    }

    #[test]
    fn tun_routes_are_removed_without_touching_the_physical_default() {
        let table = "\
Routing tables

Internet:
Destination        Gateway            Flags           Netif Expire
default            10.0.0.1           UGScg           en0
1                  198.18.0.1         UGSc            utun9
128.0/1            198.18.0.1         UGSc            utun9
198.18.0.1         198.18.0.1         UH              utun9
default            fe80::1%utun9      UGcIg           utun9        1
";
        let routes = owned_routes(table, "utun9");
        assert_eq!(
            routes,
            vec![
                OwnedRoute {
                    destination: "1".into(),
                    gateway: "198.18.0.1".into(),
                    host_route: false,
                },
                OwnedRoute {
                    destination: "128.0/1".into(),
                    gateway: "198.18.0.1".into(),
                    host_route: false,
                },
                OwnedRoute {
                    destination: "198.18.0.1".into(),
                    gateway: "198.18.0.1".into(),
                    host_route: true,
                },
                OwnedRoute {
                    destination: "default".into(),
                    gateway: "fe80::1%utun9".into(),
                    host_route: false,
                },
            ]
        );
        assert_eq!(
            route_delete_args(&routes[0]),
            vec!["-n", "delete", "-net", "1", "198.18.0.1"]
        );
        assert_eq!(
            route_delete_args(&routes[2]),
            vec!["-n", "delete", "-host", "198.18.0.1", "198.18.0.1"]
        );
        assert_eq!(
            route_delete_args(&routes[3]),
            vec!["-n", "delete", "-inet6", "default", "fe80::1%utun9"]
        );
        let physical = owned_routes(table, "en0");
        assert_eq!(physical.len(), 1);
        assert_eq!(physical[0].destination, "default");
        assert_eq!(physical[0].gateway, "10.0.0.1");
    }

    #[test]
    fn redaction_removes_common_credentials_and_bounds_output() {
        let input = format!(
            "Authorization: Bearer-abc token=secret {}",
            "x".repeat(9_000)
        );
        let output = redact(&input);
        assert!(!output.contains("Bearer-abc"));
        assert!(!output.contains("token=secret"));
        assert!(output.len() < input.len());
    }

    #[test]
    fn rejects_unsafe_tun_name_and_hash() {
        let settings = HelperSettings {
            authorized_uid: 1_000,
            authorized_gid: 1_000,
            socket_path: "/run/iran-split/helper.sock".into(),
            staging_dir: "/home/user/.local/share/iran-split/runtime".into(),
            runtime_dir: "/var/lib/iran-split".into(),
            mihomo_binary: "/opt/iran-split/mihomo".into(),
            mihomo_sha256: "not-a-hash".into(),
            tun_name: "../../tun".into(),
        };
        assert!(settings.validate().is_err());
    }

    // A leading `/` is root-relative on Windows, not absolute, so `validate`
    // rejects this Linux fixture there. The Windows layout is covered below.
    #[cfg(unix)]
    #[test]
    fn helper_toml_defaults_missing_gid() {
        let parsed: HelperSettings = toml::from_str(
            r#"
authorized_uid = 1000
socket_path = "/run/iran-split/helper.sock"
staging_dir = "/home/user/.local/share/biflow/runtime/generations"
runtime_dir = "/var/lib/iran-split"
mihomo_binary = "/usr/lib/biflow/mihomo"
mihomo_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
tun_name = "clash-iran"
"#,
        )
        .expect("toml");
        assert_eq!(parsed.authorized_gid, 0);
        assert!(parsed.validate().is_ok());
    }

    #[cfg(not(unix))]
    #[test]
    fn permission_helpers_are_noops_off_unix() {
        let path = Path::new("unused");
        set_file_permissions(path);
        set_directory_permissions(path);
    }

    #[cfg(windows)]
    #[test]
    fn windows_production_staging_is_beside_runtime_not_inside_it() {
        let settings = HelperSettings {
            authorized_uid: 0,
            authorized_gid: 0,
            socket_path: r"\\.\pipe\iran-split-helper-v1".into(),
            staging_dir: r"C:\ProgramData\iran-split\staging".into(),
            runtime_dir: r"C:\ProgramData\iran-split\runtime".into(),
            mihomo_binary: r"C:\ProgramData\iran-split\bin\mihomo.exe".into(),
            mihomo_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .into(),
            tun_name: "clash-iran".into(),
        };
        assert!(settings.validate().is_ok());
        let nested = HelperSettings {
            staging_dir: r"C:\ProgramData\iran-split\runtime\generations".into(),
            ..settings
        };
        assert!(nested.validate().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn production_linux_helper_paths_are_absolute() {
        assert!(Path::new("/run/iran-split/helper.sock").is_absolute());
        assert!(Path::new("/var/lib/iran-split").is_absolute());
        assert!(Path::new("/usr/lib/biflow/iran-split-helper").is_absolute());
    }

    #[test]
    fn clap_install_error_message_omits_paths_and_help() {
        use clap::error::ErrorKind;
        assert_eq!(
            clap_install_error_message(&clap::Error::new(ErrorKind::UnknownArgument)).as_deref(),
            Some("unexpected argument")
        );
        assert_eq!(
            clap_install_error_message(&clap::Error::new(ErrorKind::MissingRequiredArgument))
                .as_deref(),
            Some("missing or invalid install argument")
        );
        assert_eq!(
            clap_install_error_message(&clap::Error::new(ErrorKind::DisplayHelp)),
            None
        );
        let with_path = clap::Error::raw(
            ErrorKind::UnknownArgument,
            r"unexpected argument 'C:\Program Files\BiFlow\dependencies\mihomo.exe'",
        );
        let message = clap_install_error_message(&with_path).expect("message");
        assert!(!message.contains("Program Files"));
        assert!(!message.contains('\\'));
    }

    #[test]
    fn windows_mihomo_spawn_keeps_system_root_and_hides_the_console() {
        let source = include_str!("lib.rs");
        assert!(source.contains("fn apply_windows_mihomo_spawn"));
        assert!(source.contains("SYSTEMROOT"));
        assert!(source.contains("CREATE_NO_WINDOW"));
        assert!(source.contains("MIHOMO_STAY_ALIVE"));
        assert!(source.contains("exited immediately"));
    }

    #[test]
    fn mihomo_search_path_puts_the_binary_directory_first() {
        #[cfg(windows)]
        {
            let path = mihomo_search_path(Path::new(r"C:\ProgramData\iran-split\bin\mihomo.exe"));
            assert!(path.starts_with(r"C:\ProgramData\iran-split\bin;"));
            assert!(path.contains(r"C:\Windows\System32"));
        }
        #[cfg(not(windows))]
        {
            let path = mihomo_search_path(Path::new("/usr/lib/biflow/mihomo"));
            assert!(path.starts_with("/usr/lib/biflow:"));
            assert!(path.contains("/usr/bin"));
        }
    }

    #[test]
    fn windows_main_persists_clap_errors_before_exit() {
        let source = include_str!("main.rs");
        assert!(source.contains("Arguments::try_parse()"));
        assert!(source.contains("clap_install_error_message"));
        assert!(source.contains("persist_install_error"));
        assert!(source.contains("error.exit()"));
        assert!(source.contains("run_named_pipe"));
    }

    #[test]
    fn scheduled_task_xml_keeps_command_and_arguments_separate() {
        let helper = Path::new(r"C:\ProgramData\iran-split\bin\iran-split-helper.exe");
        let config = Path::new(r"C:\ProgramData\iran-split\helper.toml");
        let xml = scheduled_task_xml(helper, config);
        assert!(
            xml.contains(r"<Command>C:\ProgramData\iran-split\bin\iran-split-helper.exe</Command>")
        );
        assert!(xml.contains(
            r#"<Arguments>--config "C:\ProgramData\iran-split\helper.toml"</Arguments>"#
        ));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("S-1-5-18"));
        assert!(!xml.contains("/TR"));
        // `false` here would make `schtasks /End` a no-op, and a reinstall
        // cannot copy over a helper that is still holding its own image.
        assert!(xml.contains("<AllowHardTerminate>true</AllowHardTerminate>"));
    }

    #[test]
    fn windows_install_registers_the_task_from_xml() {
        let source = include_str!("windows.rs");
        assert!(source.contains("/XML"));
        assert!(source.contains("scheduled_task_xml"));
        assert!(source.contains("wait_until_pipe_ready"));
        assert!(source.contains("wintun.dll"));
        assert!(!source.contains("\"/TR\""));
        assert!(source.contains(r#"root.join("staging")"#));
        assert!(source.contains("fn grant_users_modify"));
        assert!(source.contains("*S-1-5-32-545:(OI)(CI)M"));
        assert!(source.contains("icacls"));
        assert!(!source.contains("LOCALAPPDATA"));
    }

    /// `fs::metadata` fails on an NPFS object, so an `exists` check would report
    /// every healthy helper as missing and fail the install it just completed.
    #[test]
    fn pipe_readiness_opens_the_pipe_instead_of_stat_ing_it() {
        let source = include_str!("windows.rs");
        assert!(source.contains("fn pipe_is_serving"));
        assert!(!source.contains("Path::new(PIPE_NAME).exists()"));
        assert!(source.contains("ERROR_FILE_NOT_FOUND"));
        assert!(source.contains("fn stop_previous_helper"));
        assert!(source.contains("fn stored_task_action"));
    }

    #[test]
    fn xml_element_reads_the_stored_exec_action() {
        let xml = "<Actions><Exec><Command>C:\\bin\\helper.exe</Command>\
                   <Arguments>--config \"C:\\helper.toml\"</Arguments></Exec></Actions>";
        assert_eq!(
            xml_element(xml, "Command").as_deref(),
            Some(r"C:\bin\helper.exe")
        );
        assert_eq!(
            xml_element(xml, "Arguments").as_deref(),
            Some(r#"--config "C:\helper.toml""#)
        );
        assert_eq!(xml_element(xml, "Missing"), None);
    }

    #[test]
    fn console_output_is_decoded_from_utf16_when_schtasks_writes_it() {
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "ERROR: access is denied.".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_console_output(&utf16), "ERROR: access is denied.");
        assert_eq!(
            decode_console_output(&utf16[2..]),
            "ERROR: access is denied."
        );
        assert_eq!(decode_console_output(b"ERROR: plain"), "ERROR: plain");
    }

    #[test]
    fn install_log_messages_collapse_to_one_line() {
        assert_eq!(
            single_line("first\r\n  second\tthird\n"),
            "first second third"
        );
        assert_eq!(single_line("  "), "");
    }

    #[test]
    fn copy_file_unless_same_is_a_noop_for_identical_paths() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("helper.bin");
        fs::write(&path, b"payload").expect("write");
        copy_file_unless_same(&path, &path).expect("same-file copy");
        assert_eq!(fs::read(&path).expect("read"), b"payload");

        let other = directory.path().join("other.bin");
        copy_file_unless_same(&path, &other).expect("distinct copy");
        assert_eq!(fs::read(&other).expect("copied"), b"payload");
    }

    #[cfg(windows)]
    #[test]
    fn copy_file_unless_same_skips_an_identical_write_locked_destination() {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_SHARE_READ: u32 = 1;
        let directory = tempfile::tempdir().expect("tempdir");
        let source = directory.path().join("source.bin");
        let destination = directory.path().join("destination.bin");
        fs::write(&source, b"same executable bytes").expect("source");
        fs::write(&destination, b"same executable bytes").expect("destination");
        let _write_lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&destination)
            .expect("read-shared write lock");

        copy_file_unless_same(&source, &destination)
            .expect("identical locked destination should not be overwritten");
    }

    #[test]
    fn generation_allowlist_accepts_client_uuid_files_and_rejects_paths() {
        let id = "11111111-1111-1111-1111-111111111111";
        assert!(is_allowed_generation_file("config.yaml"));
        assert!(is_allowed_generation_file("iran-cdn-networks.txt"));
        assert!(is_allowed_generation_file(&format!(
            "custom-{id}-domains.txt"
        )));
        assert!(is_allowed_generation_file(&format!("custom-{id}-ips.txt")));
        assert!(!is_allowed_generation_file("custom-not-a-uuid-domains.txt"));
        assert!(!is_allowed_generation_file("../config.yaml"));
        assert!(!is_allowed_generation_file("custom/evil-domains.txt"));
    }

    #[test]
    fn overlay_replaces_workdir_files_without_renaming_the_directory() {
        let directory = tempfile::tempdir().expect("tempdir");
        let running = directory.path().join("running");
        let next = directory.path().join("next");
        fs::create_dir(&running).expect("running");
        fs::create_dir(&next).expect("next");
        fs::write(running.join("config.yaml"), b"old").expect("old config");
        fs::write(running.join("private.txt"), b"old-private").expect("old private");
        fs::write(next.join("config.yaml"), b"new").expect("new config");
        fs::write(next.join("private.txt"), b"new-private").expect("new private");
        overlay_generation_files(&next, &running).expect("overlay");
        assert_eq!(
            fs::read_to_string(running.join("config.yaml")).expect("cfg"),
            "new"
        );
        assert_eq!(
            fs::read_to_string(running.join("private.txt")).expect("private"),
            "new-private"
        );
    }

    // A Windows counterpart belongs here, but every assertion about Windows
    // path semantics has to be executed on Windows to be trusted. Add it once
    // `pnpm github:action-test` can run the windows-2025 test job (ADR 0031).
}
