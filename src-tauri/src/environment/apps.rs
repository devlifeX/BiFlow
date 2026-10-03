//! What `BiFlow` depends on outside its own process: the helper, the stack
//! components, the Hiddify install the user can reconfigure behind our back,
//! the `OpenVPN` binary and `.ovpn` profiles side tunnels ride on, and the
//! clock (vmess/reality and `OpenVPN` TLS both fail on a skewed clock).

use super::{classify_host, parse_endpoint, EnvironmentReport};
use iran_split_config::{AppConfig, ClientConfig, EgressKind};
use iran_split_core::{HelperStatus, StackPhase, StackSnapshot};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeInfo {
    /// `StackSnapshot` without exit IPs; messages have addresses classified.
    pub stack: Option<Value>,
    pub helper: Option<HelperSummary>,
    pub helper_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HelperSummary {
    pub available: bool,
    pub authorized: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AppsInfo {
    pub hiddify: Option<HiddifyInfo>,
    pub mihomo: Option<BinaryInfo>,
    pub openvpn: Option<BinaryInfo>,
    pub side_tunnels: Vec<SideTunnelProfile>,
    /// `system-proxy-snapshot.json`: Pause cleared a Hiddify OS proxy and
    /// will restore it on Resume.
    pub system_proxy_saved_by_pause: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BinaryInfo {
    pub found: bool,
    pub location: &'static str,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct HiddifyInfo {
    pub data_dir_found: bool,
    pub executable: Option<BinaryInfo>,
    /// `flutter.service-mode`: `proxy`, `system-proxy`, `tun`, ...
    pub service_mode: Option<String>,
    pub region: Option<String>,
    pub started_by_user: Option<bool>,
    pub preferences_modified: Option<String>,
    pub config_modified: Option<String>,
    /// Inbounds of Hiddify's generated sing-box config: the ports it really
    /// listens on, whatever `BiFlow` was told.
    pub inbounds: Vec<HiddifyInbound>,
    pub clash_api_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HiddifyInbound {
    pub kind: String,
    pub listen: Option<&'static str>,
    pub port: Option<u16>,
    pub set_system_proxy: bool,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a log DTO of independent profile facts, not a state machine"
)]
pub struct SideTunnelProfile {
    pub preset: &'static str,
    pub enabled: bool,
    pub profile_selected: bool,
    pub profile_readable: bool,
    /// `ok`, or the reason the helper would refuse the profile.
    pub audit: Option<String>,
    pub proto: Option<String>,
    pub dev: Option<String>,
    pub remote_count: usize,
    pub remote_hosts: Vec<&'static str>,
    pub remote_port: Option<u16>,
    pub auth_user_pass: bool,
    pub credentials_saved: bool,
    pub ciphers: Vec<String>,
    pub compression: bool,
    pub inline_blocks: Vec<String>,
    /// `ca`/`cert`/`key`/`tls-auth`/... files that do not exist next to the
    /// profile. The helper resolves them relative to the original file.
    pub missing_files: usize,
    pub remote_pin_cached: bool,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct ClockInfo {
    pub utc_offset_minutes: i32,
    pub skew_seconds: Option<i64>,
    pub skew_source: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct InstallInfo {
    pub kind: &'static str,
    pub dev_profile: bool,
    pub elevated: Option<bool>,
    pub uptime_minutes: Option<u64>,
    pub locale: Option<String>,
}

// ---------------------------------------------------------------------------
// Redaction.
// ---------------------------------------------------------------------------

/// Replaces IPv4/IPv6 literals with their class and the home directory with
/// `~`, so component messages and profile directives can be logged.
pub(super) fn redact_addresses(text: &str) -> String {
    static IPV4: OnceLock<Regex> = OnceLock::new();
    static IPV6: OnceLock<Regex> = OnceLock::new();
    let ipv4 = IPV4.get_or_init(|| {
        Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("IPv4 redaction regex is valid")
    });
    // At least one `::` or five groups, so `10:17:47` timestamps survive.
    let ipv6 = IPV6.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:[0-9a-f]{1,4}:){1,7}:(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4})*)?|\b(?:[0-9a-f]{1,4}:){4,7}[0-9a-f]{1,4}\b|(?:^|[\s(\[=])::1\b",
        )
        .expect("IPv6 redaction regex is valid")
    });
    let text = ipv4.replace_all(text, |captures: &regex::Captures<'_>| {
        format!("<{}>", classify_host(&captures[0]))
    });
    let text = ipv6.replace_all(&text, |captures: &regex::Captures<'_>| {
        let matched = captures[0].trim_start_matches([' ', '\t', '(', '[', '=']);
        let prefix = &captures[0][..captures[0].len() - matched.len()];
        format!("{prefix}<{}>", classify_host(matched))
    });
    let mut text = text.into_owned();
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if home.len() > 1 {
            text = text.replace(home.as_ref(), "~");
        }
    }
    text
}

