use async_trait::async_trait;
use ipnet::IpNet;
use iran_split_config::{ClientInstance, EgressKind, PresetId};
use tokio_util::sync::CancellationToken;

use crate::{
    local_proxy_endpoint, ClientDriver, ClientError, DriverPlatform, EgressHandle, MihomoOutbound,
    ProcessBypass,
};

/// Shared driver for Hiddify, Happ, v2rayN, Nekoray, and Shadowsocks.
#[derive(Debug, Clone, Copy)]
pub struct LocalProxyDriver {
    preset: PresetId,
}

impl LocalProxyDriver {
    #[must_use]
    pub const fn new(preset: PresetId) -> Self {
        Self { preset }
    }
}

#[async_trait]
impl ClientDriver for LocalProxyDriver {
    fn preset_id(&self) -> PresetId {
        self.preset
    }

    fn kind(&self) -> EgressKind {
        EgressKind::LocalProxy
    }

    fn process_bypass(&self, platform: DriverPlatform) -> Vec<ProcessBypass> {
        let names = match platform {
            DriverPlatform::Linux => self.preset.spec().linux_bypass,
            DriverPlatform::Windows => self.preset.spec().windows_bypass,
        };
        names
            .iter()
            .map(|name| ProcessBypass {
                name: (*name).to_owned(),
                wildcard: name.contains('*'),
            })
            .collect()
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
        let Some((host, port)) = local_proxy_endpoint(instance) else {
            return Err(ClientError::InvalidConfig(
                "local proxy instance is missing a loopback endpoint".into(),
            ));
        };
        Ok(EgressHandle {
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

    async fn stop(&self, _handle: &EgressHandle) -> Result<(), ClientError> {
        Ok(())
    }

    fn mihomo_outbound(
        &self,
        instance: &ClientInstance,
        handle: &EgressHandle,
    ) -> Option<MihomoOutbound> {
        handle.outbound.clone().or_else(|| {
            let (host, port) = local_proxy_endpoint(instance)?;
            Some(MihomoOutbound {
                name: instance.proxy_name(),
                group_name: instance.group_name(),
                kind: "socks5".into(),
                server: Some(host),
                port: Some(port),
                udp: true,
                interface_name: None,
                routing_mark: None,
            })
        })
    }
}
