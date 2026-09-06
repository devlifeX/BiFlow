#![cfg(windows)]
//! Windows platform backend.
//!
//! Mirrors `iran-split-platform-linux` step for step. The privileged half runs
//! as the SYSTEM scheduled task from ADR 0029 and is reached over the versioned
//! local named pipe instead of a Unix socket; everything above that — runtime
//! generation staging, Mihomo validation, readiness probes — is the same
//! sequence so both platforms fail in the same places for the same reasons.

use async_trait::async_trait;
use iran_split_clients::{
    local_proxy_endpoint, synthesized_local_handle, ClientDriver, EgressHandle, OpenVpnDriver,
};
use iran_split_config::{
    AppConfig, ClientConfig, ClientInstance, EgressKind, ExecutableSetting, PresetId,
};
use iran_split_core::{
    CleanupReport, ClientComponentStatus, ComponentPhase, ComponentStatus, CoreError, HelperStatus,
    PlatformBackend, ProcessStatus, ProviderSummary, ReadinessReport, RuntimeGeneration,
    RuntimeHealth, TunStatus,
};
use iran_split_ipc::{
    read_frame, validate_envelope, write_frame, Envelope, HelperCommand, HelperReply,
    PROTOCOL_VERSION,
};
use iran_split_mihomo::{
    generate_config_with_handles, probe_hiddify_egress, validate_with_binary, ControllerClient,
    MihomoError, Platform, RuntimePaths,
};
use iran_split_rules::{DirectTarget, Outbound, RoutePinsDocument};
use std::{
    fs,
    io::{self, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::{
    net::{
        windows::named_pipe::{ClientOptions, NamedPipeClient},
        TcpStream,
    },
    process::{Child, Command},
    sync::{Mutex, RwLock},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;
use windows::Win32::Foundation::ERROR_PIPE_BUSY;

mod system_proxy;

pub const HELPER_PIPE: &str = r"\\.\pipe\iran-split-helper-v1";

const IPC_TIMEOUT: Duration = Duration::from_secs(5);
const PIPE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const PIPE_RETRY_DELAY: Duration = Duration::from_millis(50);
const EGRESS_PROBE_BUDGET: Duration = Duration::from_secs(45);
const READINESS_BUDGET: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
pub enum WindowsBackendError {
    #[error("helper IPC failed: {0}")]
    Protocol(#[from] iran_split_ipc::ProtocolError),
    #[error("helper connection failed: {0}")]
    Io(#[from] io::Error),
    #[error("helper request timed out")]
    Timeout,
    #[error("helper response did not match the request")]
    ResponseMismatch,
    #[error("helper returned {code}: {message}")]
    Helper { code: String, message: String },
}

#[derive(Debug, Clone)]
pub struct HelperClient {
    pipe_name: String,
}

impl HelperClient {
    #[must_use]
    pub fn new(pipe_name: impl Into<String>) -> Self {
        Self {
            pipe_name: pipe_name.into(),
        }
    }

    /// Sends one validated command to the privileged helper.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, connection, protocol negotiation, or
    /// the helper operation fails.
    pub async fn request(
        &self,
        command: HelperCommand,
    ) -> Result<HelperReply, WindowsBackendError> {
        let command_name = command.audit_name();
        let request_id = Uuid::new_v4();
        info!(
            event = "helper.request_started",
            section = "helper_ipc",
            initiator = "windows_platform_backend",
            cause = "backend_operation",
            trace_id = %request_id,
            trace_route = "desktop_engine->windows_platform_backend->helper_ipc",
            command = command_name,
            "helper request started"
        );
        if let Err(cause) = command.validate() {
            error!(
                event = "helper.request_failed",
                section = "helper_ipc",
                initiator = "windows_platform_backend",
                cause = %cause,
                trace_id = %request_id,
                trace_route = "desktop_engine->windows_platform_backend->helper_ipc->validation",
                command = command_name,
                "helper request validation failed"
            );
            return Err(cause.into());
        }
        let result = self.request_validated(command).await;
        match &result {
            Ok(_) => info!(
                event = "helper.request_completed",
                section = "helper_ipc",
                initiator = "windows_platform_backend",
                cause = "none",
                trace_id = %request_id,
                trace_route = "desktop_engine->windows_platform_backend->helper_ipc->reply",
                command = command_name,
                "helper request completed"
            ),
            Err(cause) => error!(
                event = "helper.request_failed",
                section = "helper_ipc",
                initiator = "windows_platform_backend",
                cause = %cause,
                trace_id = %request_id,
                trace_route = "desktop_engine->windows_platform_backend->helper_ipc->error",
                command = command_name,
                "helper request failed"
            ),
        }
        result
    }

    async fn request_validated(
        &self,
        command: HelperCommand,
    ) -> Result<HelperReply, WindowsBackendError> {
        let mut pipe = self.connect().await?;
        let hello = Envelope::new(HelperCommand::Hello {
            client_version: env!("CARGO_PKG_VERSION").into(),
            supported_protocols: vec![PROTOCOL_VERSION],
        });
        let hello_reply = exchange(&mut pipe, &hello).await?;
        match hello_reply.payload {
            HelperReply::Hello(reply) if reply.selected_protocol == PROTOCOL_VERSION => {}
            HelperReply::Error(error) => {
                return Err(WindowsBackendError::Helper {
                    code: error.code,
                    message: error.message,
                });
            }
            _ => return Err(WindowsBackendError::ResponseMismatch),
        }
        let request = Envelope::new(command);
        let response = exchange(&mut pipe, &request).await?;
        match response.payload {
            HelperReply::Error(error) => Err(WindowsBackendError::Helper {
                code: error.code,
                message: error.message,
            }),
            reply => Ok(reply),
        }
    }

    /// `ClientOptions::open` returns a synchronous result, not a future, so the
    /// retry loop lives inside a timeout. Only `ERROR_PIPE_BUSY` is retried;
    /// any other error means the helper is not serving the pipe.
    async fn connect(&self) -> Result<NamedPipeClient, WindowsBackendError> {
        tokio::time::timeout(PIPE_CONNECT_TIMEOUT, async {
            loop {
                match ClientOptions::new().open(self.pipe_name.as_str()) {
                    Ok(pipe) => return Ok(pipe),
                    Err(error) if is_pipe_busy(&error) => {
                        tokio::time::sleep(PIPE_RETRY_DELAY).await;
                    }
                    Err(error) => return Err(WindowsBackendError::Io(error)),
                }
            }
        })
        .await
        .map_err(|_| WindowsBackendError::Timeout)?
    }
}

fn is_pipe_busy(error: &io::Error) -> bool {
    error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
        == Some(ERROR_PIPE_BUSY.0)
}

/// The helper never serves the pipe when it is not installed, and a missing
/// pipe must read as "not installed" rather than an error banner.
fn is_helper_absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

async fn exchange(
    pipe: &mut NamedPipeClient,
    request: &Envelope<HelperCommand>,
) -> Result<Envelope<HelperReply>, WindowsBackendError> {
    tokio::time::timeout(IPC_TIMEOUT, write_frame(pipe, request))
        .await
        .map_err(|_| WindowsBackendError::Timeout)??;
    let reply: Envelope<HelperReply> = tokio::time::timeout(IPC_TIMEOUT, read_frame(pipe))
        .await
        .map_err(|_| WindowsBackendError::Timeout)??;
    validate_envelope(&reply)?;
    if reply.request_id != request.request_id {
        return Err(WindowsBackendError::ResponseMismatch);
    }
    Ok(reply)
}

#[derive(Debug, Clone)]
pub struct WindowsPaths {
    pub pipe_name: String,
    pub user_data_dir: PathBuf,
    pub system_runtime_dir: PathBuf,
    /// Directory the SYSTEM helper reads in `register_runtime_generation`.
    /// Packaged installs use `C:\ProgramData\iran-split\staging` so NSIS and
    /// the unelevated desktop share one machine-wide root (ADR 0064).
    pub generation_staging_dir: PathBuf,
    pub resources_dir: PathBuf,
    pub rules_cache_dir: PathBuf,
    pub mihomo_binary: PathBuf,
}

#[derive(Debug, Clone)]
struct PreparedGeneration {
    generation: RuntimeGeneration,
    config_path: PathBuf,
}

#[derive(Debug)]
pub struct WindowsBackend {
    config: Arc<RwLock<AppConfig>>,
    helper: HelperClient,
    paths: WindowsPaths,
    prepared: Mutex<Option<PreparedGeneration>>,
    launched_hiddify: Mutex<Option<Child>>,
    egress_exit_ip: Mutex<Option<String>>,
    egress_handles: Mutex<Vec<EgressHandle>>,
    side_tunnel_auth_files: Mutex<Vec<NamedTempFile>>,
    launched_clients: Mutex<Vec<Child>>,
    client_exit_ips: Mutex<std::collections::HashMap<iran_split_config::ClientId, String>>,
    /// Why each client failed at its last start attempt; mirrors Linux so a
    /// stopped card explains itself instead of showing no detail.
    client_failures: Mutex<std::collections::HashMap<iran_split_config::ClientId, String>>,
}

impl WindowsBackend {
    #[must_use]
    pub fn new(config: AppConfig, paths: WindowsPaths) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            helper: HelperClient::new(paths.pipe_name.clone()),
            paths,
            prepared: Mutex::new(None),
            launched_hiddify: Mutex::new(None),
            egress_exit_ip: Mutex::new(None),
            egress_handles: Mutex::new(Vec::new()),
            side_tunnel_auth_files: Mutex::new(Vec::new()),
            launched_clients: Mutex::new(Vec::new()),
            client_exit_ips: Mutex::new(std::collections::HashMap::new()),
            client_failures: Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub async fn update_config(&self, config: AppConfig) {
        *self.config.write().await = config;
    }

    /// Reads at most `maximum` recent helper service log entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the helper exchange fails or returns an unexpected
    /// reply variant.
    pub async fn service_logs(
        &self,
        maximum: u16,
    ) -> Result<Vec<iran_split_ipc::ServiceLogEntry>, WindowsBackendError> {
        match self
            .helper
            .request(HelperCommand::CollectServiceLogs {
                max_entries: maximum,
            })
            .await?
        {
            HelperReply::Logs(logs) => Ok(logs),
            _ => Err(WindowsBackendError::ResponseMismatch),
        }
    }

    async fn helper_request(&self, command: HelperCommand) -> Result<HelperReply, CoreError> {
        self.helper
            .request(command)
            .await
            .map_err(|error| CoreError::Platform(error.to_string()))
    }

    async fn start_openvpn_client(
        &self,
        client: &ClientInstance,
        cancel: CancellationToken,
    ) -> Result<EgressHandle, CoreError> {
        let mut handle = OpenVpnDriver
            .ensure(client, cancel)
            .await
            .map_err(|error| CoreError::ConfigInvalid(error.to_string()))?;
        let ClientConfig::OwnedSideTunnel {
            profile_path,
            executable,
            username,
            password,
            start_timeout_seconds,
        } = &client.config
        else {
            return Err(CoreError::ConfigInvalid(
                "openvpn instance is not a side tunnel".into(),
            ));
        };
        let profile = profile_path
            .clone()
            .ok_or_else(|| CoreError::ConfigInvalid("openvpn profile path is missing".into()))?;
        let executable = match executable {
            ExecutableSetting::Auto => None,
            ExecutableSetting::Path(path) => Some(path.clone()),
        };
        let auth_file = write_side_tunnel_auth(username.as_deref(), password.as_deref())?;
        let result = self
            .helper_request(HelperCommand::StartSideTunnel {
                driver: "openvpn".into(),
                client_id: client.id.into(),
                profile,
                executable,
                auth_file: auth_file.as_ref().map(|file| file.path().to_path_buf()),
                timeout_seconds: *start_timeout_seconds,
            })
            .await?;
        // OpenVPN re-reads the auth file on soft restarts, so it must outlive
        // the start call; it is dropped (deleted) on cleanup.
        if let Some(file) = auth_file {
            self.side_tunnel_auth_files.lock().await.push(file);
        }
        match result {
            HelperReply::SideTunnel(status) if status.running => {
                if let Some(outbound) = handle.outbound.as_mut() {
                    outbound.interface_name = status.device;
                    outbound.routing_mark = status.routing_mark;
                }
                handle.ready = true;
                Ok(handle)
            }
            HelperReply::SideTunnel(_) => {
                Err(CoreError::Platform("side tunnel did not start".into()))
            }
            _ => Err(CoreError::Platform("unexpected side tunnel reply".into())),
        }
    }

    async fn ensure_enabled_clients(&self, cancel: CancellationToken) -> Result<(), CoreError> {
        let config = self.config.read().await.clone();
        *self.egress_exit_ip.lock().await = None;
        self.client_exit_ips.lock().await.clear();
        let mut handles = Vec::new();
        for client in config.enabled_clients() {
            let required = config.default_route.client_id() == Some(client.id);
            let started = if client.preset == PresetId::Hiddify {
                self.start_hiddify_client(client, required, &cancel).await
            } else {
                match client.spec().kind {
                    EgressKind::LocalProxy => {
                        self.start_local_proxy_client(client, required, &cancel)
                            .await
                    }
                    EgressKind::OwnedSideTunnel => {
                        self.start_openvpn_client(client, cancel.clone()).await
                    }
                    EgressKind::Unsupported => Err(CoreError::ConfigInvalid(
                        "this catalog entry cannot be started".into(),
                    )),
                }
            };
            match started {
                Ok(handle) => {
                    self.client_failures.lock().await.remove(&client.id);
                    handles.push(handle);
                }
                Err(CoreError::Cancelled) => return Err(CoreError::Cancelled),
                Err(error) if !required => {
                    self.client_failures
                        .lock()
                        .await
                        .insert(client.id, error.to_string());
                    warn!(
                        event = "client.ensure_failed",
                        section = "clients",
                        initiator = "ensure_clients",
                        cause = %error,
                        trace_route = "engine_operation->windows_platform_backend->ensure_clients",
                        "optional client failed; connect continues"
                    );
                }
                Err(error) => {
                    self.client_failures
                        .lock()
                        .await
                        .insert(client.id, error.to_string());
                    return Err(error);
                }
            }
        }
        *self.egress_handles.lock().await = handles;
        Ok(())
    }

    /// Live recovery for local proxies that were dead at connect time
    /// (ADR 0076). Mirrors the Linux backend: one egress re-check per health
    /// tick once the client's port answers; on success its handle joins
    /// routing and the engine hot-applies the runtime config.
    async fn recover_local_proxy_clients(&self) -> Result<bool, CoreError> {
        let config = self.config.read().await.clone();
        let handles = self.egress_handles.lock().await.clone();
        let mut recovered = false;
        for client in clients_missing_egress(&config, &handles) {
            let Some((host, port)) = local_proxy_endpoint(client) else {
                continue;
            };
            if !Self::tcp_listening(&host, port).await {
                continue;
            }
            match probe_hiddify_egress(&host, port, Duration::from_secs(3)).await {
                Ok(exit_ip) => {
                    let Some(handle) = synthesized_local_handle(client) else {
                        continue;
                    };
                    info!(
                        event = "client.recovered_egress",
                        section = "clients",
                        initiator = "recover_clients",
                        cause = "egress_probe_succeeded",
                        trace_route = "engine->windows_platform_backend->recover_clients",
                        client = client.spec().id,
                        "a local proxy egress became reachable after connect"
                    );
                    self.client_exit_ips.lock().await.insert(client.id, exit_ip);
                    self.egress_handles.lock().await.push(handle);
                    recovered = true;
                }
                Err(error) => info!(
                    event = "client.recover_probe_failed",
                    section = "clients",
                    initiator = "recover_clients",
                    cause = %error,
                    trace_route = "engine->windows_platform_backend->recover_clients",
                    client = client.spec().id,
                    "local proxy port answers but its egress is not usable yet"
                ),
            }
        }
        Ok(recovered)
    }

    async fn hiddify_listening(config: &AppConfig) -> bool {
        let (host, port) = config.hiddify_endpoint();
        Self::tcp_listening(&host, port).await
    }

    async fn tcp_listening(host: &str, port: u16) -> bool {
        tokio::time::timeout(Duration::from_millis(750), TcpStream::connect((host, port)))
            .await
            .is_ok_and(|result| result.is_ok())
    }

    #[must_use]
    pub fn discover_hiddify(config: &AppConfig, data: &Path) -> Option<PathBuf> {
        if let ExecutableSetting::Path(path) = &config.hiddify_executable() {
            return path.is_file().then(|| path.clone());
        }
        Self::hiddify_candidates(data)
            .into_iter()
            .find(|path| path.is_file())
    }

    /// Kept in step with `deps::hiddify_candidates` in the desktop crate so the
    /// dependency card and the backend agree on where Hiddify is installed.
    #[must_use]
    pub fn hiddify_candidates(data: &Path) -> Vec<PathBuf> {
        let mut candidates = vec![
            data.join("apps/Hiddify/Hiddify.exe"),
            data.join("apps/Hiddify/hiddify.exe"),
            data.join("bin/hiddify.exe"),
        ];
        // The installed app keeps its managed Hiddify under the production
        // data dir; a dev run's isolated profile must still find it.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            candidates.push(local.join("biflow/apps/Hiddify/Hiddify.exe"));
            candidates.push(local.join("biflow/bin/hiddify.exe"));
        }
        for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(variable) {
                let root = PathBuf::from(root);
                candidates.push(root.join("Hiddify/Hiddify.exe"));
                candidates.push(root.join("Hiddify/hiddify.exe"));
                candidates.push(root.join("Programs/Hiddify/Hiddify.exe"));
                candidates.push(root.join("HiddifyNext/Hiddify.exe"));
            }
        }
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                candidates.push(directory.join("Hiddify.exe"));
                candidates.push(directory.join("hiddify.exe"));
            }
        }
        candidates
    }

    fn helper_component(result: Result<HelperStatus, CoreError>) -> ComponentStatus {
        match result {
            Ok(status) if status.available && status.authorized => ComponentStatus::new(
                ComponentPhase::Running,
                status
                    .version
                    .map(|version| format!("Helper {version} is ready")),
            ),
            Ok(status) if status.available => ComponentStatus::new(
                ComponentPhase::Degraded,
                Some("Helper is running but this user is not authorized".into()),
            ),
            Ok(_) => ComponentStatus::new(
                ComponentPhase::Unavailable,
                Some("Helper service is not installed or running".into()),
            ),
            Err(error) => ComponentStatus::new(ComponentPhase::Error, Some(error.to_string())),
        }
    }

    fn hiddify_component(
        config: &AppConfig,
        listening: bool,
        executable: Option<&Path>,
    ) -> ComponentStatus {
        if listening {
            ComponentStatus::new(
                ComponentPhase::Running,
                Some(format!(
                    "Listening on {}:{}",
                    config.hiddify_endpoint().0,
                    config.hiddify_endpoint().1
                )),
            )
        } else if let Some(path) = executable {
            ComponentStatus::new(
                ComponentPhase::Stopped,
                Some(format!("Installed at {}", path.display())),
            )
        } else {
            ComponentStatus::new(
                ComponentPhase::Unavailable,
                Some("Hiddify is not installed and its local proxy is not listening".into()),
            )
        }
    }

    async fn mihomo_component(
        config: &AppConfig,
        controller_listening: bool,
        executable: Option<&Path>,
    ) -> (ComponentStatus, ProviderSummary) {
        if !controller_listening {
            let component = executable.map_or_else(
                || {
                    ComponentStatus::new(
                        ComponentPhase::Unavailable,
                        Some("Mihomo is not installed and its controller is not listening".into()),
                    )
                },
                |path| {
                    ComponentStatus::new(
                        ComponentPhase::Stopped,
                        Some(format!("Installed at {}", path.display())),
                    )
                },
            );
            return (component, ProviderSummary::default());
        }
        let Some(controller) = Self::controller(config) else {
            return (
                ComponentStatus::new(
                    ComponentPhase::Error,
                    Some("Mihomo controller address is invalid".into()),
                ),
                ProviderSummary::default(),
            );
        };
        match controller.version().await {
            Ok(version) => {
                let providers = controller.provider_summary().await.map_or_else(
                    |_| ProviderSummary::default(),
                    |summary| ProviderSummary {
                        ready: summary.ready,
                        total: summary.total,
                        rules_loaded: summary.rules_loaded,
                        last_refresh: Some(chrono::Utc::now()),
                    },
                );
                (
                    ComponentStatus::new(
                        ComponentPhase::Running,
                        Some(format!("Controller {} is ready", version.version)),
                    ),
                    providers,
                )
            }
            Err(error) => (
                ComponentStatus::new(
                    ComponentPhase::Degraded,
                    Some(format!(
                        "Controller port is active but BiFlow cannot authenticate: {error}"
                    )),
                ),
                ProviderSummary::default(),
            ),
        }
    }

    fn controller(config: &AppConfig) -> Option<ControllerClient> {
        ControllerClient::new(
            &config.mihomo.controller_host,
            config.mihomo.controller_port,
            config.mihomo.controller_secret.clone(),
        )
        .ok()
    }

    /// Windows has no `/sys/class/net`, and enumerating adapters needs Win32
    /// calls this crate cannot make under `unsafe_code = "forbid"`. Mihomo owns
    /// the Wintun adapter, so its own running config is the authority on
    /// whether the tunnel is up.
    async fn tun_active(config: &AppConfig) -> bool {
        let Some(controller) = Self::controller(config) else {
            return false;
        };
        match controller.configs().await {
            Ok(configs) => {
                let active = tun_enabled(&configs, &config.mihomo.tun_name);
                if !active {
                    warn!(
                        event = "mihomo.tun_inactive",
                        section = "runtime_health",
                        initiator = "windows_platform_backend",
                        cause = "controller_configs",
                        tun_enable = ?configs.get("tun").and_then(|tun| tun.get("enable")),
                        tun_device = configs
                            .get("tun")
                            .and_then(|tun| tun.get("device"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(""),
                        expected_device = %config.mihomo.tun_name,
                        trace_route = "desktop_engine->windows_platform_backend->mihomo_controller",
                        "Mihomo /configs did not report an enabled TUN"
                    );
                }
                active
            }
            Err(error) => {
                warn!(
                    event = "mihomo.tun_configs_failed",
                    section = "runtime_health",
                    initiator = "windows_platform_backend",
                    cause = %error,
                    trace_route = "desktop_engine->windows_platform_backend->mihomo_controller",
                    "could not read Mihomo /configs for TUN state"
                );
                false
            }
        }
    }

    fn tun_component(tun_name: &str, active: bool) -> ComponentStatus {
        ComponentStatus::new(
            if active {
                ComponentPhase::Running
            } else {
                ComponentPhase::Stopped
            },
            Some(if active {
                format!("Interface {tun_name} is active")
            } else {
                format!("Interface {tun_name} is absent")
            }),
        )
    }

    fn dns_component(port: u16, listening: bool) -> ComponentStatus {
        ComponentStatus::new(
            if listening {
                ComponentPhase::Running
            } else {
                ComponentPhase::Stopped
            },
            Some(if listening {
                format!("DNS is listening on 127.0.0.1:{port}")
            } else {
                format!("No DNS listener on 127.0.0.1:{port}")
            }),
        )
    }

    async fn prepared(&self) -> Result<PreparedGeneration, CoreError> {
        self.prepared
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::ConfigInvalid("runtime has not been prepared".into()))
    }

    async fn launch_hiddify_if_needed(
        &self,
        config: &AppConfig,
        cancel: &CancellationToken,
    ) -> Result<(), CoreError> {
        if Self::hiddify_listening(config).await {
            return Ok(());
        }
        let executable = Self::discover_hiddify(config, &self.paths.user_data_dir)
            .ok_or(CoreError::HiddifyNotFound)?;
        let child = Command::new(executable)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .spawn()
            .map_err(|error| CoreError::Platform(error.to_string()))?;
        *self.launched_hiddify.lock().await = Some(child);
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(config.hiddify_start_timeout());
        loop {
            if Self::hiddify_listening(config).await {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(CoreError::HiddifyEgressUnavailable);
            }
            tokio::select! {
                () = cancel.cancelled() => return Err(CoreError::Cancelled),
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    }

    async fn client_component(
        client: &ClientInstance,
        handles: &[EgressHandle],
        last_failure: Option<&String>,
    ) -> ComponentStatus {
        if !client.enabled {
            return ComponentStatus::new(ComponentPhase::Unavailable, None);
        }
        match client.spec().kind {
            EgressKind::LocalProxy => match local_proxy_endpoint(client) {
                Some((host, port)) if Self::tcp_listening(&host, port).await => {
                    ComponentStatus::new(
                        ComponentPhase::Running,
                        Some(format!("Listening on {host}:{port}")),
                    )
                }
                Some((host, port)) => ComponentStatus::new(
                    ComponentPhase::Stopped,
                    Some(last_failure.cloned().unwrap_or_else(|| {
                        format!(
                            "nothing is listening on {host}:{port}; check the port on the client card"
                        )
                    })),
                ),
                None => ComponentStatus::new(
                    ComponentPhase::Stopped,
                    Some("local proxy is not listening".into()),
                ),
            },
            EgressKind::OwnedSideTunnel => {
                if handles
                    .iter()
                    .any(|handle| handle.client_id == client.id && handle.ready)
                {
                    ComponentStatus::new(ComponentPhase::Running, None)
                } else {
                    ComponentStatus::new(
                        ComponentPhase::Stopped,
                        last_failure
                            .cloned()
                            .or_else(|| iran_split_clients::side_tunnel_stopped_reason(client)),
                    )
                }
            }
            EgressKind::Unsupported => ComponentStatus::new(ComponentPhase::Unavailable, None),
        }
    }

    async fn start_hiddify_client(
        &self,
        client: &ClientInstance,
        required: bool,
        cancel: &CancellationToken,
    ) -> Result<EgressHandle, CoreError> {
        let config = self.config.read().await.clone();
        self.launch_hiddify_if_needed(&config, cancel).await?;
        let exit_ip = if required {
            self.probe_hiddify_until_ready(&config, cancel.clone())
                .await?
        } else {
            // An optional Hiddify must not hold Connect for the 45s retry
            // window; one quick probe decides and ADR 0076 recovery attaches
            // it later once it really serves.
            let (host, port) = config.hiddify_endpoint();
            probe_hiddify_egress(&host, port, Duration::from_secs(3))
                .await
                .map_err(|error| {
                    CoreError::Platform(format!(
                        "hiddify egress probe failed on {host}:{port}: {error}"
                    ))
                })?
        };
        self.client_exit_ips
            .lock()
            .await
            .insert(client.id, exit_ip.clone());
        if required {
            *self.egress_exit_ip.lock().await = Some(exit_ip);
        }
        synthesized_local_handle(client)
            .ok_or_else(|| CoreError::ConfigInvalid("hiddify handle is missing".into()))
    }

    async fn start_local_proxy_client(
        &self,
        client: &ClientInstance,
        required: bool,
        cancel: &CancellationToken,
    ) -> Result<EgressHandle, CoreError> {
        let Some((host, port)) = local_proxy_endpoint(client) else {
            return Err(CoreError::ConfigInvalid(
                "local proxy handle is missing".into(),
            ));
        };
        if !Self::tcp_listening(&host, port).await {
            if required {
                self.launch_local_proxy_if_needed(client, &host, port, cancel)
                    .await?;
            } else {
                // An optional client must not block Connect while its port
                // opens. Launch it and let ADR 0076 recovery attach the
                // egress once it actually serves.
                self.spawn_local_proxy(client).await?;
                return Err(CoreError::Platform(format!(
                    "{} was launched in the background; its egress joins routing once it serves",
                    client.spec().id
                )));
            }
        }
        // ADR 0018: every local-proxy egress is verified before the TUN starts,
        // so pinned or MATCH traffic cannot blackhole into a dead proxy.
        let exit_ip = probe_hiddify_egress(&host, port, Duration::from_secs(3))
            .await
            .map_err(|error| {
                CoreError::Platform(format!(
                    "{} egress probe failed on {host}:{port}: {error}",
                    client.spec().id
                ))
            })?;
        self.client_exit_ips
            .lock()
            .await
            .insert(client.id, exit_ip.clone());
        if required {
            *self.egress_exit_ip.lock().await = Some(exit_ip);
        }
        synthesized_local_handle(client)
            .ok_or_else(|| CoreError::ConfigInvalid("local proxy handle is missing".into()))
    }

    /// Resolves and spawns a local-proxy binary without waiting for its port:
    /// configured path first, then the preset's process names on PATH.
    async fn spawn_local_proxy(&self, client: &ClientInstance) -> Result<(), CoreError> {
        let ClientConfig::LocalProxy { executable, .. } = &client.config else {
            return Err(CoreError::ConfigInvalid(
                "client is not a local proxy".into(),
            ));
        };
        let resolved = match executable {
            ExecutableSetting::Path(path) => path.is_file().then(|| path.clone()),
            ExecutableSetting::Auto => discover_local_proxy_binary(&client.spec()),
        };
        let Some(binary) = resolved else {
            return Err(CoreError::Platform(format!(
                "{} is not running and its executable was not found; start it once or set its path on the client card",
                client.spec().id
            )));
        };
        let child = Command::new(binary)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .spawn()
            .map_err(|error| CoreError::Platform(error.to_string()))?;
        self.launched_clients.lock().await.push(child);
        Ok(())
    }

    /// Launches a required local-proxy client that is not listening yet and
    /// waits until the port answers (the default-route egress must be
    /// verified before the TUN starts).
    async fn launch_local_proxy_if_needed(
        &self,
        client: &ClientInstance,
        host: &str,
        port: u16,
        cancel: &CancellationToken,
    ) -> Result<(), CoreError> {
        let ClientConfig::LocalProxy {
            start_timeout_seconds,
            ..
        } = &client.config
        else {
            return Err(CoreError::ConfigInvalid(
                "client is not a local proxy".into(),
            ));
        };
        self.spawn_local_proxy(client).await?;
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs((*start_timeout_seconds).max(1));
        loop {
            if Self::tcp_listening(host, port).await {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(CoreError::Platform(format!(
                    "{} was launched but its local port did not open in time",
                    client.spec().id
                )));
            }
            tokio::select! {
                () = cancel.cancelled() => return Err(CoreError::Cancelled),
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    }

    async fn probe_hiddify_until_ready(
        &self,
        config: &AppConfig,
        cancel: CancellationToken,
    ) -> Result<String, CoreError> {
        let deadline = tokio::time::Instant::now() + EGRESS_PROBE_BUDGET;
        let mut last_cause;
        loop {
            if cancel.is_cancelled() {
                return Err(CoreError::Cancelled);
            }
            let (host, port) = config.hiddify_endpoint();
            match probe_hiddify_egress(&host, port, Duration::from_secs(2)).await {
                Ok(exit_ip) => {
                    info!(
                        event = "hiddify.egress_ready",
                        section = "hiddify_process",
                        initiator = "windows_platform_backend",
                        cause = "socks_probe",
                        trace_route = "desktop_engine->windows_platform_backend->hiddify_egress",
                        "Hiddify SOCKS egress is reachable"
                    );
                    return Ok(exit_ip);
                }
                Err(error) => {
                    last_cause = error.to_string();
                    warn!(
                        event = "hiddify.egress_probe_failed",
                        section = "hiddify_process",
                        initiator = "windows_platform_backend",
                        cause = %error,
                        trace_route = "desktop_engine->windows_platform_backend->hiddify_egress",
                        "Hiddify SOCKS egress probe failed; retrying before TUN starts"
                    );
                }
            }
            if tokio::time::Instant::now() >= deadline {
                error!(
                    event = "hiddify.egress_probe_exhausted",
                    section = "hiddify_process",
                    initiator = "windows_platform_backend",
                    cause = last_cause.as_str(),
                    trace_route = "desktop_engine->windows_platform_backend->hiddify_egress",
                    "Hiddify was listening but SOCKS egress did not become ready"
                );
                return Err(CoreError::HiddifyEgressUnavailable);
            }
            tokio::select! {
                () = cancel.cancelled() => return Err(CoreError::Cancelled),
                () = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
    }
}

#[async_trait]
impl PlatformBackend for WindowsBackend {
    async fn runtime_health(&self) -> RuntimeHealth {
        let config = self.config.read().await.clone();
        let hiddify_path = Self::discover_hiddify(&config, &self.paths.user_data_dir);
        let mihomo_path = self
            .paths
            .mihomo_binary
            .is_file()
            .then(|| self.paths.mihomo_binary.clone());
        let (helper_result, hiddify_listening, controller_listening, dns_listening) = tokio::join!(
            self.helper_status(),
            Self::hiddify_listening(&config),
            Self::tcp_listening(
                &config.mihomo.controller_host,
                config.mihomo.controller_port
            ),
            Self::tcp_listening(&config.mihomo.controller_host, config.mihomo.dns_port),
        );

        let helper = Self::helper_component(helper_result);
        let hiddify = Self::hiddify_component(&config, hiddify_listening, hiddify_path.as_deref());
        let (mihomo, providers) =
            Self::mihomo_component(&config, controller_listening, mihomo_path.as_deref()).await;
        let tun = Self::tun_component(
            &config.mihomo.tun_name,
            controller_listening && Self::tun_active(&config).await,
        );
        let dns = Self::dns_component(config.mihomo.dns_port, dns_listening);

        let handles = self.egress_handles.lock().await.clone();
        let exit_ips = self.client_exit_ips.lock().await.clone();
        let failures = self.client_failures.lock().await.clone();
        let mut clients = Vec::new();
        for client in &config.clients {
            let status = if client.preset == PresetId::Hiddify {
                hiddify.clone()
            } else {
                Self::client_component(client, &handles, failures.get(&client.id)).await
            };
            clients.push(ClientComponentStatus {
                id: client.id,
                preset: client.preset,
                enabled: client.enabled,
                status,
                exit_ip: exit_ips.get(&client.id).cloned(),
            });
        }

        RuntimeHealth {
            helper,
            clients,
            mihomo,
            tun,
            dns,
            providers,
        }
    }

    async fn helper_status(&self) -> Result<HelperStatus, CoreError> {
        match self.helper.request(HelperCommand::GetServiceStatus).await {
            Ok(HelperReply::ServiceStatus(status)) => Ok(HelperStatus {
                available: true,
                authorized: status.authorized,
                version: Some(status.helper_version),
            }),
            Ok(_) => Err(CoreError::Platform("unexpected helper status reply".into())),
            Err(WindowsBackendError::Io(error)) if is_helper_absent(&error) => {
                Ok(HelperStatus::default())
            }
            Err(WindowsBackendError::Timeout) => Ok(HelperStatus::default()),
            Err(error) => Err(CoreError::Platform(error.to_string())),
        }
    }

    async fn ensure_hiddify(&self, cancel: CancellationToken) -> Result<(), CoreError> {
        let config = self.config.read().await.clone();
        self.launch_hiddify_if_needed(&config, &cancel).await?;
        let exit_ip = self.probe_hiddify_until_ready(&config, cancel).await?;
        *self.egress_exit_ip.lock().await = Some(exit_ip);
        Ok(())
    }

    async fn ensure_clients(&self, cancel: CancellationToken) -> Result<(), CoreError> {
        self.ensure_enabled_clients(cancel).await
    }

    async fn recover_clients(&self) -> Result<bool, CoreError> {
        self.recover_local_proxy_clients().await
    }

    async fn probe_primary_egress(&self) -> Option<Result<(), String>> {
        let config = self.config.read().await.clone();
        let client = config.client(config.default_route.client_id()?)?;
        if !client.enabled || client.spec().kind != EgressKind::LocalProxy {
            return None;
        }
        let (host, port) = local_proxy_endpoint(client)?;
        Some(
            probe_hiddify_egress(&host, port, Duration::from_secs(3))
                .await
                .map(|_| ())
                .map_err(|error| {
                    format!(
                        "{} egress probe failed on {host}:{port}: {error}",
                        client.spec().id
                    )
                }),
        )
    }

    async fn prepare_runtime(&self) -> Result<RuntimeGeneration, CoreError> {
        let config = self.config.read().await.clone();
        let generation_id = Uuid::new_v4();
        let staging_root = self
            .paths
            .generation_staging_dir
            .join(generation_id.to_string());
        fs::create_dir_all(&staging_root).map_err(|error| platform_error(&error))?;
        let runtime_paths = RuntimePaths {
            private_networks: PathBuf::from("private.txt"),
            iran_domains: PathBuf::from("iran-domains.txt"),
            iran_business_domains: PathBuf::from("iran-business-domains.txt"),
            iran_networks: PathBuf::from("iran-networks.txt"),
            custom_direct_domains: PathBuf::from("custom-direct-domains.txt"),
            custom_direct_ips: PathBuf::from("custom-direct-ips.txt"),
        };
        let rules_path = self.paths.user_data_dir.join("direct-rules.json");
        let custom: RoutePinsDocument = if rules_path.exists() {
            serde_json::from_slice(&fs::read(rules_path).map_err(|error| platform_error(&error))?)
                .map_err(|error| CoreError::ConfigInvalid(error.to_string()))?
        } else {
            RoutePinsDocument::default()
        };
        let handles = self.egress_handles.lock().await.clone();
        let generated = generate_config_with_handles(
            &config,
            Platform::Windows,
            &runtime_paths,
            &custom,
            &handles,
        )
        .map_err(|error| CoreError::ConfigInvalid(error.to_string()))?;
        for name in [
            "private.txt",
            "iran-domains.txt",
            "iran-networks.txt",
            "iran-business-domains.txt",
        ] {
            copy_rule_file(
                &self.paths.resources_dir,
                &self.paths.rules_cache_dir,
                &staging_root,
                name,
            )?;
        }
        write_custom_provider_files(&staging_root, &custom, &config)?;
        write_atomic(&staging_root.join("config.yaml"), generated.yaml.as_bytes())?;
        let generation = RuntimeGeneration {
            generation_id,
            config_sha256: generated.sha256,
        };
        *self.prepared.lock().await = Some(PreparedGeneration {
            generation: generation.clone(),
            config_path: staging_root.join("config.yaml"),
        });
        info!(
            event = "runtime.generation_prepared",
            section = "runtime_generation",
            initiator = "windows_platform_backend",
            cause = "stack_start",
            trace_id = %generation_id,
            trace_route = "desktop_engine->windows_platform_backend->prepare_runtime",
            "runtime generation staged for the helper"
        );
        Ok(generation)
    }

    async fn validate_runtime(&self, generation: &RuntimeGeneration) -> Result<(), CoreError> {
        if !self.paths.mihomo_binary.is_file() {
            return Err(CoreError::MihomoNotFound);
        }
        let prepared = self.prepared().await?;
        if prepared.generation != *generation {
            return Err(CoreError::ConfigInvalid(
                "generation differs from the latest prepared runtime".into(),
            ));
        }
        validate_with_binary(
            &self.paths.mihomo_binary,
            &prepared.config_path,
            Duration::from_secs(10),
        )
        .await
        .map_err(|error| CoreError::ConfigInvalid(error.to_string()))
    }

    async fn start_core(&self, generation: &RuntimeGeneration) -> Result<(), CoreError> {
        match self
            .helper_request(HelperCommand::RegisterRuntimeGeneration {
                generation_id: generation.generation_id,
                config_sha256: generation.config_sha256.clone(),
            })
            .await?
        {
            HelperReply::GenerationRegistered { generation_id }
                if generation_id == generation.generation_id => {}
            _ => return Err(CoreError::Platform("generation registration failed".into())),
        }
        match self
            .helper_request(HelperCommand::StartMihomo {
                generation_id: generation.generation_id,
                config_sha256: generation.config_sha256.clone(),
            })
            .await?
        {
            HelperReply::ProcessStatus(status) if status.running => Ok(()),
            _ => Err(CoreError::MihomoStartFailed(
                "helper did not report a running process".into(),
            )),
        }
    }

    async fn stop_core(&self) -> Result<(), CoreError> {
        match self.helper_request(HelperCommand::StopMihomo).await? {
            HelperReply::ProcessStatus(status) if !status.running => Ok(()),
            _ => Err(CoreError::Platform("helper did not stop Mihomo".into())),
        }
    }

    async fn stop_user_proxy(&self) -> Result<(), CoreError> {
        let config = self.config.read().await.clone();
        if !config.hiddify_stop_with_stack() {
            return Ok(());
        }
        if let Some(mut child) = self.launched_hiddify.lock().await.take() {
            let trace_id = Uuid::new_v4();
            if let Err(cause) = child.start_kill() {
                warn!(
                    event = "hiddify.stop_signal_failed",
                    section = "hiddify_process",
                    initiator = "windows_platform_backend",
                    cause = %cause,
                    trace_id = %trace_id,
                    trace_route = "desktop_engine->windows_platform_backend->hiddify_process",
                    "could not send the stop signal to the Hiddify child process"
                );
            }
            match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
                Ok(Ok(status)) => info!(
                    event = "hiddify.process_stopped",
                    section = "hiddify_process",
                    initiator = "windows_platform_backend",
                    cause = "stop_with_stack",
                    trace_id = %trace_id,
                    trace_route = "desktop_engine->windows_platform_backend->hiddify_process",
                    exit_status = %status,
                    "Hiddify child process stopped"
                ),
                Ok(Err(cause)) => warn!(
                    event = "hiddify.wait_failed",
                    section = "hiddify_process",
                    initiator = "windows_platform_backend",
                    cause = %cause,
                    trace_id = %trace_id,
                    trace_route = "desktop_engine->windows_platform_backend->hiddify_process",
                    "could not collect the stopped Hiddify child process"
                ),
                Err(cause) => warn!(
                    event = "hiddify.stop_timed_out",
                    section = "hiddify_process",
                    initiator = "windows_platform_backend",
                    cause = %cause,
                    trace_id = %trace_id,
                    trace_route = "desktop_engine->windows_platform_backend->hiddify_process",
                    timeout_seconds = 5_u64,
                    "Hiddify child process did not stop before the timeout"
                ),
            }
        }
        Ok(())
    }

    async fn clear_hiddify_system_proxy(&self) -> Result<bool, CoreError> {
        let config = self.config.read().await.clone();
        let persist = system_proxy::snapshot_path(&self.paths.user_data_dir);
        let (host, port) = config.hiddify_endpoint();
        system_proxy::clear_if_hiddify(&host, port, &persist)
            .await
            .map(|cleared| cleared.is_some())
    }

    async fn restore_hiddify_system_proxy(&self) -> Result<(), CoreError> {
        let persist = system_proxy::snapshot_path(&self.paths.user_data_dir);
        system_proxy::restore(&persist).await
    }

    async fn core_process(&self) -> Result<ProcessStatus, CoreError> {
        match self
            .helper_request(HelperCommand::GetMihomoProcessStatus)
            .await?
        {
            HelperReply::ProcessStatus(status) => Ok(ProcessStatus {
                running: status.running,
                pid: status.pid,
            }),
            _ => Err(CoreError::Platform(
                "unexpected process status reply".into(),
            )),
        }
    }

    async fn tun_status(&self) -> Result<TunStatus, CoreError> {
        let config = self.config.read().await.clone();
        Ok(TunStatus {
            active: Self::tun_active(&config).await,
            name: Some(config.mihomo.tun_name),
        })
    }

    async fn check_readiness(
        &self,
        cancel: CancellationToken,
    ) -> Result<ReadinessReport, CoreError> {
        let config = self.config.read().await.clone();
        let controller = ControllerClient::new(
            &config.mihomo.controller_host,
            config.mihomo.controller_port,
            config.mihomo.controller_secret.clone(),
        )
        .map_err(|error| CoreError::ConfigInvalid(error.to_string()))?;
        info!(
            event = "mihomo.readiness_wait_started",
            section = "runtime_health",
            initiator = "windows_platform_backend",
            cause = "core_started",
            trace_route = "desktop_engine->windows_platform_backend->mihomo_controller",
            "waiting for the Mihomo controller and rule providers"
        );
        let providers = match controller
            .wait_until_ready(READINESS_BUDGET, cancel.clone())
            .await
        {
            Ok(providers) => {
                info!(
                    event = "mihomo.readiness_wait_completed",
                    section = "runtime_health",
                    initiator = "windows_platform_backend",
                    cause = "none",
                    trace_route = "desktop_engine->windows_platform_backend->mihomo_controller",
                    ready = providers.ready,
                    total = providers.total,
                    rules_loaded = providers.rules_loaded,
                    "Mihomo controller and rule providers are ready"
                );
                providers
            }
            Err(error) => {
                error!(
                    event = "mihomo.readiness_wait_failed",
                    section = "runtime_health",
                    initiator = "windows_platform_backend",
                    cause = %error,
                    trace_route = "desktop_engine->windows_platform_backend->mihomo_controller",
                    "Mihomo readiness wait failed"
                );
                return Err(readiness_error(error));
            }
        };
        if cancel.is_cancelled() {
            return Err(CoreError::Cancelled);
        }
        let exit_ip = self.egress_exit_ip.lock().await.clone();
        // The pre-TUN egress probe is only mandatory when unmatched traffic
        // goes to a local proxy; a Direct or side-tunnel default has no
        // loopback egress to confirm.
        let default_is_local_proxy = config.default_route.client_id().is_some_and(|client_id| {
            config.enabled_clients().into_iter().any(|client| {
                client.id == client_id && client.spec().kind == EgressKind::LocalProxy
            })
        });
        if default_is_local_proxy && exit_ip.is_none() {
            error!(
                event = "hiddify.egress_missing_after_tun",
                section = "runtime_health",
                initiator = "windows_platform_backend",
                cause = "pre_tun_probe_missing",
                trace_route = "desktop_engine->windows_platform_backend->hiddify_egress",
                "default local-proxy egress was not confirmed before TUN start"
            );
            return Err(CoreError::HiddifyEgressUnavailable);
        }
        Ok(ReadinessReport {
            controller_ready: true,
            egress_ready: true,
            providers: ProviderSummary {
                ready: providers.ready,
                total: providers.total,
                rules_loaded: providers.rules_loaded,
                last_refresh: Some(chrono::Utc::now()),
            },
            exit_ip,
        })
    }

    async fn cleanup_owned_state(&self) -> Result<CleanupReport, CoreError> {
        self.egress_handles.lock().await.clear();
        *self.egress_exit_ip.lock().await = None;
        self.client_exit_ips.lock().await.clear();
        self.side_tunnel_auth_files.lock().await.clear();
        match self
            .helper_request(HelperCommand::CleanupOwnedNetworkState)
            .await?
        {
            HelperReply::CleanupReport(report) => Ok(CleanupReport {
                process_stopped: report.process_stopped,
                tun_removed: report.tun_removed,
                dns_restored: report.dns_restored,
                routes_removed: report.routes_removed,
                warnings: report.warnings,
            }),
            _ => Err(CoreError::Platform("unexpected cleanup reply".into())),
        }
    }
}

/// Mihomo reports its live configuration, including the Wintun device it owns.
///
/// Windows builds often echo `device: Meta`, an empty name, or a Wintun path
/// instead of the configured `clash-iran`. `enable: true` is the authority
/// that Mihomo owns a tunnel; the name is only logged.
#[must_use]
pub fn tun_enabled(configs: &serde_json::Value, _tun_name: &str) -> bool {
    let Some(tun) = tun_section(configs) else {
        return false;
    };
    json_flag_enabled(tun.get("enable"))
}

fn tun_section(configs: &serde_json::Value) -> Option<&serde_json::Value> {
    configs
        .get("tun")
        .or_else(|| configs.get("config").and_then(|config| config.get("tun")))
}

fn json_flag_enabled(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(enabled)) => *enabled,
        Some(serde_json::Value::Number(number)) => {
            number.as_u64().is_some_and(|value| value != 0)
                || number.as_i64().is_some_and(|value| value != 0)
        }
        Some(serde_json::Value::String(text)) => {
            let text = text.trim();
            text.eq_ignore_ascii_case("true") || text == "1"
        }
        _ => false,
    }
}

fn platform_error(error: &io::Error) -> CoreError {
    CoreError::Platform(error.to_string())
}

/// Finds a launchable binary for a `LocalProxy` preset by its process names
/// (wildcards excluded), preferring the first — the GUI app — over cores.
/// Enabled local-proxy clients whose connect-time egress handle is missing —
/// the only candidates for live recovery (ADR 0076). Side tunnels are owned
/// processes with their own lifecycle and are never re-attached here.
fn clients_missing_egress<'config>(
    config: &'config AppConfig,
    handles: &[EgressHandle],
) -> Vec<&'config ClientInstance> {
    config
        .enabled_clients()
        .into_iter()
        .filter(|client| client.spec().kind == EgressKind::LocalProxy)
        .filter(|client| !handles.iter().any(|handle| handle.client_id == client.id))
        .collect()
}

fn discover_local_proxy_binary(spec: &iran_split_config::PresetSpec) -> Option<PathBuf> {
    discover_local_proxy_binary_in(
        spec,
        &local_proxy_search_dirs(),
        &well_known_local_proxy_binaries(spec),
    )
}

fn local_proxy_search_dirs() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable) {
            let root = PathBuf::from(root);
            directories.push(root.join("Programs"));
            directories.push(root);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        directories.extend(std::env::split_paths(&path));
    }
    directories
}

fn well_known_local_proxy_binaries(spec: &iran_split_config::PresetSpec) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if spec.preset != PresetId::Happ {
        return candidates;
    }
    for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable) {
            let root = PathBuf::from(root);
            candidates.push(root.join("Happ").join("Happ.exe"));
            candidates.push(root.join("Programs").join("Happ").join("Happ.exe"));
        }
    }
    candidates
}