fn redact_json(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact_addresses(text),
        Value::Array(items) => items.iter_mut().for_each(redact_json),
        Value::Object(fields) => {
            fields.retain(|key, _| key != "exit_ip");
            fields.values_mut().for_each(redact_json);
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Runtime (stack + helper).
// ---------------------------------------------------------------------------

pub(super) fn runtime_info(
    stack: Option<&StackSnapshot>,
    helper: Option<&Result<HelperStatus, String>>,
) -> RuntimeInfo {
    let stack = stack.and_then(|snapshot| {
        let mut value = serde_json::to_value(snapshot).ok()?;
        redact_json(&mut value);
        Some(value)
    });
    let (helper, helper_error) = match helper {
        Some(Ok(status)) => (
            Some(HelperSummary {
                available: status.available,
                authorized: status.authorized,
                version: status.version.clone(),
            }),
            None,
        ),
        Some(Err(error)) => (None, Some(redact_addresses(error))),
        None => (None, None),
    };
    RuntimeInfo {
        stack,
        helper,
        helper_error,
    }
}

// ---------------------------------------------------------------------------
// Binaries.
// ---------------------------------------------------------------------------

pub(super) fn path_location(path: &Path, data_dir: Option<&Path>) -> &'static str {
    let text = path.to_string_lossy();
    let lower = text.to_ascii_lowercase();
    if data_dir.is_some_and(|data| path.starts_with(data)) {
        "biflow_data"
    } else if lower.ends_with(".appimage") || lower.contains("/.mount_") {
        "appimage"
    } else if lower.contains("program files") {
        "program_files"
    } else if lower.contains("\\appdata\\") || lower.contains("/appdata/") {
        "user_appdata"
    } else if lower.contains("programdata") {
        "programdata"
    } else if ["/usr/", "/opt/", "/sbin/", "/bin/"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        "system"
    } else if dirs::home_dir().is_some_and(|home| path.starts_with(home)) {
        "home"
    } else {
        "other"
    }
}

/// `OpenVPN` paths the helper searches (`iran-split-helper` `candidate_binaries`).
pub(super) fn openvpn_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        let mut paths = Vec::new();
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(variable) {
                paths.push(
                    PathBuf::from(root)
                        .join("OpenVPN")
                        .join("bin")
                        .join("openvpn.exe"),
                );
            }
        }
        paths.push(PathBuf::from(r"C:\Program Files\OpenVPN\bin\openvpn.exe"));
        paths
    } else {
        [
            "/usr/sbin/openvpn",
            "/usr/bin/openvpn",
            "/sbin/openvpn",
            "/bin/openvpn",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
}

/// First line of `--version`, with addresses and home paths removed.
pub(super) fn first_version_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| redact_addresses(&line.chars().take(160).collect::<String>()))
}

pub(super) async fn binary_info(
    path: Option<&Path>,
    version_arg: &str,
    data_dir: Option<&Path>,
) -> BinaryInfo {
    let Some(path) = path else {
        return BinaryInfo {
            found: false,
            location: "missing",
            version: None,
        };
    };
    let version =
        super::run_command_timeout(path.as_os_str(), &[version_arg], Duration::from_secs(5))
            .await
            .ok()
            .and_then(|output| first_version_line(&output));
    BinaryInfo {
        found: true,
        location: path_location(path, data_dir),
        version,
    }
}

// ---------------------------------------------------------------------------
// Hiddify.
// ---------------------------------------------------------------------------

fn modified(path: &Path) -> Option<String> {
    let time = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339())
}

