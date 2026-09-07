#![cfg(target_os = "linux")]

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
    helper_ipc_reply_timeout, read_frame, validate_envelope, write_frame, Envelope, HelperCommand,
    HelperReply, HELPER_IPC_FRAME_TIMEOUT_SECS, PROTOCOL_VERSION,
};
use iran_split_mihomo::{
    generate_config_with_handles, probe_hiddify_egress, validate_with_binary, ControllerClient,
    MihomoError, Platform, RuntimePaths,
};
use iran_split_rules::{DirectTarget, Outbound, RoutePinsDocument};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::{
    net::{TcpStream, UnixStream},
    process::{Child, Command},
    sync::{Mutex, RwLock},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

mod system_proxy;

const IPC_TIMEOUT: Duration = Duration::from_secs(HELPER_IPC_FRAME_TIMEOUT_SECS);

#[derive(Debug, Error)]
pub enum LinuxBackendError {
    #[error("helper IPC failed: {0}")]
    Protocol(#[from] iran_split_ipc::ProtocolError),
    #[error("helper connection failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("helper request timed out")]
    Timeout,
    #[error("helper response did not match the request")]
    ResponseMismatch,
    #[error("helper returned {code}: {message}")]
    Helper { code: String, message: String },
}

#[derive(Debug, Clone)]
pub struct HelperClient {
    socket_path: PathBuf,
}

impl HelperClient {
    #[must_use]
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// Sends one validated command to the privileged helper.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, connection, protocol negotiation, or
    /// the helper operation fails.
    pub async fn request(&self, command: HelperCommand) -> Result<HelperReply, LinuxBackendError> {
        let command_name = command.audit_name();
        let request_id = Uuid::new_v4();
        info!(
            event = "helper.request_started",
            section = "helper_ipc",
            initiator = "linux_platform_backend",
            cause = "backend_operation",
            trace_id = %request_id,
            trace_route = "desktop_engine->linux_platform_backend->helper_ipc",
            command = command_name,
            "helper request started"
        );
        if let Err(cause) = command.validate() {
            error!(
                event = "helper.request_failed",
                section = "helper_ipc",
                initiator = "linux_platform_backend",
                cause = %cause,
                trace_id = %request_id,
                trace_route = "desktop_engine->linux_platform_backend->helper_ipc->validation",
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
                initiator = "linux_platform_backend",
                cause = "none",
                trace_id = %request_id,
                trace_route = "desktop_engine->linux_platform_backend->helper_ipc->reply",
                command = command_name,
                "helper request completed"
            ),
            Err(cause) => error!(
                event = "helper.request_failed",
                section = "helper_ipc",
                initiator = "linux_platform_backend",
                cause = %cause,
                trace_id = %request_id,
                trace_route = "desktop_engine->linux_platform_backend->helper_ipc->error",
                command = command_name,
                "helper request failed"
            ),
        }
        result
    }

    async fn request_validated(
        &self,
        command: HelperCommand,
    ) -> Result<HelperReply, LinuxBackendError> {
        let mut stream = tokio::time::timeout(IPC_TIMEOUT, UnixStream::connect(&self.socket_path))
            .await
            .map_err(|_| LinuxBackendError::Timeout)??;
        let hello = Envelope::new(HelperCommand::Hello {
            client_version: env!("CARGO_PKG_VERSION").into(),
            supported_protocols: vec![PROTOCOL_VERSION],
        });
        let hello_reply = exchange(&mut stream, &hello, IPC_TIMEOUT).await?;
        match hello_reply.payload {
            HelperReply::Hello(reply) if reply.selected_protocol == PROTOCOL_VERSION => {}
            HelperReply::Error(error) => {
                return Err(LinuxBackendError::Helper {
                    code: error.code,
                    message: error.message,
                });
            }
            _ => return Err(LinuxBackendError::ResponseMismatch),
        }
        let request = Envelope::new(command);
        let budget = helper_ipc_reply_timeout(&request.payload);
        let response = exchange(&mut stream, &request, budget).await?;
        match response.payload {
            HelperReply::Error(error) => Err(LinuxBackendError::Helper {
                code: error.code,
                message: error.message,
            }),
            reply => Ok(reply),
        }
    }
}

async fn exchange(
    stream: &mut UnixStream,
    request: &Envelope<HelperCommand>,
    budget: Duration,
) -> Result<Envelope<HelperReply>, LinuxBackendError> {
    tokio::time::timeout(IPC_TIMEOUT, write_frame(stream, request))
        .await
        .map_err(|_| LinuxBackendError::Timeout)??;
    let reply: Envelope<HelperReply> = tokio::time::timeout(budget, read_frame(stream))
        .await
        .map_err(|_| LinuxBackendError::Timeout)??;
    validate_envelope(&reply)?;
    if reply.request_id != request.request_id {
        return Err(LinuxBackendError::ResponseMismatch);
    }
    Ok(reply)
}

#[derive(Debug, Clone)]
pub struct LinuxPaths {
    pub socket_path: PathBuf,
    pub user_data_dir: PathBuf,
    pub system_runtime_dir: PathBuf,
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
pub struct LinuxBackend {
    config: Arc<RwLock<AppConfig>>,
    helper: HelperClient,
    paths: LinuxPaths,
    prepared: Mutex<Option<PreparedGeneration>>,
    launched_hiddify: Mutex<Option<Child>>,
    egress_exit_ip: Mutex<Option<String>>,
    egress_handles: Mutex<Vec<EgressHandle>>,
    side_tunnel_auth_files: Mutex<Vec<NamedTempFile>>,
    launched_clients: Mutex<Vec<Child>>,
    client_exit_ips: Mutex<std::collections::HashMap<iran_split_config::ClientId, String>>,
    /// Why each client failed at its last start attempt. Connect only warns
    /// for optional clients, so without this the card said "Starting" or
    /// "Stopped" with no detail and the operator had to read debug.log.
    client_failures: Mutex<std::collections::HashMap<iran_split_config::ClientId, String>>,
    /// Connect-time override for `StartSideTunnel` (progressive 15/30/60s UX).
    side_tunnel_connect_timeout: Mutex<Option<u64>>,
}

impl LinuxBackend {
    #[must_use]
    pub fn new(config: AppConfig, paths: LinuxPaths) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            helper: HelperClient::new(&paths.socket_path),
            paths,
            prepared: Mutex::new(None),
            launched_hiddify: Mutex::new(None),
            egress_exit_ip: Mutex::new(None),
            egress_handles: Mutex::new(Vec::new()),
            side_tunnel_auth_files: Mutex::new(Vec::new()),
            launched_clients: Mutex::new(Vec::new()),
            client_exit_ips: Mutex::new(std::collections::HashMap::new()),
            client_failures: Mutex::new(std::collections::HashMap::new()),
            side_tunnel_connect_timeout: Mutex::new(None),
        }
    }

    async fn effective_side_tunnel_timeout(&self, configured: u64) -> u64 {
        self.side_tunnel_connect_timeout
            .lock()
            .await
            .unwrap_or(configured)
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
    ) -> Result<Vec<iran_split_ipc::ServiceLogEntry>, LinuxBackendError> {
        match self
            .helper
            .request(HelperCommand::CollectServiceLogs {
                max_entries: maximum,
            })
            .await?
        {
            HelperReply::Logs(logs) => Ok(logs),
            _ => Err(LinuxBackendError::ResponseMismatch),
        }
    }

    async fn helper_request(&self, command: HelperCommand) -> Result<HelperReply, CoreError> {
        self.helper
            .request(command)
            .await
            .map_err(|error| CoreError::Platform(error.to_string()))
    }

    async fn register_runtime(&self, generation: &RuntimeGeneration) -> Result<(), CoreError> {
        match self
            .helper_request(HelperCommand::RegisterRuntimeGeneration {
                generation_id: generation.generation_id,
                config_sha256: generation.config_sha256.clone(),
            })
            .await?
        {
            HelperReply::GenerationRegistered { generation_id }
                if generation_id == generation.generation_id =>
            {
                Ok(())
            }
            _ => Err(CoreError::Platform("generation registration failed".into())),
        }
    }

    async fn spawn_core(&self, generation: &RuntimeGeneration) -> Result<(), CoreError> {
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

    async fn hot_reload_running(&self, rebind_host: Option<&str>) -> Result<(), CoreError> {
        let config = self.config.read().await.clone();
        let controller = ControllerClient::new(
            &config.mihomo.controller_host,
            config.mihomo.controller_port,
            config.mihomo.controller_secret.clone(),
        )
        .map_err(|error| CoreError::ConfigInvalid(error.to_string()))?;
        info!(
            event = "mihomo.hot_reload_started",
            section = "rules",
            initiator = "linux_platform_backend",
            cause = "live_pin_apply",
            trace_route = "engine->linux_platform_backend->mihomo_controller",
            "reloading Mihomo config without restarting the process"
        );
        controller
            .hot_reload()
            .await
            .map_err(|error| CoreError::MihomoStartFailed(error.to_string()))?;
        info!(
            event = "mihomo.hot_reload_succeeded",
            section = "rules",
            initiator = "linux_platform_backend",
            cause = "controller_204",
            trace_route = "engine->linux_platform_backend->mihomo_controller",
            "live Mihomo config reloaded"
        );
        let Some(host) = rebind_host else {
            return Ok(());
        };
        match controller.close_connections_for_pin_apply(host).await {
            Ok(closed) => info!(
                event = "mihomo.connections_rebound",
                section = "rules",
                initiator = "linux_platform_backend",
                cause = "pin_apply",
                trace_route = "engine->linux_platform_backend->mihomo_controller",
                closed,
                "closed live connections for the moved pin so they reconnect on the new outbound"
            ),
            Err(cause) => warn!(
                event = "mihomo.connections_rebind_failed",
                section = "rules",
                initiator = "linux_platform_backend",
                cause = %cause,
                trace_route = "engine->linux_platform_backend->mihomo_controller",
                "could not close matching connections after the live reload"
            ),
        }
        Ok(())
    }

    /// Loopback SOCKS endpoint of a client that is already serving traffic.
    ///
    /// `ready` are the handles collected so far in this connect, because
    /// `egress_handles` is only published after every client has started.
    async fn ready_proxy_endpoint(&self, ready: &[EgressHandle]) -> Option<(String, u16)> {
        let config = self.config.read().await.clone();
        let published = self.egress_handles.lock().await.clone();
        config
            .enabled_clients()
            .into_iter()
            .filter(|candidate| {
                ready
                    .iter()
                    .chain(published.iter())
                    .any(|handle| handle.client_id == candidate.id && handle.ready)
            })
            .find_map(local_proxy_endpoint)
    }

    /// Resolves the profile's server name over `DoH` through a client that is
    /// already serving traffic, so a poisoned resolver cannot send `OpenVPN`
    /// to an unroutable address. Returns `None` when nothing is serving yet
    /// or the name already is an address; the profile is then used as-is.
    async fn pin_side_tunnel_remote(
        &self,
        profile: &Path,
        proxy: Option<&(String, u16)>,
    ) -> Option<(std::net::IpAddr, u16)> {
        let facts = iran_split_clients::audit_openvpn_profile(profile).ok()?;
        let host = facts.remote_hosts.first()?.clone();
        let port = facts.remote_port?;
        if host.parse::<std::net::IpAddr>().is_ok() {
            return None;
        }
        let Some((proxy_host, proxy_port)) = proxy else {
            info!(
                event = "side_tunnel.remote_resolve_skipped",
                section = "clients",
                initiator = "start_openvpn_client",
                cause = "no_ready_client_to_resolve_through",
                trace_route = "engine->platform_backend->doh_resolver",
                "no client is serving yet; using the profile's own server name"
            );
            return None;
        };
        match iran_split_clients::resolve_through_proxy(
            &host,
            (proxy_host.as_str(), *proxy_port),
            Duration::from_secs(5),
        )
        .await
        {
            Ok(addresses) => {
                let address = *addresses.first()?;
                info!(
                    event = "side_tunnel.remote_resolved",
                    section = "clients",
                    initiator = "start_openvpn_client",
                    cause = "doh_through_ready_client",
                    trace_route = "engine->platform_backend->doh_resolver",
                    "resolved the side-tunnel server over DoH without logging the address"
                );
                Some((address, port))
            }
            Err(error) => {
                warn!(
                    event = "side_tunnel.remote_resolve_failed",
                    section = "clients",
                    initiator = "start_openvpn_client",
                    cause = %error,
                    trace_route = "engine->platform_backend->doh_resolver",
                    "could not pin the side-tunnel server address; using the profile as written"
                );
                None
            }
        }
    }

    async fn start_openvpn_client(
        &self,
        client: &ClientInstance,
        cancel: CancellationToken,
        ready: &[EgressHandle],
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
        let auth_file = write_side_tunnel_auth(
            &self.paths.user_data_dir,
            username.as_deref(),
            password.as_deref(),
        )?;
        let proxy = self.ready_proxy_endpoint(ready).await;
        let pinned_remote = self.pin_side_tunnel_remote(&profile, proxy.as_ref()).await;
        let timeout_seconds = self
            .effective_side_tunnel_timeout(*start_timeout_seconds)
            .await;
        let result = self
            .helper_request(HelperCommand::StartSideTunnel {
                driver: "openvpn".into(),
                client_id: client.id.into(),
                profile,
                executable,
                auth_file: auth_file.as_ref().map(|file| file.path().to_path_buf()),
                timeout_seconds,
                pinned_remote,
                socks_proxy: proxy,
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
                        self.start_openvpn_client(client, cancel.clone(), &handles)
                            .await
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
                    self.egress_handles.lock().await.clone_from(&handles);
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
                        trace_route = "engine_operation->linux_platform_backend->ensure_clients",
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
    /// (ADR 0076). Connect probes each optional client exactly once; when the
    /// operator starts and connects Happ minutes later, its pinned domains
    /// keep the client group (ADR 0082) but still need this re-check to attach
    /// the live SOCKS bind. The engine then hot-applies the routing.
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
                    self.client_failures.lock().await.remove(&client.id);
                    let Some(handle) = synthesized_local_handle(client) else {
                        continue;
                    };
                    info!(
                        event = "client.recovered_egress",
                        section = "clients",
                        initiator = "recover_clients",
                        cause = "egress_probe_succeeded",
                        trace_route = "engine->linux_platform_backend->recover_clients",
                        client = client.spec().id,
                        "a local proxy egress became reachable after connect"
                    );
                    self.client_exit_ips.lock().await.insert(client.id, exit_ip);
                    self.egress_handles.lock().await.push(handle);
                    recovered = true;
                }
                Err(error) => {
                    self.client_failures.lock().await.insert(
                        client.id,
                        format!(
                            "{host}:{port} accepts connections but no traffic flows; connect the client itself"
                        ),
                    );
                    info!(
                        event = "client.recover_probe_failed",
                        section = "clients",
                        initiator = "recover_clients",
                        cause = %error,
                        trace_route = "engine->linux_platform_backend->recover_clients",
                        client = client.spec().id,
                        "local proxy port answers but its egress is not usable yet"
                    );
                }
            }
        }
        Ok(recovered)
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
            // window (measured 21s stalls in production debug.log). One quick
            // probe decides; ADR 0076 recovery attaches it once it serves.
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
                // opens (production debug.log shows ~18s stalls waiting for
                // Happ). Launch it and let ADR 0076 recovery attach the
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

    async fn hiddify_listening(config: &AppConfig) -> bool {
        let (host, port) = config.hiddify_endpoint();
        Self::tcp_listening(&host, port).await
    }

    async fn tcp_listening(host: &str, port: u16) -> bool {
        tokio::time::timeout(Duration::from_millis(750), TcpStream::connect((host, port)))
            .await
            .is_ok_and(|result| result.is_ok())
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

    fn discover_hiddify(config: &AppConfig, data: &Path) -> Option<PathBuf> {
        if let ExecutableSetting::Path(path) = &config.hiddify_executable() {
            return path.is_file().then(|| path.clone());
        }
        let mut candidates = vec![
            data.join("bin/hiddify"),
            data.join("bin/hiddify-app"),
            data.join("apps/Hiddify.AppImage"),
            PathBuf::from("/usr/bin/hiddify"),
            PathBuf::from("/usr/bin/hiddify-app"),
            PathBuf::from("/usr/local/bin/hiddify"),
            PathBuf::from("/opt/Hiddify/Hiddify"),
            PathBuf::from("/opt/hiddify/hiddify"),
            PathBuf::from("/opt/hiddify/hiddify-app"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            candidates.push(home.join(".local/bin/hiddify"));
            candidates.push(home.join(".local/bin/hiddify-app"));
            // The installed app keeps its managed Hiddify here. A dev run
            // uses an isolated profile, so without these entries it could
            // not find the Hiddify the user already installed.
            candidates.push(home.join(".local/share/biflow/bin/hiddify"));
            candidates.push(home.join(".local/share/biflow/apps/Hiddify.AppImage"));
        }
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                candidates.push(dir.join("hiddify"));
                candidates.push(dir.join("hiddify-app"));
            }
        }
        candidates.into_iter().find(|path| path.is_file())
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
        let Ok(controller) = ControllerClient::new(
            &config.mihomo.controller_host,
            config.mihomo.controller_port,
            config.mihomo.controller_secret.clone(),
        ) else {
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

    fn tun_component(tun_name: &str) -> ComponentStatus {
        let active = Path::new("/sys/class/net").join(tun_name).exists();
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

    async fn probe_hiddify_until_ready(
        &self,
        config: &AppConfig,
        cancel: CancellationToken,
    ) -> Result<String, CoreError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
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
                        initiator = "linux_platform_backend",
                        cause = "socks_probe",
                        trace_route = "desktop_engine->linux_platform_backend->hiddify_egress",
                        "Hiddify SOCKS egress is reachable"
                    );
                    return Ok(exit_ip);
                }
                Err(error) => {
                    last_cause = error.to_string();
                    warn!(
                        event = "hiddify.egress_probe_failed",
                        section = "hiddify_process",
                        initiator = "linux_platform_backend",
                        cause = %error,
                        trace_route = "desktop_engine->linux_platform_backend->hiddify_egress",
                        "Hiddify SOCKS egress probe failed; retrying before TUN starts"
                    );
                }
            }
            if tokio::time::Instant::now() >= deadline {
                error!(
                    event = "hiddify.egress_probe_exhausted",
                    section = "hiddify_process",
                    initiator = "linux_platform_backend",
                    cause = last_cause.as_str(),
                    trace_route = "desktop_engine->linux_platform_backend->hiddify_egress",
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
impl PlatformBackend for LinuxBackend {
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
        let tun = Self::tun_component(&config.mihomo.tun_name);
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
            Err(LinuxBackendError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(HelperStatus::default())
            }
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

    async fn retry_failed_side_tunnels(
        &self,
        cancel: CancellationToken,
    ) -> Result<bool, CoreError> {
        let config = self.config.read().await.clone();
        let handles = self.egress_handles.lock().await.clone();
        let mut recovered = false;
        for client in side_tunnels_missing_egress(&config, &handles) {
            if cancel.is_cancelled() {
                return Err(CoreError::Cancelled);
            }
            let ready = self.egress_handles.lock().await.clone();
            match self
                .start_openvpn_client(client, cancel.clone(), &ready)
                .await
            {
                Ok(handle) => {
                    self.client_failures.lock().await.remove(&client.id);
                    self.egress_handles.lock().await.push(handle);
                    recovered = true;
                    info!(
                        event = "side_tunnel.retry_succeeded",
                        section = "clients",
                        initiator = "retry_side_tunnels",
                        cause = "openvpn_started",
                        trace_route = "desktop->engine->linux_platform_backend->retry_side_tunnels",
                        client = client.spec().id,
                        "side tunnel started after a longer timeout"
                    );
                }
                Err(error) => {
                    self.client_failures
                        .lock()
                        .await
                        .insert(client.id, error.to_string());
                    warn!(
                        event = "side_tunnel.retry_failed",
                        section = "clients",
                        initiator = "retry_side_tunnels",
                        cause = %error,
                        trace_route = "desktop->engine->linux_platform_backend->retry_side_tunnels",
                        client = client.spec().id,
                        "side tunnel retry did not start"
                    );
                }
            }
        }
        Ok(recovered)
    }

    async fn set_side_tunnel_connect_timeout(&self, seconds: Option<u64>) {
        *self.side_tunnel_connect_timeout.lock().await = seconds;
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
            .user_data_dir
            .join("runtime")
            .join("generations")
            .join(generation_id.to_string());
        fs::create_dir_all(&staging_root).map_err(|error| platform_error(&error))?;
        let runtime_paths = RuntimePaths {
            private_networks: PathBuf::from("private.txt"),
            iran_domains: PathBuf::from("iran-domains.txt"),
            iran_business_domains: PathBuf::from("iran-business-domains.txt"),
            iran_networks: PathBuf::from("iran-networks.txt"),
            iran_cdn_networks: PathBuf::from("iran-cdn-networks.txt"),
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
            Platform::Linux,
            &runtime_paths,
            &custom,
            &handles,
        )
        .map_err(|error| CoreError::ConfigInvalid(error.to_string()))?;
        copy_rule_file(
            &self.paths.resources_dir,
            &self.paths.rules_cache_dir,
            &staging_root,
            "private.txt",
        )?;
        copy_rule_file(
            &self.paths.resources_dir,
            &self.paths.rules_cache_dir,
            &staging_root,
            "iran-domains.txt",
        )?;
        copy_rule_file(
            &self.paths.resources_dir,
            &self.paths.rules_cache_dir,
            &staging_root,
            "iran-networks.txt",
        )?;
        copy_rule_file(
            &self.paths.resources_dir,
            &self.paths.rules_cache_dir,
            &staging_root,
            "iran-business-domains.txt",
        )?;
        copy_rule_file(
            &self.paths.resources_dir,
            &self.paths.rules_cache_dir,
            &staging_root,
            "iran-cdn-networks.txt",
        )?;
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
        self.register_runtime(generation).await?;
        self.spawn_core(generation).await
    }

    async fn reload_core(
        &self,
        generation: &RuntimeGeneration,
        rebind_host: Option<String>,
    ) -> Result<(), CoreError> {
        self.register_runtime(generation).await?;
        match self
            .helper_request(HelperCommand::OverlayRuntimeGeneration {
                generation_id: generation.generation_id,
                config_sha256: generation.config_sha256.clone(),
            })
            .await?
        {
            HelperReply::ProcessStatus(status) if status.running => {
                self.hot_reload_running(rebind_host.as_deref()).await
            }
            HelperReply::ProcessStatus(_) => self.spawn_core(generation).await,
            _ => Err(CoreError::Platform(
                "generation overlay did not return process status".into(),
            )),
        }
    }

    async fn stop_core(&self) -> Result<(), CoreError> {
        match self.helper_request(HelperCommand::StopMihomo).await? {
            HelperReply::ProcessStatus(status) if !status.running => {}
            _ => return Err(CoreError::Platform("helper did not stop Mihomo".into())),
        }
        Ok(())
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
                    initiator = "linux_platform_backend",
                    cause = %cause,
                    trace_id = %trace_id,
                    trace_route = "desktop_engine->linux_platform_backend->hiddify_process",
                    "could not send the stop signal to the Hiddify child process"
                );
            }
            match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
                Ok(Ok(status)) => info!(
                    event = "hiddify.process_stopped",
                    section = "hiddify_process",
                    initiator = "linux_platform_backend",
                    cause = "stop_with_stack",
                    trace_id = %trace_id,
                    trace_route = "desktop_engine->linux_platform_backend->hiddify_process",
                    exit_status = %status,
                    "Hiddify child process stopped"
                ),
                Ok(Err(cause)) => warn!(
                    event = "hiddify.wait_failed",
                    section = "hiddify_process",
                    initiator = "linux_platform_backend",
                    cause = %cause,
                    trace_id = %trace_id,
                    trace_route = "desktop_engine->linux_platform_backend->hiddify_process",
                    "could not collect the stopped Hiddify child process"
                ),
                Err(cause) => warn!(
                    event = "hiddify.stop_timed_out",
                    section = "hiddify_process",
                    initiator = "linux_platform_backend",
                    cause = %cause,
                    trace_id = %trace_id,
                    trace_route = "desktop_engine->linux_platform_backend->hiddify_process",
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
        let config = self.config.read().await;
        let name = config.mihomo.tun_name.clone();
        Ok(TunStatus {
            active: Path::new("/sys/class/net").join(&name).exists(),
            name: Some(name),
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
            initiator = "linux_platform_backend",
            cause = "core_started",
            trace_route = "desktop_engine->linux_platform_backend->mihomo_controller",
            "waiting for the Mihomo controller and rule providers"
        );
        let providers = match controller
            .wait_until_ready(Duration::from_secs(20), cancel.clone())
            .await
        {
            Ok(providers) => {
                info!(
                    event = "mihomo.readiness_wait_completed",
                    section = "runtime_health",
                    initiator = "linux_platform_backend",
                    cause = "none",
                    trace_route = "desktop_engine->linux_platform_backend->mihomo_controller",
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
                    initiator = "linux_platform_backend",
                    cause = %error,
                    trace_route = "desktop_engine->linux_platform_backend->mihomo_controller",
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
                initiator = "linux_platform_backend",
                cause = "pre_tun_probe_missing",
                trace_route = "desktop_engine->linux_platform_backend->hiddify_egress",
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

fn platform_error(error: &std::io::Error) -> CoreError {
    CoreError::Platform(error.to_string())
}

/// Finds a launchable binary for a `LocalProxy` preset by its process names
/// (wildcards excluded), preferring the first — the GUI app — over cores.
///
/// Debian's Happ package installs `/usr/bin/happ` → `/opt/happ/bin/Happ`.
/// PATH lookup must ignore case and must also try those well-known paths,
/// because a packaged Tauri PATH often omits `/usr/bin` or only has the
/// lowercase symlink.
fn discover_local_proxy_binary(spec: &iran_split_config::PresetSpec) -> Option<PathBuf> {
    let directories = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    discover_local_proxy_binary_in(spec, &directories, &well_known_local_proxy_binaries(spec))
}

fn well_known_local_proxy_binaries(spec: &iran_split_config::PresetSpec) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if spec.preset == PresetId::Happ {
        candidates.extend([
            PathBuf::from("/usr/bin/happ"),
            PathBuf::from("/usr/bin/Happ"),
            PathBuf::from("/opt/happ/bin/Happ"),
            PathBuf::from("/opt/happ/bin/happ"),
        ]);
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            candidates.push(home.join(".local/bin/happ"));
            candidates.push(home.join(".local/bin/Happ"));
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
        .linux_bypass
        .iter()
        .copied()
        .filter(|name| !name.contains('*'));
    for name in names {
        for directory in directories {
            if let Some(path) = file_named_ignore_case(directory, name) {
                return Some(path);
            }
        }
    }
    None
}

/// Enabled local-proxy clients whose connect-time egress handle is missing —
/// the only candidates for live recovery (ADR 0076). Side tunnels are owned
/// processes with their own lifecycle and are never re-attached here.
fn side_tunnels_missing_egress<'config>(
    config: &'config AppConfig,
    handles: &[EgressHandle],
) -> Vec<&'config ClientInstance> {
    config
        .enabled_clients()
        .into_iter()
        .filter(|client| client.spec().kind == EgressKind::OwnedSideTunnel)
        .filter(|client| {
            !handles
                .iter()
                .any(|handle| handle.client_id == client.id && handle.ready)
        })
        .collect()
}

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

fn file_named_ignore_case(directory: &Path, name: &str) -> Option<PathBuf> {
    let exact = directory.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    let entries = fs::read_dir(directory).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|file| file.to_str())
                    .is_some_and(|file| file.eq_ignore_ascii_case(name))
        })
}

/// Writes `username\npassword` to a 0600 temp file for `--auth-user-pass`.
/// Returns `None` when the instance has no credentials.
/// Writes the credential file where the helper can actually read it.
///
/// The helper unit sets `PrivateTmp=yes`, so a file in the desktop user's
/// `/tmp` does not exist inside the helper's mount namespace and `OpenVPN`
/// dies immediately with "exit status: 1". `ProtectHome=read-only` still
/// lets the helper read the user's data directory, so the file goes there,
/// owner-only, and is deleted when the handle drops.
fn write_side_tunnel_auth(
    user_data_dir: &Path,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<Option<NamedTempFile>, CoreError> {
    let (Some(username), Some(password)) = (username, password) else {
        return Ok(None);
    };
    if username.is_empty() {
        return Ok(None);
    }
    let directory = user_data_dir.join("side-tunnel");
    fs::create_dir_all(&directory).map_err(|error| platform_error(&error))?;
    let mut file = NamedTempFile::new_in(&directory).map_err(|error| platform_error(&error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(file.path(), fs::Permissions::from_mode(0o600))
            .map_err(|error| platform_error(&error))?;
    }
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
        MihomoError::Unauthorized => CoreError::ControllerUnauthorized,
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

#[allow(dead_code)]
fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iran_split_config::DefaultRoute;

    #[test]
    fn side_tunnel_auth_avoids_the_helpers_private_tmp() {
        let home = tempfile::tempdir().expect("tempdir");
        let file = write_side_tunnel_auth(home.path(), Some("user"), Some("secret"))
            .expect("auth")
            .expect("some");
        // PrivateTmp=yes hides /tmp from the helper: a credential file there
        // makes OpenVPN die instantly with "exit status: 1".
        assert!(
            file.path().starts_with(home.path()),
            "auth file must live under the user data directory"
        );
        assert_ne!(
            file.path().parent(),
            Some(std::env::temp_dir().as_path()),
            "the bare temp root is exactly what the helper cannot see"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(file.path())
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "credentials must stay owner-only");
        }
        let contents = fs::read_to_string(file.path()).expect("read");
        assert_eq!(contents, "user\nsecret\n");
        assert!(write_side_tunnel_auth(home.path(), None, Some("secret"))
            .expect("no username")
            .is_none());
    }

    #[tokio::test]
    async fn preparation_publishes_only_allowlisted_generation_files() {
        let directory = tempfile::tempdir().expect("tempdir");
        let resources = directory.path().join("resources");
        fs::create_dir_all(&resources).expect("resources");
        for name in [
            "private.txt",
            "iran-domains.txt",
            "iran-networks.txt",
            "iran-business-domains.txt",
            "iran-cdn-networks.txt",
        ] {
            fs::write(resources.join(name), "example\n").expect("fixture");
        }
        let paths = LinuxPaths {
            socket_path: directory.path().join("helper.sock"),
            user_data_dir: directory.path().join("user-data"),
            system_runtime_dir: PathBuf::from("/var/lib/iran-split"),
            resources_dir: resources,
            rules_cache_dir: directory.path().join("rules-cache"),
            mihomo_binary: directory.path().join("mihomo"),
        };
        let backend = LinuxBackend::new(AppConfig::default(), paths.clone());
        let generation = backend.prepare_runtime().await.expect("prepare");
        let root = paths
            .user_data_dir
            .join("runtime")
            .join("generations")
            .join(generation.generation_id.to_string());
        let names = fs::read_dir(&root)
            .expect("generation")
            .map(|entry| entry.expect("entry").file_name())
            .collect::<std::collections::HashSet<_>>();
        // config.yaml, five bundled providers, two custom-direct, two per-client.
        assert_eq!(names.len(), 10);
        assert!(names.contains(std::ffi::OsStr::new("iran-business-domains.txt")));
        assert!(names.contains(std::ffi::OsStr::new("iran-cdn-networks.txt")));
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
        assert!(!config.contains("/var/lib/iran-split/generations/"));
    }

    #[test]
    fn discovers_installed_hiddify_appimage() {
        let directory = tempfile::tempdir().expect("tempdir");
        let apps = directory.path().join("apps");
        fs::create_dir_all(&apps).expect("apps");
        let appimage = apps.join("Hiddify.AppImage");
        fs::write(&appimage, b"elf").expect("appimage");
        let found = LinuxBackend::discover_hiddify(&AppConfig::default(), directory.path());
        assert_eq!(found, Some(appimage));
    }

    fn test_paths(directory: &tempfile::TempDir) -> LinuxPaths {
        LinuxPaths {
            socket_path: directory.path().join("helper.sock"),
            user_data_dir: directory.path().join("user-data"),
            system_runtime_dir: PathBuf::from("/var/lib/iran-split"),
            resources_dir: directory.path().join("resources"),
            rules_cache_dir: directory.path().join("rules-cache"),
            mihomo_binary: directory.path().join("mihomo"),
        }
    }

    /// A Happ instance that can never start: closed port and missing binary.
    fn unstartable_happ(directory: &tempfile::TempDir) -> ClientInstance {
        let mut happ = ClientInstance::from_preset(PresetId::Happ);
        if let ClientConfig::LocalProxy {
            host,
            port,
            executable,
            ..
        } = &mut happ.config
        {
            *host = "127.0.0.1".into();
            *port = 1;
            *executable = ExecutableSetting::Path(directory.path().join("missing-happ"));
        }
        happ
    }

    #[tokio::test]
    async fn a_required_primary_that_cannot_start_aborts_connect() {
        let directory = tempfile::tempdir().expect("tempdir");
        let happ = unstartable_happ(&directory);
        let mut config = AppConfig::default();
        config.clients.clear();
        config.default_route = DefaultRoute::client(happ.id);
        config.clients.push(happ);
        let backend = LinuxBackend::new(config, test_paths(&directory));
        let error = backend
            .ensure_enabled_clients(CancellationToken::new())
            .await;
        assert!(error.is_err(), "the default-route client must be required");
    }

    #[tokio::test]
    async fn an_optional_local_proxy_launch_does_not_wait_for_its_port() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut config = AppConfig::default();
        config.clients.clear();
        config.default_route = DefaultRoute::Direct;
        // A binary that exists and exits immediately: the old code waited the
        // full start timeout for port 1 to open; the new code returns at once.
        let mut happ = ClientInstance::from_preset(PresetId::Happ);
        if let ClientConfig::LocalProxy {
            host,
            port,
            executable,
            start_timeout_seconds,
            ..
        } = &mut happ.config
        {
            *host = "127.0.0.1".into();
            *port = 1;
            *executable = ExecutableSetting::Path(PathBuf::from("/usr/bin/true"));
            *start_timeout_seconds = 45;
        }
        config.clients.push(happ);
        let backend = LinuxBackend::new(config, test_paths(&directory));
        let started = tokio::time::Instant::now();
        backend
            .ensure_enabled_clients(CancellationToken::new())
            .await
            .expect("optional launch must not abort connect");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "optional client launch must not block on its port"
        );
        assert!(backend.egress_handles.lock().await.is_empty());
    }

    #[tokio::test]
    async fn a_dead_optional_secondary_does_not_block_connect() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut config = AppConfig::default();
        // Drop the default Hiddify instance so a live dev-host proxy is
        // never probed; DIRECT keeps every remaining client optional.
        config.clients.clear();
        config.default_route = DefaultRoute::Direct;
        config.clients.push(unstartable_happ(&directory));
        let backend = LinuxBackend::new(config, test_paths(&directory));
        backend
            .ensure_enabled_clients(CancellationToken::new())
            .await
            .expect("optional client failure must not abort connect");
        assert!(backend.egress_handles.lock().await.is_empty());
    }

    #[test]
    fn recovery_candidates_are_enabled_local_proxies_without_handles() {
        let mut config = AppConfig::default();
        let happ = ClientInstance::from_preset(PresetId::Happ);
        let happ_id = happ.id;
        let mut disabled = ClientInstance::from_preset(PresetId::V2rayn);
        disabled.enabled = false;
        let side_tunnel = ClientInstance::from_preset(PresetId::Windscribe);
        config.clients.push(happ);
        config.clients.push(disabled);
        config.clients.push(side_tunnel);

        // The default Hiddify instance and Happ lack handles; only the
        // disabled client and the side tunnel are excluded.
        let candidates = clients_missing_egress(&config, &[]);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|client| client.id == happ_id));

        // A handle from connect (or a previous recovery) removes a candidate.
        let handled = config
            .clients
            .iter()
            .find(|client| client.id == happ_id)
            .and_then(synthesized_local_handle)
            .expect("happ handle");
        let candidates = clients_missing_egress(&config, &[handled]);
        assert!(!candidates.iter().any(|client| client.id == happ_id));
    }

    #[tokio::test]
    async fn recovery_skips_clients_whose_port_is_closed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = LinuxPaths {
            socket_path: directory.path().join("helper.sock"),
            user_data_dir: directory.path().join("user-data"),
            system_runtime_dir: PathBuf::from("/var/lib/iran-split"),
            resources_dir: directory.path().join("resources"),
            rules_cache_dir: directory.path().join("rules-cache"),
            mihomo_binary: directory.path().join("mihomo"),
        };
        let mut config = AppConfig::default();
        // Drop the default Hiddify instance: on a dev host a real Hiddify may
        // be listening, and this test must never probe a live proxy.
        config.clients.clear();
        // Port 1 on loopback is never listening in the test environment.
        let mut happ = ClientInstance::from_preset(PresetId::Happ);
        if let ClientConfig::LocalProxy { host, port, .. } = &mut happ.config {
            *host = "127.0.0.1".into();
            *port = 1;
        }
        config.clients.push(happ);
        let backend = LinuxBackend::new(config, paths);
        let recovered = backend
            .recover_local_proxy_clients()
            .await
            .expect("recovery");
        assert!(!recovered);
        assert!(backend.egress_handles.lock().await.is_empty());
    }

    #[test]
    fn discovers_happ_when_path_filename_is_lowercase() {
        let directory = tempfile::tempdir().expect("tempdir");
        let binary = directory.path().join("happ");
        fs::write(&binary, b"elf").expect("write");
        let found = discover_local_proxy_binary_in(
            &PresetId::Happ.spec(),
            &[directory.path().to_path_buf()],
            &[],
        );
        assert_eq!(found, Some(binary));
    }

    #[test]
    fn discovers_happ_from_a_well_known_install_path() {
        let directory = tempfile::tempdir().expect("tempdir");
        let binary = directory.path().join("Happ");
        fs::write(&binary, b"elf").expect("write");
        let found = discover_local_proxy_binary_in(&PresetId::Happ.spec(), &[], &[binary.clone()]);
        assert_eq!(found, Some(binary));
    }

    #[test]
    fn side_tunnel_ipc_budget_follows_the_command_timeout() {
        let production = include_str!("lib.rs")
            .split("mod tests {")
            .next()
            .expect("production source");
        assert!(production.contains("helper_ipc_reply_timeout"));
        assert!(!production.contains("SIDE_TUNNEL_IPC_TIMEOUT"));
    }
}
