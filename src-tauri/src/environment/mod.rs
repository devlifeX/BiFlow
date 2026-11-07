//! Host environment snapshot for bug reports (ADR 0110).
//!
//! Every `debug.log` must say what else is running on the machine: operating
//! system, firewall and security products, other VPN/proxy clients, adapters,
//! default routes, system and environment proxies, listeners on the ports
//! the app relies on, and how a kubeconfig cluster would be routed. The same
//! build behaves differently per host, and without this context a report is
//! guesswork.
//!
//! Only classifications are logged: no hostnames, usernames, resolver or
//! proxy addresses, process lists beyond known VPN/proxy names, or cluster
//! hosts. Ports are kept because they are what identifies a conflict.

mod apps;
// Each collector's parsers are unit-tested on every host; off their own
// platform only the tests call them, so unused-item lints are expected there.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
mod linux;
#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod windows;

use apps::{AppsInfo, ClockInfo, InstallInfo, RuntimeInfo};
use iran_split_config::{AppConfig, ClientConfig, DefaultRoute};
use iran_split_core::{HelperStatus, StackSnapshot};
use iran_split_rules::{Outbound, RuleSet};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tracing::{info, warn};
use uuid::Uuid;

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const COMMAND_TIMEOUT: Duration = Duration::from_secs(25);

/// Service and scheduled-task names worth reporting. Same matching rules as
/// [`KNOWN_VPN_PROCESSES`].
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const KNOWN_SERVICES: &[&str] = &[
    "iran-split-helper",
    "biflowhelper",
    "openvpn*",
    "ovpnagent",
    "windscribe*",
    "tailscale*",
    "wireguard*",
    "cloudflarewarp",
    "warp-svc",
    "zerotier*",
    "nordvpn*",
    "expressvpn*",
    "protonvpn*",
    "surfshark*",
    "fortisslvpn*",
    "fortinet*",
    "vpnagent",
    "pangps",
    "dnscache",
    "bfe",
    "mpssvc",
    "iphlpsvc",
    "winhttpautoproxysvc",
    "sharedaccess",
    "networkmanager",
    "systemd-resolved",
    "docker",
];

/// Known VPN, proxy, and tunnel programs. Exact names are compared after
/// lowercasing and removing `.exe`; entries ending in `*` are prefixes.
/// Nothing outside this list is ever logged from the process table.
const KNOWN_VPN_PROCESSES: &[&str] = &[
    "hiddify*",
    "v2rayn*",
    "v2ray",
    "xray",
    "sing-box",
    "clash*",
    "verge-mihomo*",
    "mihomo*",
    "nekoray*",
    "nekobox*",
    "happ",
    "happd",
    "openvpn*",
    "windscribe*",
    "wireguard*",
    "wg-quick",
    "tailscale*",
    "zerotier*",
    "psiphon*",
    "outline*",
    "protonvpn*",
    "nordvpn*",
    "expressvpn*",
    "surfshark*",
    "warp-svc",
    "cloudflare warp",
    "vpnagent",
    "vpnui",
    "forticlient*",
    "fortitray",
    "pangps",
    "pritunl*",
    "vpnclient",
    "lantern*",
    "tor",
    "obfs4proxy",
    "shadowsocks*",
    "ss-local",
    "trojan*",
    "hysteria*",
    "tuic*",
    "naive",
    "geph*",
    "amnezia*",
    "proxifier*",
    "netch",
    "docker*",
    "com.docker.backend",
    "vmmem*",
    "wslservice",
];

