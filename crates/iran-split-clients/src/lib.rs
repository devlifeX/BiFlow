//! Client drivers: one trait, two egress kinds, a shipped catalog of presets.
//!
//! Products are catalog rows. They are never new [`iran_split_config::DefaultRoute`]
//! or outbound variants.

mod local_proxy;
mod openvpn;
mod profile_audit;

use async_trait::async_trait;
use ipnet::IpNet;
use iran_split_config::{ClientConfig, ClientId, ClientInstance, EgressKind, PresetId, PresetSpec};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub use local_proxy::LocalProxyDriver;
pub use openvpn::OpenVpnDriver;
pub use profile_audit::{
    audit_openvpn_profile, openvpn_arguments, OpenVpnProfileError, OpenVpnProfileFacts,
};

/// Process-name DIRECT rule so TUN cannot recurse into a local proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessBypass {
    pub name: String,
    pub wildcard: bool,
}

/// What Mihomo should emit for one ready instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MihomoOutbound {
    pub name: String,
    pub group_name: String,
    pub kind: String,
    pub server: Option<String>,
    pub port: Option<u16>,
    pub udp: bool,
    pub interface_name: Option<String>,
    pub routing_mark: Option<u32>,
}

/// Runtime facts returned by [`ClientDriver::ensure`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressHandle {
    pub client_id: ClientId,
    pub preset: PresetId,
    pub kind: EgressKind,
    pub ready: bool,
    pub degraded: bool,
    pub outbound: Option<MihomoOutbound>,
    pub transport_excludes: Vec<IpNet>,
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("client driver is not available for this preset")]
    DriverUnavailable,
    #[error("client configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("local proxy is not reachable")]
    ProxyUnavailable,
    #[error("side tunnel failed: {0}")]
    SideTunnel(String),
    #[error("operation was cancelled")]
    Cancelled,
}

/// Platform the driver uses for process-name rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverPlatform {
    Linux,
    Windows,
}

/// Starts, stops, and describes one catalog preset.
#[async_trait]
pub trait ClientDriver: Send + Sync {
    fn preset_id(&self) -> PresetId;
    fn kind(&self) -> EgressKind;
    fn process_bypass(&self, platform: DriverPlatform) -> Vec<ProcessBypass>;
    fn transport_excludes(&self) -> Vec<IpNet>;
    async fn ensure(
        &self,
        instance: &ClientInstance,
        cancel: CancellationToken,
    ) -> Result<EgressHandle, ClientError>;
    async fn stop(&self, handle: &EgressHandle) -> Result<(), ClientError>;
    fn mihomo_outbound(
        &self,
        instance: &ClientInstance,
        handle: &EgressHandle,
    ) -> Option<MihomoOutbound>;
}

/// Catalog row plus the driver that can run it, if this version ships one.
#[must_use]
pub fn catalog() -> Vec<PresetSpec> {
    PresetId::all().iter().map(|preset| preset.spec()).collect()
}

/// Driver for a working preset. Catalog-only and unsupported presets return `None`.
#[must_use]
pub fn driver_for(preset: PresetId) -> Option<Box<dyn ClientDriver>> {
    match preset {
        PresetId::Hiddify
        | PresetId::Happ
        | PresetId::V2rayn
        | PresetId::Nekoray
        | PresetId::Shadowsocks => Some(Box::new(LocalProxyDriver::new(preset))),
        // Windscribe rides the OpenVPN driver: the user generates a standard
        // .ovpn profile with service credentials at build.windscribe.com.
        PresetId::Openvpn | PresetId::Windscribe => Some(Box::new(OpenVpnDriver)),
        PresetId::Wireguard => None,
    }
}

/// Process-bypass union for every enabled `LocalProxy` instance.
#[must_use]
pub fn process_bypass_union(
    clients: &[ClientInstance],
    platform: DriverPlatform,
) -> Vec<ProcessBypass> {
    let mut rules = Vec::new();
    for client in clients.iter().filter(|client| client.enabled) {
        if client.spec().kind != EgressKind::LocalProxy {
            continue;
        }
        if let Some(driver) = driver_for(client.preset) {
            rules.extend(driver.process_bypass(platform));
        }
    }
    rules
}

