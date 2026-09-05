use async_trait::async_trait;
use ipnet::IpNet;
use iran_split_config::{ClientConfig, ClientInstance, EgressKind, PresetId};
use tokio_util::sync::CancellationToken;

use crate::{
    audit_openvpn_profile, ClientDriver, ClientError, DriverPlatform, EgressHandle, MihomoOutbound,
    ProcessBypass,
};

/// `OwnedSideTunnel` driver. The helper owns the binary; this type only
/// describes Mihomo bind settings and audits the profile before elevate.
#[derive(Debug, Clone, Default)]
pub struct OpenVpnDriver;

#[async_trait]
impl ClientDriver for OpenVpnDriver {
    fn preset_id(&self) -> PresetId {
        PresetId::Openvpn
    }

    fn kind(&self) -> EgressKind {
        EgressKind::OwnedSideTunnel
    }

    fn process_bypass(&self, _platform: DriverPlatform) -> Vec<ProcessBypass> {
        Vec::new()
    }

    fn transport_excludes(&self) -> Vec<IpNet> {
        Vec::new()
    }

    async fn ensure(
        &self,
        instance: &ClientInstance,
        cancel: CancellationToken,
    ) -> Result<EgressHandle, ClientError> {
        if cancel.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let ClientConfig::OwnedSideTunnel { profile_path, .. } = &instance.config else {
            return Err(ClientError::InvalidConfig(
                "OpenVPN instance is missing a side-tunnel profile".into(),
            ));
        };
        let Some(profile) = profile_path.as_ref() else {
            return Err(ClientError::InvalidConfig(
                "OpenVPN instance has no profile path".into(),
            ));
        };
        let facts = audit_openvpn_profile(profile)
            .map_err(|error| ClientError::InvalidConfig(error.to_string()))?;
        Ok(EgressHandle {
            client_id: instance.id,
            preset: instance.preset,
            kind: EgressKind::OwnedSideTunnel,
            ready: false,
            degraded: false,
            outbound: Some(MihomoOutbound {
                name: instance.proxy_name(),
                group_name: instance.group_name(),
                kind: "direct".into(),
                server: None,
                port: None,
                udp: true,
                interface_name: None,
                routing_mark: None,
            }),
            transport_excludes: facts.server_networks,
        })
    }

    async fn stop(&self, _handle: &EgressHandle) -> Result<(), ClientError> {
        Ok(())
    }

    fn mihomo_outbound(
        &self,
        instance: &ClientInstance,
        handle: &EgressHandle,
    ) -> Option<MihomoOutbound> {
        handle.outbound.clone().or_else(|| {
            Some(MihomoOutbound {
                name: instance.proxy_name(),
                group_name: instance.group_name(),
                kind: "direct".into(),
                server: None,
                port: None,
                udp: true,
                interface_name: None,
                routing_mark: None,
            })
        })
    }
}