#[derive(Debug, Clone, Serialize, Default)]
pub struct EnvironmentReport {
    /// Native taskbar lifecycle only; never includes window titles or coordinates.
    pub taskbar_speed: Option<&'static str>,
    /// Address-free native composition/clip state, separate from HWND attachment.
    pub taskbar_renderer: Option<&'static str>,
    /// Physical interface class or safe error classification; no adapter identity.
    pub taskbar_adapter: Option<&'static str>,
    pub collected_at: String,
    pub collection_ms: u64,
    pub system: SystemInfo,
    pub firewall: Vec<FirewallProfile>,
    pub security_products: Vec<SecurityProduct>,
    pub adapters: Vec<Adapter>,
    pub default_routes: Vec<RouteInfo>,
    pub dns_servers: Vec<DnsServers>,
    pub system_proxy: Vec<ProxySetting>,
    pub env_proxy: Vec<ProxySetting>,
    pub vpn_processes: Vec<String>,
    pub listeners: Vec<Listener>,
    /// `false` when the platform could not enumerate listening sockets, so a
    /// missing client port is unknown rather than closed.
    pub listener_scan: bool,
    pub hosts_file: HostsFileInfo,
    pub kube: KubeInfo,
    pub biflow: BiflowInfo,
    /// Policy rules and non-trivial routes, addresses reduced to classes.
    pub routing: RoutingInfo,
    pub kernel: Option<KernelInfo>,
    pub services: Vec<ServiceState>,
    /// Outbound block rules that name a VPN, proxy, or `BiFlow` executable.
    pub firewall_block_rules: Vec<String>,
    pub loopback: LoopbackInfo,
    pub runtime: RuntimeInfo,
    pub apps: AppsInfo,
    pub clock: ClockInfo,
    pub install: InstallInfo,
    pub findings: Vec<String>,
    pub collection_errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SystemInfo {
    pub os: &'static str,
    pub architecture: &'static str,
    pub os_name: Option<String>,
    pub os_version: Option<String>,
    pub os_build: Option<String>,
    pub kernel: Option<String>,
    pub desktop: Option<String>,
    pub session_type: Option<String>,
    pub app_version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FirewallProfile {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SecurityProduct {
    pub kind: String,
    pub name: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub description: String,
    pub status: String,
    pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RouteInfo {
    pub prefix: String,
    pub interface: String,
    pub gateway: &'static str,
    pub route_metric: Option<u32>,
    pub interface_metric: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DnsServers {
    pub interface: String,
    pub servers: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProxySetting {
    pub source: String,
    pub enabled: bool,
    pub scheme: Option<String>,
    pub host: Option<&'static str>,
    pub port: Option<u16>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Listener {
    pub port: u16,
    pub address: &'static str,
    pub process: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct HostsFileInfo {
    pub readable: bool,
    pub entries: usize,
    pub localhost: Vec<&'static str>,
}

/// Whether loopback works while `BiFlow` is up. A throwaway listener on each
/// family proves the path itself; client ports show what `localhost:<port>`
/// reaches.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct LoopbackInfo {
    /// Address classes `localhost` resolves to, in resolver order.
    pub localhost_resolves: Vec<&'static str>,
    pub ipv4: Option<LoopbackProbe>,
    pub ipv6: Option<LoopbackProbe>,
    pub clients: Vec<ClientLoopback>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LoopbackProbe {
    pub connected: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ClientLoopback {
    pub preset: &'static str,
    pub port: u16,
    pub ipv4: bool,
    pub ipv6: bool,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct RoutingInfo {
    pub rules: Vec<String>,
    pub routes: Vec<String>,
    /// Interfaces that carry an IPv6 default route.
    pub ipv6_default_interfaces: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct KernelInfo {
    pub tun_device: bool,
    pub ipv6_disabled: Option<bool>,
    pub ip_forward: Option<u8>,
    /// `rp_filter` for `all`, `default`, and tunnel interfaces. `1` (strict)
    /// drops replies that policy routing sends through TUN.
    pub rp_filter: Vec<(String, u8)>,
    pub modules: Vec<String>,
    pub network_manager_active: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServiceState {
    pub name: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct KubeInfo {
    pub config_files: usize,
    pub clusters: Vec<KubeCluster>,
    /// File names of `exec` credential plugins (e.g. `kubelogin`). They
    /// reach their own identity servers, routed separately from the cluster.
    pub auth_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KubeCluster {
    pub host: &'static str,
    /// `ipv4`, `ipv6`, or `domain`. An IPv6 API server was invisible behind
    /// `public_ip` while Windows strict-route blocked all IPv6 (ADR 0112).
    pub family: &'static str,
    pub port: Option<u16>,
    pub proxy_url: bool,
    pub route: Option<String>,
    pub route_reason: Option<String>,
    /// TCP connect from this process; comparing stopped vs running shows
    /// whether `BiFlow` is what breaks the cluster. The address is not logged.
    pub tcp_connect: Option<bool>,
    pub tcp_connect_ms: Option<u64>,
    #[serde(skip)]
    pub target: Option<(String, u16)>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct BiflowInfo {
    pub clients: Vec<ClientSummary>,
    pub default_route: String,
    pub mixed_port: u16,
    pub controller_port: u16,
    pub dns_port: u16,
    pub tun_name: String,
    pub direct_dns_preset: String,
    pub stack_phase: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientSummary {
    pub preset: &'static str,
    pub kind: String,
    pub enabled: bool,
    pub is_default: bool,
    pub host: Option<&'static str>,
    pub port: Option<u16>,
    pub profile_selected: Option<bool>,
}

/// What the desktop knows that the host cannot tell us.
pub struct CollectContext {
    pub app_version: String,
    pub config: Option<AppConfig>,
    pub rules: Option<RuleSet>,
    pub stack: Option<StackSnapshot>,
    pub helper: Option<Result<HelperStatus, String>>,
    pub data_dir: Option<PathBuf>,
    pub install_kind: &'static str,
    pub mihomo_path: Option<PathBuf>,
    pub hiddify_executable: Option<PathBuf>,
    pub hiddify_data_dir: Option<PathBuf>,
}

/// Raw platform data before the app-aware filtering.
#[derive(Debug, Default)]
struct PlatformData {
    processes: Vec<String>,
    listeners: Vec<Listener>,
    services: Vec<ServiceState>,
}

static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static LAST_REPORT: OnceLock<Mutex<Option<EnvironmentReport>>> = OnceLock::new();

fn last_slot() -> &'static Mutex<Option<EnvironmentReport>> {
    LAST_REPORT.get_or_init(|| Mutex::new(None))
}

/// Most recent report, for the support bundle.
pub fn last_report() -> Option<EnvironmentReport> {
    last_slot().lock().ok().and_then(|slot| slot.clone())
}

/// Logs the cached report again (for example right after the log was
/// cleared) so the file is never without host context.
pub fn replay_last(trigger: &str) -> bool {
    let Some(report) = last_report() else {
        return false;
    };
    emit(trigger, &report, true);
    true
}

/// Collects and logs a snapshot. A second request while one is running is
/// dropped; the running one already describes the same host.
pub async fn snapshot(trigger: &'static str, context: CollectContext) {
    if IN_FLIGHT.swap(true, Ordering::SeqCst) {
        info!(
            event = "environment.snapshot_skipped",
            section = "environment",
            initiator = "environment_snapshot",
            cause = "collection_in_progress",
            trace_route = "desktop->environment_snapshot",
            trigger,
            "environment snapshot already running"
        );
        return;
    }
    let report = collect(context).await;
    emit(trigger, &report, false);
    if let Ok(mut slot) = last_slot().lock() {
        *slot = Some(report);
    }
    IN_FLIGHT.store(false, Ordering::SeqCst);
}

fn emit(trigger: &str, report: &EnvironmentReport, cached: bool) {
    let details = serde_json::to_string(report)
        .unwrap_or_else(|error| format!("{{\"serialization_error\":\"{error}\"}}"));
    let other_vpn = report.vpn_processes.join(",");
    let findings = report.findings.join(";");
    let firewall = report
        .firewall
        .iter()
        .map(|profile| {
            format!(
                "{}={}",
                profile.name,
                if profile.enabled { "on" } else { "off" }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    info!(
        event = "environment.snapshot",
        section = "environment",
        initiator = "environment_snapshot",
        cause = trigger,
        trace_id = %Uuid::new_v4(),
        trace_route = "desktop->environment_snapshot->debug.log",
        trigger,
        cached,
        collected_at = %report.collected_at,
        collection_ms = report.collection_ms,
        os = report.system.os,
        os_name = report.system.os_name.as_deref().unwrap_or("unknown"),
        os_version = report.system.os_version.as_deref().unwrap_or("unknown"),
        os_build = report.system.os_build.as_deref().unwrap_or("unknown"),
        firewall = %firewall,
        vpn_processes = %other_vpn,
        findings = %findings,
        details = %details,
        "host environment snapshot"
    );
    for finding in &report.findings {
        warn!(
            event = "environment.finding",
            section = "environment",
            initiator = "environment_snapshot",
            cause = %finding,
            trace_route = "desktop->environment_snapshot->findings",
            trigger,
            "host environment may conflict with BiFlow"
        );
    }
}

async fn collect(context: CollectContext) -> EnvironmentReport {
    let started = Instant::now();
    let mut report = EnvironmentReport {
        collected_at: chrono::Utc::now().to_rfc3339(),
        system: SystemInfo {
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            app_version: context.app_version.clone(),
            ..SystemInfo::default()
        },
        ..EnvironmentReport::default()
    };
    let stack_phase = context.stack.as_ref().map_or_else(
        || "unknown".to_owned(),
        |stack| format!("{:?}", stack.phase).to_ascii_lowercase(),
    );
    report.biflow = biflow_info(context.config.as_ref(), &stack_phase);
    report.runtime = apps::runtime_info(context.stack.as_ref(), context.helper.as_ref());
    report.install = apps::install_info(context.install_kind);
    let data_dir = context.data_dir.as_deref();
    let openvpn = apps::openvpn_candidates()
        .into_iter()
        .find(|path| path.exists());
    let (mihomo, openvpn_binary, hiddify_binary, clock) = tokio::join!(
        apps::binary_info(context.mihomo_path.as_deref(), "-v", data_dir),
        apps::binary_info(openvpn.as_deref(), "--version", data_dir),
        async {
            // Hiddify is a GUI; running it with `--version` would open a
            // window, so only its location is reported.
            context
                .hiddify_executable
                .as_deref()
                .map(|path| apps::BinaryInfo {
                    found: true,
                    location: apps::path_location(path, data_dir),
                    version: None,
                })
        },
        apps::clock_info(),
    );
    let mut hiddify = apps::hiddify_info(context.hiddify_data_dir.as_deref());
    hiddify.executable = Some(hiddify_binary.unwrap_or(apps::BinaryInfo {
        found: false,
        location: "missing",
        version: None,
    }));
    report.apps = AppsInfo {
        hiddify: Some(hiddify),
        mihomo: Some(mihomo),
        openvpn: Some(openvpn_binary),
        side_tunnels: apps::side_tunnel_profiles(context.config.as_ref(), data_dir),
        system_proxy_saved_by_pause: data_dir
            .is_some_and(|dir| dir.join("system-proxy-snapshot.json").exists()),
    };
    report.clock = clock;
    report.loopback = loopback_info(context.config.as_ref()).await;
    report.env_proxy = process_env_proxies(|name| std::env::var(name).ok());
    report.hosts_file = hosts_file_info(&hosts_path());
    report.kube = kube_info(context.config.as_ref(), context.rules.as_ref());
    probe_kube_clusters(&mut report.kube.clusters).await;

    let tun_name = context
        .config
        .as_ref()
        .map(|config| config.mihomo.tun_name.clone())
        .unwrap_or_default();
    let platform = collect_platform(&mut report, &tun_name).await;
    report.vpn_processes = known_vpn_processes(&platform.processes);
    report.listener_scan = !platform.listeners.is_empty();
    let ports = interesting_ports(context.config.as_ref());
    report.listeners = relevant_listeners(platform.listeners, &ports);
    report.services = known_services(platform.services);
    #[cfg(windows)]
    {
        report.taskbar_renderer = Some(iran_split_taskbar::diagnostic_renderer());
        report.taskbar_adapter = Some(crate::taskbar::adapter_state());
        report.taskbar_speed = Some(match iran_split_taskbar::diagnostic_status() {
            iran_split_taskbar::Status::Disabled => "disabled",
            iran_split_taskbar::Status::Attached => "attached",
            iran_split_taskbar::Status::Unavailable => "unavailable",
            iran_split_taskbar::Status::Failed => "failed",
        });
    }
    #[cfg(target_os = "linux")]
    {
        report.taskbar_renderer = Some("appindicator_label");
        report.taskbar_adapter = Some(crate::taskbar::adapter_state());
        report.taskbar_speed = Some(crate::taskbar::panel_state());
    }
    #[cfg(target_os = "macos")]
    {
        report.taskbar_renderer = Some("menu_bar_label");
        report.taskbar_adapter = Some(crate::taskbar::adapter_state());
        report.taskbar_speed = Some(crate::taskbar::panel_state());
    }
    report.findings = findings(&report, context.config.as_ref());
    report.collection_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    report
}

// ---------------------------------------------------------------------------
// Loopback self-test.
// ---------------------------------------------------------------------------

const LOOPBACK_TIMEOUT: Duration = Duration::from_millis(1500);

async fn connects(address: std::net::SocketAddr) -> Option<u64> {
    let started = Instant::now();
    match tokio::time::timeout(LOOPBACK_TIMEOUT, tokio::net::TcpStream::connect(address)).await {
        Ok(Ok(_)) => Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        _ => None,
    }
}

/// Binds an ephemeral listener and connects to it. `None` when the family
/// cannot even bind (IPv6 disabled), which is not a `BiFlow` problem.
async fn loopback_probe(address: IpAddr) -> Option<LoopbackProbe> {
    let listener = tokio::net::TcpListener::bind((address, 0)).await.ok()?;
    let target = listener.local_addr().ok()?;
    let accept = tokio::spawn(async move { listener.accept().await.is_ok() });
    let started = Instant::now();
    let connected = connects(target).await.is_some();
    accept.abort();
    Some(LoopbackProbe {
        connected,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

async fn loopback_info(config: Option<&AppConfig>) -> LoopbackInfo {
    let localhost_resolves = tokio::net::lookup_host("localhost:0")
        .await
        .map(|addresses| {
            let mut classes = Vec::new();
            for address in addresses {
                let class = match address.ip() {
                    IpAddr::V4(ip) if ip.is_loopback() => "loopback_v4",
                    IpAddr::V6(ip) if ip.is_loopback() => "loopback_v6",
                    other => classify_host(&other.to_string()),
                };
                if !classes.contains(&class) {
                    classes.push(class);
                }
            }
            classes
        })
        .unwrap_or_default();
    let v4 = IpAddr::from([127, 0, 0, 1]);
    let v6 = IpAddr::from(std::net::Ipv6Addr::LOCALHOST);
    let (ipv4, ipv6) = tokio::join!(loopback_probe(v4), loopback_probe(v6));
    let mut clients = Vec::new();
    for client in config.map(AppConfig::enabled_clients).unwrap_or_default() {
        if let ClientConfig::LocalProxy { port, .. } = client.config {
            let (on_v4, on_v6) =
                tokio::join!(connects((v4, port).into()), connects((v6, port).into()));
            clients.push(ClientLoopback {
                preset: client.spec().id,
                port,
                ipv4: on_v4.is_some(),
                ipv6: on_v6.is_some(),
            });
        }
    }
    LoopbackInfo {
        localhost_resolves,
        ipv4,
        ipv6,
        clients,
    }
}

fn loopback_findings(report: &EnvironmentReport, findings: &mut Vec<String>) {
    let loopback = &report.loopback;
    if loopback.ipv4.as_ref().is_some_and(|probe| !probe.connected) {
        findings.push("loopback_ipv4_blocked".into());
    }
    if loopback.ipv6.as_ref().is_some_and(|probe| !probe.connected) {
        findings.push("loopback_ipv6_blocked".into());
    }
    if loopback
        .localhost_resolves
        .iter()
        .any(|class| !class.starts_with("loopback"))
    {
        findings.push("localhost_resolves_non_loopback".into());
    }
    let v6_first = loopback.localhost_resolves.first() == Some(&"loopback_v6");
    for client in &loopback.clients {
        if !client.ipv4 && !client.ipv6 {
            findings.push(format!(
                "client_port_refused_on_loopback:{}:{}",
                client.preset, client.port
            ));
        } else if client.ipv4 && !client.ipv6 && v6_first {
            // `localhost:<port>` tries ::1 first; an IPv4-only listener then
            // depends on the caller falling back to 127.0.0.1.
            findings.push(format!(
                "client_ipv4_only_but_localhost_prefers_ipv6:{}:{}",
                client.preset, client.port
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Classification helpers (pure, tested on every host).
// ---------------------------------------------------------------------------

/// Reduces an address or hostname to a class that is safe to log.
fn classify_host(host: &str) -> &'static str {
    let trimmed = host.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.is_empty() {
        return "empty";
    }
    if trimmed.eq_ignore_ascii_case("localhost") || trimmed.ends_with(".localhost") {
        return "loopback_name";
    }
    match trimmed.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            let [a, b, ..] = ip.octets();
            if ip.is_loopback() {
                "loopback"
            } else if ip.is_unspecified() {
                "unspecified"
            } else if a == 198 && (b == 18 || b == 19) {
                "fake_ip"
            } else if a == 100 && (64..128).contains(&b) {
                "cgnat"
            } else if ip.is_private() {
                "private"
            } else if ip.is_link_local() {
                "link_local"
            } else {
                "public_ip"
            }
        }
        Ok(IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            if ip.is_loopback() {
                "loopback"
            } else if ip.is_unspecified() {
                "unspecified"
            } else if first & 0xfe00 == 0xfc00 {
                "private"
            } else if first & 0xffc0 == 0xfe80 {
                "link_local"
            } else {
                "public_ip"
            }
        }
        Err(_) => {
            let lower = trimmed.to_ascii_lowercase();
            match lower.rsplit_once('.').map(|(_, tld)| tld) {
                Some("ir") => "domain_ir",
                Some("local" | "lan") | None => "local_name",
                Some(_) => "domain",
            }
        }
    }
}

fn is_loopback_class(class: &str) -> bool {
    matches!(class, "loopback" | "loopback_name" | "unspecified")
}

#[derive(Debug, PartialEq, Eq)]
struct Endpoint {
    scheme: Option<String>,
    host: String,
    port: Option<u16>,
}

/// Parses `scheme://user:pass@host:port/path`, `host:port`, or `[v6]:port`.
fn parse_endpoint(value: &str) -> Option<Endpoint> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (scheme, rest) = match value.split_once("://") {
        Some((scheme, rest)) => (Some(scheme.to_ascii_lowercase()), rest),
        None => (None, value),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let (host, tail) = stripped.split_once(']')?;
        (
            host,
            tail.strip_prefix(':').and_then(|port| port.parse().ok()),
        )
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, port.parse().ok()),
            _ => (authority, None),
        }
    };
    if host.is_empty() {
        return None;
    }
    Some(Endpoint {
        scheme,
        host: host.to_owned(),
        port,
    })
}

fn proxy_setting(source: &str, value: &str, detail: Option<String>) -> ProxySetting {
    let endpoint = parse_endpoint(value);
    ProxySetting {
        source: source.to_owned(),
        enabled: endpoint.is_some(),
        scheme: endpoint
            .as_ref()
            .and_then(|endpoint| endpoint.scheme.clone()),
        host: endpoint
            .as_ref()
            .map(|endpoint| classify_host(&endpoint.host)),
        port: endpoint.as_ref().and_then(|endpoint| endpoint.port),
        detail,
    }
}

const PROXY_ENV_NAMES: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// Proxy variables in the app's own environment. CLI tools such as kubectl,
/// git, and curl honour them before TUN ever sees the connection.
fn process_env_proxies(read: impl Fn(&str) -> Option<String>) -> Vec<ProxySetting> {
    let mut settings = Vec::new();
    for name in PROXY_ENV_NAMES {
        if let Some(value) = read(name).filter(|value| !value.trim().is_empty()) {
            settings.push(proxy_setting(&format!("env:{name}"), &value, None));
        }
    }
    for name in ["NO_PROXY", "no_proxy"] {
        if let Some(value) = read(name).filter(|value| !value.trim().is_empty()) {
            settings.push(no_proxy_setting(&format!("env:{name}"), &value));
        }
    }
    settings
}

fn no_proxy_setting(source: &str, value: &str) -> ProxySetting {
    let entries = value
        .split([',', ';'])
        .filter(|entry| !entry.trim().is_empty())
        .count();
    ProxySetting {
        source: source.to_owned(),
        enabled: true,
        scheme: None,
        host: None,
        port: None,
        detail: Some(format!("entries={entries}")),
    }
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), allow(dead_code))]
fn adapter_kind(name: &str, description: &str) -> &'static str {
    let text = format!("{name} {description}").to_ascii_lowercase();
    let rules: &[(&str, &'static str)] = &[
        ("loopback", "loopback"),
        ("wireguard", "wireguard"),
        ("wintun", "wintun"),
        ("ovpn-dco", "openvpn_dco"),
        ("tap-windows", "tap"),
        ("tap-", "tap"),
        ("windscribe", "vpn"),
        ("tailscale", "vpn"),
        ("zerotier", "vpn"),
        ("cloudflare", "vpn"),
        ("fortinet", "vpn"),
        ("cisco", "vpn"),
        ("anyconnect", "vpn"),
        ("pangp", "vpn"),
        ("vpn", "vpn"),
        ("meta", "tun"),
        ("tun", "tun"),
        ("wsl", "wsl"),
        ("hyper-v", "hyperv"),
        ("vethernet", "hyperv"),
        ("docker", "docker"),
        ("veth", "container"),
        ("br-", "bridge"),
        ("vmware", "vmware"),
        ("virtualbox", "virtualbox"),
        ("bluetooth", "bluetooth"),
        ("wi-fi", "wifi"),
        ("wireless", "wifi"),
        ("wlan", "wifi"),
        ("ethernet", "ethernet"),
    ];
    rules
        .iter()
        .find(|(needle, _)| text.contains(needle))
        .map_or("other", |(_, kind)| kind)
}

fn normalize_process(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    lower
        .strip_suffix(".exe")
        .map_or(lower.clone(), str::to_owned)
}

fn is_known_vpn_process(name: &str) -> bool {
    matches_known(KNOWN_VPN_PROCESSES, name)
}

fn known_vpn_processes(processes: &[String]) -> Vec<String> {
    processes
        .iter()
        .filter(|name| is_known_vpn_process(name))
        .map(|name| normalize_process(name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn matches_known(list: &[&str], name: &str) -> bool {
    let name = normalize_process(name);
    list.iter().any(|pattern| {
        pattern
            .strip_suffix('*')
            .map_or(name == *pattern, |prefix| name.starts_with(prefix))
    })
}

fn known_services(services: Vec<ServiceState>) -> Vec<ServiceState> {
    let mut kept: Vec<ServiceState> = services
        .into_iter()
        .filter(|service| matches_known(KNOWN_SERVICES, &service.name))
        .collect();
    kept.sort_by(|left, right| left.name.cmp(&right.name));
    kept.dedup();
    kept
}

fn interesting_ports(config: Option<&AppConfig>) -> BTreeSet<u16> {
    let mut ports = BTreeSet::new();
    if let Some(config) = config {
        ports.extend([
            config.mihomo.mixed_port,
            config.mihomo.controller_port,
            config.mihomo.dns_port,
        ]);
        for client in &config.clients {
            if let ClientConfig::LocalProxy { port, .. } = client.config {
                ports.insert(port);
            }
            if let Some(port) = client.spec().default_port {
                ports.insert(port);
            }
        }
    }
    ports.extend([53, 1080, 2080, 7890, 7891, 7897, 9090, 10808, 10809, 12334]);
    ports
}

/// Keeps listeners on ports the app cares about plus any socket owned by a
/// known VPN/proxy process; the rest of the host's sockets are not logged.
fn relevant_listeners(listeners: Vec<Listener>, ports: &BTreeSet<u16>) -> Vec<Listener> {
    listeners
        .into_iter()
        .filter(|listener| {
            ports.contains(&listener.port)
                || listener
                    .process
                    .as_deref()
                    .is_some_and(is_known_vpn_process)
        })
        .map(|mut listener| {
            listener.process = listener.process.map(|name| {
                if is_known_vpn_process(&name) || ports.contains(&listener.port) {
                    normalize_process(&name)
                } else {
                    "other".into()
                }
            });
            listener
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn biflow_info(config: Option<&AppConfig>, stack_phase: &str) -> BiflowInfo {
    let Some(config) = config else {
        return BiflowInfo {
            stack_phase: stack_phase.to_owned(),
            ..BiflowInfo::default()
        };
    };
    let default_id = match config.default_route {
        DefaultRoute::Direct => None,
        DefaultRoute::Client { client_id } => Some(client_id),
    };
    BiflowInfo {
        clients: config
            .clients
            .iter()
            .map(|client| {
                let (host, port, profile_selected) = match &client.config {
                    ClientConfig::LocalProxy { host, port, .. } => {
                        (Some(classify_host(host)), Some(*port), None)
                    }
                    ClientConfig::OwnedSideTunnel { profile_path, .. } => {
                        (None, None, Some(profile_path.is_some()))
                    }
                    ClientConfig::Unsupported => (None, None, None),
                };
                ClientSummary {
                    preset: client.spec().id,
                    kind: format!("{:?}", client.spec().kind).to_ascii_lowercase(),
                    enabled: client.enabled,
                    is_default: default_id == Some(client.id),
                    host,
                    port,
                    profile_selected,
                }
            })
            .collect(),
        default_route: match default_id.and_then(|id| config.client(id)) {
            Some(client) => format!("client:{}", client.spec().id),
            None if default_id.is_some() => "client:missing".into(),
            None => "direct".into(),
        },
        mixed_port: config.mihomo.mixed_port,
        controller_port: config.mihomo.controller_port,
        dns_port: config.mihomo.dns_port,
        tun_name: config.mihomo.tun_name.clone(),
        direct_dns_preset: format!("{:?}", config.mihomo.direct_dns_preset).to_ascii_lowercase(),
        stack_phase: stack_phase.to_owned(),
    }
}

fn hosts_path() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root)
            .join("System32")
            .join("drivers")
            .join("etc")
            .join("hosts")
    } else {
        PathBuf::from("/etc/hosts")
    }
}

fn hosts_file_info(path: &Path) -> HostsFileInfo {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HostsFileInfo::default();
    };
    parse_hosts(&text)
}

fn parse_hosts(text: &str) -> HostsFileInfo {
    let mut info = HostsFileInfo {
        readable: true,
        ..HostsFileInfo::default()
    };
    let mut localhost = BTreeSet::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let mut fields = line.split_whitespace();
        let Some(address) = fields.next() else {
            continue;
        };
        info.entries += 1;
        if fields.any(|name| name.eq_ignore_ascii_case("localhost")) {
            localhost.insert(classify_host(address));
        }
    }
    info.localhost = localhost.into_iter().collect();
    info
}

fn kubeconfig_paths() -> Vec<PathBuf> {
    if let Some(value) = std::env::var_os("KUBECONFIG").filter(|value| !value.is_empty()) {
        return std::env::split_paths(&value).collect();
    }
    dirs::home_dir()
        .map(|home| vec![home.join(".kube").join("config")])
        .unwrap_or_default()
}

fn kube_info(config: Option<&AppConfig>, rules: Option<&RuleSet>) -> KubeInfo {
    let mut info = KubeInfo::default();
    for path in kubeconfig_paths() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        info.config_files += 1;
        info.clusters.extend(parse_kubeconfig(&text, |host| {
            route_for(config, rules, host)
        }));
        for command in parse_kube_auth_commands(&text) {
            if !info.auth_commands.contains(&command) {
                info.auth_commands.push(command);
            }
        }
    }
    info
}

fn route_for(
    config: Option<&AppConfig>,
    rules: Option<&RuleSet>,
    host: &str,
) -> Option<(String, String)> {
    let decision = rules?.decide(host).ok()?;
    let outbound = match decision.outbound {
        Outbound::Direct => "direct".to_owned(),
        Outbound::Client { client_id } => config
            .and_then(|config| config.client(client_id))
            .map_or_else(
                || "client:missing".into(),
                |client| format!("client:{}", client.spec().id),
            ),
    };
    Some((
        outbound,
        format!("{:?}", decision.reason).to_ascii_lowercase(),
    ))
}

/// Extracts `server:` and `proxy-url:` from a kubeconfig without a YAML
/// dependency. Each `cluster:` block holds one server.
fn parse_kubeconfig(
    text: &str,
    route: impl Fn(&str) -> Option<(String, String)>,
) -> Vec<KubeCluster> {
    let mut clusters: Vec<KubeCluster> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches("- ").trim();
        if let Some(value) = trimmed.strip_prefix("server:") {
            let value = value.trim().trim_matches(['"', '\'']);
            let endpoint = parse_endpoint(value);
            let host = endpoint
                .as_ref()
                .map_or("", |endpoint| endpoint.host.as_str());
            let decision = endpoint.as_ref().and_then(|_| route(host));
            let port = endpoint.as_ref().and_then(|endpoint| {
                endpoint.port.or(match endpoint.scheme.as_deref() {
                    Some("https") => Some(443),
                    Some("http") => Some(80),
                    _ => None,
                })
            });
            clusters.push(KubeCluster {
                host: classify_host(host),
                family: match host.parse::<IpAddr>() {
                    Ok(IpAddr::V4(_)) => "ipv4",
                    Ok(IpAddr::V6(_)) => "ipv6",
                    Err(_) => "domain",
                },
                port,
                proxy_url: false,
                route: decision.as_ref().map(|(outbound, _)| outbound.clone()),
                route_reason: decision.map(|(_, reason)| reason),
                tcp_connect: None,
                tcp_connect_ms: None,
                target: port
                    .filter(|_| !host.is_empty())
                    .map(|port| (host.to_owned(), port)),
            });
        } else if trimmed.starts_with("proxy-url:") {
            if let Some(cluster) = clusters.last_mut() {
                cluster.proxy_url = true;
            }
        }
    }
    clusters
}

/// `command:` values under `exec:` credential plugins, reduced to a file name.
fn parse_kube_auth_commands(text: &str) -> Vec<String> {
    let mut commands = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches("- ").trim();
        if let Some(value) = trimmed.strip_prefix("command:") {
            let value = value.trim().trim_matches(['"', '\'']);
            let name = value
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .chars()
                .take(40)
                .collect::<String>();
            if !name.is_empty() {
                commands.insert(name);
            }
        }
    }
    commands.into_iter().collect()
}

/// Connects to each cluster endpoint once (TCP only, 3 s). Resolves names
/// through the system resolver, exactly like kubectl.
async fn probe_kube_clusters(clusters: &mut [KubeCluster]) {
    for cluster in clusters.iter_mut().take(8) {
        let Some((host, port)) = cluster.target.clone() else {
            continue;
        };
        let started = Instant::now();
        let connected = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::TcpStream::connect((host.as_str(), port)),
        )
        .await
        .is_ok_and(|result| result.is_ok());
        cluster.tcp_connect = Some(connected);
        cluster.tcp_connect_ms =
            Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    }
}

/// Heuristics that point at the conflicts reports keep hitting. Each entry
/// is a stable `name:detail` string so logs can be grepped across users.
fn findings(report: &EnvironmentReport, config: Option<&AppConfig>) -> Vec<String> {
    let biflow_ports: BTreeSet<u16> = config
        .map(|config| {
            [
                config.mihomo.mixed_port,
                config.mihomo.controller_port,
                config.mihomo.dns_port,
            ]
            .into_iter()
            .collect()
        })
        .unwrap_or_default();
    let mut findings = Vec::new();
    if let Some(config) = config {
        client_findings(report, config, &mut findings);
        if config.behavior.show_taskbar_speed
            && matches!(report.taskbar_speed, Some("unavailable" | "failed"))
        {
            findings.push("taskbar_speed_unavailable:see_taskbar_lifecycle_events".into());
        }
        if config.behavior.show_taskbar_speed
            && matches!(report.taskbar_renderer, Some("clipped" | "unlayered_gdi"))
        {
            findings.push("taskbar_surface_unavailable:see_taskbar_paint_surface".into());
        }
        if config.behavior.show_taskbar_speed
            && report
                .taskbar_adapter
                .is_some_and(|state| !matches!(state, "wifi" | "ethernet" | "disabled"))
        {
            findings.push("taskbar_adapter_unavailable:see_interface_stats_events".into());
        }
    }
    port_findings(report, config, &biflow_ports, &mut findings);
    network_findings(report, config, &mut findings);
    host_findings(report, config, &mut findings);
    state_findings(report, config, &mut findings);
    loopback_findings(report, &mut findings);
    apps::app_findings(report, config, &mut findings);
    findings
}

/// Leftovers after Pause/Disconnect, IPv6 escaping TUN, and host settings
/// that silently break policy routing or the helper.
fn state_findings(
    report: &EnvironmentReport,
    config: Option<&AppConfig>,
    findings: &mut Vec<String>,
) {
    let phase = report.biflow.stack_phase.as_str();
    if matches!(phase, "paused" | "stopped") {
        idle_findings(report, config, findings);
    }
    if matches!(phase, "running" | "degraded") {
        running_findings(report, config, findings);
    }
    host_state_findings(report, findings);
}

fn is_biflow_tun(config: Option<&AppConfig>, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "meta"
        || config.is_some_and(|config| config.mihomo.tun_name.eq_ignore_ascii_case(&name))
}

fn idle_findings(
    report: &EnvironmentReport,
    config: Option<&AppConfig>,
    findings: &mut Vec<String>,
) {
    let is_tun = |name: &str| is_biflow_tun(config, name);
    {
        for adapter in report
            .adapters
            .iter()
            .filter(|adapter| is_tun(&adapter.name))
        {
            if matches!(
                adapter.status.to_ascii_lowercase().as_str(),
                "up" | "unknown"
            ) {
                findings.push(format!("leftover_tun_adapter:{}", adapter.name));
            }
        }
        let leftover_rules = report
            .routing
            .rules
            .iter()
            .chain(&report.routing.routes)
            .filter(|line| {
                line.split_whitespace()
                    .any(|token| is_tun(token.trim_end_matches(',')))
            })
            .count();
        if leftover_rules > 0 {
            findings.push(format!("leftover_tun_routes:{leftover_rules}"));
        }
        if report
            .dns_servers
            .iter()
            .any(|entry| entry.servers.contains(&"fake_ip"))
        {
            findings.push("leftover_fake_ip_dns".into());
        }
        if let Some(mixed) = config.map(|config| config.mihomo.mixed_port) {
            if report
                .system_proxy
                .iter()
                .any(|setting| setting.enabled && setting.port == Some(mixed))
            {
                findings.push(format!("leftover_system_proxy_to_mihomo:{mixed}"));
            }
        }
    }
}

fn running_findings(
    report: &EnvironmentReport,
    config: Option<&AppConfig>,
    findings: &mut Vec<String>,
) {
    if let Some(stack) = &report.runtime.stack {
        for component in ["mihomo", "tun", "dns"] {
            if stack
                .get(component)
                .and_then(|status| status.get("phase"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|phase| phase != "running")
            {
                findings.push(format!("active_stack_runtime_unhealthy:{component}"));
            }
        }
    }
    let is_tun = |name: &str| is_biflow_tun(config, name);
    {
        let client_ports: BTreeSet<u16> = config
            .map(|config| {
                config
                    .clients
                    .iter()
                    .filter_map(|client| match client.config {
                        ClientConfig::LocalProxy { port, .. } => Some(port),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for setting in &report.system_proxy {
            if let (true, Some(port)) = (setting.enabled, setting.port) {
                if client_ports.contains(&port) {
                    findings.push(format!("system_proxy_bypasses_split_routing:{port}"));
                }
            }
        }
        let outside: Vec<&String> = report
            .routing
            .ipv6_default_interfaces
            .iter()
            .filter(|name| !is_tun(name))
            .collect();
        if !outside.is_empty()
            && !report
                .routing
                .ipv6_default_interfaces
                .iter()
                .any(|name| is_tun(name))
        {
            findings.push(format!("ipv6_default_route_outside_tun:{}", outside[0]));
        }
    }
}

fn host_state_findings(report: &EnvironmentReport, findings: &mut Vec<String>) {
    if let Some(kernel) = &report.kernel {
        if !kernel.tun_device {
            findings.push("tun_device_missing".into());
        }
        for (interface, value) in &kernel.rp_filter {
            if *value == 1 {
                findings.push(format!("rp_filter_strict:{interface}"));
            }
        }
        for kind in &kernel.network_manager_active {
            if matches!(kind.as_str(), "vpn" | "wireguard") {
                findings.push(format!("network_manager_vpn_active:{kind}"));
            }
        }
    }
    for service in &report.services {
        let name = service.name.to_ascii_lowercase();
        let state = service.state.to_ascii_lowercase();
        let active = matches!(state.as_str(), "active" | "running");
        // A SYSTEM scheduled task is invisible to a standard user's
        // `Get-ScheduledTask`; only report it when the helper is also down.
        let helper_reachable = report
            .runtime
            .helper
            .as_ref()
            .is_some_and(|helper| helper.available);
        if (name == "iran-split-helper" || name == "biflowhelper") && !active && !helper_reachable {
            findings.push(format!("helper_service:{}", service.state));
        } else if active
            && [
                "windscribe",
                "nordvpn",
                "expressvpn",
                "protonvpn",
                "surfshark",
                "cloudflarewarp",
                "warp-svc",
                "fortisslvpn",
                "vpnagent",
                "pangps",
            ]
            .iter()
            .any(|vpn| name.starts_with(vpn))
        {
            findings.push(format!("other_vpn_service_running:{}", service.name));
        }
    }
    for rule in &report.firewall_block_rules {
        findings.push(format!("firewall_blocks_program:{rule}"));
    }
}

fn client_findings(report: &EnvironmentReport, config: &AppConfig, findings: &mut Vec<String>) {
    for client in config.enabled_clients() {
        let ClientConfig::LocalProxy { port, host, .. } = &client.config else {
            continue;
        };
        let listening: Vec<&Listener> = report
            .listeners
            .iter()
            .filter(|listener| listener.port == *port)
            .collect();
        if listening.is_empty() && report.listener_scan {
            findings.push(format!(
                "client_port_not_listening:{}:{port}",
                client.spec().id
            ));
        }
        if classify_host(host) == "loopback_name"
            && !listening.is_empty()
            && listening
                .iter()
                .all(|listener| listener.address == "loopback")
        {
            findings.push(format!(
                "client_host_is_localhost_but_listener_is_ipv4_only:{}:{port}",
                client.spec().id
            ));
        }
    }
}

fn port_findings(
    report: &EnvironmentReport,
    config: Option<&AppConfig>,
    biflow_ports: &BTreeSet<u16>,
    findings: &mut Vec<String>,
) {
    let client_ports: BTreeSet<u16> = config
        .map(|config| {
            config
                .clients
                .iter()
                .filter_map(|client| match client.config {
                    ClientConfig::LocalProxy { port, .. } => Some(port),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    for listener in &report.listeners {
        if let (true, Some(process)) = (biflow_ports.contains(&listener.port), &listener.process) {
            if !process.starts_with("mihomo") && !process.starts_with("biflow") {
                findings.push(format!("port_conflict:{}:{process}", listener.port));
            }
        }
    }
    for setting in report.system_proxy.iter().filter(|setting| setting.enabled) {
        match (setting.host, setting.port) {
            (Some(host), Some(port)) if is_loopback_class(host) => {
                if !client_ports.contains(&port) && !biflow_ports.contains(&port) {
                    findings.push(format!(
                        "system_proxy_unknown_local_port:{}:{port}",
                        setting.source
                    ));
                }
            }
            (Some(host), _) => {
                findings.push(format!("system_proxy_not_local:{}:{host}", setting.source));
            }
            _ => {}
        }
    }
    for setting in &report.env_proxy {
        if setting.enabled && setting.host.is_some() {
            findings.push(format!("env_proxy_set:{}", setting.source));
        }
    }
}

fn network_findings(
    report: &EnvironmentReport,
    config: Option<&AppConfig>,
    findings: &mut Vec<String>,
) {
    for cluster in &report.kube.clusters {
        if cluster.proxy_url {
            findings.push(format!("kube_cluster_proxy_url:{}", cluster.host));
        }
        if cluster.tcp_connect == Some(false) {
            findings.push(format!(
                "kube_cluster_unreachable:{}:{}",
                cluster.family,
                cluster.route.as_deref().unwrap_or("unknown")
            ));
        }
        if cluster.route.as_deref() == Some("direct") {
            findings.push(format!(
                "kube_cluster_direct:{}:{}",
                cluster.host,
                cluster.route_reason.as_deref().unwrap_or("unknown")
            ));
        }
    }
    let tun_name = config.map(|config| config.mihomo.tun_name.to_ascii_lowercase());
    for adapter in &report.adapters {
        let up = matches!(
            adapter.status.to_ascii_lowercase().as_str(),
            "up" | "unknown"
        );
        let name = adapter.name.to_ascii_lowercase();
        let ours = tun_name.as_deref() == Some(name.as_str())
            || name == "meta"
            || is_side_tunnel_device(&name, config);
        if up && !ours && matches!(adapter.kind, "wireguard" | "vpn" | "tun") {
            findings.push(format!("other_tunnel_adapter_up:{}", adapter.name));
        }
    }
    // One IPv4 and one IPv6 default route is normal; count per family.
    for family_is_v6 in [false, true] {
        let full_defaults = report
            .default_routes
            .iter()
            .filter(|route| route.prefix.contains(':') == family_is_v6)
            .filter(|route| route.prefix.ends_with("/0"))
            .count();
        if full_defaults > 1 {
            let family = if family_is_v6 { "v6" } else { "v4" };
            findings.push(format!("multiple_default_routes:{family}:{full_defaults}"));
        }
    }
    for route in &report.default_routes {
        // Mihomo's own TUN installs the split-default routes.
        if route.prefix.ends_with("/1") && !is_biflow_tun(config, &route.interface) {
            findings.push(format!("split_default_route:{}", route.interface));
        }
    }
}

/// The helper names side-tunnel devices `tun-<first 8 of client id>`.
fn is_side_tunnel_device(name: &str, config: Option<&AppConfig>) -> bool {
    let Some(suffix) = name.strip_prefix("tun-") else {
        return false;
    };
    config.is_some_and(|config| {
        config
            .clients
            .iter()
            .any(|client| client.id.as_hyphenated().starts_with(suffix))
    })
}

fn host_findings(
    report: &EnvironmentReport,
    config: Option<&AppConfig>,
    findings: &mut Vec<String>,
) {
    for process in &report.vpn_processes {
        // Only local-proxy clients run as their own named process. A side
        // tunnel such as Windscribe rides OpenVPN, so a running Windscribe
        // GUI is a second VPN (with its own firewall), not BiFlow's client.
        let ours = config.is_some_and(|config| {
            config.enabled_clients().iter().any(|client| {
                matches!(client.config, ClientConfig::LocalProxy { .. })
                    && process.starts_with(client.spec().id)
            })
        });
        if !ours
            && !process.starts_with("mihomo")
            && !process.starts_with("openvpn")
            && !["docker", "vmmem", "wsl"]
                .iter()
                .any(|name| process.contains(name))
        {
            findings.push(format!("other_vpn_process:{process}"));
        }
    }
    for product in &report.security_products {
        let name = product.name.to_ascii_lowercase();
        if product.enabled != Some(false)
            && !name.contains("windows defender")
            && !name.contains("microsoft defender")
        {
            findings.push(format!(
                "third_party_security:{}:{}",
                product.kind, product.name
            ));
        }
    }
    for profile in &report.firewall {
        if !profile.enabled && cfg!(windows) {
            findings.push(format!("firewall_disabled:{}", profile.name));
        }
    }
    if report
        .hosts_file
        .localhost
        .iter()
        .any(|class| !is_loopback_class(class))
    {
        findings.push("hosts_localhost_not_loopback".into());
    }
}

// ---------------------------------------------------------------------------
// Platform collectors.
// ---------------------------------------------------------------------------

#[cfg(any(windows, target_os = "linux"))]
async fn run_command(program: &str, args: &[&str]) -> Result<String, String> {
    run_command_timeout(OsStr::new(program), args, COMMAND_TIMEOUT).await
}

/// Runs a bounded command without a console window. Errors name only the
/// program's file name and the I/O error kind.
async fn run_command_timeout(
    program: &OsStr,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let label = Path::new(program).file_name().map_or_else(
        || "command".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    let program_label = label.as_str();
    let mut command = tokio::process::Command::new(program);
    command.args(args).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| format!("{program_label} timed out"))?
        .map_err(|error| format!("{program_label} could not start: {}", error.kind()))?;
    if !output.status.success() && output.stdout.is_empty() {
        // The first stderr line is the only clue a field report carries.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| apps::redact_addresses(&line.chars().take(240).collect::<String>()))
            .unwrap_or_default();
        return Err(format!(
            "{program_label} exited with {}: {reason}",
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
use linux::collect_platform;
#[cfg(windows)]
use windows::collect_platform;

#[cfg(not(any(windows, target_os = "linux")))]
#[allow(clippy::unused_async)]
async fn collect_platform(_report: &mut EnvironmentReport, _tun_name: &str) -> PlatformData {
    PlatformData::default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn active_stack_reports_missing_tun_and_dns_after_resume() {
        let mut report = EnvironmentReport::default();
        report.biflow.stack_phase = "running".into();
        report.runtime.stack = Some(serde_json::json!({
            "mihomo": {"phase": "running"}, "tun": {"phase": "stopped"},
            "dns": {"phase": "error"}
        }));
        assert_eq!(
            findings(&report, None),
            vec![
                "active_stack_runtime_unhealthy:tun",
                "active_stack_runtime_unhealthy:dns"
            ]
        );
    }

    #[test]
    fn physical_adapter_failures_are_findings_only_when_enabled() {
        let mut report = super::EnvironmentReport {
            taskbar_adapter: Some("interface_table_failed"),
            ..Default::default()
        };
        let mut config = iran_split_config::AppConfig::default();
        config.behavior.show_taskbar_speed = true;
        let unavailable = |report: &super::EnvironmentReport,
                           config: &iran_split_config::AppConfig| {
            super::findings(report, Some(config))
                .iter()
                .any(|finding| finding.starts_with("taskbar_adapter_unavailable:"))
        };
        assert!(unavailable(&report, &config));
        report.taskbar_adapter = Some("no_active_physical_adapter");
        assert!(unavailable(&report, &config));
        for state in ["wifi", "ethernet", "disabled"] {
            report.taskbar_adapter = Some(state);
            assert!(!unavailable(&report, &config));
        }
        config.behavior.show_taskbar_speed = false;
        report.taskbar_adapter = Some("interface_table_failed");
        assert!(!unavailable(&report, &config));
    }
    #[test]
    fn enabled_but_unavailable_taskbar_is_a_safe_environment_finding() {
        let report = super::EnvironmentReport {
            taskbar_speed: Some("unavailable"),
            ..Default::default()
        };
        let mut config = iran_split_config::AppConfig::default();
        config.behavior.show_taskbar_speed = true;
        assert!(super::findings(&report, Some(&config))
            .iter()
            .any(|finding| finding.starts_with("taskbar_speed_unavailable:")));
        config.behavior.show_taskbar_speed = false;
        assert!(!super::findings(&report, Some(&config))
            .iter()
            .any(|finding| finding.starts_with("taskbar_speed_unavailable:")));
    }
    #[test]
    fn attachment_does_not_hide_a_broken_taskbar_surface() {
        let mut report = super::EnvironmentReport {
            taskbar_speed: Some("attached"),
            taskbar_renderer: Some("unlayered_gdi"),
            ..Default::default()
        };
        let mut config = iran_split_config::AppConfig::default();
        config.behavior.show_taskbar_speed = true;
        for renderer in ["unlayered_gdi", "clipped"] {
            report.taskbar_renderer = Some(renderer);
            assert!(super::findings(&report, Some(&config))
                .iter()
                .any(|finding| finding.starts_with("taskbar_surface_unavailable:")));
        }
        report.taskbar_renderer = Some("layered_gdi");
        assert!(!super::findings(&report, Some(&config))
            .iter()
            .any(|finding| finding.starts_with("taskbar_surface_unavailable:")));
    }
    use super::*;

    #[test]
    fn classify_host_never_returns_the_address() {
        assert_eq!(classify_host("127.0.0.1"), "loopback");
        assert_eq!(classify_host("localhost"), "loopback_name");
        assert_eq!(classify_host("[::1]"), "loopback");
        assert_eq!(classify_host("10.1.2.3"), "private");
        assert_eq!(classify_host("100.100.1.1"), "cgnat");
        assert_eq!(classify_host("198.18.0.5"), "fake_ip");
        assert_eq!(classify_host("8.8.8.8"), "public_ip");
        assert_eq!(classify_host("api.example.ir"), "domain_ir");
        assert_eq!(classify_host("k8s.example.com"), "domain");
        assert_eq!(classify_host("kubernetes"), "local_name");
        assert_eq!(classify_host("fd00::1"), "private");
    }

    #[test]
    fn parse_endpoint_handles_credentials_ipv6_and_bare_host_port() {
        assert_eq!(
            parse_endpoint("socks5h://user:pw@127.0.0.1:12334/x"),
            Some(Endpoint {
                scheme: Some("socks5h".into()),
                host: "127.0.0.1".into(),
                port: Some(12334),
            })
        );
        assert_eq!(
            parse_endpoint("[::1]:8080").map(|endpoint| (endpoint.host, endpoint.port)),
            Some(("::1".into(), Some(8080)))
        );
        assert_eq!(
            parse_endpoint("localhost:2080").map(|endpoint| endpoint.port),
            Some(Some(2080))
        );
        assert_eq!(parse_endpoint("  "), None);
    }

    #[test]
    fn env_proxies_are_classified_not_copied() {
        let settings = process_env_proxies(|name| match name {
            "HTTPS_PROXY" => Some("http://secret-user:pw@10.0.0.9:3128".into()),
            "NO_PROXY" => Some("localhost,.corp,10.0.0.0/8".into()),
            _ => None,
        });
        assert_eq!(settings.len(), 2);
        assert_eq!(settings[0].host, Some("private"));
        assert_eq!(settings[0].port, Some(3128));
        assert_eq!(settings[1].detail.as_deref(), Some("entries=3"));
        let encoded = serde_json::to_string(&settings).unwrap();
        assert!(!encoded.contains("10.0.0.9"));
        assert!(!encoded.contains("secret-user"));
        assert!(!encoded.contains(".corp"));
    }

    #[test]
    fn known_vpn_processes_ignore_unrelated_names() {
        let processes = vec![
            "Hiddify.exe".to_owned(),
            "v2rayN".to_owned(),
            "monitor".to_owned(),
            "editor".to_owned(),
            "Windscribe.exe".to_owned(),
            "chrome".to_owned(),
            "tor".to_owned(),
        ];
        assert_eq!(
            known_vpn_processes(&processes),
            vec!["hiddify", "tor", "v2rayn", "windscribe"]
        );
    }

    #[test]
    fn listeners_keep_only_relevant_ports_and_known_processes() {
        let ports = BTreeSet::from([12334]);
        let kept = relevant_listeners(
            vec![
                Listener {
                    port: 12334,
                    address: "loopback",
                    process: Some("SomethingElse".into()),
                },
                Listener {
                    port: 5432,
                    address: "unspecified",
                    process: Some("postgres".into()),
                },
                Listener {
                    port: 7777,
                    address: "loopback",
                    process: Some("xray.exe".into()),
                },
            ],
            &ports,
        );
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|listener| listener.port != 5432));
        assert!(kept
            .iter()
            .any(|listener| listener.process.as_deref() == Some("xray")));
    }

    #[test]
    fn kubeconfig_servers_are_classified_and_routed() {
        let text = r#"
apiVersion: v1
clusters:
- cluster:
    certificate-authority-data: AAAA
    server: https://10.20.30.40:6443
  name: office
- cluster:
    server: "https://k8s.example.com"
    proxy-url: socks5://127.0.0.1:1080
  name: cloud
"#;
        assert_eq!(parse_kube_auth_commands(text), Vec::<String>::new());
        assert_eq!(
            parse_kube_auth_commands(
                "users:\n- user:\n    exec:\n      command: /usr/local/bin/kubelogin\n"
            ),
            vec!["kubelogin"]
        );
        let clusters = parse_kubeconfig(text, |host| {
            Some((
                if host.starts_with("10.") {
                    "direct".into()
                } else {
                    "client:hiddify".into()
                },
                "private".into(),
            ))
        });
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].host, "private");
        assert_eq!(clusters[0].family, "ipv4");
        assert_eq!(clusters[1].family, "domain");
        assert_eq!(clusters[0].target, Some(("10.20.30.40".to_owned(), 6443)));
        assert_eq!(clusters[0].port, Some(6443));
        assert_eq!(clusters[0].route.as_deref(), Some("direct"));
        assert!(!clusters[0].proxy_url);
        assert_eq!(clusters[1].host, "domain");
        assert_eq!(clusters[1].port, Some(443));
        assert!(clusters[1].proxy_url);
        assert!(!serde_json::to_string(&clusters)
            .unwrap()
            .contains("example.com"));
    }

    #[test]
    fn hosts_file_reports_count_and_localhost_class() {
        let info =
            parse_hosts("# c\n127.0.0.1 localhost\n::1 localhost\n10.0.0.2 localhost\n1.2.3.4 a\n");
        assert_eq!(info.entries, 4);
        assert_eq!(info.localhost, vec!["loopback", "private"]);
    }

    #[test]
    fn missing_client_listener_and_port_conflicts_are_findings() {
        let config = AppConfig::default();
        let report = EnvironmentReport {
            listener_scan: true,
            listeners: vec![Listener {
                port: config.mihomo.controller_port,
                address: "loopback",
                process: Some("otherapp".into()),
            }],
            ..EnvironmentReport::default()
        };
        let found = findings(&report, Some(&config)).join("\n");
        assert!(found.contains("client_port_not_listening:hiddify:12334"));
        assert!(found.contains(&format!(
            "port_conflict:{}:otherapp",
            config.mihomo.controller_port
        )));
    }

    #[test]
    fn windscribe_gui_is_a_second_vpn_even_with_a_windscribe_client() {
        let mut config = AppConfig::default();
        config.clients.push(iran_split_config::ClientInstance {
            id: iran_split_config::ClientId::new(),
            preset: iran_split_config::PresetId::Windscribe,
            enabled: true,
            allow_direct_when_down: false,
            config: ClientConfig::from_preset(iran_split_config::PresetId::Windscribe),
        });
        let report = EnvironmentReport {
            vpn_processes: vec!["windscribe".into(), "openvpn".into(), "hiddify".into()],
            ..EnvironmentReport::default()
        };
        let found = findings(&report, Some(&config));
        assert_eq!(found, vec!["other_vpn_process:windscribe".to_owned()]);
    }

    #[test]
    fn own_side_tunnel_adapter_is_not_another_vpn() {
        let config = AppConfig::default();
        let prefix = &config.clients[0].id.as_hyphenated()[..8];
        let adapter = |name: &str| Adapter {
            name: name.into(),
            description: "tun".into(),
            status: "unknown".into(),
            kind: "tun",
        };
        let report = EnvironmentReport {
            adapters: vec![adapter(&format!("tun-{prefix}")), adapter("tailscale0")],
            ..EnvironmentReport::default()
        };
        let found = findings(&report, Some(&config));
        assert_eq!(found, vec!["other_tunnel_adapter_up:tailscale0".to_owned()]);
    }

    #[test]
    fn biflow_summary_contains_no_secrets() {
        let config = AppConfig::default();
        let info = biflow_info(Some(&config), "running");
        let encoded = serde_json::to_string(&info).unwrap();
        assert!(!encoded.contains(&config.mihomo.controller_secret));
        assert_eq!(info.default_route, "client:hiddify");
        assert_eq!(info.clients[0].host, Some("loopback"));
    }

    #[tokio::test]
    async fn live_collection_completes_on_this_host_without_leaking_config() {
        let config = AppConfig::default();
        let report = collect(CollectContext {
            app_version: "0.0.0-test".into(),
            config: Some(config.clone()),
            rules: None,
            stack: Some(StackSnapshot::default()),
            helper: Some(Err("not connected in tests".into())),
            data_dir: None,
            install_kind: "deb",
            mihomo_path: None,
            hiddify_executable: None,
            hiddify_data_dir: crate::hiddify_reset::resolve_data_dir(),
        })
        .await;
        assert_eq!(report.system.os, std::env::consts::OS);
        assert_eq!(report.biflow.clients.len(), 1);
        let encoded = serde_json::to_string(&report).expect("report serializes");
        assert!(!encoded.contains(&config.mihomo.controller_secret));
        if std::env::var_os("BIFLOW_PRINT_ENVIRONMENT").is_some() {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
        }
    }

    #[tokio::test]
    async fn loopback_self_test_connects_on_this_host() {
        let info = loopback_info(Some(&AppConfig::default())).await;
        assert!(info.ipv4.as_ref().is_some_and(|probe| probe.connected));
        assert!(info
            .localhost_resolves
            .iter()
            .all(|class| class.starts_with("loopback")));
        assert_eq!(info.clients.len(), 1);
    }

    #[test]
    fn loopback_findings_name_blocked_families_and_ipv4_only_clients() {
        let report = EnvironmentReport {
            loopback: LoopbackInfo {
                localhost_resolves: vec!["loopback_v6", "loopback_v4"],
                ipv4: Some(LoopbackProbe {
                    connected: true,
                    elapsed_ms: 1,
                }),
                ipv6: Some(LoopbackProbe {
                    connected: false,
                    elapsed_ms: 1500,
                }),
                clients: vec![
                    ClientLoopback {
                        preset: "hiddify",
                        port: 12334,
                        ipv4: true,
                        ipv6: false,
                    },
                    ClientLoopback {
                        preset: "happ",
                        port: 10808,
                        ipv4: false,
                        ipv6: false,
                    },
                ],
            },
            ..EnvironmentReport::default()
        };
        let mut found = Vec::new();
        loopback_findings(&report, &mut found);
        assert_eq!(
            found,
            vec![
                "loopback_ipv6_blocked",
                "client_ipv4_only_but_localhost_prefers_ipv6:hiddify:12334",
                "client_port_refused_on_loopback:happ:10808",
            ]
        );
    }

    #[test]
    fn own_tun_split_routes_dual_stack_defaults_and_hidden_helper_task_are_not_findings() {
        let config = AppConfig::default();
        let route = |prefix: &str, interface: &str| RouteInfo {
            prefix: prefix.into(),
            interface: interface.into(),
            gateway: "private",
            route_metric: None,
            interface_metric: None,
        };
        let mut report = EnvironmentReport {
            default_routes: vec![
                route("128.0.0.0/1", &config.mihomo.tun_name),
                route("0.0.0.0/0", "Ethernet 2"),
                route("::/0", "Ethernet 2"),
            ],
            services: vec![ServiceState {
                name: "BiFlowHelper".into(),
                state: "missing".into(),
            }],
            vpn_processes: vec!["com.docker.backend".into(), "wslservice".into()],
            ..EnvironmentReport::default()
        };
        report.runtime.helper = Some(apps::HelperSummary {
            available: true,
            authorized: true,
            version: None,
        });
        assert_eq!(findings(&report, Some(&config)), Vec::<String>::new());
        report.runtime.helper = None;
        assert_eq!(
            findings(&report, Some(&config)),
            vec!["helper_service:missing".to_owned()]
        );
    }

    #[tokio::test]
    async fn kube_probe_reports_connect_result_without_the_address() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("listener");
        let port = listener.local_addr().expect("addr").port();
        let text = format!("clusters:\n- cluster:\n    server: https://127.0.0.1:{port}\n");
        let mut clusters = parse_kubeconfig(&text, |_| None);
        probe_kube_clusters(&mut clusters).await;
        assert_eq!(clusters[0].tcp_connect, Some(true));
        drop(listener);
        let encoded = serde_json::to_string(&clusters).expect("json");
        assert!(!encoded.contains("127.0.0.1"));
        assert!(!encoded.contains("target"));
    }
}