/// Local-proxy endpoint from an instance, when the config is that kind.
#[must_use]
pub fn local_proxy_endpoint(instance: &ClientInstance) -> Option<(String, u16)> {
    match &instance.config {
        ClientConfig::LocalProxy { host, port, .. } => Some((host.clone(), *port)),
        ClientConfig::OwnedSideTunnel { .. } | ClientConfig::Unsupported => None,
    }
}

/// Why a side tunnel is not running, in the operator's words.
///
/// A stopped side tunnel used to report no detail at all, so a client that
/// failed at connect looked identical to one that was never started. Connect
/// only warns for optional clients, so the card was the operator's only
/// chance to learn the cause.
#[must_use]
pub fn side_tunnel_stopped_reason(instance: &ClientInstance) -> Option<String> {
    let ClientConfig::OwnedSideTunnel { profile_path, .. } = &instance.config else {
        return None;
    };
    let Some(path) = profile_path else {
        return Some("choose a .ovpn profile on this card".into());
    };
    if path
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        // A stored bare file name cannot be opened. Older builds truncated the
        // path when other fields were saved; re-picking the file repairs it.
        return Some(format!(
            "the saved profile path is incomplete ({}); choose the file again",
            path.display()
        ));
    }
    if !path.is_file() {
        return Some(format!(
            "profile file not found: {}; choose the file again",
            path.display()
        ));
    }
    Some("the tunnel is not running; press Connect".into())
}

/// Synthesize a ready local-proxy handle from config (used when generate
/// runs without a live ensure, e.g. unit tests).
#[must_use]
pub fn synthesized_local_handle(instance: &ClientInstance) -> Option<EgressHandle> {
    let (host, port) = local_proxy_endpoint(instance)?;
    Some(EgressHandle {
        client_id: instance.id,
        preset: instance.preset,
        kind: EgressKind::LocalProxy,
        ready: true,
        degraded: false,
        outbound: Some(MihomoOutbound {
            name: instance.proxy_name(),
            group_name: instance.group_name(),
            kind: "socks5".into(),
            server: Some(host),
            port: Some(port),
            udp: true,
            interface_name: None,
            routing_mark: None,
        }),
        transport_excludes: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_every_planned_preset() {
        let rows = catalog();
        assert_eq!(rows.len(), 8);
        assert!(driver_for(PresetId::Hiddify).is_some());
        assert!(driver_for(PresetId::Openvpn).is_some());
        assert!(driver_for(PresetId::Happ).is_some());
        assert!(driver_for(PresetId::Wireguard).is_none());
        assert!(driver_for(PresetId::Windscribe).is_some());
    }

    #[test]
    fn stopped_side_tunnel_explains_itself() {
        let mut client = ClientInstance::from_preset(PresetId::Windscribe);
        assert!(side_tunnel_stopped_reason(&client)
            .expect("no profile")
            .contains("choose a .ovpn profile"));

        let set_profile = |client: &mut ClientInstance, value: &str| {
            if let ClientConfig::OwnedSideTunnel { profile_path, .. } = &mut client.config {
                *profile_path = Some(std::path::PathBuf::from(value));
            }
        };

        // A bare file name is what older builds stored after a settings save.
        set_profile(&mut client, "Windscribe-Berlin.ovpn");
        assert!(side_tunnel_stopped_reason(&client)
            .expect("bare name")
            .contains("incomplete"));

        set_profile(&mut client, "/nonexistent/office.ovpn");
        assert!(side_tunnel_stopped_reason(&client)
            .expect("missing file")
            .contains("not found"));

        let file = tempfile::NamedTempFile::new().expect("temp profile");
        set_profile(&mut client, &file.path().to_string_lossy());
        assert!(side_tunnel_stopped_reason(&client)
            .expect("ready")
            .contains("press Connect"));

        // Local proxies keep their own detail text.
        let hiddify = ClientInstance::from_preset(PresetId::Hiddify);
        assert!(side_tunnel_stopped_reason(&hiddify).is_none());
    }
}
