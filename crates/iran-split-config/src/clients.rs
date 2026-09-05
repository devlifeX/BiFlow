use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::{issue, ExecutableSetting, ValidationIssue};

/// Stable identifier for one user-added client instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClientId(Uuid);

impl ClientId {
    /// Allocates a new random instance id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parses a hyphenated lowercase UUID.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is not a UUID.
    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Ok(Self(value.parse()?))
    }

    /// Hyphenated lowercase form used in YAML group names and provider files.
    #[must_use]
    pub fn as_hyphenated(&self) -> String {
        self.0.as_hyphenated().to_string()
    }

    /// Whether `value` matches the helper allowlist token `[0-9a-f-]{36}`.
    #[must_use]
    pub fn is_allowlist_token(value: &str) -> bool {
        value.len() == 36
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
            && value.bytes().any(|byte| byte.is_ascii_hexdigit())
    }
}

impl Default for ClientId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ClientId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.as_hyphenated())
    }
}

impl From<Uuid> for ClientId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<ClientId> for Uuid {
    fn from(value: ClientId) -> Self {
        value.0
    }
}

/// Shipped catalog identifier. New products are a row here, not a new outbound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetId {
    Hiddify,
    Openvpn,
    Happ,
    V2rayn,
    Nekoray,
    Shadowsocks,
    Wireguard,
    Windscribe,
}

impl PresetId {
    /// Catalog row for this identifier.
    #[must_use]
    pub const fn spec(self) -> PresetSpec {
        match self {
            Self::Hiddify => HIDDIFY_SPEC,
            Self::Openvpn => OPENVPN_SPEC,
            Self::Happ => HAPP_SPEC,
            Self::V2rayn => V2RAYN_SPEC,
            Self::Nekoray => NEKORAY_SPEC,
            Self::Shadowsocks => SHADOWSOCKS_SPEC,
            Self::Wireguard => WIREGUARD_SPEC,
            Self::Windscribe => WINDSCRIBE_SPEC,
        }
    }

    /// Every shipped catalog row, in display order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Hiddify,
            Self::Openvpn,
            Self::Happ,
            Self::V2rayn,
            Self::Nekoray,
            Self::Shadowsocks,
            Self::Wireguard,
            Self::Windscribe,
        ]
    }
}

impl std::fmt::Display for PresetId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.spec().id)
    }
}

/// How unmatched traffic leaves Mihomo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DefaultRoute {
    Direct,
    Client { client_id: ClientId },
}

impl DefaultRoute {
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

/// Stable egress kind. New products reuse one of these; they never add a third.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressKind {
    LocalProxy,
    OwnedSideTunnel,
    Unsupported,
}

/// Whether a catalog row can be started in this version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetStatus {
    Working,
    Catalog,
    Unsupported,
}

/// Official vendor download pages, one per supported platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetDownloads {
    pub linux: &'static str,
    pub windows: &'static str,
}

/// One shipped catalog row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetSpec {
    pub id: &'static str,
    pub preset: PresetId,
    pub kind: EgressKind,
    pub status: PresetStatus,
    pub default_host: &'static str,
    pub default_port: Option<u16>,
    pub linux_bypass: &'static [&'static str],
    pub windows_bypass: &'static [&'static str],
    pub install_hint: &'static str,
    pub downloads: PresetDownloads,
}

const HIDDIFY_SPEC: PresetSpec = PresetSpec {
    id: "hiddify",
    preset: PresetId::Hiddify,
    kind: EgressKind::LocalProxy,
    status: PresetStatus::Working,
    default_host: "127.0.0.1",
    default_port: Some(12_334),
    linux_bypass: &["hiddify", "*Hiddify*"],
    windows_bypass: &["hiddify.exe", "Hiddify.exe", "HiddifyNext.exe", "*Hiddify*"],
    install_hint: "Install Hiddify Next and keep its mixed port on loopback.",
    downloads: PresetDownloads {
        linux: "https://github.com/hiddify/hiddify-app/releases/latest",
        windows: "https://github.com/hiddify/hiddify-app/releases/latest",
    },
};

const OPENVPN_SPEC: PresetSpec = PresetSpec {
    id: "openvpn",
    preset: PresetId::Openvpn,
    kind: EgressKind::OwnedSideTunnel,
    status: PresetStatus::Working,
    default_host: "",
    default_port: None,
    linux_bypass: &[],
    windows_bypass: &[],
    install_hint:
        "Install OpenVPN and choose a .ovpn profile. BiFlow never lets it take the default route.",
    downloads: PresetDownloads {
        linux: "https://openvpn.net/community-downloads/",
        windows: "https://openvpn.net/community-downloads/",
    },
};