/// Reads only whitelisted keys: never profiles, subscriptions, or outbounds.
pub(super) fn hiddify_info(data_dir: Option<&Path>) -> HiddifyInfo {
    let Some(dir) = data_dir else {
        return HiddifyInfo::default();
    };
    let mut info = HiddifyInfo {
        data_dir_found: true,
        ..HiddifyInfo::default()
    };
    let preferences = dir.join("shared_preferences.json");
    if let Some(value) = read_json(&preferences) {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(|text| text.chars().take(40).collect())
        };
        info.service_mode = text("flutter.service-mode");
        info.region = text("flutter.region");
        info.started_by_user = value
            .get("flutter.started_by_user")
            .and_then(Value::as_bool);
        info.preferences_modified = modified(&preferences);
    }
    let config = dir.join("data").join("current-config.json");
    if let Some(value) = read_json(&config) {
        parse_hiddify_config(&value, &mut info);
        info.config_modified = modified(&config);
    }
    info
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub(super) fn parse_hiddify_config(value: &Value, info: &mut HiddifyInfo) {
    info.inbounds = value
        .get("inbounds")
        .and_then(Value::as_array)
        .map(|inbounds| {
            inbounds
                .iter()
                .map(|inbound| HiddifyInbound {
                    kind: inbound
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .chars()
                        .take(24)
                        .collect(),
                    listen: inbound
                        .get("listen")
                        .and_then(Value::as_str)
                        .map(classify_host),
                    port: inbound
                        .get("listen_port")
                        .and_then(Value::as_u64)
                        .and_then(|port| u16::try_from(port).ok()),
                    set_system_proxy: inbound.get("set_system_proxy").and_then(Value::as_bool)
                        == Some(true),
                })
                .collect()
        })
        .unwrap_or_default();
    info.clash_api_port = value
        .pointer("/experimental/clash_api/external_controller")
        .and_then(Value::as_str)
        .and_then(parse_endpoint)
        .and_then(|endpoint| endpoint.port);
}

// ---------------------------------------------------------------------------
// OpenVPN side-tunnel profiles.
// ---------------------------------------------------------------------------

const FILE_DIRECTIVES: &[&str] = &[
    "ca",
    "cert",
    "key",
    "tls-auth",
    "tls-crypt",
    "tls-crypt-v2",
    "pkcs12",
    "auth-user-pass",
    "secret",
    "dh",
    "crl-verify",
];

pub(super) fn side_tunnel_profiles(
    config: Option<&AppConfig>,
    data_dir: Option<&Path>,
) -> Vec<SideTunnelProfile> {
    let Some(config) = config else {
        return Vec::new();
    };
    let pins = data_dir
        .and_then(|dir| std::fs::read_to_string(dir.join("side-tunnel-remotes.json")).ok())
        .unwrap_or_default();
    config
        .clients
        .iter()
        .filter(|client| client.spec().kind == EgressKind::OwnedSideTunnel)
        .map(|client| {
            let ClientConfig::OwnedSideTunnel {
                profile_path,
                username,
                password,
                ..
            } = &client.config
            else {
                return SideTunnelProfile {
                    preset: client.spec().id,
                    enabled: client.enabled,
                    ..SideTunnelProfile::default()
                };
            };
            let mut profile = profile_path
                .as_deref()
                .map(|path| inspect_profile(path, &pins))
                .unwrap_or_default();
            profile.preset = client.spec().id;
            profile.enabled = client.enabled;
            profile.profile_selected = profile_path.is_some();
            profile.credentials_saved = username.as_deref().is_some_and(|name| !name.is_empty())
                && password.as_deref().is_some_and(|secret| !secret.is_empty());
            profile
        })
        .collect()
}

fn inspect_profile(path: &Path, pins: &str) -> SideTunnelProfile {
    let audit = match iran_split_clients::audit_openvpn_profile(path) {
        Ok(_) => "ok".to_owned(),
        Err(error) => error.to_string(),
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return SideTunnelProfile {
            audit: Some(audit),
            ..SideTunnelProfile::default()
        };
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut profile = parse_ovpn(&text, |file| {
        base.join(file).exists() || Path::new(file).exists()
    });
    profile.profile_readable = true;
    profile.audit = Some(audit);
    profile.remote_pin_cached = remote_hosts(&text)
        .iter()
        .any(|host| pins.contains(&format!("\"{host}\"")));
    profile
}

fn remote_hosts(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next()? == "remote").then(|| fields.next().map(str::to_owned))?
        })
        .collect()
}

