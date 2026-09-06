mod connect_prep;
mod deps;
mod diagnostics;
mod github_update;
mod helper_install;
mod hiddify_reset;
mod network;
mod profile_picker;
mod reachability;
mod traffic;
mod tray;
mod version;
mod window_state;

use chrono::Utc;
use iran_split_config::{
    AppConfig, ClientId, ConfigStore, DefaultRoute, PresetId, ValidationIssue,
};
use iran_split_core::{
    ComponentPhase, Engine, LifecycleBusy, OperationAccepted, PlatformBackend, StackPhase,
    StackSnapshot,
};
use iran_split_mihomo::{ActiveConnection, ControllerClient};
use iran_split_rules::{
    bundled_snapshot_is_complete, ensure_bundled_snapshot, CloudRuleStore, CloudRulesStatus,
    DirectRulesDocument, DohResolver, Outbound, RuleManager, RuleSet,
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, Runtime, Size, Window, WindowEvent,
};
use tracing::{error, info, warn};
use uuid::Uuid;

#[cfg(target_os = "linux")]
use iran_split_platform_linux::{LinuxBackend as NativeBackend, LinuxPaths};
#[cfg(target_os = "windows")]
use iran_split_platform_win::{WindowsBackend as NativeBackend, WindowsPaths, HELPER_PIPE};

#[derive(Debug)]
struct AppServices {
    config_store: ConfigStore,
    engine: Arc<Engine<NativeBackend>>,
    backend: Arc<NativeBackend>,
    rules: RuleManager,
    cloud_rules: CloudRuleStore,
    network: network::NetworkMonitor,
    paths: AppPaths,
    updates: Arc<UpdateCoordinator>,
    traffic: tokio::sync::Mutex<traffic::SessionAccumulator>,
}

#[derive(Debug, Clone, Serialize)]
struct UpdateSnapshot {
    operation_id: Option<Uuid>,
    initiator: Option<String>,
    phase: String,
    percent: Option<u8>,
    version: Option<String>,
    error: Option<String>,
    app_available: bool,
    rules_available: bool,
    thirdparty_available: bool,
}

impl Default for UpdateSnapshot {
    fn default() -> Self {
        Self {
            operation_id: None,
            initiator: None,
            phase: "idle".into(),
            percent: None,
            version: None,
            error: None,
            app_available: false,
            rules_available: false,
            thirdparty_available: false,
        }
    }
}

struct UpdateCoordinator {
    lock: tokio::sync::Mutex<()>,
    cancel: AtomicBool,
    snapshot: std::sync::Mutex<UpdateSnapshot>,
    pending_package: std::sync::Mutex<Option<github_update::UpdateInfo>>,
}