const HAPP_SPEC: PresetSpec = PresetSpec {
    id: "happ",
    preset: PresetId::Happ,
    kind: EgressKind::LocalProxy,
    status: PresetStatus::Working,
    default_host: "127.0.0.1",
    // Happ runs an Xray core; its local SOCKS listener defaults to 10808.
    default_port: Some(10_808),
    linux_bypass: &["Happ", "*Happ*", "sing-box", "xray", "v2ray"],
    windows_bypass: &[
        "Happ.exe",
        "*Happ*",
        "sing-box.exe",
        "xray.exe",
        "v2ray.exe",
    ],
    install_hint: "Run Happ and expose a local SOCKS or mixed port (default 10808).",
    downloads: PresetDownloads {
        linux: "https://www.happ.su/main/download",
        windows: "https://www.happ.su/main/download",
    },
};

const V2RAYN_SPEC: PresetSpec = PresetSpec {
    id: "v2rayn",
    preset: PresetId::V2rayn,
    kind: EgressKind::LocalProxy,
    status: PresetStatus::Working,
    default_host: "127.0.0.1",
    default_port: Some(10_808),
    linux_bypass: &["v2rayN", "v2rayn", "xray", "v2ray"],
    windows_bypass: &["v2rayN.exe", "v2rayn.exe", "xray.exe", "v2ray.exe"],
    install_hint: "Run v2rayN and keep the local SOCKS port (default 10808) on loopback.",
    downloads: PresetDownloads {
        linux: "https://github.com/2dust/v2rayN/releases/latest",
        windows: "https://github.com/2dust/v2rayN/releases/latest",
    },
};

const NEKORAY_SPEC: PresetSpec = PresetSpec {
    id: "nekoray",
    preset: PresetId::Nekoray,
    kind: EgressKind::LocalProxy,
    status: PresetStatus::Working,
    default_host: "127.0.0.1",
    default_port: Some(2080),
    linux_bypass: &["nekoray", "nekobox", "nekobox_core"],
    windows_bypass: &["nekoray.exe", "nekobox.exe", "nekobox_core.exe"],
    install_hint: "Run Nekoray / NekoBox and expose its mixed SOCKS port.",
    downloads: PresetDownloads {
        linux: "https://github.com/MatsuriDayo/nekoray/releases/latest",
        windows: "https://github.com/MatsuriDayo/nekoray/releases/latest",
    },
};

const SHADOWSOCKS_SPEC: PresetSpec = PresetSpec {
    id: "shadowsocks",
    preset: PresetId::Shadowsocks,
    kind: EgressKind::LocalProxy,
    status: PresetStatus::Working,
    default_host: "127.0.0.1",
    default_port: Some(1080),
    linux_bypass: &["ss-local", "shadowsocks", "sslocal"],
    windows_bypass: &["ss-local.exe", "shadowsocks.exe", "sslocal.exe"],
    install_hint: "Run a local Shadowsocks client and point BiFlow at its SOCKS port.",
    downloads: PresetDownloads {
        linux: "https://github.com/shadowsocks/shadowsocks-rust/releases/latest",
        windows: "https://github.com/shadowsocks/shadowsocks-windows/releases/latest",
    },
};

const WIREGUARD_SPEC: PresetSpec = PresetSpec {
    id: "wireguard",
    preset: PresetId::Wireguard,
    kind: EgressKind::OwnedSideTunnel,
    status: PresetStatus::Catalog,
    default_host: "",
    default_port: None,
    linux_bypass: &[],
    windows_bypass: &[],
    install_hint: "WireGuard will use the same side-tunnel driver as OpenVPN. The driver is not in this version.",
    downloads: PresetDownloads {
        linux: "https://www.wireguard.com/install/",
        windows: "https://www.wireguard.com/install/",
    },
};

const WINDSCRIBE_SPEC: PresetSpec = PresetSpec {
    id: "windscribe",
    preset: PresetId::Windscribe,
    kind: EgressKind::OwnedSideTunnel,
    status: PresetStatus::Working,
    default_host: "",
    default_port: None,
    linux_bypass: &[],
    windows_bypass: &[],
    install_hint: "Generate an OpenVPN profile with your Windscribe service credentials at build.windscribe.com and choose the .ovpn here. Do not run the Windscribe GUI at the same time.",
    downloads: PresetDownloads {
        linux: "https://windscribe.com/getconfig/openvpn",
        windows: "https://windscribe.com/getconfig/openvpn",
    },
};