/// Summarizes directives that decide whether `OpenVPN` can start. Remote
/// hosts are classified; key material and paths are never copied.
pub(super) fn parse_ovpn(text: &str, file_exists: impl Fn(&str) -> bool) -> SideTunnelProfile {
    let mut profile = SideTunnelProfile::default();
    let mut inline: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(open) = inline.as_deref() {
            if line == format!("</{open}>") {
                inline = None;
            }
            continue;
        }
        if line.starts_with('#') || line.starts_with(';') || line.is_empty() {
            continue;
        }
        if let Some(tag) = line
            .strip_prefix('<')
            .and_then(|rest| rest.strip_suffix('>'))
            .filter(|tag| !tag.starts_with('/'))
        {
            profile.inline_blocks.push(tag.chars().take(24).collect());
            inline = Some(tag.to_owned());
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(directive) = fields.next() else {
            continue;
        };
        let arguments: Vec<&str> = fields.collect();
        match directive {
            "proto" => profile.proto = arguments.first().map(|value| (*value).to_owned()),
            "dev" => profile.dev = arguments.first().map(|value| (*value).to_owned()),
            "remote" => {
                profile.remote_count += 1;
                if let Some(host) = arguments.first() {
                    let class = classify_host(host);
                    if !profile.remote_hosts.contains(&class) {
                        profile.remote_hosts.push(class);
                    }
                }
                if profile.remote_port.is_none() {
                    profile.remote_port = arguments.get(1).and_then(|port| port.parse().ok());
                }
                if let Some(proto) = arguments.get(2) {
                    profile.proto.get_or_insert_with(|| (*proto).to_owned());
                }
            }
            "port" => {
                profile.remote_port = profile
                    .remote_port
                    .or_else(|| arguments.first().and_then(|port| port.parse().ok()));
            }
            "cipher" | "data-ciphers" | "ncp-ciphers" | "data-ciphers-fallback" => {
                profile.ciphers.push(
                    format!("{directive} {}", arguments.join(" "))
                        .chars()
                        .take(80)
                        .collect(),
                );
            }
            "compress" | "comp-lzo" => profile.compression = true,
            _ => {}
        }
        if directive == "auth-user-pass" {
            profile.auth_user_pass = true;
        }
        if FILE_DIRECTIVES.contains(&directive) {
            if let Some(file) = arguments.first() {
                if !file_exists(file.trim_matches('"')) {
                    profile.missing_files += 1;
                }
            }
        }
    }
    profile
}

// ---------------------------------------------------------------------------
// Clock and install.
// ---------------------------------------------------------------------------

/// Compares the local clock with an HTTP `Date` header. Only the skew and
/// which source answered are logged.
pub(super) async fn clock_info() -> ClockInfo {
    let mut info = ClockInfo {
        utc_offset_minutes: chrono::Local::now().offset().local_minus_utc() / 60,
        ..ClockInfo::default()
    };
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(4))
        .build()
    else {
        return info;
    };
    for (name, url) in [
        ("gstatic", "http://www.gstatic.com/generate_204"),
        (
            "msftconnecttest",
            "http://www.msftconnecttest.com/connecttest.txt",
        ),
    ] {
        let before = chrono::Utc::now();
        let Ok(response) = client.head(url).send().await else {
            continue;
        };
        let Some(date) = response
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok())
        else {
            continue;
        };
        let after = chrono::Utc::now();
        let midpoint = before + (after - before) / 2;
        info.skew_seconds = Some((midpoint - date.with_timezone(&chrono::Utc)).num_seconds());
        info.skew_source = Some(name);
        break;
    }
    info
}

pub(super) fn install_info(kind: &'static str) -> InstallInfo {
    InstallInfo {
        kind,
        dev_profile: crate::profile::is_development(),
        elevated: None,
        uptime_minutes: None,
        locale: std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .ok()
            .filter(|value| !value.is_empty()),
    }
}

// ---------------------------------------------------------------------------
// Findings.
// ---------------------------------------------------------------------------

