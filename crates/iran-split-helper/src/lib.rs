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
    sync::{Arc, LazyLock},
    time::Duration,
};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};
use uuid::Uuid;

const PROCESS_STOP_TIMEOUT: Duration = Duration::from_secs(10);
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
    #[error("helper configuration I/O failed: {0}")]
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
    registered: Mutex<HashMap<Uuid, String>>,
    logs: Arc<Mutex<VecDeque<ServiceLogEntry>>>,
    pub(crate) side_tunnels: Mutex<HashMap<Uuid, openvpn::RunningSideTunnel>>,
}

impl Supervisor {
    #[must_use]
    pub fn new(settings: HelperSettings) -> Self {
        Self {
            settings,
            child: Mutex::new(None),
            registered: Mutex::new(HashMap::new()),
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES))),
            side_tunnels: Mutex::new(HashMap::new()),
        }
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
        let mut child = spawn_mihomo(&self.settings, &generation_root, &config_path)?;
        if let Some(stdout) = child.stdout.take() {
            capture_lines(stdout, Arc::clone(&self.logs), "info");
        }
        if let Some(stderr) = child.stderr.take() {
            capture_lines(stderr, Arc::clone(&self.logs), "warn");
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
            started_at: now_string(),
        };
        let status = process_status(Some(&managed));
        *current = Some(managed);
        drop(current);
        self.push_log("info", "mihomo_started", BTreeMap::new())
            .await;
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
        let mut current = self.child.lock().await;
        if let Some(managed) = current.as_mut() {
            stop_managed_child(managed).await?;
        }
        *current = None;
        drop(current);
        self.push_log("info", "mihomo_stopped", BTreeMap::new())
            .await;
        Ok(ProcessStatus {
            running: false,
            pid: None,
            generation_id: None,
            started_at: None,
        })
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
        self.stop_all_side_tunnels().await;
        let process_stopped = !self.stop().await?.running;
        let interface_path = Path::new("/sys/class/net").join(&self.settings.tun_name);
        if interface_path.exists() {
            #[cfg(unix)]
            delete_owned_interface(&self.settings.tun_name).await?;
            #[cfg(not(unix))]
            delete_owned_interface(&self.settings.tun_name);
        }
        let tun_removed = !interface_path.exists();
        let warnings = if tun_removed {
            Vec::new()
        } else {
            vec![format!(
                "owned interface {} remains after cleanup",
                self.settings.tun_name
            )]
        };
        let report = CleanupReport {
            process_stopped,
            tun_removed,
            routes_removed: 0,
            dns_restored: true,
            warnings,
        };
        self.push_log("info", "network_cleanup_finished", BTreeMap::new())
            .await;
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

/// Copies `source` onto `destination` unless they already resolve to the same
/// file. Windows `fs::copy` onto self fails; a leftover `ProgramData` helper
/// used as the elevate source hits that path.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn copy_file_unless_same(
    source: &Path,
    destination: &Path,
) -> Result<(), HelperServiceError> {
    if paths_refer_to_same_file(source, destination) {
        return Ok(());
    }
    fs::copy(source, destination)?;
    Ok(())
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

fn capture_lines<R>(reader: R, logs: Arc<Mutex<VecDeque<ServiceLogEntry>>>, level: &'static str)
where
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
                fields: BTreeMap::from([("message".into(), redact(&line))]),
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

#[cfg(unix)]
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

#[cfg(not(unix))]
fn delete_owned_interface(_name: &str) {}

mod commands;

#[cfg(unix)]
mod linux;

#[cfg(unix)]
pub use linux::run_linux;

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
        delete_owned_interface("unused");
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