/// One user-added instance of a catalog preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInstance {
    pub id: ClientId,
    pub preset: PresetId,
    pub enabled: bool,
    /// Per-client fail-closed exclusion: when this client is down, let its
    /// traffic fall back to DIRECT instead of REJECT (accepts the IP leak).
    #[serde(default)]
    pub allow_direct_when_down: bool,
    pub config: ClientConfig,
}

impl ClientInstance {
    /// Default first-run Hiddify instance so Connect still behaves as today.
    #[must_use]
    pub fn hiddify_default(id: ClientId) -> Self {
        Self {
            id,
            preset: PresetId::Hiddify,
            enabled: true,
            allow_direct_when_down: false,
            config: ClientConfig::local_proxy_default(PresetId::Hiddify),
        }
    }

    /// Catalog-driven defaults for a newly added preset.
    #[must_use]
    pub fn from_preset(preset: PresetId) -> Self {
        Self {
            id: ClientId::new(),
            preset,
            enabled: true,
            allow_direct_when_down: false,
            config: ClientConfig::from_preset(preset),
        }
    }

    #[must_use]
    pub const fn spec(&self) -> PresetSpec {
        self.preset.spec()
    }

    #[must_use]
    pub fn sanitized_id(&self) -> String {
        self.id.as_hyphenated()
    }

    /// Mihomo group name derived from the instance id, never the UI label.
    #[must_use]
    pub fn group_name(&self) -> String {
        format!("client-{}", self.id.as_hyphenated())
    }

    /// Mihomo proxy name derived from the instance id.
    #[must_use]
    pub fn proxy_name(&self) -> String {
        format!("proxy-{}", self.id.as_hyphenated())
    }

    /// Rule-provider file names accepted by the helper allowlist.
    #[must_use]
    pub fn provider_files(&self) -> [String; 2] {
        let id = self.id.as_hyphenated();
        [
            format!("custom-{id}-domains.txt"),
            format!("custom-{id}-ips.txt"),
        ]
    }
}

/// Tagged by egress kind, not by product name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientConfig {
    LocalProxy {
        host: String,
        port: u16,
        executable: ExecutableSetting,
        start_timeout_seconds: u64,
        stop_with_stack: bool,
    },
    OwnedSideTunnel {
        profile_path: Option<PathBuf>,
        executable: ExecutableSetting,
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
        start_timeout_seconds: u64,
    },
    Unsupported,
}

impl ClientConfig {
    #[must_use]
    pub fn from_preset(preset: PresetId) -> Self {
        let spec = preset.spec();
        match spec.kind {
            EgressKind::LocalProxy => Self::local_proxy_default(preset),
            EgressKind::OwnedSideTunnel => Self::OwnedSideTunnel {
                profile_path: None,
                executable: ExecutableSetting::Auto,
                username: None,
                password: None,
                start_timeout_seconds: 45,
            },
            EgressKind::Unsupported => Self::Unsupported,
        }
    }

    #[must_use]
    pub fn local_proxy_default(preset: PresetId) -> Self {
        let spec = preset.spec();
        Self::LocalProxy {
            host: spec.default_host.into(),
            port: spec.default_port.unwrap_or(1080),
            executable: ExecutableSetting::Auto,
            start_timeout_seconds: 45,
            stop_with_stack: preset == PresetId::Hiddify,
        }
    }

    #[must_use]
    pub fn redacted(&self) -> Self {
        match self {
            Self::LocalProxy {
                host,
                port,
                executable,
                start_timeout_seconds,
                stop_with_stack,
            } => {
                let executable = match executable {
                    ExecutableSetting::Path(path) => ExecutableSetting::Path(
                        path.file_name().map_or_else(|| path.clone(), PathBuf::from),
                    ),
                    ExecutableSetting::Auto => ExecutableSetting::Auto,
                };
                Self::LocalProxy {
                    host: host.clone(),
                    port: *port,
                    executable,
                    start_timeout_seconds: *start_timeout_seconds,
                    stop_with_stack: *stop_with_stack,
                }
            }
            Self::OwnedSideTunnel {
                profile_path,
                executable,
                username,
                start_timeout_seconds,
                ..
            } => {
                let executable = match executable {
                    ExecutableSetting::Path(path) => ExecutableSetting::Path(
                        path.file_name().map_or_else(|| path.clone(), PathBuf::from),
                    ),
                    ExecutableSetting::Auto => ExecutableSetting::Auto,
                };
                let profile_path = profile_path
                    .as_ref()
                    .map(|path| path.file_name().map_or_else(|| path.clone(), PathBuf::from));
                Self::OwnedSideTunnel {
                    profile_path,
                    executable,
                    username: username.clone(),
                    password: Some("[REDACTED]".into()),
                    start_timeout_seconds: *start_timeout_seconds,
                }
            }
            Self::Unsupported => Self::Unsupported,
        }
    }
}