fn phase_is(report: &EnvironmentReport, phases: &[StackPhase]) -> bool {
    let phase = report.biflow.stack_phase.as_str();
    phases
        .iter()
        .any(|candidate| format!("{candidate:?}").eq_ignore_ascii_case(phase))
}

pub(super) fn app_findings(
    report: &EnvironmentReport,
    config: Option<&AppConfig>,
    findings: &mut Vec<String>,
) {
    runtime_findings(report, findings);
    hiddify_findings(report, config, findings);
    side_tunnel_findings(report, findings);
    if let Some(skew) = report.clock.skew_seconds {
        if skew.abs() > 90 {
            findings.push(format!("clock_skew_seconds:{skew}"));
        }
    }
    if report.install.elevated == Some(true) {
        findings.push("app_running_elevated".into());
    }
    if report
        .apps
        .mihomo
        .as_ref()
        .is_some_and(|mihomo| !mihomo.found)
    {
        findings.push("mihomo_binary_missing".into());
    }
    if report.apps.system_proxy_saved_by_pause
        && phase_is(report, &[StackPhase::Paused, StackPhase::Stopped])
    {
        findings.push("pause_cleared_hiddify_system_proxy".into());
    }
}

fn runtime_findings(report: &EnvironmentReport, findings: &mut Vec<String>) {
    match &report.runtime.helper {
        Some(helper) if !helper.available => findings.push("helper_unavailable".into()),
        Some(helper) if !helper.authorized => findings.push("helper_unauthorized".into()),
        Some(helper) => {
            if let Some(version) = &helper.version {
                if version != &report.system.app_version {
                    findings.push(format!(
                        "helper_version_mismatch:helper={version}:app={}",
                        report.system.app_version
                    ));
                }
            }
        }
        None if report.runtime.helper_error.is_some() => {
            findings.push("helper_unreachable".into());
        }
        None => {}
    }
    let Some(stack) = &report.runtime.stack else {
        return;
    };
    if let Some(code) = stack.pointer("/last_error/code").and_then(Value::as_str) {
        findings.push(format!("stack_last_error:{code}"));
    }
    for client in stack
        .get("clients")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let phase = client
            .pointer("/status/phase")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let enabled = client.get("enabled").and_then(Value::as_bool) == Some(true);
        if enabled && matches!(phase, "error" | "unavailable" | "degraded") {
            findings.push(format!(
                "client_status:{}:{phase}",
                client
                    .get("preset")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ));
        }
    }
}

fn hiddify_findings(
    report: &EnvironmentReport,
    config: Option<&AppConfig>,
    findings: &mut Vec<String>,
) {
    let Some(hiddify) = &report.apps.hiddify else {
        return;
    };
    let configured = config.and_then(|config| {
        config.enabled_clients().into_iter().find_map(|client| {
            match (&client.preset, &client.config) {
                (iran_split_config::PresetId::Hiddify, ClientConfig::LocalProxy { port, .. }) => {
                    Some(*port)
                }
                _ => None,
            }
        })
    });
    let Some(configured) = configured else {
        return;
    };
    if matches!(hiddify.service_mode.as_deref(), Some("tun" | "vpn"))
        || hiddify.inbounds.iter().any(|inbound| inbound.kind == "tun")
    {
        findings.push("hiddify_tun_mode_conflicts_with_biflow_tun".into());
    }
    let mixed: Vec<u16> = hiddify
        .inbounds
        .iter()
        .filter(|inbound| matches!(inbound.kind.as_str(), "mixed" | "socks" | "http"))
        .filter_map(|inbound| inbound.port)
        .collect();
    if !mixed.is_empty() && !mixed.contains(&configured) {
        findings.push(format!(
            "hiddify_port_mismatch:biflow={configured}:hiddify={}",
            mixed[0]
        ));
    }
    if hiddify.inbounds.iter().any(|inbound| {
        inbound
            .listen
            .is_some_and(|listen| !matches!(listen, "loopback" | "loopback_name"))
    }) {
        findings.push("hiddify_listens_beyond_loopback".into());
    }
    if hiddify
        .executable
        .as_ref()
        .is_some_and(|executable| !executable.found)
    {
        findings.push("hiddify_executable_not_found".into());
    }
}