impl std::fmt::Debug for UpdateCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateCoordinator")
            .field("cancel", &self.cancel.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl UpdateCoordinator {
    fn new() -> Self {
        Self {
            lock: tokio::sync::Mutex::new(()),
            cancel: AtomicBool::new(false),
            snapshot: std::sync::Mutex::new(UpdateSnapshot::default()),
            pending_package: std::sync::Mutex::new(None),
        }
    }

    fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn snapshot(&self) -> UpdateSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn store(&self, snapshot: UpdateSnapshot) {
        *self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot;
    }

    fn remember_package(&self, info: Option<github_update::UpdateInfo>) {
        *self
            .pending_package
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = info;
    }

    fn pending_package(&self) -> Option<github_update::UpdateInfo> {
        self.pending_package
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn try_begin(
        &self,
        initiator: &'static str,
        phase: &str,
    ) -> Option<(tokio::sync::MutexGuard<'_, ()>, Uuid)> {
        let guard = self.lock.try_lock().ok()?;
        self.cancel.store(false, Ordering::SeqCst);
        let operation_id = Uuid::new_v4();
        self.store(UpdateSnapshot {
            operation_id: Some(operation_id),
            initiator: Some(initiator.into()),
            phase: phase.into(),
            ..UpdateSnapshot::default()
        });
        Some((guard, operation_id))
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

fn update_check_cancelled(app: &AppHandle) -> bool {
    services(app)
        .ok()
        .is_some_and(|services| services.updates.is_cancelled())
}

#[derive(Debug, Clone)]
struct AppPaths {
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    resources: PathBuf,
    dependencies: PathBuf,
}

#[cfg(target_os = "linux")]
const PRODUCTION_HELPER_SOCKET: &str = "/run/iran-split/helper.sock";
#[cfg(target_os = "linux")]
const PRODUCTION_SYSTEM_RUNTIME: &str = "/var/lib/iran-split";

/// Written by the elevated installer in `iran-split-helper::install` (ADR 0029).
#[cfg(target_os = "windows")]
const WINDOWS_SYSTEM_RUNTIME: &str = r"C:\ProgramData\iran-split\runtime";
#[cfg(target_os = "windows")]
const WINDOWS_PROGRAMDATA_MIHOMO: &str = r"C:\ProgramData\iran-split\bin\mihomo.exe";

/// The pipe and system runtime root are fixed by the SYSTEM scheduled task, so
/// unlike Linux there is no development override to apply.
#[cfg(target_os = "windows")]
fn windows_helper_paths() -> (String, PathBuf) {
    (
        HELPER_PIPE.to_owned(),
        PathBuf::from(WINDOWS_SYSTEM_RUNTIME),
    )
}

/// The helper copies Mihomo next to itself, so that copy is the fallback when
/// the user has not installed one under the app data directory.
#[cfg(target_os = "windows")]
fn windows_programdata_mihomo() -> PathBuf {
    PathBuf::from(WINDOWS_PROGRAMDATA_MIHOMO)
}

#[cfg(target_os = "linux")]
fn linux_helper_paths() -> (PathBuf, PathBuf) {
    #[cfg(debug_assertions)]
    {
        linux_helper_paths_with_overrides(
            std::env::var_os("BIFLOW_DEV_HELPER_SOCKET"),
            std::env::var_os("BIFLOW_DEV_SYSTEM_RUNTIME"),
        )
    }
    #[cfg(not(debug_assertions))]
    {
        (
            PathBuf::from(PRODUCTION_HELPER_SOCKET),
            PathBuf::from(PRODUCTION_SYSTEM_RUNTIME),
        )
    }
}

#[cfg(all(target_os = "linux", debug_assertions))]
fn linux_helper_paths_with_overrides(
    socket: Option<std::ffi::OsString>,
    runtime: Option<std::ffi::OsString>,
) -> (PathBuf, PathBuf) {
    (
        socket.map_or_else(|| PathBuf::from(PRODUCTION_HELPER_SOCKET), PathBuf::from),
        runtime.map_or_else(|| PathBuf::from(PRODUCTION_SYSTEM_RUNTIME), PathBuf::from),
    )
}

#[cfg(target_os = "linux")]
fn linux_mihomo_binary(default: PathBuf) -> PathBuf {
    #[cfg(debug_assertions)]
    {
        linux_mihomo_binary_with_override(default, std::env::var_os("BIFLOW_DEV_MIHOMO_BINARY"))
    }
    #[cfg(not(debug_assertions))]
    {
        default
    }
}

#[cfg(all(target_os = "linux", debug_assertions))]
fn linux_mihomo_binary_with_override(
    default: PathBuf,
    override_path: Option<std::ffi::OsString>,
) -> PathBuf {
    override_path.map_or(default, PathBuf::from)
}

const BUNDLE_IDENTIFIER: &str = "app.biflow.desktop";

#[cfg(target_os = "linux")]
const WEBKIT_DISABLE_DMABUF_RENDERER: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
#[cfg(target_os = "linux")]
const WEBKIT_DISABLE_COMPOSITING_MODE: &str = "WEBKIT_DISABLE_COMPOSITING_MODE";
#[cfg(target_os = "linux")]
const LIBGL_ALWAYS_SOFTWARE: &str = "LIBGL_ALWAYS_SOFTWARE";

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxWebviewWorkarounds {
    disable_dmabuf: bool,
    disable_compositing: bool,
    software_gl: bool,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxWebviewEnv {
    dmabuf_already_set: bool,
    compositing_already_set: bool,
    software_gl_already_set: bool,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxGpuKind {
    virtual_or_nvidia: bool,
    virtual_machine: bool,
}

fn single_instance_dbus_id(identifier: &str, version: &str) -> String {
    format!("{identifier}.v{}", version.replace('.', "_"))
}

#[cfg(target_os = "linux")]
fn linux_dmi_is_virtual(vendor: Option<&str>) -> bool {
    vendor.is_some_and(|value| {
        let vendor = value.trim().to_ascii_lowercase();
        vendor.contains("vmware")
            || vendor.contains("qemu")
            || vendor.contains("virtualbox")
            || vendor.contains("microsoft corporation")
            || vendor.contains("xen")
            || vendor.contains("bochs")
            || vendor.contains("parallels")
            || vendor.contains("amazon")
            || vendor.contains("google")
    })
}

#[cfg(target_os = "linux")]
fn linux_virtual_machine() -> bool {
    linux_dmi_is_virtual(
        fs::read_to_string("/sys/class/dmi/id/sys_vendor")
            .ok()
            .as_deref(),
    )
}

#[cfg(target_os = "linux")]
fn linux_nvidia_gpu() -> bool {
    Path::new("/dev/nvidia0").exists() || Path::new("/proc/driver/nvidia/version").exists()
}

#[cfg(target_os = "linux")]
fn linux_webview_workarounds(env: LinuxWebviewEnv, gpu: LinuxGpuKind) -> LinuxWebviewWorkarounds {
    LinuxWebviewWorkarounds {
        disable_dmabuf: !env.dmabuf_already_set,
        disable_compositing: gpu.virtual_or_nvidia && !env.compositing_already_set,
        software_gl: gpu.virtual_machine && !env.software_gl_already_set,
    }
}

#[cfg(target_os = "linux")]
fn linux_webview_reexec_needed(already_relaunched: bool, planned: LinuxWebviewWorkarounds) -> bool {
    !already_relaunched
        && (planned.disable_dmabuf || planned.disable_compositing || planned.software_gl)
}

#[cfg(target_os = "linux")]
fn apply_linux_webview_workarounds() {
    use std::os::unix::process::CommandExt;
    let virtual_machine = linux_virtual_machine();
    let planned = linux_webview_workarounds(
        LinuxWebviewEnv {
            dmabuf_already_set: std::env::var_os(WEBKIT_DISABLE_DMABUF_RENDERER).is_some(),
            compositing_already_set: std::env::var_os(WEBKIT_DISABLE_COMPOSITING_MODE).is_some(),
            software_gl_already_set: std::env::var_os(LIBGL_ALWAYS_SOFTWARE).is_some(),
        },
        LinuxGpuKind {
            virtual_or_nvidia: virtual_machine || linux_nvidia_gpu(),
            virtual_machine,
        },
    );
    if !linux_webview_reexec_needed(
        std::env::var_os("BIFLOW_WEBKIT_WORKAROUNDS").is_some(),
        planned,
    ) {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/proc/self/exe"));
    let mut command = linux_webview_reexec_command(exe, std::env::args_os().skip(1), planned);
    // Replace this process so a waiting parent (and its terminal) does not linger.
    let cause = command.exec();
    eprintln!("BiFlow could not relaunch with WebKit view workarounds: {cause}");
}

#[cfg(target_os = "linux")]
fn linux_webview_reexec_command(
    exe: PathBuf,
    args: impl IntoIterator<Item = std::ffi::OsString>,
    planned: LinuxWebviewWorkarounds,
) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command.args(args);
    command.env("BIFLOW_WEBKIT_WORKAROUNDS", "1");
    if planned.disable_dmabuf {
        command.env(WEBKIT_DISABLE_DMABUF_RENDERER, "1");
    }
    if planned.disable_compositing {
        command.env(WEBKIT_DISABLE_COMPOSITING_MODE, "1");
    }
    if planned.software_gl {
        command.env(LIBGL_ALWAYS_SOFTWARE, "1");
    }
    command
}

#[cfg(target_os = "linux")]
fn log_linux_webview_workarounds() {
    info!(
        event = "webview.linux_workarounds",
        section = "window",
        initiator = "application_process",
        cause = "webkitgtk_dmabuf_blank_view",
        trace_route = "application_process->run->webkit_env",
        dmabuf_disabled = std::env::var_os(WEBKIT_DISABLE_DMABUF_RENDERER).is_some(),
        compositing_disabled = std::env::var_os(WEBKIT_DISABLE_COMPOSITING_MODE).is_some(),
        software_gl = std::env::var_os(LIBGL_ALWAYS_SOFTWARE).is_some(),
        "Linux WebKit view workarounds applied"
    );
}

impl AppPaths {
    fn discover(app: &AppHandle) -> Result<Self, String> {
        // A dev run must never open the production profile: schema migration
        // is one-way, so `dev.sh` points config/data/cache at a sibling
        // profile and the installed app keeps working.
        let profile = std::env::var_os("BIFLOW_DEV_PROFILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let (config_root, data_root, cache_root) = match &profile {
            Some(root) => (root.join("config"), root.join("data"), root.join("cache")),
            None => (
                dirs::config_dir()
                    .ok_or("configuration directory is unavailable")?
                    .join("biflow"),
                dirs::data_local_dir()
                    .ok_or("local data directory is unavailable")?
                    .join("biflow"),
                dirs::cache_dir()
                    .ok_or("cache directory is unavailable")?
                    .join("biflow"),
            ),
        };
        let config = config_root.join("config.toml");
        let data = data_root;
        let cache = cache_root;
        let resource_root = app
            .path()
            .resource_dir()
            .map_err(|error| error.to_string())?;
        let resources = packaged_rule_snapshot_dir(&resource_root);
        let dependencies = resource_root.join("dependencies");
        fs::create_dir_all(&data).map_err(|error| error.to_string())?;
        fs::create_dir_all(&cache).map_err(|error| error.to_string())?;
        Ok(Self {
            config,
            data,
            cache,
            resources,
            dependencies,
        })
    }
}

fn packaged_rule_snapshot_dir(resource_root: &Path) -> PathBuf {
    [
        resource_root.join("rules"),
        resource_root.join("resources").join("rules"),
        resource_root.join("_up_").join("resources").join("rules"),
    ]
    .into_iter()
    .find(|dir| bundled_snapshot_is_complete(dir))
    .unwrap_or_else(|| resource_root.join("rules"))
}

#[derive(Debug, Clone, Serialize)]
struct BootstrapResult {
    app_version: String,
    platform: String,
    mock_mode: bool,
    snapshot: StackSnapshot,
    settings: AppConfig,
    direct_rules: DirectRulesDocument,
    cloud_rules: CloudRulesStatus,
    dependencies: Vec<deps::DependencyStatus>,
    network_status: network::NetworkStatus,
}

#[derive(Debug, Clone, Serialize)]
struct RouteTestResult {
    target: String,
    outbound: Outbound,
    reason: String,
    matched_rule: Option<String>,
    reachable: Option<bool>,
    tested_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct DiagnosticStep {
    id: String,
    label: String,
    status: String,
    detail: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DiagnosticsReport {
    operation_id: Uuid,
    steps: Vec<DiagnosticStep>,
    finished: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ExportResult {
    path: String,
    files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "IPC shape matches the About page channel flags"
)]
struct UpdateStatus {
    available: bool,
    version: Option<String>,
    notes: Option<String>,
    app_available: bool,
    rules_available: bool,
    thirdparty_available: bool,
}

#[derive(Debug, Clone, Serialize)]
struct UpdateProgress {
    phase: String,
    percent: Option<u8>,
    version: Option<String>,
    error: Option<String>,
    operation_id: Option<Uuid>,
    app_available: Option<bool>,
    rules_available: Option<bool>,
    thirdparty_available: Option<bool>,
}

fn update_download_percent(downloaded: u64, total: Option<u64>) -> Option<u8> {
    let total = total?;
    if total == 0 {
        return Some(0);
    }
    Some(
        u8::try_from((u128::from(downloaded).saturating_mul(100) / u128::from(total)).min(100))
            .unwrap_or(100),
    )
}

fn progress_from_snapshot(snapshot: &UpdateSnapshot) -> UpdateProgress {
    UpdateProgress {
        phase: snapshot.phase.clone(),
        percent: snapshot.percent,
        version: snapshot.version.clone(),
        error: snapshot.error.clone(),
        operation_id: snapshot.operation_id,
        app_available: Some(snapshot.app_available),
        rules_available: Some(snapshot.rules_available),
        thirdparty_available: Some(snapshot.thirdparty_available),
    }
}

fn status_from_snapshot(snapshot: &UpdateSnapshot) -> UpdateStatus {
    UpdateStatus {
        available: snapshot.app_available
            || snapshot.rules_available
            || snapshot.thirdparty_available,
        version: snapshot.version.clone(),
        notes: None,
        app_available: snapshot.app_available,
        rules_available: snapshot.rules_available,
        thirdparty_available: snapshot.thirdparty_available,
    }
}

fn emit_update_progress<R: Runtime>(app: &AppHandle<R>, mut progress: UpdateProgress) {
    if let Ok(services) = services(app) {
        let current = services.updates.snapshot();
        if progress.operation_id.is_none() {
            progress.operation_id = current.operation_id;
        }
        services.updates.store(UpdateSnapshot {
            operation_id: progress.operation_id,
            initiator: current.initiator,
            phase: progress.phase.clone(),
            percent: progress.percent,
            version: progress.version.clone(),
            error: progress.error.clone(),
            app_available: progress.app_available.unwrap_or(current.app_available),
            rules_available: progress.rules_available.unwrap_or(current.rules_available),
            thirdparty_available: progress
                .thirdparty_available
                .unwrap_or(current.thirdparty_available),
        });
    }
    if let Err(cause) = app.emit("update-progress", &progress) {
        warn!(
            event = "update.progress_emit_failed",
            section = "updates",
            initiator = "install_update",
            cause = %cause,
            trace_route = "install_update->frontend_event",
            "update progress event could not be emitted"
        );
    }
}

async fn pause_stack_for_update(services: &AppServices) -> Result<(), String> {
    if matches!(
        services.engine.snapshot().phase,
        StackPhase::Running | StackPhase::Degraded
    ) {
        services
            .engine
            .pause_stack()
            .await
            .map_err(|error| error.to_string())?;
        services
            .engine
            .wait_for_phase(StackPhase::Paused, Duration::from_secs(25))
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn services<R: Runtime>(app: &AppHandle<R>) -> Result<&AppServices, String> {
    app.try_state::<AppServices>()
        .map(|state| state.inner())
        .ok_or_else(|| "application services are not initialized".into())
}

#[tauri::command]
async fn bootstrap_app(app: AppHandle) -> Result<BootstrapResult, String> {
    diagnostics::trace_action("startup", "tauri_command", "bootstrap_app", async move {
        let services = services(&app)?;
        if let Err(cause) = verify_direct_rules_after_upgrade(&services.paths.data) {
            error!(
                event = "update.rules_lost",
                section = "updates",
                initiator = "bootstrap_app",
                cause = cause.as_str(),
                trace_route = "bootstrap->direct_rules_guard",
                "custom route pins were missing after an application update"
            );
            emit_update_progress(
                &app,
                UpdateProgress {
                    phase: "failed".into(),
                    percent: None,
                    version: None,
                    error: Some(cause),
                    operation_id: None,
                    app_available: None,
                    rules_available: None,
                    thirdparty_available: None,
                },
            );
        }
        services.engine.refresh_health().await;
        let settings = services
            .config_store
            .load_or_create()
            .map_err(|error| error.to_string())?
            .redacted();
        Ok(BootstrapResult {
            app_version: version::app_version().to_owned(),
            platform: std::env::consts::OS.into(),
            mock_mode: false,
            snapshot: services.engine.snapshot(),
            settings,
            direct_rules: services.rules.list().await,
            cloud_rules: services
                .cloud_rules
                .status()
                .map_err(|error| error.to_string())?,
            dependencies: deps::dependency_status(&services.paths.data),
            network_status: network::NetworkStatus::default(),
        })
    })
    .await
}

#[tauri::command]
async fn get_network_status(app: AppHandle) -> Result<network::NetworkStatus, String> {
    diagnostics::trace_action(
        "network",
        "tauri_command",
        "get_network_status",
        async move { Ok(services(&app)?.network.check().await) },
    )
    .await
}

#[tauri::command]
async fn check_reachability(
    app: AppHandle,
) -> Result<Vec<reachability::ReachabilityResult>, String> {
    diagnostics::trace_action(
        "network",
        "tauri_command",
        "check_reachability",
        async move {
            let services = services(&app)?;
            // Route VPN-path probes through a running local proxy. google.com
            // prefers Happ; facebook.com prefers Hiddify. Mixed-port probes
            // would silently hit DIRECT because the desktop is bypassed.
            let snapshot = services.engine.snapshot();
            let config = services
                .config_store
                .load()
                .or_else(|_| services.config_store.load_or_create())
                .map_err(|error| error.to_string())?;
            Ok(reachability::check_all(reachability_proxies(&snapshot, &config)).await)
        },
    )
    .await
}

fn client_preset_running(snapshot: &StackSnapshot, preset: PresetId) -> bool {
    snapshot
        .clients
        .iter()
        .any(|client| client.preset == preset && client.status.phase == ComponentPhase::Running)
}

fn running_local_proxy(
    snapshot: &StackSnapshot,
    config: &AppConfig,
    preset: PresetId,
) -> Option<(String, u16)> {
    if !client_preset_running(snapshot, preset) {
        return None;
    }
    config
        .clients
        .iter()
        .find(|client| client.enabled && client.preset == preset)
        .and_then(iran_split_clients::local_proxy_endpoint)
}

fn reachability_proxies(snapshot: &StackSnapshot, config: &AppConfig) -> reachability::VpnProxies {
    reachability::VpnProxies {
        default: running_local_proxy(snapshot, config, PresetId::Hiddify),
        happ: running_local_proxy(snapshot, config, PresetId::Happ),
    }
}

const TRAFFIC_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[tauri::command]
async fn get_traffic_totals(app: AppHandle) -> Result<traffic::TrafficTotals, String> {
    diagnostics::trace_action(
        "traffic",
        "tauri_command",
        "get_traffic_totals",
        async move {
            let services = services(&app)?;
            let mut store = services.traffic.lock().await;
            let connected = matches!(
                services.engine.snapshot().phase,
                StackPhase::Running | StackPhase::Degraded
            );
            let (session_sent, session_received) = if connected {
                match session_connection_totals(services).await {
                    Ok(totals) => totals,
                    Err(cause) => {
                        warn!(
                            event = "traffic.session_probe_failed",
                            section = "traffic",
                            initiator = "get_traffic_totals",
                            cause = %cause,
                            trace_route = "tauri_command->mihomo_controller",
                            "session traffic totals were unavailable; using last known session"
                        );
                        store.last_generation()
                    }
                }
            } else {
                (0, 0)
            };
            Ok(traffic::accumulate(
                &mut store,
                session_sent,
                session_received,
                connected,
            ))
        },
    )
    .await
}

async fn session_connection_totals(services: &AppServices) -> Result<(u64, u64), String> {
    let config = services
        .config_store
        .load()
        .or_else(|_| services.config_store.load_or_create())
        .map_err(|error| error.to_string())?;
    let client = ControllerClient::new(
        &config.mihomo.controller_host,
        config.mihomo.controller_port,
        &config.mihomo.controller_secret,
    )
    .map_err(|error| error.to_string())?;
    tokio::time::timeout(TRAFFIC_PROBE_TIMEOUT, client.connection_totals())
        .await
        .map_err(|_| "traffic probe timed out".to_owned())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn list_active_connections(app: AppHandle) -> Result<Vec<ActiveConnection>, String> {
    diagnostics::trace_action(
        "connections",
        "tauri_command",
        "list_active_connections",
        async move {
            let services = services(&app)?;
            if !matches!(
                services.engine.snapshot().phase,
                StackPhase::Running | StackPhase::Degraded
            ) {
                return Ok(Vec::new());
            }
            let rows = active_mihomo_connections(services).await?;
            info!(
                event = "connections.listed",
                section = "connections",
                initiator = "tauri_command",
                cause = "diagnostics_poll",
                trace_route = "tauri_command->mihomo_controller",
                count = rows.len(),
                "listed live Mihomo connections without host values"
            );
            Ok(if reachability::hide_google_in_this_build() {
                rows.into_iter()
                    .filter(|row| !reachability::is_google_host(&row.host))
                    .collect()
            } else {
                rows
            })
        },
    )
    .await
}

async fn active_mihomo_connections(
    services: &AppServices,
) -> Result<Vec<ActiveConnection>, String> {
    let config = services
        .config_store
        .load()
        .or_else(|_| services.config_store.load_or_create())
        .map_err(|error| error.to_string())?;
    let client = ControllerClient::new(
        &config.mihomo.controller_host,
        config.mihomo.controller_port,
        &config.mihomo.controller_secret,
    )
    .map_err(|error| error.to_string())?;
    tokio::time::timeout(TRAFFIC_PROBE_TIMEOUT, client.active_connections())
        .await
        .map_err(|_| "connection listing timed out".to_owned())?
        .map_err(|error| error.to_string())
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri injects AppHandle command arguments by value"
)]
#[tauri::command]
fn get_stack_snapshot(app: AppHandle) -> Result<StackSnapshot, String> {
    diagnostics::trace_sync("stack", "tauri_command", "get_stack_snapshot", || {
        Ok(services(&app)?.engine.snapshot())
    })
}

#[tauri::command]
async fn start_stack(
    app: AppHandle,
    side_tunnel_timeout_seconds: Option<u64>,
) -> Result<OperationAccepted, String> {
    diagnostics::trace_action("stack", "tauri_command", "start_stack", async move {
        start_stack_inner(&app, side_tunnel_timeout_seconds).await
    })
    .await
}

async fn start_stack_inner<R: Runtime>(
    app: &AppHandle<R>,
    side_tunnel_timeout_seconds: Option<u64>,
) -> Result<OperationAccepted, String> {
    let engine = &services(app)?.engine;
    if engine.snapshot().phase == StackPhase::Running {
        return Ok(OperationAccepted {
            operation_id: uuid::Uuid::new_v4(),
            already_complete: true,
        });
    }
    engine
        .reserve_lifecycle(LifecycleBusy::Connecting)
        .await
        .map_err(|error| error.to_string())?;
    if let Err(error) = prepare_stack_start(app).await {
        engine.release_lifecycle(LifecycleBusy::Connecting).await;
        return Err(error);
    }
    engine
        .start_stack(side_tunnel_timeout_seconds)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn retry_side_tunnels(
    app: AppHandle,
    side_tunnel_timeout_seconds: u64,
) -> Result<bool, String> {
    diagnostics::trace_action("stack", "tauri_command", "retry_side_tunnels", async move {
        let services = services(&app)?;
        if side_tunnel_timeout_seconds == 0 || side_tunnel_timeout_seconds > 300 {
            return Err("side tunnel timeout must be between 1 and 300 seconds".into());
        }
        services
            .engine
            .retry_side_tunnels(side_tunnel_timeout_seconds)
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

async fn prepare_stack_start<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let services = services(app)?;
    let helper_ready = connect_prep::helper_is_ready(services.engine.snapshot().helper.phase);
    let statuses = deps::dependency_status(&services.paths.data);
    let hiddify_required = services
        .config_store
        .load()
        .map(|config| {
            config
                .clients
                .iter()
                .any(|client| client.preset == PresetId::Hiddify && client.enabled)
        })
        .unwrap_or(true);
    let hiddify = statuses
        .iter()
        .any(|item| item.id == "hiddify" && item.installed);
    let mihomo = statuses
        .iter()
        .any(|item| item.id == "mihomo" && item.installed);
    let hiddify_satisfied = hiddify || !hiddify_required;
    for requirement in connect_prep::missing_requirements(helper_ready, hiddify_satisfied, mihomo) {
        info!(
            event = "connect.install_required",
            section = "stack",
            initiator = "prepare_stack_start",
            cause = "missing_dependency",
            trace_route = "start_stack->prepare_stack_start",
            requirement = requirement.as_str(),
            "installing a required service before connect"
        );
        match requirement {
            connect_prep::ConnectRequirement::Helper => {
                helper_install::install_helper(app).await?;
                services.engine.refresh_health().await;
                if !connect_prep::helper_is_ready(services.engine.snapshot().helper.phase) {
                    return Err("privileged helper is still unavailable after installation".into());
                }
            }
            connect_prep::ConnectRequirement::Hiddify => {
                install_required_dependency(services, deps::DependencyId::Hiddify).await?;
            }
            connect_prep::ConnectRequirement::Mihomo => {
                install_required_dependency(services, deps::DependencyId::Mihomo).await?;
            }
        }
    }
    services.engine.refresh_health().await;
    Ok(())
}

async fn install_required_dependency(
    services: &AppServices,
    id: deps::DependencyId,
) -> Result<(), String> {
    let result = deps::install_dependency(id, &services.paths.data, &services.paths.dependencies)
        .await
        .map_err(|error| error.to_string())?;
    if !result.installed {
        return Err(format!("{} installation did not complete", id.as_str()));
    }
    let statuses = deps::dependency_status(&services.paths.data);
    if !statuses
        .iter()
        .any(|item| item.id == id.as_str() && item.installed)
    {
        return Err(format!(
            "{} is still missing after installation",
            id.as_str()
        ));
    }
    Ok(())
}

#[tauri::command]
async fn stop_stack(app: AppHandle) -> Result<OperationAccepted, String> {
    diagnostics::trace_action("stack", "tauri_command", "stop_stack", async move {
        services(&app)?
            .engine
            .stop_stack()
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn pause_stack(app: AppHandle) -> Result<OperationAccepted, String> {
    diagnostics::trace_action("stack", "tauri_command", "pause_stack", async move {
        services(&app)?
            .engine
            .pause_stack()
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn resume_stack(app: AppHandle) -> Result<OperationAccepted, String> {
    diagnostics::trace_action("stack", "tauri_command", "resume_stack", async move {
        services(&app)?
            .engine
            .resume_stack()
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn restart_stack(app: AppHandle) -> Result<OperationAccepted, String> {
    diagnostics::trace_action("stack", "tauri_command", "restart_stack", async move {
        let services = services(&app)?;
        services
            .engine
            .stop_stack()
            .await
            .map_err(|error| error.to_string())?;
        services
            .engine
            .wait_for_phase(StackPhase::Stopped, Duration::from_secs(25))
            .await
            .map_err(|error| error.to_string())?;
        services
            .engine
            .start_stack(None)
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn cancel_operation(app: AppHandle, operation_id: Uuid) -> Result<bool, String> {
    diagnostics::trace_action("stack", "tauri_command", "cancel_operation", async move {
        info!(operation_id = %operation_id, "operation cancellation requested");
        let services = services(&app)?;
        services.updates.request_cancel();
        Ok(services.engine.cancel_operation(operation_id).await)
    })
    .await
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri injects AppHandle command arguments by value"
)]
#[tauri::command]
fn get_settings(app: AppHandle) -> Result<AppConfig, String> {
    diagnostics::trace_sync("settings", "tauri_command", "get_settings", || {
        Ok(services(&app)?
            .config_store
            .load_or_create()
            .map_err(|error| error.to_string())?
            .redacted())
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri injects AppHandle command arguments by value"
)]
#[tauri::command]
fn validate_settings(mut draft: AppConfig, app: AppHandle) -> Result<Vec<ValidationIssue>, String> {
    diagnostics::trace_sync("settings", "tauri_command", "validate_settings", || {
        let current = services(&app)?
            .config_store
            .load_or_create()
            .map_err(|error| error.to_string())?;
        draft.mihomo.controller_secret = current.mihomo.controller_secret;
        let issues = draft.validate();
        if !issues.is_empty() {
            warn!(
                event = "settings.validation_failed",
                section = "settings",
                initiator = "tauri_command",
                cause = "invalid_draft",
                issue_count = issues.len(),
                "settings validation returned issues"
            );
        }
        Ok(issues)
    })
}

#[tauri::command]
async fn save_settings(
    mut draft: AppConfig,
    expected_revision: u64,
    app: AppHandle,
) -> Result<AppConfig, String> {
    diagnostics::trace_action("settings", "tauri_command", "save_settings", async move {
        info!(expected_revision, "saving redacted settings revision");
        let services = services(&app)?;
        let current = services
            .config_store
            .load_or_create()
            .map_err(|error| error.to_string())?;
        restore_redacted_secrets(&current, &mut draft);
        draft
            .mihomo
            .controller_secret
            .clone_from(&current.mihomo.controller_secret);
        draft.sanitize_default_route();
        let saved = services
            .config_store
            .save(draft, expected_revision)
            .map_err(|error| error.to_string())?;
        info!(
            event = "settings.saved",
            section = "settings",
            initiator = "tauri_command",
            cause = "user_save",
            expected_revision,
            direct_dns_preset = %saved.mihomo.direct_dns_preset,
            "saved settings"
        );
        services.backend.update_config(saved.clone()).await;
        Ok(saved.redacted())
    })
    .await
}

#[tauri::command]
async fn discard_client_pins(
    id: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action(
        "rules",
        "tauri_command",
        "discard_client_pins",
        async move {
            let client_id = ClientId::parse(&id).map_err(|error| error.to_string())?;
            let services = services(&app)?;
            let mut document = services.rules.list().await;
            if document.revision != expected_revision {
                return Err(format!(
                    "rule revision conflict: expected {expected_revision}, found {}",
                    document.revision
                ));
            }
            if document.delete_client_pins(client_id) > 0 {
                document.revision = document.revision.saturating_add(1);
                services
                    .rules
                    .restore(document.clone())
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(document)
        },
    )
    .await
}

#[tauri::command]
async fn reassign_client_pins(
    from: String,
    to: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action(
        "rules",
        "tauri_command",
        "reassign_client_pins",
        async move {
            let from_id = ClientId::parse(&from).map_err(|error| error.to_string())?;
            let to_outbound = parse_outbound(&to)?;
            let services = services(&app)?;
            let mut document = services.rules.list().await;
            if document.revision != expected_revision {
                return Err(format!(
                    "rule revision conflict: expected {expected_revision}, found {}",
                    document.revision
                ));
            }
            if document.move_client_pins(from_id, to_outbound) > 0 {
                document.revision = document.revision.saturating_add(1);
                services
                    .rules
                    .restore(document.clone())
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(document)
        },
    )
    .await
}

#[tauri::command]
async fn list_direct_rules(app: AppHandle) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action("rules", "tauri_command", "list_direct_rules", async move {
        Ok(services(&app)?.rules.list().await)
    })
    .await
}

#[tauri::command]
async fn pin_route(
    input: String,
    outbound: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action("rules", "tauri_command", "pin_route", async move {
        let services = services(&app)?;
        let outbound = parse_outbound(&outbound)?;
        let policy = pin_policy_for(services, outbound)?;
        info!(
            expected_revision,
            outbound = ?outbound,
            input_kind = if input.parse::<std::net::IpAddr>().is_ok() {
                "ip"
            } else {
                "domain"
            },
            "pinning a host to one outbound without logging its value"
        );
        let previous = services.rules.list().await;
        let next = services
            .rules
            .pin_with_policy(&input, outbound, policy, expected_revision)
            .await;
        persist_and_apply_rules(&app, previous, next).await
    })
    .await
}

#[tauri::command]
async fn add_direct_rule(
    input: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action("rules", "tauri_command", "add_direct_rule", async move {
        info!(
            expected_revision,
            input_kind = if input.parse::<std::net::IpAddr>().is_ok() {
                "ip"
            } else {
                "domain"
            },
            "adding direct rule without logging its value"
        );
        let services = services(&app)?;
        let previous = services.rules.list().await;
        let next = services.rules.add(&input, expected_revision).await;
        persist_and_apply_rules(&app, previous, next).await
    })
    .await
}

#[tauri::command]
async fn remove_direct_rule(
    input: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action("rules", "tauri_command", "remove_direct_rule", async move {
        info!(
            expected_revision,
            "removing direct rule without logging its value"
        );
        let services = services(&app)?;
        let previous = services.rules.list().await;
        let next = services.rules.remove(&input, expected_revision).await;
        persist_and_apply_rules(&app, previous, next).await
    })
    .await
}

async fn persist_and_apply_rules(
    app: &AppHandle,
    previous: DirectRulesDocument,
    next: Result<DirectRulesDocument, iran_split_rules::RuleError>,
) -> Result<DirectRulesDocument, String> {
    let next = next.map_err(|error| error.to_string())?;
    let services = services(app)?;
    if let Err(cause) = services.engine.apply_user_rules().await {
        warn!(
            event = "rules.apply_failed",
            section = "rules",
            initiator = "tauri_command",
            cause = %cause,
            trace_route = "tauri_command->engine->apply_user_rules",
            "live rule apply failed; restoring the previous document"
        );
        services
            .rules
            .restore(previous)
            .await
            .map_err(|error| error.to_string())?;
        return Err(cause.to_string());
    }
    Ok(next)
}

#[tauri::command]
async fn create_rule_list(
    name: String,
    outbound: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action("rules", "tauri_command", "create_rule_list", async move {
        let outbound = parse_outbound(&outbound)?;
        services(&app)?
            .rules
            .create_list(&name, outbound, expected_revision)
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn rename_rule_list(
    list_id: String,
    name: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action("rules", "tauri_command", "rename_rule_list", async move {
        let list_id = parse_list_id(&list_id)?;
        services(&app)?
            .rules
            .rename_list(list_id, &name, expected_revision)
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn delete_rule_list(
    list_id: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action("rules", "tauri_command", "delete_rule_list", async move {
        let list_id = parse_list_id(&list_id)?;
        let services = services(&app)?;
        let previous = services.rules.list().await;
        let next = services.rules.delete_list(list_id, expected_revision).await;
        persist_and_apply_rules(&app, previous, next).await
    })
    .await
}

#[tauri::command]
async fn set_rule_list_outbound(
    list_id: String,
    outbound: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action(
        "rules",
        "tauri_command",
        "set_rule_list_outbound",
        async move {
            let list_id = parse_list_id(&list_id)?;
            let outbound = parse_outbound(&outbound)?;
            let services = services(&app)?;
            let policy = pin_policy_for(services, outbound)?;
            let previous = services.rules.list().await;
            let next = services
                .rules
                .set_list_outbound(list_id, outbound, policy, expected_revision)
                .await;
            persist_and_apply_rules(&app, previous, next).await
        },
    )
    .await
}

#[tauri::command]
async fn pin_to_rule_list(
    input: String,
    list_id: String,
    expected_revision: u64,
    app: AppHandle,
) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action("rules", "tauri_command", "pin_to_rule_list", async move {
        let list_id = parse_list_id(&list_id)?;
        let services = services(&app)?;
        let document = services.rules.list().await;
        let outbound = document
            .list_meta(list_id)
            .map(|list| list.outbound)
            .ok_or_else(|| "unknown list".to_owned())?;
        let policy = pin_policy_for(services, outbound)?;
        info!(
            expected_revision,
            input_kind = if input.parse::<std::net::IpAddr>().is_ok() {
                "ip"
            } else {
                "domain"
            },
            "pinning a host into a named list without logging its value"
        );
        let previous = services.rules.list().await;
        let next = services
            .rules
            .pin_to_list(&input, list_id, policy, expected_revision)
            .await;
        persist_and_apply_rules(&app, previous, next).await
    })
    .await
}

fn parse_list_id(value: &str) -> Result<uuid::Uuid, String> {
    value
        .parse::<uuid::Uuid>()
        .map_err(|_| format!("unknown list id: {value}"))
}

#[derive(Debug, Clone, Serialize)]
struct ListCheckEntry {
    target: String,
    status: &'static str,
    latency_ms: Option<u64>,
    detail: Option<String>,
}

const LIST_CHECK_SLOW_MS: u128 = 1_500;

#[tauri::command]
async fn check_rule_list(list_id: String, app: AppHandle) -> Result<Vec<ListCheckEntry>, String> {
    diagnostics::trace_action("rules", "tauri_command", "check_rule_list", async move {
        let list_id = parse_list_id(&list_id)?;
        let services = services(&app)?;
        let document = services.rules.list().await;
        let outbound = document
            .list_meta(list_id)
            .map(|list| list.outbound)
            .ok_or_else(|| "unknown list".to_owned())?;
        let domains: Vec<String> = document
            .pins_in_list(list_id)
            .into_iter()
            .filter_map(|pin| match &pin.target {
                iran_split_rules::DirectTarget::Domain(domain) => Some(domain.clone()),
                iran_split_rules::DirectTarget::Ip(_) => None,
            })
            .take(3)
            .collect();
        if domains.is_empty() {
            return Ok(Vec::new());
        }
        let config = services
            .config_store
            .load_or_create()
            .map_err(|error| error.to_string())?;
        let snapshot = services.engine.snapshot();
        let stack_running = matches!(snapshot.phase, StackPhase::Running | StackPhase::Degraded);
        // With the stack up, probe through Mihomo's mixed port so the real
        // routing rules decide the path. Otherwise a LocalProxy list can be
        // probed straight through its client's SOCKS endpoint.
        let proxy_url = if stack_running {
            format!("http://127.0.0.1:{}", config.mihomo.mixed_port)
        } else {
            let client_id = match outbound {
                Outbound::Client { client_id } => Some(client_id),
                Outbound::Direct => None,
            };
            let endpoint = client_id
                .and_then(|id| config.client(id))
                .filter(|client| client.enabled)
                .and_then(iran_split_clients::local_proxy_endpoint);
            let Some((host, port)) = endpoint else {
                return Err("connect first, or bind the list to a local proxy client".into());
            };
            format!("socks5h://{host}:{port}")
        };
        let mut results = Vec::with_capacity(domains.len());
        for domain in domains {
            results.push(probe_list_entry(&proxy_url, &domain).await);
        }
        Ok(results)
    })
    .await
}

async fn probe_list_entry(proxy_url: &str, domain: &str) -> ListCheckEntry {
    let build = || -> Result<reqwest::Client, reqwest::Error> {
        reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy_url)?)
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()
    };
    let client = match build() {
        Ok(client) => client,
        Err(error) => {
            return ListCheckEntry {
                target: domain.to_owned(),
                status: "fail",
                latency_ms: None,
                detail: Some(error.without_url().to_string()),
            };
        }
    };
    let started = std::time::Instant::now();
    // Any HTTP status counts as "the host answered through this egress";
    // only transport errors are failures.
    match client.head(format!("https://{domain}/")).send().await {
        Ok(_) => {
            let elapsed = started.elapsed().as_millis();
            ListCheckEntry {
                target: domain.to_owned(),
                status: if elapsed > LIST_CHECK_SLOW_MS {
                    "slow"
                } else {
                    "ok"
                },
                latency_ms: u64::try_from(elapsed).ok(),
                detail: None,
            }
        }
        Err(error) => ListCheckEntry {
            target: domain.to_owned(),
            status: "fail",
            latency_ms: None,
            detail: Some(error.without_url().to_string()),
        },
    }
}

#[tauri::command]
async fn refresh_direct_rules(app: AppHandle) -> Result<DirectRulesDocument, String> {
    diagnostics::trace_action(
        "rules",
        "tauri_command",
        "refresh_direct_rules",
        async move {
            services(&app)?
                .rules
                .refresh()
                .await
                .map_err(|error| error.to_string())
        },
    )
    .await
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri injects AppHandle command arguments by value"
)]
#[tauri::command]
fn get_cloud_rules_status(app: AppHandle) -> Result<CloudRulesStatus, String> {
    diagnostics::trace_sync(
        "cloud_rules",
        "tauri_command",
        "get_cloud_rules_status",
        || {
            services(&app)?
                .cloud_rules
                .status()
                .map_err(|error| error.to_string())
        },
    )
}

#[tauri::command]
async fn sync_cloud_rules(app: AppHandle) -> Result<CloudRulesStatus, String> {
    diagnostics::trace_action(
        "cloud_rules",
        "tauri_command",
        "sync_cloud_rules",
        async move {
            services(&app)?
                .cloud_rules
                .sync()
                .await
                .map_err(|error| error.to_string())
        },
    )
    .await
}

#[tauri::command]
async fn install_helper(app: AppHandle) -> Result<helper_install::InstallHelperResult, String> {
    diagnostics::trace_action("helper", "tauri_command", "install_helper", async move {
        // A dev run gets its transient helper from dev.sh. Running the
        // production installer here would reconfigure the system helper
        // with dev-profile paths and break the installed app.
        if std::env::var_os("BIFLOW_DEV_PROFILE").is_some_and(|value| !value.is_empty()) {
            return Err(
                "development run: restart ./dev.sh to provision the transient helper; \
                 the production helper installer is disabled in dev"
                    .into(),
            );
        }
        helper_install::install_helper(&app).await
    })
    .await
}

#[tauri::command]
async fn fresh_hiddify_start(app: AppHandle) -> Result<hiddify_reset::FreshStartReport, String> {
    diagnostics::trace_action(
        "hiddify_reset",
        "tauri_command",
        "fresh_hiddify_start",
        async move { hiddify_reset::fresh_start(&app).await },
    )
    .await
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri injects AppHandle command arguments by value"
)]
#[tauri::command]
fn list_dependencies(app: AppHandle) -> Result<Vec<deps::DependencyStatus>, String> {
    diagnostics::trace_sync("dependencies", "tauri_command", "list_dependencies", || {
        Ok(deps::dependency_status(&services(&app)?.paths.data))
    })
}

#[tauri::command]
async fn install_dependency(id: String, app: AppHandle) -> Result<deps::InstallResult, String> {
    diagnostics::trace_action(
        "dependencies",
        "tauri_command",
        "install_dependency",
        async move {
            let parsed = deps::DependencyId::parse(&id).map_err(|error| error.to_string())?;
            info!(
                dependency_id = parsed.as_str(),
                "dependency installation requested"
            );
            let services = services(&app)?;
            let result = deps::install_dependency(
                parsed,
                &services.paths.data,
                &services.paths.dependencies,
            )
            .await
            .map_err(|error| error.to_string())?;
            services.engine.refresh_health().await;
            Ok(result)
        },
    )
    .await
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command strings into owned values"
)]
#[tauri::command]
fn get_install_guide(id: String) -> Result<deps::InstallGuide, String> {
    diagnostics::trace_sync("dependencies", "tauri_command", "get_install_guide", || {
        let parsed = deps::DependencyId::parse(&id).map_err(|error| error.to_string())?;
        info!(
            dependency_id = parsed.as_str(),
            "dependency guide requested"
        );
        Ok(deps::install_guide(parsed))
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command strings into owned values"
)]
#[tauri::command]
fn client_binary_installed(preset: String) -> Result<bool, String> {
    diagnostics::trace_sync(
        "dependencies",
        "tauri_command",
        "client_binary_installed",
        || {
            let spec = iran_split_config::PresetId::all()
                .iter()
                .map(|preset| preset.spec())
                .find(|spec| spec.id == preset)
                .ok_or_else(|| format!("unknown preset: {preset}"))?;
            Ok(match spec.kind {
                // Side tunnels need the system OpenVPN binary the app
                // cannot ship; local proxies are probed at Connect instead.
                iran_split_config::EgressKind::OwnedSideTunnel => openvpn_binary_installed(),
                iran_split_config::EgressKind::LocalProxy => true,
                iran_split_config::EgressKind::Unsupported => false,
            })
        },
    )
}

fn openvpn_binary_installed() -> bool {
    // Kept in step with `candidate_binaries` in the helper's openvpn module.
    let fixed: &[&str] = if cfg!(windows) {
        &[
            r"C:\Program Files\OpenVPN\bin\openvpn.exe",
            r"C:\Program Files (x86)\OpenVPN\bin\openvpn.exe",
        ]
    } else {
        &[
            "/usr/sbin/openvpn",
            "/usr/bin/openvpn",
            "/sbin/openvpn",
            "/bin/openvpn",
        ]
    };
    if fixed.iter().any(|path| Path::new(path).is_file()) {
        return true;
    }
    let name = if cfg!(windows) {
        "openvpn.exe"
    } else {
        "openvpn"
    };
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(name).is_file())
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command strings into owned values"
)]
#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    diagnostics::trace_sync("dependencies", "tauri_command", "open_external_url", || {
        info!("opening an allowlisted external URL without logging its value");
        deps::open_allowlisted_url(&url).map_err(|error| error.to_string())
    })
}

#[tauri::command]
async fn pick_client_profile(app: AppHandle) -> Result<Option<String>, String> {
    diagnostics::trace_action(
        "clients",
        "tauri_command",
        "pick_client_profile",
        async move { profile_picker::pick_profile(&app).await },
    )
    .await
}

#[tauri::command]
async fn apply_live_settings(app: AppHandle) -> Result<(), String> {
    diagnostics::trace_action(
        "settings",
        "tauri_command",
        "apply_live_settings",
        async move {
            services(&app)?
                .engine
                .apply_user_rules()
                .await
                .map_err(|error| error.to_string())
        },
    )
    .await
}

#[tauri::command]
async fn test_route(target: String, app: AppHandle) -> Result<RouteTestResult, String> {
    diagnostics::trace_action("routing", "tauri_command", "test_route", async move {
        let services = services(&app)?;
        let document = services.rules.list().await;
        let domains = read_snapshot_lines(&services.cloud_rules.resolve("iran-domains.txt"))?
            .into_iter()
            .map(|line| line.trim_start_matches("+.").to_owned())
            .collect::<Vec<_>>();
        let business =
            read_snapshot_lines(&services.cloud_rules.resolve("iran-business-domains.txt"))
                .unwrap_or_default()
                .into_iter()
                .map(|line| line.trim_start_matches("+.").to_owned())
                .collect::<Vec<_>>();
        let cidrs = read_snapshot_lines(&services.cloud_rules.resolve("private.txt"))?
            .into_iter()
            .chain(read_snapshot_lines(
                &services.cloud_rules.resolve("iran-networks.txt"),
            )?)
            .map(|line| {
                line.parse()
                    .map_err(|error| format!("invalid bundled CIDR: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let config = services
            .config_store
            .load()
            .or_else(|_| services.config_store.load_or_create())
            .map_err(|error| error.to_string())?;
        let enabled = config
            .enabled_clients()
            .into_iter()
            .map(|client| client.id)
            .collect();
        let default_outbound = match config.default_route {
            DefaultRoute::Direct => iran_split_rules::Outbound::Direct,
            DefaultRoute::Client { client_id } => iran_split_rules::Outbound::client(client_id),
        };
        let decision = RuleSet::from_sources(
            &document,
            domains,
            cidrs,
            business,
            default_outbound,
            &enabled,
        )
        .decide(&target)
        .map_err(|error| error.to_string())?;
        info!(
            outbound = ?decision.outbound,
            reason = ?decision.reason,
            target_kind = if target.parse::<std::net::IpAddr>().is_ok() {
                "ip"
            } else {
                "domain"
            },
            "route test completed without logging its target"
        );
        Ok(RouteTestResult {
            target,
            outbound: decision.outbound,
            reason: format!("{:?}", decision.reason).to_lowercase(),
            matched_rule: decision.matched_rule,
            reachable: None,
            tested_at: Utc::now().to_rfc3339(),
        })
    })
    .await
}

#[tauri::command]
async fn run_full_diagnostics(app: AppHandle) -> Result<DiagnosticsReport, String> {
    diagnostics::trace_action(
        "diagnostics",
        "tauri_command",
        "run_full_diagnostics",
        async move {
            let services = services(&app)?;
            let operation_id = Uuid::new_v4();
            let started = Utc::now().to_rfc3339();
            let helper = services.backend.helper_status().await;
            let snapshot = services.engine.snapshot();
            let config = services
                .config_store
                .load_or_create()
                .map_err(|error| error.to_string())?;
            let helper_ok = helper
                .as_ref()
                .is_ok_and(|status| status.available && status.authorized);
            let steps = vec![
                diagnostic_step(
                    "helper",
                    "Helper authorization",
                    helper_ok,
                    helper.err().map(|error| error.to_string()),
                    &started,
                ),
                diagnostic_step(
                    "config",
                    "Configuration validation",
                    config.validate().is_empty(),
                    None,
                    &started,
                ),
                diagnostic_step(
                    "core",
                    "Mihomo process",
                    snapshot.mihomo.phase == iran_split_core::ComponentPhase::Running,
                    Some(format!("phase: {:?}", snapshot.mihomo.phase)),
                    &started,
                ),
                diagnostic_step(
                    "providers",
                    "Rule providers",
                    snapshot.providers.total > 0
                        && snapshot.providers.ready == snapshot.providers.total,
                    Some(format!(
                        "{} of {} ready",
                        snapshot.providers.ready, snapshot.providers.total
                    )),
                    &started,
                ),
                diagnostic_step(
                    "tun",
                    "Owned TUN state",
                    snapshot.tun.phase != iran_split_core::ComponentPhase::Error,
                    Some(format!("phase: {:?}", snapshot.tun.phase)),
                    &started,
                ),
                diagnostic_step(
                    "egress",
                    "Foreign egress",
                    snapshot.exit_ip.is_some(),
                    snapshot.exit_ip.clone(),
                    &started,
                ),
            ];
            for step in &steps {
                if step.status == "warning" {
                    warn!(
                        event = "diagnostics.step_warning",
                        section = "diagnostics",
                        initiator = "full_diagnostics",
                        cause = step.detail.as_deref().unwrap_or("check did not pass"),
                        trace_id = %operation_id,
                        trace_route = "tauri_command->full_diagnostics->diagnostic_step",
                        step_id = step.id,
                        "diagnostic step reported a warning"
                    );
                }
            }
            Ok(DiagnosticsReport {
                operation_id,
                steps,
                finished: true,
            })
        },
    )
    .await
}

fn diagnostic_step(
    id: &str,
    label: &str,
    passed: bool,
    detail: Option<String>,
    started: &str,
) -> DiagnosticStep {
    DiagnosticStep {
        id: id.into(),
        label: label.into(),
        status: if passed { "passed" } else { "warning" }.into(),
        detail,
        started_at: Some(started.into()),
        finished_at: Some(Utc::now().to_rfc3339()),
    }
}

#[tauri::command]
async fn query_logs(
    app: AppHandle,
    maximum: u16,
) -> Result<Vec<iran_split_ipc::ServiceLogEntry>, String> {
    diagnostics::trace_action("diagnostics", "tauri_command", "query_logs", async move {
        #[cfg(target_os = "linux")]
        {
            services(&app)?
                .backend
                .service_logs(maximum.clamp(1, 2_000))
                .await
                .map_err(|error| error.to_string())
        }
        #[cfg(target_os = "windows")]
        {
            let _ = (app, maximum);
            Ok(Vec::new())
        }
    })
    .await
}

#[tauri::command]
fn get_debug_log_status(_app: AppHandle) -> Result<diagnostics::DebugLogStatus, String> {
    diagnostics::trace_sync(
        "diagnostics",
        "tauri_command",
        "get_debug_log_status",
        diagnostics::status,
    )
}

#[tauri::command]
fn reveal_debug_log(_app: AppHandle) -> Result<diagnostics::DebugLogStatus, String> {
    diagnostics::trace_sync(
        "diagnostics",
        "tauri_command",
        "reveal_debug_log",
        diagnostics::reveal,
    )
}

#[tauri::command]
fn delete_debug_log(_app: AppHandle) -> Result<diagnostics::DebugLogStatus, String> {
    diagnostics::clear()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri injects AppHandle command arguments by value"
)]
#[tauri::command]
fn export_support_bundle(app: AppHandle) -> Result<ExportResult, String> {
    diagnostics::trace_sync(
        "diagnostics",
        "tauri_command",
        "export_support_bundle",
        || {
            let services = services(&app)?;
            let bundle = services
                .paths
                .cache
                .join(format!("support-{}", Uuid::new_v4()));
            fs::create_dir_all(&bundle).map_err(|error| error.to_string())?;
            let settings = services
                .config_store
                .load_or_create()
                .map_err(|error| error.to_string())?
                .redacted();
            let files = vec![
                "versions.json",
                "config-redacted.json",
                "snapshot.json",
                "debug.log",
            ];
            write_json(
                &bundle.join(files[0]),
                &serde_json::json!({
                    "app": app.package_info().version.to_string(),
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                }),
            )?;
            write_json(&bundle.join(files[1]), &settings)?;
            write_json(&bundle.join(files[2]), &services.engine.snapshot())?;
            diagnostics::copy_log(&bundle.join(files[3]))?;
            Ok(ExportResult {
                path: bundle.to_string_lossy().into_owned(),
                files: files.into_iter().map(str::to_owned).collect(),
            })
        },
    )
}

/// GitHub's public Releases API can flake on DNS/TLS. Retry once so a
/// transient failure does not become a Retry button. Each attempt is bounded
/// like `DBack`'s About-page check (about one minute).
const UPDATE_CHECK_ATTEMPTS: u32 = 2;
const UPDATE_CHECK_FIRST_BACKOFF: Duration = Duration::from_secs(1);
const UPDATE_CHECK_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);
const UPDATE_INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

async fn check_github_once() -> Result<github_update::UpdateInfo, String> {
    tokio::time::timeout(
        UPDATE_CHECK_ATTEMPT_TIMEOUT,
        github_update::check(
            version::app_version(),
            github_update::detect_install_kind(),
            UPDATE_CHECK_ATTEMPT_TIMEOUT,
        ),
    )
    .await
    .map_err(|_| "update check timed out".to_owned())?
}

fn status_from_github_update(info: &github_update::UpdateInfo) -> UpdateStatus {
    UpdateStatus {
        available: info.available,
        version: info.available.then(|| info.latest_version.clone()),
        notes: if info.notes.is_empty() {
            None
        } else {
            Some(info.notes.clone())
        },
        app_available: info.available,
        rules_available: false,
        thirdparty_available: false,
    }
}

fn merge_update_channels(
    mut status: UpdateStatus,
    rules_available: bool,
    thirdparty_available: bool,
) -> UpdateStatus {
    status.rules_available = rules_available;
    status.thirdparty_available = thirdparty_available;
    status.available = status.app_available || rules_available || thirdparty_available;
    status
}

async fn enrich_update_channels(app: &AppHandle, status: &mut UpdateStatus) {
    let Ok(services) = services(app) else {
        return;
    };
    match services.cloud_rules.peek_remote_revision().await {
        Ok(remote) => {
            status.rules_available =
                services.cloud_rules.cached_revision().as_deref() != Some(remote.as_str());
        }
        Err(cause) => {
            warn!(
                event = "update.rules_probe_failed",
                section = "updates",
                initiator = "check_for_update",
                cause = %cause,
                trace_route = "updater->cloud_rule_store->manifest",
                "rule snapshot revision could not be compared"
            );
        }
    }
    let thirdparty_available = deps::dependency_status(&services.paths.data)
        .into_iter()
        .any(|item| item.id == "mihomo" && !item.installed);
    *status = merge_update_channels(status.clone(), status.rules_available, thirdparty_available);
}

async fn collect_update_status(
    app: &AppHandle,
    initiator: &'static str,
) -> Result<UpdateStatus, String> {
    let found = fetch_github_update(app, initiator).await?;
    if let Ok(services) = services(app) {
        services.updates.remember_package(Some(found.clone()));
    }
    let mut status = status_from_github_update(&found);
    enrich_update_channels(app, &mut status).await;
    Ok(status)
}

async fn apply_sidecar_updates(app: &AppHandle, operation_id: Uuid) -> Result<(), String> {
    let services = services(app)?;
    info!(
        event = "update.sidecars_started",
        section = "updates",
        initiator = "install_update",
        cause = "versioned_assets",
        trace_route = "tauri_command->cloud_rules->mihomo_install",
        trace_id = %operation_id,
        "applying versioned rule and third-party updates"
    );
    emit_update_progress(
        app,
        UpdateProgress {
            phase: "installing".into(),
            percent: Some(10),
            version: None,
            error: None,
            operation_id: None,
            app_available: None,
            rules_available: None,
            thirdparty_available: None,
        },
    );
    if let Err(cause) = services.cloud_rules.sync().await {
        warn!(
            event = "update.rules_sync_failed",
            section = "updates",
            initiator = "install_update",
            cause = %cause,
            trace_route = "install_update->cloud_rule_store->sync",
            trace_id = %operation_id,
            "cloud rule update failed; last good snapshot remains"
        );
    }
    let mihomo_missing = deps::dependency_status(&services.paths.data)
        .into_iter()
        .any(|item| item.id == "mihomo" && !item.installed);
    if mihomo_missing {
        deps::install_dependency(
            deps::DependencyId::Mihomo,
            &services.paths.data,
            &services.paths.dependencies,
        )
        .await
        .map_err(|error| error.to_string())?;
        services.engine.refresh_health().await;
    }
    Ok(())
}

/// Doubles the wait after each failed attempt. Attempt `0` is immediate.
#[must_use]
fn update_check_backoff(attempt: u32) -> Duration {
    UPDATE_CHECK_FIRST_BACKOFF.saturating_mul(1 << attempt.min(4))
}

async fn fetch_github_update(
    app: &AppHandle,
    initiator: &'static str,
) -> Result<github_update::UpdateInfo, String> {
    let mut last_error = String::new();
    for attempt in 0..UPDATE_CHECK_ATTEMPTS {
        if update_check_cancelled(app) {
            return Err("update check cancelled".into());
        }
        match check_github_once().await {
            Ok(update) => {
                if attempt > 0 {
                    info!(
                        event = "update.check_recovered",
                        section = "updates",
                        initiator = initiator,
                        cause = "retry_succeeded",
                        trace_route = "updater->github_releases_api",
                        attempts = attempt + 1,
                        "update check succeeded after a transient failure"
                    );
                }
                return Ok(update);
            }
            Err(error) => {
                last_error = error;
                warn!(
                    event = "update.check_attempt_failed",
                    section = "updates",
                    initiator = initiator,
                    cause = last_error.as_str(),
                    trace_route = "updater->github_releases_api",
                    attempt = attempt + 1,
                    attempts = UPDATE_CHECK_ATTEMPTS,
                    "update check attempt failed"
                );
            }
        }
        if attempt + 1 < UPDATE_CHECK_ATTEMPTS {
            if update_check_cancelled(app) {
                return Err("update check cancelled".into());
            }
            tokio::time::sleep(update_check_backoff(attempt)).await;
        }
    }
    error!(
        event = "update.check_failed",
        section = "updates",
        initiator = initiator,
        cause = last_error.as_str(),
        trace_route = "updater->github_releases_api",
        attempts = UPDATE_CHECK_ATTEMPTS,
        "update check failed after every retry"
    );
    Err(last_error)
}

#[tauri::command]
async fn get_update_state(app: AppHandle) -> Result<UpdateProgress, String> {
    Ok(progress_from_snapshot(&services(&app)?.updates.snapshot()))
}

#[tauri::command]
async fn check_for_update(app: AppHandle) -> Result<UpdateStatus, String> {
    diagnostics::trace_action("updates", "tauri_command", "check_for_update", async move {
        let Some((_guard, operation_id)) = services(&app)?
            .updates
            .try_begin("tauri_command", "checking")
        else {
            let snapshot = services(&app)?.updates.snapshot();
            emit_update_progress(&app, progress_from_snapshot(&snapshot));
            return Ok(status_from_snapshot(&snapshot));
        };
        emit_update_progress(
            &app,
            UpdateProgress {
                phase: "checking".into(),
                percent: None,
                version: None,
                error: None,
                operation_id: Some(operation_id),
                app_available: None,
                rules_available: None,
                thirdparty_available: None,
            },
        );
        match collect_update_status(&app, "tauri_command").await {
            Ok(status) => {
                emit_update_progress(
                    &app,
                    UpdateProgress {
                        phase: if status.available {
                            "available".into()
                        } else {
                            "current".into()
                        },
                        percent: None,
                        version: status.version.clone(),
                        error: None,
                        operation_id: Some(operation_id),
                        app_available: Some(status.app_available),
                        rules_available: Some(status.rules_available),
                        thirdparty_available: Some(status.thirdparty_available),
                    },
                );
                Ok(status)
            }
            Err(cause) => {
                emit_update_progress(
                    &app,
                    UpdateProgress {
                        phase: "failed".into(),
                        percent: None,
                        version: None,
                        error: Some(cause.clone()),
                        operation_id: Some(operation_id),
                        app_available: None,
                        rules_available: None,
                        thirdparty_available: None,
                    },
                );
                Err(cause)
            }
        }
    })
    .await
}

const DIRECT_RULES_UPGRADE_GUARD: &str = "direct-rules.upgrade-guard";

fn record_direct_rules_upgrade_guard(data: &Path) {
    let marker = if data.join("direct-rules.json").is_file() {
        "present"
    } else {
        "absent"
    };
    if let Err(cause) = fs::write(data.join(DIRECT_RULES_UPGRADE_GUARD), marker) {
        warn!(
            event = "update.rules_guard_write_failed",
            section = "updates",
            initiator = "install_update",
            cause = %cause,
            trace_route = "install_update->direct_rules_guard",
            "could not record whether custom route pins existed before the upgrade"
        );
    }
}

fn verify_direct_rules_after_upgrade(data: &Path) -> Result<(), String> {
    let guard = data.join(DIRECT_RULES_UPGRADE_GUARD);
    let Ok(marker) = fs::read_to_string(&guard) else {
        return Ok(());
    };
    if let Err(cause) = fs::remove_file(&guard) {
        warn!(
            event = "update.rules_guard_clear_failed",
            section = "updates",
            initiator = "bootstrap_app",
            cause = %cause,
            trace_route = "bootstrap->direct_rules_guard",
            "upgrade guard file could not be removed"
        );
    }
    if marker.trim() == "present" && !data.join("direct-rules.json").is_file() {
        return Err("custom route pins were lost during the update".into());
    }
    Ok(())
}

async fn restore_stack_after_failed_install(services: &AppServices) {
    if matches!(services.engine.snapshot().phase, StackPhase::Paused) {
        if let Err(cause) = services.engine.resume_stack().await {
            warn!(
                event = "update.stack_restore_failed",
                section = "updates",
                initiator = "install_update",
                cause = %cause,
                trace_route = "install_update->resume_stack",
                "paused stack could not be restored after a failed install"
            );
        }
    }
}

fn schedule_update_exit(app: &AppHandle, operation_id: Uuid) -> OperationAccepted {
    diagnostics::flush();
    let exit_app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        exit_app.exit(0);
    });
    OperationAccepted {
        operation_id,
        already_complete: false,
    }
}

async fn resolve_github_package(app: &AppHandle) -> Result<github_update::UpdateInfo, String> {
    if let Some(pending) = services(app)?.updates.pending_package() {
        return Ok(pending);
    }
    let found = fetch_github_update(app, "install_update").await?;
    services(app)?.updates.remember_package(Some(found.clone()));
    Ok(found)
}

async fn download_github_package(
    app: &AppHandle,
    operation_id: Uuid,
    info: &github_update::UpdateInfo,
) -> Result<PathBuf, String> {
    let asset = info
        .asset
        .as_ref()
        .ok_or_else(|| "no update package for this platform".to_owned())?;
    let target_version = info.latest_version.clone();
    info!(
        event = "update.download_started",
        section = "updates",
        initiator = "install_update",
        cause = "update_available",
        trace_route = "tauri_command->github_update->download",
        trace_id = %operation_id,
        update_version = %target_version,
        "application update download started"
    );
    emit_update_progress(
        app,
        UpdateProgress {
            phase: "downloading".into(),
            percent: Some(0),
            version: Some(target_version.clone()),
            error: None,
            operation_id: Some(operation_id),
            app_available: None,
            rules_available: None,
            thirdparty_available: None,
        },
    );
    let app_for_progress = app.clone();
    let version_for_progress = target_version.clone();
    let cancel_app = app.clone();
    github_update::download_asset(
        version::app_version(),
        asset,
        UPDATE_INSTALL_TIMEOUT,
        move |written, total| {
            emit_update_progress(
                &app_for_progress,
                UpdateProgress {
                    phase: "downloading".into(),
                    percent: update_download_percent(written, total),
                    version: Some(version_for_progress.clone()),
                    error: None,
                    operation_id: None,
                    app_available: None,
                    rules_available: None,
                    thirdparty_available: None,
                },
            );
        },
        move || update_check_cancelled(&cancel_app),
    )
    .await
    .inspect_err(|cause| {
        emit_update_progress(
            app,
            UpdateProgress {
                phase: "failed".into(),
                percent: None,
                version: Some(target_version.clone()),
                error: Some(cause.clone()),
                operation_id: Some(operation_id),
                app_available: None,
                rules_available: None,
                thirdparty_available: None,
            },
        );
        error!(
            event = "update.download_failed",
            section = "updates",
            initiator = "install_update",
            cause = "download_error",
            trace_route = "tauri_command->github_update->download",
            trace_id = %operation_id,
            "application update could not be downloaded"
        );
    })
}

async fn apply_downloaded_package(
    app: &AppHandle,
    operation_id: Uuid,
    info: &github_update::UpdateInfo,
    package: &Path,
) -> Result<OperationAccepted, String> {
    let target_version = info.latest_version.clone();
    emit_update_progress(
        app,
        UpdateProgress {
            phase: "installing".into(),
            percent: Some(100),
            version: Some(target_version.clone()),
            error: None,
            operation_id: Some(operation_id),
            app_available: None,
            rules_available: None,
            thirdparty_available: None,
        },
    );
    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let outcome = match github_update::apply_package(
        github_update::detect_install_kind(),
        package,
        &current_exe,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(cause) => {
            restore_stack_after_failed_install(services(app)?).await;
            emit_update_progress(
                app,
                UpdateProgress {
                    phase: "failed".into(),
                    percent: None,
                    version: Some(target_version),
                    error: Some(cause.clone()),
                    operation_id: Some(operation_id),
                    app_available: None,
                    rules_available: None,
                    thirdparty_available: None,
                },
            );
            return Err(cause);
        }
    };
    info!(
        event = "update.install_succeeded",
        section = "updates",
        initiator = "install_update",
        cause = "download_and_install_complete",
        trace_route = "tauri_command->github_update->apply",
        trace_id = %operation_id,
        update_version = %target_version,
        "application update installed"
    );
    match outcome {
        github_update::ApplyOutcome::ManualRestart => {
            emit_update_progress(
                app,
                UpdateProgress {
                    phase: "installed".into(),
                    percent: Some(100),
                    version: Some(target_version),
                    error: None,
                    operation_id: Some(operation_id),
                    app_available: None,
                    rules_available: None,
                    thirdparty_available: None,
                },
            );
            Ok(OperationAccepted {
                operation_id,
                already_complete: true,
            })
        }
        github_update::ApplyOutcome::HelperRestart => {
            emit_update_progress(
                app,
                UpdateProgress {
                    phase: "restarting".into(),
                    percent: Some(100),
                    version: Some(target_version),
                    error: None,
                    operation_id: Some(operation_id),
                    app_available: None,
                    rules_available: None,
                    thirdparty_available: None,
                },
            );
            Ok(schedule_update_exit(app, operation_id))
        }
    }
}

async fn perform_complete_update_install(
    app: &AppHandle,
    operation_id: Uuid,
) -> Result<OperationAccepted, String> {
    apply_sidecar_updates(app, operation_id).await?;
    let update = resolve_github_package(app).await?;
    let mut status = status_from_github_update(&update);
    enrich_update_channels(app, &mut status).await;
    if !update.available || update.asset.is_none() {
        emit_update_progress(
            app,
            UpdateProgress {
                phase: if status.available {
                    "available".into()
                } else {
                    "current".into()
                },
                percent: Some(100),
                version: status.version,
                error: None,
                operation_id: Some(operation_id),
                app_available: Some(status.app_available),
                rules_available: Some(status.rules_available),
                thirdparty_available: Some(status.thirdparty_available),
            },
        );
        return Ok(OperationAccepted {
            operation_id,
            already_complete: true,
        });
    }
    let package = download_github_package(app, operation_id, &update).await?;
    record_direct_rules_upgrade_guard(&services(app)?.paths.data);
    pause_stack_for_update(services(app)?).await?;
    apply_downloaded_package(app, operation_id, &update, &package).await
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<OperationAccepted, String> {
    diagnostics::trace_action("updates", "tauri_command", "install_update", async move {
        let Some((_guard, operation_id)) = services(&app)?
            .updates
            .try_begin("install_update", "downloading")
        else {
            let snapshot = services(&app)?.updates.snapshot();
            emit_update_progress(&app, progress_from_snapshot(&snapshot));
            return Ok(OperationAccepted {
                operation_id: snapshot.operation_id.unwrap_or_else(Uuid::nil),
                already_complete: false,
            });
        };
        perform_complete_update_install(&app, operation_id).await
    })
    .await
}

/// The UI only ever sees `redacted()` configs, so a settings save echoes the
/// redaction markers back: the password as "[REDACTED]", and profile/executable
/// paths truncated to their bare file names. Every marker must be swapped back
/// for the stored value or a save destroys it — a username edit used to
/// overwrite the full profile path with its basename, leaving the side tunnel
/// dead with "profile is unreadable" at the next connect.
fn restore_redacted_secrets(current: &AppConfig, draft: &mut AppConfig) {
    use iran_split_config::{ClientConfig, ExecutableSetting};

    // A draft path is a redaction echo when it is a bare file name matching
    // the stored path's file name. A picker always produces absolute paths,
    // so a genuinely new choice never looks like this.
    fn is_redacted_echo(draft: &std::path::Path, current: &std::path::Path) -> bool {
        draft
            .parent()
            .is_none_or(|parent| parent.as_os_str().is_empty())
            && current.file_name() == Some(draft.as_os_str())
    }

    fn restore_executable(draft: &mut ExecutableSetting, current: &ExecutableSetting) {
        if let (ExecutableSetting::Path(draft_path), ExecutableSetting::Path(current_path)) =
            (&*draft, current)
        {
            if is_redacted_echo(draft_path, current_path) {
                *draft = ExecutableSetting::Path(current_path.clone());
            }
        }
    }

    for draft_client in &mut draft.clients {
        let Some(current_client) = current.client(draft_client.id) else {
            continue;
        };
        match (&mut draft_client.config, &current_client.config) {
            (
                ClientConfig::OwnedSideTunnel {
                    profile_path,
                    executable,
                    password,
                    ..
                },
                ClientConfig::OwnedSideTunnel {
                    profile_path: current_profile,
                    executable: current_executable,
                    password: current_password,
                    ..
                },
            ) => {
                if password.as_deref() == Some("[REDACTED]") {
                    password.clone_from(current_password);
                }
                if let (Some(draft_path), Some(current_path)) =
                    (profile_path.as_ref(), current_profile.as_ref())
                {
                    if is_redacted_echo(draft_path, current_path) {
                        *profile_path = Some(current_path.clone());
                    }
                }
                restore_executable(executable, current_executable);
            }
            (
                ClientConfig::LocalProxy { executable, .. },
                ClientConfig::LocalProxy {
                    executable: current_executable,
                    ..
                },
            ) => restore_executable(executable, current_executable),
            _ => {}
        }
    }
}

fn pin_policy_for(
    services: &AppServices,
    outbound: iran_split_rules::Outbound,
) -> Result<iran_split_rules::PinPolicy, String> {
    let iran_split_rules::Outbound::Client { client_id } = outbound else {
        return Ok(iran_split_rules::PinPolicy::Direct);
    };
    let config = services
        .config_store
        .load()
        .or_else(|_| services.config_store.load_or_create())
        .map_err(|error| error.to_string())?;
    Ok(config
        .client(client_id)
        .map_or(iran_split_rules::PinPolicy::LocalProxy, |client| {
            iran_split_rules::PinPolicy::for_kind(client.spec().kind)
        }))
}

fn parse_outbound(value: &str) -> Result<iran_split_rules::Outbound, String> {
    if value == "direct" {
        return Ok(iran_split_rules::Outbound::Direct);
    }
    let client_id = ClientId::parse(value).map_err(|_| format!("unknown outbound: {value}"))?;
    Ok(iran_split_rules::Outbound::client(client_id))
}

fn read_snapshot_lines(path: &Path) -> Result<Vec<String>, String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    Ok(source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[allow(
    clippy::too_many_lines,
    reason = "Linux and Windows backend construction stay in one startup path"
)]
fn create_services(app: &AppHandle) -> Result<AppServices, String> {
    info!(
        event = "services.initializing",
        section = "startup",
        initiator = "tauri_setup",
        cause = "application_start",
        trace_route = "application_process->tauri_setup->create_services",
        "application services initializing"
    );
    let paths = AppPaths::discover(app)?;
    let config_store = ConfigStore::new(&paths.config);
    let config = config_store
        .load_or_create()
        .map_err(|error| error.to_string())?;
    let legacy_pins = iran_split_rules::LegacyPinClients {
        vpn: config.hiddify_client().map(|client| client.id),
        openvpn: config
            .clients
            .iter()
            .find(|client| client.preset == PresetId::Openvpn)
            .map(|client| client.id),
    };
    let rules_cache = paths.cache.join("rules");
    fs::create_dir_all(&rules_cache).map_err(|error| error.to_string())?;
    let bundled_rules = open_bundled_rules_dir(&paths)?;
    #[cfg(target_os = "linux")]
    let mihomo_binary = linux_mihomo_binary(
        deps::first_existing(&deps::mihomo_candidates(&paths.data))
            .unwrap_or_else(|| paths.data.join("bin/mihomo")),
    );
    #[cfg(target_os = "linux")]
    let backend = {
        let (socket_path, system_runtime_dir) = linux_helper_paths();
        info!(
            event = "helper.paths_selected",
            section = "startup",
            initiator = "create_services",
            cause = "platform_configuration",
            trace_route = "application_process->create_services->linux_backend",
            socket_path = %socket_path.display(),
            runtime_path = %system_runtime_dir.display(),
            "Linux helper paths selected"
        );
        Arc::new(NativeBackend::new(
            config,
            LinuxPaths {
                socket_path,
                user_data_dir: paths.data.clone(),
                system_runtime_dir,
                resources_dir: bundled_rules.clone(),
                rules_cache_dir: rules_cache.clone(),
                mihomo_binary,
            },
        ))
    };
    #[cfg(target_os = "windows")]
    let backend = {
        let (pipe_name, system_runtime_dir) = windows_helper_paths();
        // Packaged Connect stages here, and the elevated installer records the
        // same root in helper.toml, so SYSTEM can publish the generation.
        let generation_staging_dir = PathBuf::from(helper_install::WINDOWS_HELPER_STAGING);
        let mihomo_binary = deps::first_existing(&deps::mihomo_candidates(&paths.data))
            .unwrap_or_else(windows_programdata_mihomo);
        info!(
            event = "helper.paths_selected",
            section = "startup",
            initiator = "create_services",
            cause = "platform_configuration",
            trace_route = "application_process->create_services->windows_backend",
            pipe_name = pipe_name.as_str(),
            runtime_path = %system_runtime_dir.display(),
            staging_path = %generation_staging_dir.display(),
            mihomo_binary = %mihomo_binary.display(),
            "Windows helper paths selected"
        );
        Arc::new(NativeBackend::new(
            config,
            WindowsPaths {
                pipe_name,
                user_data_dir: paths.data.clone(),
                system_runtime_dir,
                generation_staging_dir,
                resources_dir: bundled_rules.clone(),
                rules_cache_dir: rules_cache.clone(),
                mihomo_binary,
            },
        ))
    };
    let runtime = tauri::async_runtime::handle();
    let engine = Engine::new(Arc::clone(&backend), runtime.inner());
    let rules = RuleManager::load_with_legacy(
        paths.data.join("direct-rules.json"),
        Arc::new(DohResolver::default()),
        legacy_pins,
    )
    .map_err(|error| error.to_string())?;
    let cloud_rules = CloudRuleStore::load(bundled_rules, rules_cache);
    let network = network::NetworkMonitor::new().map_err(|error| error.to_string())?;
    info!(
        event = "services.initialized",
        section = "startup",
        initiator = "tauri_setup",
        cause = "none",
        trace_route = "application_process->tauri_setup->create_services",
        "application services initialized"
    );
    Ok(AppServices {
        config_store,
        engine,
        backend,
        rules,
        cloud_rules,
        network,
        paths,
        updates: Arc::new(UpdateCoordinator::new()),
        traffic: tokio::sync::Mutex::new(traffic::SessionAccumulator::default()),
    })
}

fn open_bundled_rules_dir(paths: &AppPaths) -> Result<PathBuf, String> {
    let fallback = paths.data.join("bundled-rules");
    let bundled =
        ensure_bundled_snapshot(&paths.resources, &fallback).map_err(|error| error.to_string())?;
    if bundled != paths.resources {
        warn!(
            event = "cloud_rules.embedded_snapshot_used",
            section = "startup",
            initiator = "create_services",
            cause = "packaged_rules_missing",
            trace_route = "application_process->create_services->bundled_rules",
            "packaged Iran rule snapshot was missing; using the embedded copy"
        );
    }
    Ok(bundled)
}

fn handle_tray_icon<R: Runtime>(tray: &TrayIcon<R>, event: &TrayIconEvent) {
    if matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    ) {
        info!(
            event = "window.open_requested",
            section = "window",
            initiator = "tray_icon",
            cause = "left_click",
            trace_route = "tray_icon->show_main",
            "main window open requested"
        );
        show_main(tray.app_handle());
    }
}

fn connect_from_tray<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = diagnostics::trace_action("stack", "tray_menu", "start_stack", async move {
            start_stack_inner(&app, Some(15)).await
        })
        .await;
        if let Err(cause) = result {
            error!(
                event = "tray.action_failed",
                section = "stack",
                initiator = "tray_menu",
                cause,
                trace_route = "tray_menu->start_stack",
                "tray connect action failed"
            );
        }
    });
}

fn pause_from_tray<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = diagnostics::trace_action("stack", "tray_menu", "pause_stack", async move {
            services(&app)?
                .engine
                .pause_stack()
                .await
                .map_err(|error| error.to_string())
        })
        .await;
        if let Err(cause) = result {
            error!(
                event = "tray.action_failed",
                section = "stack",
                initiator = "tray_menu",
                cause,
                trace_route = "tray_menu->pause_stack",
                "tray pause action failed"
            );
        }
    });
}

fn resume_from_tray<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = diagnostics::trace_action("stack", "tray_menu", "resume_stack", async move {
            services(&app)?
                .engine
                .resume_stack()
                .await
                .map_err(|error| error.to_string())
        })
        .await;
        if let Err(cause) = result {
            error!(
                event = "tray.action_failed",
                section = "stack",
                initiator = "tray_menu",
                cause,
                trace_route = "tray_menu->resume_stack",
                "tray resume action failed"
            );
        }
    });
}

fn disconnect_from_tray<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = diagnostics::trace_action("stack", "tray_menu", "stop_stack", async move {
            services(&app)?
                .engine
                .stop_stack()
                .await
                .map_err(|error| error.to_string())
        })
        .await;
        if let Err(cause) = result {
            error!(
                event = "tray.action_failed",
                section = "stack",
                initiator = "tray_menu",
                cause,
                trace_route = "tray_menu->stop_stack",
                "tray disconnect action failed"
            );
        }
    });
}

fn handle_tray_menu<R: Runtime>(app: &AppHandle<R>, event: &MenuEvent) {
    match event.id.as_ref() {
        "connect" => connect_from_tray(app),
        "pause" => pause_from_tray(app),
        "resume" => resume_from_tray(app),
        "disconnect" => disconnect_from_tray(app),
        "quit" => {
            info!(
                event = "session.quit_requested",
                section = "lifecycle",
                initiator = "tray_menu",
                cause = "quit_selected",
                trace_route = "tray_menu->application_exit",
                "quit requested without disconnect"
            );
            app.exit(0);
        }
        "dashboard" => open_dashboard_from_tray(app),
        unknown => warn!(
            event = "tray.unknown_action",
            section = "tray",
            initiator = "tray_menu",
            cause = "unknown_menu_id",
            trace_route = "tray_menu->event_dispatch",
            menu_id = unknown,
            "unknown tray action ignored"
        ),
    }
}

fn build_tray_menu<R: Runtime>(
    app: &AppHandle<R>,
    phase: StackPhase,
    busy: Option<LifecycleBusy>,
) -> tauri::Result<Menu<R>> {
    let labels = tray::labels_for(phase);
    let enabled = tray::actions_enabled(busy);
    let dashboard = MenuItem::with_id(app, "dashboard", "Dashboard", true, None::<&str>)?;
    let connection = MenuItem::with_id(
        app,
        labels.connection_id,
        labels.connection_label,
        enabled,
        None::<&str>,
    )?;
    let pause = MenuItem::with_id(
        app,
        labels.pause_id,
        labels.pause_label,
        enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let dashboard_separator = PredefinedMenuItem::separator(app)?;
    let first_separator = PredefinedMenuItem::separator(app)?;
    let second_separator = PredefinedMenuItem::separator(app)?;
    Menu::with_items(
        app,
        &[
            &dashboard,
            &dashboard_separator,
            &connection,
            &first_separator,
            &pause,
            &second_separator,
            &quit,
        ],
    )
}

fn apply_tray_menu<R: Runtime>(app: &AppHandle<R>, snapshot: &StackSnapshot) {
    let Ok(menu) = build_tray_menu(app, snapshot.phase, snapshot.busy) else {
        warn!(
            event = "tray.menu_build_failed",
            section = "tray",
            initiator = "apply_tray_menu",
            cause = "menu_construction",
            trace_route = "snapshot_watcher->build_tray_menu",
            "tray menu could not be rebuilt"
        );
        return;
    };
    let Some(icon) = app.tray_by_id("main") else {
        return;
    };
    if let Err(cause) = icon.set_menu(Some(menu)) {
        warn!(
            event = "tray.menu_update_failed",
            section = "tray",
            initiator = "apply_tray_menu",
            cause = %cause,
            trace_route = "snapshot_watcher->tray.set_menu",
            "tray menu could not be replaced"
        );
    }
}

fn setup_tray<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let snapshot = app
        .try_state::<AppServices>()
        .map_or_else(StackSnapshot::default, |services| {
            services.engine.snapshot()
        });
    let menu = build_tray_menu(app.handle(), snapshot.phase, snapshot.busy)?;
    let icon = app.default_window_icon().cloned().ok_or_else(|| {
        tauri::Error::from(std::io::Error::other("default window icon is missing"))
    })?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| handle_tray_icon(tray, &event))
        .on_menu_event(|app, event| handle_tray_menu(app, &event))
        .build(app)?;
    Ok(())
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(cause) = window.unminimize() {
            warn!(
                event = "window.unminimize_failed",
                section = "window",
                initiator = "show_main",
                cause = %cause,
                trace_route = "show_main->window.unminimize",
                "main window could not be restored from minimized"
            );
        }
        if let Err(cause) = window.show() {
            error!(
                event = "window.show_failed",
                section = "window",
                initiator = "show_main",
                cause = %cause,
                trace_route = "show_main->window.show",
                "main window could not be shown"
            );
        }
        if let Err(cause) = window.set_focus() {
            warn!(
                event = "window.focus_failed",
                section = "window",
                initiator = "show_main",
                cause = %cause,
                trace_route = "show_main->window.set_focus",
                "main window could not be focused"
            );
        }
    } else {
        error!(
            event = "window.missing",
            section = "window",
            initiator = "show_main",
            cause = "main_window_not_found",
            trace_route = "show_main->get_webview_window",
            "main window is unavailable"
        );
    }
}

fn open_dashboard_from_tray<R: Runtime>(app: &AppHandle<R>) {
    show_main(app);
    if let Err(cause) = app.emit("app-navigate", "dashboard") {
        warn!(
            event = "tray.navigate_failed",
            section = "tray",
            initiator = "tray_menu",
            cause = %cause,
            trace_route = "tray_menu->app_navigate",
            "dashboard navigation event could not be emitted"
        );
    }
}

fn initialize_diagnostics() {
    let path = diagnostics::default_log_path().expect("debug.log directory is unavailable");
    diagnostics::initialize(&path, version::app_version())
        .unwrap_or_else(|error| panic!("BiFlow debug.log initialization failed: {error}"));
}

fn setup_application(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let services = create_services(app.handle()).map_err(|cause| {
        error!(
            event = "startup.services_failed",
            section = "startup",
            initiator = "tauri_setup",
            cause,
            trace_route = "application_process->tauri_setup->create_services",
            "application service initialization failed"
        );
        std::io::Error::other(cause)
    })?;
    let mut snapshots = services.engine.subscribe();
    let engine = Arc::clone(&services.engine);
    let health_engine = Arc::clone(&services.engine);
    let data_dir = services.paths.data.clone();
    let handle = app.handle().clone();
    app.manage(services);
    tauri::async_runtime::spawn(async move {
        while snapshots.changed().await.is_ok() {
            let snapshot = snapshots.borrow().clone();
            if let Err(cause) = handle.emit("stack-snapshot", snapshot.clone()) {
                warn!(
                    event = "snapshot.emit_failed",
                    section = "stack",
                    initiator = "snapshot_watcher",
                    cause = %cause,
                    trace_route = "engine->snapshot_watcher->frontend_event",
                    "stack snapshot event could not be emitted"
                );
            }
            apply_tray_menu(&handle, &snapshot);
        }
        warn!(
            event = "snapshot.channel_closed",
            section = "stack",
            initiator = "snapshot_watcher",
            cause = "engine_snapshot_sender_closed",
            trace_route = "engine->snapshot_watcher",
            "stack snapshot watcher stopped"
        );
    });
    tauri::async_runtime::spawn(async move {
        if let Err(cause) = diagnostics::trace_action(
            "startup",
            "tauri_setup",
            "reconcile_startup",
            engine.reconcile_startup(),
        )
        .await
        {
            error!(
                event = "startup.reconciliation_failed",
                section = "startup",
                initiator = "tauri_setup",
                cause = %cause,
                trace_route = "tauri_setup->engine->reconcile_startup",
                "startup reconciliation could not be queued"
            );
        }
    });
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if matches!(
                health_engine.snapshot().phase,
                StackPhase::Stopped
                    | StackPhase::Running
                    | StackPhase::Paused
                    | StackPhase::Degraded
                    | StackPhase::Error
            ) {
                health_engine.refresh_health().await;
                // ADR 0076: a local proxy (e.g. Happ) the operator connected
                // after Connect rejoins live routing without a full reconnect.
                health_engine.recover_clients().await;
            }
        }
    });
    if let Some(window) = app.get_webview_window("main") {
        restore_main_window_size(&window, &data_dir);
    }
    setup_tray(app).map_err(|cause| {
        error!(
            event = "startup.tray_failed",
            section = "startup",
            initiator = "tauri_setup",
            cause = %cause,
            trace_route = "tauri_setup->setup_tray",
            "system tray initialization failed"
        );
        cause
    })?;
    info!(
        event = "startup.completed",
        section = "startup",
        initiator = "tauri_setup",
        cause = "none",
        trace_route = "application_process->tauri_setup->event_loop",
        "application setup completed"
    );
    Ok(())
}

fn restore_main_window_size<R: Runtime>(window: &tauri::WebviewWindow<R>, data: &Path) {
    let saved = window_state::load(&data.join("window-size.json"));
    let (work_width, work_height) = monitor_work_area_logical(window)
        .unwrap_or((window_state::DEFAULT_WIDTH, window_state::DEFAULT_HEIGHT));
    let size = window_state::clamp_logical(saved.width, saved.height, work_width, work_height);
    if let Err(cause) = window.set_min_size(Some(Size::Logical(LogicalSize::new(
        window_state::MIN_WIDTH,
        window_state::MIN_HEIGHT,
    )))) {
        warn!(
            event = "window.min_size_failed",
            section = "window",
            initiator = "restore_main_window_size",
            cause = %cause,
            trace_route = "tauri_setup->window.set_min_size",
            "minimum window size could not be applied"
        );
    }
    if let Err(cause) = window.set_size(Size::Logical(LogicalSize::new(size.width, size.height))) {
        warn!(
            event = "window.size_restore_failed",
            section = "window",
            initiator = "restore_main_window_size",
            cause = %cause,
            trace_route = "tauri_setup->window.set_size",
            "saved window size could not be applied"
        );
    } else {
        info!(
            event = "window.size_restored",
            section = "window",
            initiator = "restore_main_window_size",
            cause = "persisted_size",
            trace_route = "tauri_setup->window.set_size",
            "main window size restored within the current work area"
        );
    }
}

fn monitor_work_area_logical<R: Runtime>(window: &tauri::WebviewWindow<R>) -> Option<(f64, f64)> {
    let monitor = window.current_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return None;
    }
    let work = monitor.work_area();
    Some((
        f64::from(work.size.width) / scale,
        f64::from(work.size.height) / scale,
    ))
}

fn persist_main_window_size<R: Runtime>(window: &Window<R>) {
    let Ok(services) = services(window.app_handle()) else {
        return;
    };
    let Ok(physical) = window.inner_size() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    if scale <= 0.0 {
        return;
    }
    let logical = physical.to_logical::<f64>(scale);
    let (work_width, work_height) = window
        .current_monitor()
        .ok()
        .flatten()
        .and_then(|monitor| {
            let scale = monitor.scale_factor();
            if scale <= 0.0 {
                return None;
            }
            let work = monitor.work_area();
            Some((
                f64::from(work.size.width) / scale,
                f64::from(work.size.height) / scale,
            ))
        })
        .unwrap_or((window_state::DEFAULT_WIDTH, window_state::DEFAULT_HEIGHT));
    let size = window_state::clamp_logical(logical.width, logical.height, work_width, work_height);
    if let Err(cause) = window_state::save(&services.paths.data.join("window-size.json"), size) {
        warn!(
            event = "window.size_persist_failed",
            section = "window",
            initiator = "persist_main_window_size",
            cause = %cause,
            trace_route = "window_control->window_size_file",
            "window size could not be written"
        );
    }
}

fn handle_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::Resized(_) = event {
        persist_main_window_size(window);
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        info!(
            event = "window.close_requested",
            section = "window",
            initiator = "window_control",
            cause = "user_close",
            trace_route = "window_control->hide_main_window",
            "main window close requested; application remains in tray"
        );
        api.prevent_close();
        if let Err(cause) = window.hide() {
            error!(
                event = "window.hide_failed",
                section = "window",
                initiator = "window_control",
                cause = %cause,
                trace_route = "window_control->window.hide",
                "main window could not be hidden"
            );
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Starts the `BiFlow` Tauri application and blocks on its event loop.
///
/// # Panics
///
/// Panics when the diagnostic log or Tauri event loop cannot initialize.
pub fn run() {
    #[cfg(target_os = "linux")]
    apply_linux_webview_workarounds();
    initialize_diagnostics();
    #[cfg(target_os = "linux")]
    log_linux_webview_workarounds();
    let builder = tauri::Builder::default()
        .plugin(
            tauri_plugin_single_instance::Builder::new()
                .dbus_id(single_instance_dbus_id(
                    BUNDLE_IDENTIFIER,
                    version::app_version(),
                ))
                .callback(|app, _, _| {
                    info!(
                        event = "window.open_requested",
                        section = "window",
                        initiator = "second_process",
                        cause = "single_instance_activation",
                        trace_route = "second_process->single_instance_plugin->show_main",
                        "existing application instance activated"
                    );
                    show_main(app);
                })
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(setup_application)
        .on_window_event(handle_window_event)
        .invoke_handler(tauri::generate_handler![
            bootstrap_app,
            get_stack_snapshot,
            get_network_status,
            check_reachability,
            get_traffic_totals,
            list_active_connections,
            start_stack,
            stop_stack,
            pause_stack,
            resume_stack,
            restart_stack,
            retry_side_tunnels,
            cancel_operation,
            get_settings,
            validate_settings,
            save_settings,
            discard_client_pins,
            reassign_client_pins,
            list_direct_rules,
            add_direct_rule,
            pin_route,
            remove_direct_rule,
            create_rule_list,
            rename_rule_list,
            delete_rule_list,
            set_rule_list_outbound,
            pin_to_rule_list,
            check_rule_list,
            refresh_direct_rules,
            get_cloud_rules_status,
            sync_cloud_rules,
            list_dependencies,
            install_dependency,
            install_helper,
            get_install_guide,
            open_external_url,
            client_binary_installed,
            pick_client_profile,
            apply_live_settings,
            run_full_diagnostics,
            test_route,
            query_logs,
            get_debug_log_status,
            reveal_debug_log,
            delete_debug_log,
            export_support_bundle,
            fresh_hiddify_start,
            check_for_update,
            get_update_state,
            install_update,
        ]);

    let app = builder
        .build(tauri::generate_context!())
        .expect("BiFlow failed to build");
    app.run(|_, event| match event {
        tauri::RunEvent::ExitRequested { code, .. } => diagnostics::exit_requested(code),
        tauri::RunEvent::Exit => diagnostics::close_session(),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::{
        merge_update_channels, packaged_rule_snapshot_dir, single_instance_dbus_id,
        update_check_backoff, update_download_percent, UpdateProgress, UpdateStatus,
        BUNDLE_IDENTIFIER, UPDATE_CHECK_ATTEMPTS, UPDATE_CHECK_FIRST_BACKOFF,
    };
    use std::{fs, time::Duration};

    #[test]
    fn settings_save_restores_redacted_profile_path_and_password() {
        use iran_split_config::{AppConfig, ClientConfig, ClientInstance, PresetId};
        use std::path::PathBuf;

        let mut stored = ClientInstance::from_preset(PresetId::Windscribe);
        stored.config = ClientConfig::OwnedSideTunnel {
            profile_path: Some(PathBuf::from("/home/user/vpn/Windscribe-Berlin.ovpn")),
            executable: iran_split_config::ExecutableSetting::Auto,
            username: Some("user".into()),
            password: Some("secret".into()),
            start_timeout_seconds: 45,
        };
        let mut current = AppConfig::default();
        current.clients.push(stored.clone());

        // The UI edits a redacted copy: bare file name, masked password.
        let mut draft = current.clone();
        draft.clients.last_mut().expect("client").config = ClientConfig::OwnedSideTunnel {
            profile_path: Some(PathBuf::from("Windscribe-Berlin.ovpn")),
            executable: iran_split_config::ExecutableSetting::Auto,
            username: Some("edited".into()),
            password: Some("[REDACTED]".into()),
            start_timeout_seconds: 45,
        };
        super::restore_redacted_secrets(&current, &mut draft);
        let ClientConfig::OwnedSideTunnel {
            profile_path,
            username,
            password,
            ..
        } = &draft.clients.last().expect("client").config
        else {
            panic!("side tunnel config expected");
        };
        assert_eq!(
            profile_path.as_deref(),
            Some(std::path::Path::new(
                "/home/user/vpn/Windscribe-Berlin.ovpn"
            )),
            "a redaction echo must restore the stored absolute path"
        );
        assert_eq!(username.as_deref(), Some("edited"));
        assert_eq!(password.as_deref(), Some("secret"));

        // A genuinely new absolute path must survive untouched.
        let mut fresh = current.clone();
        fresh.clients.last_mut().expect("client").config = ClientConfig::OwnedSideTunnel {
            profile_path: Some(PathBuf::from("/tmp/other.ovpn")),
            executable: iran_split_config::ExecutableSetting::Auto,
            username: None,
            password: None,
            start_timeout_seconds: 45,
        };
        super::restore_redacted_secrets(&current, &mut fresh);
        let ClientConfig::OwnedSideTunnel { profile_path, .. } =
            &fresh.clients.last().expect("client").config
        else {
            panic!("side tunnel config expected");
        };
        assert_eq!(
            profile_path.as_deref(),
            Some(std::path::Path::new("/tmp/other.ovpn"))
        );
    }

    #[test]
    fn update_check_attempt_timeout_bounds_a_hang() {
        assert_eq!(super::UPDATE_CHECK_ATTEMPT_TIMEOUT, Duration::from_secs(60));
        assert_eq!(super::UPDATE_INSTALL_TIMEOUT, Duration::from_secs(10 * 60));
        let coordinator = super::UpdateCoordinator::new();
        let first = coordinator.try_begin("test", "checking");
        assert!(first.is_some());
        assert!(coordinator.try_begin("test", "checking").is_none());
        assert_eq!(coordinator.snapshot().phase, "checking");
        drop(first);
    }

    #[test]
    fn missing_direct_rules_after_upgrade_is_a_failure() {
        let directory = tempfile::tempdir().expect("tempdir");
        let data = directory.path();
        fs::write(data.join("direct-rules.upgrade-guard"), "present").expect("guard");
        let error = super::verify_direct_rules_after_upgrade(data).expect_err("lost");
        assert!(error.contains("lost"));
        fs::write(data.join("direct-rules.json"), "{}").expect("pins");
        fs::write(data.join("direct-rules.upgrade-guard"), "present").expect("guard");
        super::verify_direct_rules_after_upgrade(data).expect("kept");
    }

    #[test]
    fn update_check_backoff_grows_and_stays_bounded() {
        assert_eq!(update_check_backoff(0), UPDATE_CHECK_FIRST_BACKOFF);
        assert_eq!(update_check_backoff(1), UPDATE_CHECK_FIRST_BACKOFF * 2);
        assert_eq!(update_check_backoff(2), UPDATE_CHECK_FIRST_BACKOFF * 4);
        // Every attempt after the last still yields a finite, capped wait.
        assert_eq!(update_check_backoff(9), UPDATE_CHECK_FIRST_BACKOFF * 16);

        let waits: Vec<Duration> = (0..UPDATE_CHECK_ATTEMPTS - 1)
            .map(update_check_backoff)
            .collect();
        assert!(!waits.is_empty(), "one attempt is not a retry");
        let total: Duration = waits.iter().sum();
        assert!(
            total < Duration::from_secs(10),
            "a flaky check must not stall the About page for {total:?}"
        );
    }

    #[test]
    fn update_download_percent_is_bounded() {
        assert_eq!(update_download_percent(0, Some(100)), Some(0));
        assert_eq!(update_download_percent(50, Some(100)), Some(50));
        assert_eq!(update_download_percent(100, Some(100)), Some(100));
        assert_eq!(update_download_percent(150, Some(100)), Some(100));
        assert_eq!(update_download_percent(10, None), None);
    }

    #[test]
    fn packaged_rule_snapshot_dir_finds_complete_nested_layout() {
        let directory = tempfile::tempdir().expect("tempdir");
        let nested = directory
            .path()
            .join("_up_")
            .join("resources")
            .join("rules");
        fs::create_dir_all(&nested).expect("nested rules");
        for name in [
            "iran-domains.txt",
            "iran-networks.txt",
            "private.txt",
            "iran-business-domains.txt",
        ] {
            fs::write(nested.join(name), b"ok").expect("rule file");
        }
        assert_eq!(packaged_rule_snapshot_dir(directory.path()), nested);
    }

    #[test]
    fn packaged_rule_snapshot_dir_defaults_to_rules_when_missing() {
        let directory = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            packaged_rule_snapshot_dir(directory.path()),
            directory.path().join("rules")
        );
    }

    #[test]
    fn merge_update_channels_marks_any_pending_channel() {
        let none = merge_update_channels(
            UpdateStatus {
                available: false,
                version: None,
                notes: None,
                app_available: false,
                rules_available: false,
                thirdparty_available: false,
            },
            false,
            false,
        );
        assert!(!none.available);

        let rules_only = merge_update_channels(
            UpdateStatus {
                available: false,
                version: None,
                notes: None,
                app_available: false,
                rules_available: false,
                thirdparty_available: false,
            },
            true,
            false,
        );
        assert!(rules_only.available);
        assert!(rules_only.rules_available);
        assert!(!rules_only.app_available);

        let app = merge_update_channels(
            UpdateStatus {
                available: true,
                version: Some("3.1.0".into()),
                notes: None,
                app_available: true,
                rules_available: false,
                thirdparty_available: false,
            },
            false,
            true,
        );
        assert!(app.available);
        assert!(app.app_available);
        assert!(app.thirdparty_available);
        assert_eq!(app.version.as_deref(), Some("3.1.0"));
    }

    #[test]
    fn update_progress_serializes_expected_phases() {
        let progress = UpdateProgress {
            phase: "downloading".into(),
            percent: Some(42),
            version: Some("1.2.0".into()),
            error: None,
            operation_id: None,
            app_available: None,
            rules_available: None,
            thirdparty_available: None,
        };
        let json = serde_json::to_value(progress).expect("serialize update progress");
        assert_eq!(json["phase"], "downloading");
        assert_eq!(json["percent"], 42);
    }

    #[cfg(target_os = "linux")]
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_dmi_detects_vmware_and_ignores_bare_metal() {
        assert!(linux_dmi_is_virtual(Some("VMware, Inc.\n")));
        assert!(linux_dmi_is_virtual(Some("QEMU")));
        assert!(!linux_dmi_is_virtual(Some("Dell Inc.")));
        assert!(!linux_dmi_is_virtual(None));
    }

    #[test]
    fn single_instance_id_includes_the_full_package_version() {
        assert_eq!(
            single_instance_dbus_id(BUNDLE_IDENTIFIER, "1.2.5"),
            "app.biflow.desktop.v1_2_5"
        );
        assert_ne!(
            single_instance_dbus_id(BUNDLE_IDENTIFIER, "1.2.5"),
            single_instance_dbus_id(BUNDLE_IDENTIFIER, "1.2.6")
        );
        let config = include_str!("../tauri.conf.json");
        assert!(
            config.contains(&format!("\"identifier\": \"{BUNDLE_IDENTIFIER}\"")),
            "BUNDLE_IDENTIFIER must match tauri.conf.json"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_webview_workarounds_disable_dmabuf_and_vmware_compositing() {
        let unset = LinuxWebviewEnv {
            dmabuf_already_set: false,
            compositing_already_set: false,
            software_gl_already_set: false,
        };
        let virtual_gpu = linux_webview_workarounds(
            unset,
            LinuxGpuKind {
                virtual_or_nvidia: true,
                virtual_machine: true,
            },
        );
        assert!(virtual_gpu.disable_dmabuf);
        assert!(virtual_gpu.disable_compositing);
        assert!(virtual_gpu.software_gl);
        let nvidia = linux_webview_workarounds(
            unset,
            LinuxGpuKind {
                virtual_or_nvidia: true,
                virtual_machine: false,
            },
        );
        assert!(nvidia.disable_compositing);
        assert!(!nvidia.software_gl);
        let respected = linux_webview_workarounds(
            LinuxWebviewEnv {
                dmabuf_already_set: true,
                compositing_already_set: true,
                software_gl_already_set: true,
            },
            LinuxGpuKind {
                virtual_or_nvidia: true,
                virtual_machine: true,
            },
        );
        assert!(!respected.disable_dmabuf);
        assert!(!respected.disable_compositing);
        assert!(!respected.software_gl);
        let typical = linux_webview_workarounds(
            unset,
            LinuxGpuKind {
                virtual_or_nvidia: false,
                virtual_machine: false,
            },
        );
        assert!(typical.disable_dmabuf);
        assert!(!typical.disable_compositing);
        assert!(!typical.software_gl);
        assert!(linux_webview_reexec_needed(false, virtual_gpu));
        assert!(!linux_webview_reexec_needed(true, virtual_gpu));
        assert!(!linux_webview_reexec_needed(false, respected));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_webview_reexec_command_sets_workaround_env() {
        use std::ffi::OsStr;
        let command = linux_webview_reexec_command(
            PathBuf::from("/usr/bin/BiFlow"),
            [std::ffi::OsString::from("--flag")],
            LinuxWebviewWorkarounds {
                disable_dmabuf: true,
                disable_compositing: true,
                software_gl: false,
            },
        );
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/BiFlow"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, [OsStr::new("--flag")]);
        let env: Vec<(String, String)> = command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((
                    key.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        assert!(env
            .iter()
            .any(|(key, value)| key == "BIFLOW_WEBKIT_WORKAROUNDS" && value == "1"));
        assert!(env
            .iter()
            .any(|(key, value)| key == WEBKIT_DISABLE_DMABUF_RENDERER && value == "1"));
        assert!(env
            .iter()
            .any(|(key, value)| key == WEBKIT_DISABLE_COMPOSITING_MODE && value == "1"));
        assert!(!env.iter().any(|(key, _)| key == LIBGL_ALWAYS_SOFTWARE));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_deb_packages_install_via_apt_not_self_replace() {
        assert_eq!(
            super::github_update::install_kind_from(None, false),
            super::github_update::InstallKind::Deb
        );
        assert_eq!(
            super::github_update::install_kind_from(
                Some(std::ffi::OsStr::new("/tmp/BiFlow.AppImage")),
                false
            ),
            super::github_update::InstallKind::AppImage
        );
    }

    #[test]
    fn coordinator_caches_the_checked_package() {
        let coordinator = super::UpdateCoordinator::new();
        coordinator.remember_package(Some(super::github_update::UpdateInfo {
            available: true,
            current_version: "3.5.0".into(),
            latest_version: "3.6.0".into(),
            notes: String::new(),
            asset: Some(super::github_update::Asset {
                name: "BiFlow_3.6.0_amd64.deb".into(),
                url: "https://example.invalid/package".into(),
                size: 1,
            }),
        }));
        let pending = coordinator.pending_package().expect("cached");
        assert_eq!(pending.latest_version, "3.6.0");
        coordinator.remember_package(None);
        assert!(coordinator.pending_package().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn production_linux_helper_paths_are_fixed() {
        assert_eq!(PRODUCTION_HELPER_SOCKET, "/run/iran-split/helper.sock");
        assert_eq!(PRODUCTION_SYSTEM_RUNTIME, "/var/lib/iran-split");
    }

    #[cfg(all(target_os = "linux", debug_assertions))]
    #[test]
    fn debug_linux_helper_paths_accept_development_overrides() {
        const SOCKET: &str = "/run/biflow-dev-test/helper.sock";
        const RUNTIME: &str = "/run/biflow-dev-test/runtime";
        let paths = linux_helper_paths_with_overrides(Some(SOCKET.into()), Some(RUNTIME.into()));

        assert_eq!(paths, (PathBuf::from(SOCKET), PathBuf::from(RUNTIME)));
    }

    #[cfg(all(target_os = "linux", debug_assertions))]
    #[test]
    fn debug_linux_mihomo_path_accepts_development_override() {
        const MIHOMO: &str = "/run/biflow-dev-test/mihomo";
        let path = linux_mihomo_binary_with_override(
            PathBuf::from("/default/mihomo"),
            Some(MIHOMO.into()),
        );

        assert_eq!(path, PathBuf::from(MIHOMO));
    }
}