fn discover_local_proxy_binary_in(
    spec: &iran_split_config::PresetSpec,
    directories: &[PathBuf],
    extra: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(path) = extra.iter().find(|path| path.is_file()) {
        return Some(path.clone());
    }
    let names = spec
        .windows_bypass
        .iter()
        .copied()
        .filter(|name| !name.contains('*'));
    for name in names {
        let stem = Path::new(name)
            .file_stem()
            .and_then(|file| file.to_str())
            .unwrap_or(name);
        for directory in directories {
            for candidate in [directory.join(name), directory.join(stem).join(name)] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Writes `username\npassword` to a private temp file for `--auth-user-pass`.
/// Returns `None` when the instance has no credentials.
fn write_side_tunnel_auth(
    username: Option<&str>,
    password: Option<&str>,
) -> Result<Option<NamedTempFile>, CoreError> {
    let (Some(username), Some(password)) = (username, password) else {
        return Ok(None);
    };
    if username.is_empty() {
        return Ok(None);
    }
    let mut file = NamedTempFile::new().map_err(|error| platform_error(&error))?;
    writeln!(file, "{username}")
        .and_then(|()| writeln!(file, "{password}"))
        .and_then(|()| file.flush())
        .map_err(|error| platform_error(&error))?;
    Ok(Some(file))
}

fn readiness_error(error: MihomoError) -> CoreError {
    match error {
        MihomoError::Cancelled => CoreError::Cancelled,
        MihomoError::ReadinessTimeout(_) => CoreError::ControllerTimeout,
        other => CoreError::MihomoStartFailed(other.to_string()),
    }
}

fn copy_rule_file(
    bundled: &Path,
    cache: &Path,
    staging: &Path,
    name: &str,
) -> Result<(), CoreError> {
    let source = iran_split_rules::resolve_provider_path(cache, bundled, name);
    let metadata = fs::symlink_metadata(&source).map_err(|error| platform_error(&error))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(CoreError::ConfigInvalid(format!(
            "bootstrap rule {name} is not a regular file"
        )));
    }
    let bytes = fs::read(source).map_err(|error| platform_error(&error))?;
    write_atomic(&staging.join(name), &bytes)
}

fn write_custom_provider_files(
    staging: &Path,
    document: &RoutePinsDocument,
    config: &AppConfig,
) -> Result<(), CoreError> {
    let (domains, ips) = split_pins(document, Outbound::Direct);
    write_lines(&staging.join("custom-direct-domains.txt"), &domains)?;
    write_lines(&staging.join("custom-direct-ips.txt"), &ips)?;
    for client in config.enabled_clients() {
        let (client_domains, client_ips) = split_pins(document, Outbound::client(client.id));
        let [domains_file, ips_file] = client.provider_files();
        write_lines(&staging.join(domains_file), &client_domains)?;
        write_lines(&staging.join(ips_file), &client_ips)?;
    }
    Ok(())
}

fn split_pins(document: &RoutePinsDocument, outbound: Outbound) -> (Vec<String>, Vec<String>) {
    let mut domains = Vec::new();
    let mut ips = Vec::new();
    for pin in document.pins.iter().filter(|pin| pin.outbound == outbound) {
        match &pin.target {
            DirectTarget::Domain(domain) => domains.push(format!("+.{domain}")),
            DirectTarget::Ip(address) => ips.push(host_cidr(*address)),
        }
    }
    domains.sort();
    domains.dedup();
    ips.sort();
    ips.dedup();
    (domains, ips)
}

fn host_cidr(address: IpAddr) -> String {
    match address {
        IpAddr::V4(address) => format!("{address}/32"),
        IpAddr::V6(address) => format!("{address}/128"),
    }
}

fn write_lines(path: &Path, lines: &[String]) -> Result<(), CoreError> {
    let mut content = lines.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    write_atomic(path, content.as_bytes())
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), CoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::ConfigInvalid("runtime file has no parent".into()))?;
    fs::create_dir_all(parent).map_err(|error| platform_error(&error))?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|error| platform_error(&error))?;
    temporary
        .write_all(content)
        .map_err(|error| platform_error(&error))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| platform_error(&error))?;
    temporary
        .persist(path)
        .map_err(|error| platform_error(&error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        discover_local_proxy_binary_in, is_helper_absent, is_pipe_busy, tun_enabled, AppConfig,
        WindowsBackend, WindowsPaths, HELPER_PIPE,
    };
    use iran_split_config::PresetId;
    use iran_split_core::PlatformBackend;
    use serde_json::json;
    use std::{fs, io, path::PathBuf};

    fn paths(root: &std::path::Path) -> WindowsPaths {
        let resources = root.join("resources");
        fs::create_dir_all(&resources).expect("resources");
        for name in [
            "private.txt",
            "iran-domains.txt",
            "iran-networks.txt",
            "iran-business-domains.txt",
        ] {
            fs::write(resources.join(name), "example\n").expect("fixture");
        }
        WindowsPaths {
            pipe_name: HELPER_PIPE.to_owned(),
            user_data_dir: root.join("user-data"),
            system_runtime_dir: PathBuf::from(r"C:\ProgramData\iran-split\runtime"),
            generation_staging_dir: root.join("staging"),
            resources_dir: resources,
            rules_cache_dir: root.join("rules-cache"),
            mihomo_binary: root.join("mihomo.exe"),
        }
    }

    #[test]
    fn pipe_name_is_versioned_and_fixed() {
        assert_eq!(HELPER_PIPE, r"\\.\pipe\iran-split-helper-v1");
        assert!(!HELPER_PIPE.contains(".."));
    }

    #[test]
    fn packaged_connect_stages_into_the_helper_generation_root() {
        let production = include_str!("lib.rs")
            .split("mod tests {")
            .next()
            .expect("production source");
        assert!(production.contains("generation_staging_dir"));
        assert!(production.contains(".generation_staging_dir"));
        // Split the needle so a whole-file `include_str!` cannot match this
        // assertion. `#![cfg(windows)]` means Linux CI never runs this test.
        let forbidden = [
            "user_data_dir.join(",
            r#""runtime""#,
            ").join(",
            r#""generations""#,
            ")",
        ]
        .concat();
        assert!(!production.contains(&forbidden));
    }

    #[test]
    fn detects_busy_pipe_errors_for_bounded_retries() {
        assert!(is_pipe_busy(&io::Error::from_raw_os_error(231)));
        assert!(!is_pipe_busy(&io::Error::from_raw_os_error(2)));
    }

    #[test]
    fn a_missing_pipe_reads_as_an_uninstalled_helper() {
        assert!(is_helper_absent(&io::Error::from(io::ErrorKind::NotFound)));
        assert!(is_helper_absent(&io::Error::from(
            io::ErrorKind::ConnectionRefused
        )));
        assert!(!is_helper_absent(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
    }

    #[test]
    fn tun_is_active_when_mihomo_reports_enable() {
        let active = json!({"tun": {"enable": true, "device": "clash-iran"}});
        assert!(tun_enabled(&active, "clash-iran"));
        assert!(tun_enabled(&active, "CLASH-IRAN"));
        // Windows Mihomo commonly echoes Meta or a Wintun path, not clash-iran.
        assert!(tun_enabled(
            &json!({"tun": {"enable": true, "device": "Meta"}}),
            "clash-iran"
        ));
        assert!(tun_enabled(
            &json!({"tun": {"enable": true, "device": ""}}),
            "clash-iran"
        ));
        assert!(tun_enabled(
            &json!({"tun": {"enable": 1, "device": "Meta"}}),
            "clash-iran"
        ));
        assert!(tun_enabled(
            &json!({"config": {"tun": {"enable": "true"}}}),
            "clash-iran"
        ));

        assert!(!tun_enabled(
            &json!({"tun": {"enable": false, "device": "clash-iran"}}),
            "clash-iran"
        ));
        assert!(!tun_enabled(&json!({}), "clash-iran"));
        assert!(tun_enabled(&json!({"tun": {"enable": true}}), "clash-iran"));
    }

    #[tokio::test]
    async fn preparation_publishes_only_allowlisted_generation_files() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = paths(directory.path());
        let backend = WindowsBackend::new(AppConfig::default(), paths.clone());
        let generation = backend.prepare_runtime().await.expect("prepare");
        let root = paths
            .generation_staging_dir
            .join(generation.generation_id.to_string());
        let names = fs::read_dir(&root)
            .expect("generation")
            .map(|entry| entry.expect("entry").file_name())
            .collect::<std::collections::HashSet<_>>();

        // config.yaml, four bundled providers, two custom-direct, two per-client.
        assert_eq!(names.len(), 9);
        assert!(names.iter().any(|name| {
            let name = name.to_string_lossy();
            name.starts_with("custom-")
                && name.ends_with("-domains.txt")
                && name != "custom-direct-domains.txt"
        }));
        assert!(names.iter().any(|name| {
            let name = name.to_string_lossy();
            name.starts_with("custom-")
                && name.ends_with("-ips.txt")
                && name != "custom-direct-ips.txt"
        }));
        assert!(names.contains(std::ffi::OsStr::new("config.yaml")));
        let config = fs::read_to_string(root.join("config.yaml")).expect("config");
        assert!(config.contains("path: private.txt"));
        // Mihomo Meta 1.19+ rejects provider paths outside the process workdir.
        assert!(!config.contains(r"C:\ProgramData"));
        // strict_route is the Windows-only half of the shared generator.
        assert!(config.contains("strict-route: true"));
        assert!(config.contains("find-process-mode: always"));
        assert!(config.contains("auto-redirect: false"));
        assert!(config.contains("ipv6: false"));
        // DoH is pinned to the default client's group (client registry
        // replaced the fixed "VPN" group name).
        assert!(config.contains("dns-query#client-"));
    }

    #[tokio::test]
    async fn validation_reports_a_missing_mihomo_before_touching_the_helper() {
        let directory = tempfile::tempdir().expect("tempdir");
        let backend = WindowsBackend::new(AppConfig::default(), paths(directory.path()));
        let generation = backend.prepare_runtime().await.expect("prepare");
        let error = backend
            .validate_runtime(&generation)
            .await
            .expect_err("missing mihomo.exe");
        assert!(matches!(error, iran_split_core::CoreError::MihomoNotFound));
    }

    #[test]
    fn hiddify_candidates_cover_the_packaged_and_installed_layouts() {
        let candidates = WindowsBackend::hiddify_candidates(std::path::Path::new(r"C:\data"));
        assert!(candidates
            .iter()
            .any(|path| path.ends_with("apps/Hiddify/Hiddify.exe")));
        assert!(candidates
            .iter()
            .all(|path| !path.to_string_lossy().contains("..")));
    }

    #[test]
    fn discovers_happ_from_a_well_known_install_path() {
        let directory = tempfile::tempdir().expect("tempdir");
        let binary = directory.path().join("Happ.exe");
        fs::write(&binary, b"mz").expect("write");
        let found = discover_local_proxy_binary_in(&PresetId::Happ.spec(), &[], &[binary.clone()]);
        assert_eq!(found, Some(binary));
    }
}