fn side_tunnel_findings(report: &EnvironmentReport, findings: &mut Vec<String>) {
    let mut openvpn_needed = false;
    for profile in report
        .apps
        .side_tunnels
        .iter()
        .filter(|profile| profile.enabled)
    {
        openvpn_needed = true;
        let preset = profile.preset;
        if !profile.profile_selected {
            findings.push(format!("side_tunnel_no_profile:{preset}"));
            continue;
        }
        if !profile.profile_readable {
            findings.push(format!("side_tunnel_profile_unreadable:{preset}"));
            continue;
        }
        if let Some(audit) = profile.audit.as_deref().filter(|audit| *audit != "ok") {
            findings.push(format!("side_tunnel_profile_rejected:{preset}:{audit}"));
        }
        if profile.missing_files > 0 {
            findings.push(format!(
                "side_tunnel_profile_missing_files:{preset}:{}",
                profile.missing_files
            ));
        }
        if profile.auth_user_pass && !profile.credentials_saved {
            findings.push(format!("side_tunnel_credentials_missing:{preset}"));
        }
        if profile.remote_count == 0 {
            findings.push(format!("side_tunnel_no_remote:{preset}"));
        }
        if profile
            .remote_hosts
            .iter()
            .any(|host| host.starts_with("domain"))
            && !profile.remote_pin_cached
        {
            findings.push(format!("side_tunnel_remote_not_pinned:{preset}"));
        }
        if cfg!(windows)
            && profile
                .dev
                .as_deref()
                .is_some_and(|dev| dev.starts_with("tap"))
            && !report.adapters.iter().any(|adapter| adapter.kind == "tap")
        {
            findings.push(format!("side_tunnel_tap_without_adapter:{preset}"));
        }
    }
    if openvpn_needed
        && report
            .apps
            .openvpn
            .as_ref()
            .is_some_and(|openvpn| !openvpn.found)
    {
        findings.push("openvpn_binary_missing".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_addresses_keeps_timestamps_and_classifies_ips() {
        let text = "at 10:17:47 dial 192.168.1.5:443 via fe80::1 and ::1 failed, 8.8.8.8";
        let redacted = redact_addresses(text);
        assert!(redacted.contains("10:17:47"));
        assert!(redacted.contains("<private>:443"));
        assert!(redacted.contains("<link_local>"));
        assert!(redacted.contains("<loopback>"));
        assert!(redacted.contains("<public_ip>"));
        assert!(!redacted.contains("192.168"));
        assert!(!redacted.contains("8.8.8.8"));
    }

    #[test]
    fn stack_snapshot_drops_exit_ips_and_redacts_messages() {
        let mut snapshot = StackSnapshot {
            exit_ip: Some("203.0.113.9".into()),
            ..StackSnapshot::default()
        };
        snapshot.helper.message = Some("connect 10.0.0.2 refused".into());
        let info = runtime_info(
            Some(&snapshot),
            Some(&Ok(HelperStatus {
                available: true,
                authorized: true,
                version: Some("6.2.0".into()),
            })),
        );
        let encoded = serde_json::to_string(&info).unwrap();
        assert!(!encoded.contains("203.0.113.9"));
        assert!(!encoded.contains("exit_ip"));
        assert!(encoded.contains("<private>"));
    }

    #[test]
    fn hiddify_config_reports_real_ports_and_tun_inbound() {
        let value = serde_json::json!({
            "inbounds": [
                {"type": "mixed", "listen": "127.0.0.1", "listen_port": 2334, "set_system_proxy": true},
                {"type": "tun", "interface_name": "tun0"}
            ],
            "outbounds": [{"type": "vless", "server": "secret.example.com", "uuid": "abc"}],
            "experimental": {"clash_api": {"external_controller": "127.0.0.1:16756", "secret": "s3cr3t"}}
        });
        let mut info = HiddifyInfo::default();
        parse_hiddify_config(&value, &mut info);
        assert_eq!(info.inbounds.len(), 2);
        assert_eq!(info.inbounds[0].port, Some(2334));
        assert!(info.inbounds[0].set_system_proxy);
        assert_eq!(info.clash_api_port, Some(16756));
        let encoded = serde_json::to_string(&info).unwrap();
        assert!(!encoded.contains("secret.example.com"));
        assert!(!encoded.contains("s3cr3t"));

        let report = EnvironmentReport {
            apps: AppsInfo {
                hiddify: Some(info),
                ..AppsInfo::default()
            },
            ..EnvironmentReport::default()
        };
        let mut found = Vec::new();
        hiddify_findings(&report, Some(&AppConfig::default()), &mut found);
        assert!(found.contains(&"hiddify_tun_mode_conflicts_with_biflow_tun".to_owned()));
        assert!(found.contains(&"hiddify_port_mismatch:biflow=12334:hiddify=2334".to_owned()));
    }

    #[test]
    fn ovpn_summary_classifies_remotes_and_counts_missing_files() {
        let text = "client\ndev tun\nproto udp\nremote ber-449.example.com 443\nremote 185.1.2.3 1194 tcp\n\
                    auth-user-pass\nca ca.crt\ncert missing.crt\n<key>\nSECRET\n</key>\n\
                    data-ciphers AES-256-GCM:AES-128-GCM\ncomp-lzo\n";
        let profile = parse_ovpn(text, |file| file == "ca.crt");
        assert_eq!(profile.proto.as_deref(), Some("udp"));
        assert_eq!(profile.dev.as_deref(), Some("tun"));
        assert_eq!(profile.remote_count, 2);
        assert_eq!(profile.remote_hosts, vec!["domain", "public_ip"]);
        assert_eq!(profile.remote_port, Some(443));
        assert!(profile.auth_user_pass);
        assert_eq!(profile.missing_files, 1);
        assert_eq!(profile.inline_blocks, vec!["key"]);
        assert!(profile.compression);
        let encoded = serde_json::to_string(&profile).unwrap();
        assert!(!encoded.contains("SECRET"));
        assert!(!encoded.contains("example.com"));
        assert!(!encoded.contains("185.1.2.3"));
    }

    #[test]
    fn side_tunnel_findings_explain_why_windscribe_cannot_start() {
        let report = EnvironmentReport {
            apps: AppsInfo {
                openvpn: Some(BinaryInfo {
                    found: false,
                    location: "missing",
                    version: None,
                }),
                side_tunnels: vec![SideTunnelProfile {
                    preset: "windscribe",
                    enabled: true,
                    profile_selected: true,
                    profile_readable: true,
                    audit: Some("ok".into()),
                    remote_count: 1,
                    remote_hosts: vec!["domain"],
                    auth_user_pass: true,
                    missing_files: 2,
                    ..SideTunnelProfile::default()
                }],
                ..AppsInfo::default()
            },
            ..EnvironmentReport::default()
        };
        let mut found = Vec::new();
        side_tunnel_findings(&report, &mut found);
        assert_eq!(
            found,
            vec![
                "side_tunnel_profile_missing_files:windscribe:2",
                "side_tunnel_credentials_missing:windscribe",
                "side_tunnel_remote_not_pinned:windscribe",
                "openvpn_binary_missing",
            ]
        );
    }

    #[test]
    fn helper_version_mismatch_and_last_error_are_findings() {
        let mut report = EnvironmentReport::default();
        report.system.app_version = "6.2.48".into();
        report.runtime = RuntimeInfo {
            stack: Some(serde_json::json!({
                "last_error": {"code": "controller_timeout"},
                "clients": [{"preset": "windscribe", "enabled": true, "status": {"phase": "error"}}]
            })),
            helper: Some(HelperSummary {
                available: true,
                authorized: true,
                version: Some("6.2.40".into()),
            }),
            helper_error: None,
        };
        let mut found = Vec::new();
        runtime_findings(&report, &mut found);
        assert_eq!(
            found,
            vec![
                "helper_version_mismatch:helper=6.2.40:app=6.2.48",
                "stack_last_error:controller_timeout",
                "client_status:windscribe:error",
            ]
        );
    }

    #[test]
    fn path_location_never_returns_the_path() {
        assert_eq!(
            path_location(Path::new("/usr/sbin/openvpn"), None),
            "system"
        );
        assert_eq!(
            path_location(Path::new("/data/bin/mihomo"), Some(Path::new("/data"))),
            "biflow_data"
        );
        assert_eq!(
            path_location(
                Path::new("C:\\Program Files\\OpenVPN\\bin\\openvpn.exe"),
                None
            ),
            "program_files"
        );
    }
}