/// Whether `name` is a per-client rule-provider file the helper may copy.
#[must_use]
pub fn is_custom_client_generation_file(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("custom-") else {
        return false;
    };
    let id = rest
        .strip_suffix("-domains.txt")
        .or_else(|| rest.strip_suffix("-ips.txt"));
    id.is_some_and(ClientId::is_allowlist_token)
}

pub(crate) fn validate_clients(
    clients: &[ClientInstance],
    default_route: DefaultRoute,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut seen = std::collections::HashSet::new();
    for (index, client) in clients.iter().enumerate() {
        let field = format!("clients.{index}");
        if !seen.insert(client.preset) {
            issues.push(issue(
                &field,
                "DUPLICATE_PRESET",
                "v1 allows one instance per catalog preset",
            ));
        }
        if client.preset.spec().status == PresetStatus::Unsupported && client.enabled {
            issues.push(issue(
                &field,
                "UNSUPPORTED_PRESET",
                "this catalog entry cannot be enabled",
            ));
        }
        if client.preset.spec().status == PresetStatus::Catalog && client.enabled {
            issues.push(issue(
                &field,
                "DRIVER_UNAVAILABLE",
                "this catalog entry has no driver in this version",
            ));
        }
        match &client.config {
            ClientConfig::LocalProxy {
                host,
                port,
                start_timeout_seconds,
                ..
            } => {
                crate::validate_loopback(&format!("{field}.host"), host, issues);
                if *port == 0 {
                    issues.push(issue(
                        &format!("{field}.port"),
                        "INVALID_PORT",
                        "port cannot be zero",
                    ));
                }
                if *start_timeout_seconds == 0 || *start_timeout_seconds > 300 {
                    issues.push(issue(
                        &format!("{field}.start_timeout_seconds"),
                        "OUT_OF_RANGE",
                        "start timeout must be between 1 and 300 seconds",
                    ));
                }
            }
            ClientConfig::OwnedSideTunnel {
                start_timeout_seconds,
                ..
            } => {
                if *start_timeout_seconds == 0 || *start_timeout_seconds > 300 {
                    issues.push(issue(
                        &format!("{field}.start_timeout_seconds"),
                        "OUT_OF_RANGE",
                        "start timeout must be between 1 and 300 seconds",
                    ));
                }
            }
            ClientConfig::Unsupported => {}
        }
    }
    if let DefaultRoute::Client { client_id } = default_route {
        if !clients.iter().any(|client| client.id == client_id) {
            issues.push(issue(
                "default_route",
                "UNKNOWN_CLIENT",
                "MATCH default refers to a client that is not in the registry",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_allowlist_matches_hyphenated_uuid() {
        let id = ClientId::parse("11111111-1111-1111-1111-111111111111").expect("uuid");
        assert!(ClientId::is_allowlist_token(&id.as_hyphenated()));
        assert!(!ClientId::is_allowlist_token("../evil"));
        assert!(!ClientId::is_allowlist_token("short"));
        assert!(is_custom_client_generation_file(&format!(
            "custom-{}-domains.txt",
            id.as_hyphenated()
        )));
        assert!(is_custom_client_generation_file(&format!(
            "custom-{}-ips.txt",
            id.as_hyphenated()
        )));
        assert!(!is_custom_client_generation_file(
            "custom-not-a-uuid-domains.txt"
        ));
        assert!(!is_custom_client_generation_file("../custom-x-domains.txt"));
    }

    #[test]
    fn catalog_exposes_every_planned_preset() {
        assert_eq!(PresetId::all().len(), 8);
        assert_eq!(PresetId::Hiddify.spec().kind, EgressKind::LocalProxy);
        assert_eq!(PresetId::Hiddify.spec().status, PresetStatus::Working);
        assert_eq!(PresetId::Openvpn.spec().kind, EgressKind::OwnedSideTunnel);
        assert_eq!(PresetId::Openvpn.spec().status, PresetStatus::Working);
        assert_eq!(PresetId::Wireguard.spec().status, PresetStatus::Catalog);
        assert_eq!(PresetId::Windscribe.spec().status, PresetStatus::Working);
        assert_eq!(
            PresetId::Windscribe.spec().kind,
            EgressKind::OwnedSideTunnel
        );
    }
}
