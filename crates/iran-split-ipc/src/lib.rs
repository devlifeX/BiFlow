use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::BTreeMap, io, time::Duration};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Per-frame write/connect budget for helper IPC.
pub const HELPER_IPC_FRAME_TIMEOUT_SECS: u64 = 5;

/// Small margin beyond [`HelperCommand::StartSideTunnel`]'s
/// `timeout_seconds` so route installation and reply framing do not
/// race the engine's read deadline after `OpenVPN` comes up.
pub const HELPER_IPC_SIDE_TUNNEL_MARGIN_SECS: u64 = 15;

/// How long the desktop waits for a helper reply after sending a command.
///
/// Most commands finish in milliseconds; side tunnels inherit the caller's
/// `OpenVPN` budget plus a fixed margin so slow bring-up is not reported as
/// `"helper request timed out"`.
#[must_use]
pub fn helper_ipc_reply_timeout(command: &HelperCommand) -> Duration {
    match command {
        HelperCommand::StartSideTunnel {
            timeout_seconds, ..
        } => {
            Duration::from_secs(timeout_seconds.saturating_add(HELPER_IPC_SIDE_TUNNEL_MARGIN_SECS))
        }
        _ => Duration::from_secs(HELPER_IPC_FRAME_TIMEOUT_SECS),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub protocol_version: u16,
    pub request_id: Uuid,
    pub payload: T,
}

impl<T> Envelope<T> {
    #[must_use]
    pub fn new(payload: T) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            payload,
        }
    }

    #[must_use]
    pub const fn reply<U>(&self, payload: U) -> Envelope<U> {
        Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: self.request_id,
            payload,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum HelperCommand {
    Hello {
        client_version: String,
        supported_protocols: Vec<u16>,
    },
    GetServiceStatus,
    RegisterRuntimeGeneration {
        generation_id: Uuid,
        config_sha256: String,
    },
    /// Copies a registered generation into the running Mihomo workdir
    /// without restarting the process. Pin applies use this so TUN stays up.
    OverlayRuntimeGeneration {
        generation_id: Uuid,
        config_sha256: String,
    },
    StartMihomo {
        generation_id: Uuid,
        config_sha256: String,
    },
    StopMihomo,
    RestartMihomo {
        generation_id: Uuid,
        config_sha256: String,
    },
    GetMihomoProcessStatus,
    CleanupOwnedNetworkState,
    CollectServiceLogs {
        max_entries: u16,
    },
    PrepareForUpdate,
    StartSideTunnel {
        driver: String,
        client_id: Uuid,
        profile: std::path::PathBuf,
        executable: Option<std::path::PathBuf>,
        auth_file: Option<std::path::PathBuf>,
        timeout_seconds: u64,
        /// Address the engine resolved over `DoH` through a working client, so
        /// `OpenVPN` never depends on a resolver the network can poison. The
        /// helper still rejects anything that is not a routable public IP.
        #[serde(default)]
        pinned_remote: Option<(std::net::IpAddr, u16)>,
        /// Loopback SOCKS5 endpoint of a client that already works. Used only
        /// as a fallback when a direct attempt exits early, which is what a
        /// network that blocks the server by address looks like.
        #[serde(default)]
        socks_proxy: Option<(String, u16)>,
    },
    StopSideTunnel {
        client_id: Uuid,
    },
}

impl HelperCommand {
    /// Validates bounded command arguments before helper execution.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidMessage`] for an empty or oversized
    /// protocol list, malformed generation hash, or out-of-range log request.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Hello {
                supported_protocols,
                ..
            } if supported_protocols.is_empty() || supported_protocols.len() > 8 => {
                Err(ProtocolError::InvalidMessage(
                    "supported protocol list must contain 1-8 items".into(),
                ))
            }
            Self::RegisterRuntimeGeneration { config_sha256, .. }
            | Self::OverlayRuntimeGeneration { config_sha256, .. }
            | Self::StartMihomo { config_sha256, .. }
            | Self::RestartMihomo { config_sha256, .. }
                if !valid_sha256(config_sha256) =>
            {
                Err(ProtocolError::InvalidMessage(
                    "generation SHA-256 must be 64 lowercase hexadecimal characters".into(),
                ))
            }
            Self::CollectServiceLogs { max_entries }
                if *max_entries == 0 || *max_entries > 2_000 =>
            {
                Err(ProtocolError::InvalidMessage(
                    "log request must contain between 1 and 2000 entries".into(),
                ))
            }
            Self::StartSideTunnel {
                driver,
                timeout_seconds,
                profile,
                ..
            } if driver != "openvpn"
                || *timeout_seconds == 0
                || *timeout_seconds > 300
                || profile.as_os_str().is_empty() =>
            {
                Err(ProtocolError::InvalidMessage(
                    "side tunnel request is missing a supported driver, profile, or timeout".into(),
                ))
            }
            _ => Ok(()),
        }
    }

    #[must_use]
    pub const fn audit_name(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::GetServiceStatus => "get_service_status",
            Self::RegisterRuntimeGeneration { .. } => "register_runtime_generation",
            Self::OverlayRuntimeGeneration { .. } => "overlay_runtime_generation",
            Self::StartMihomo { .. } => "start_mihomo",
            Self::StopMihomo => "stop_mihomo",
            Self::RestartMihomo { .. } => "restart_mihomo",
            Self::GetMihomoProcessStatus => "get_mihomo_process_status",
            Self::CleanupOwnedNetworkState => "cleanup_owned_network_state",
            Self::CollectServiceLogs { .. } => "collect_service_logs",
            Self::PrepareForUpdate => "prepare_for_update",
            Self::StartSideTunnel { .. } => "start_side_tunnel",
            Self::StopSideTunnel { .. } => "stop_side_tunnel",
        }
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", content = "value", rename_all = "snake_case")]
pub enum HelperReply {
    Hello(HelloReply),
    ServiceStatus(ServiceStatus),
    GenerationRegistered { generation_id: Uuid },
    ProcessStatus(ProcessStatus),
    CleanupReport(CleanupReport),
    Logs(Vec<ServiceLogEntry>),
    ReadyForUpdate,
    SideTunnel(SideTunnelStatus),
    Ack,
    Error(HelperError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SideTunnelStatus {
    pub client_id: Option<Uuid>,
    pub driver: String,
    pub running: bool,
    pub device: Option<String>,
    pub routing_mark: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloReply {
    pub helper_version: String,
    pub selected_protocol: u16,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub helper_version: String,
    pub protocol_version: u16,
    pub authorized: bool,
    pub active_generation: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub generation_id: Option<Uuid>,
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CleanupReport {
    pub process_stopped: bool,
    pub tun_removed: bool,
    pub routes_removed: u32,
    pub dns_restored: bool,
    pub warnings: Vec<String>,
}

impl CleanupReport {
    #[must_use]
    pub fn clean(&self) -> bool {
        self.process_stopped && self.tun_removed && self.dns_restored && self.warnings.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceLogEntry {
    pub timestamp: String,
    pub level: String,
    pub event: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IPC I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("IPC JSON is malformed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IPC message length {actual} exceeds the {maximum}-byte limit")]
    Oversized { actual: usize, maximum: usize },
    #[error("unsupported protocol version {actual}; expected {expected}")]
    VersionMismatch { actual: u16, expected: u16 },
    #[error("invalid IPC message: {0}")]
    InvalidMessage(String),
}

/// Writes one length-prefixed JSON message to an asynchronous stream.
///
/// # Errors
///
/// Returns [`ProtocolError`] when serialization fails, the encoded message is
/// oversized, or the stream cannot be written or flushed.
pub async fn write_frame<W, T>(writer: &mut W, message: &T) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(message)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::Oversized {
            actual: bytes.len(),
            maximum: MAX_MESSAGE_BYTES,
        });
    }
    let length = u32::try_from(bytes.len()).map_err(|_| ProtocolError::Oversized {
        actual: bytes.len(),
        maximum: MAX_MESSAGE_BYTES,
    })?;
    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads one bounded length-prefixed JSON message from an asynchronous stream.
///
/// # Errors
///
/// Returns [`ProtocolError`] for I/O failure, a zero-length or oversized frame,
/// or malformed JSON.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<T, ProtocolError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut length_bytes = [0_u8; 4];
    reader.read_exact(&mut length_bytes).await?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::Oversized {
            actual: length,
            maximum: MAX_MESSAGE_BYTES,
        });
    }
    if length == 0 {
        return Err(ProtocolError::InvalidMessage(
            "zero-length IPC frame".into(),
        ));
    }
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Verifies that an envelope uses this crate's protocol version.
///
/// # Errors
///
/// Returns [`ProtocolError::VersionMismatch`] when the peer uses another
/// protocol version.
pub fn validate_envelope<T>(message: &Envelope<T>) -> Result<(), ProtocolError> {
    if message.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch {
            actual: message.protocol_version,
            expected: PROTOCOL_VERSION,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn framed_round_trip_preserves_request_id() {
        let (mut client, mut server) = tokio::io::duplex(4_096);
        let expected = Envelope::new(HelperCommand::GetServiceStatus);
        let request_id = expected.request_id;
        let sender = tokio::spawn(async move { write_frame(&mut client, &expected).await });
        let actual: Envelope<HelperCommand> = read_frame(&mut server).await.expect("read frame");
        sender.await.expect("join").expect("write frame");
        assert_eq!(actual.request_id, request_id);
        validate_envelope(&actual).expect("valid envelope");
    }

    #[tokio::test]
    async fn rejects_oversized_length_before_allocating() {
        let (mut client, mut server) = tokio::io::duplex(16);
        let oversized = u32::try_from(MAX_MESSAGE_BYTES).expect("message limit fits u32") + 1;
        client
            .write_all(&oversized.to_be_bytes())
            .await
            .expect("write length");
        let error = read_frame::<_, Envelope<HelperCommand>>(&mut server)
            .await
            .expect_err("oversized frame");
        assert!(matches!(error, ProtocolError::Oversized { .. }));
    }

    #[test]
    fn validates_generation_hash_and_log_bound() {
        let invalid = HelperCommand::StartMihomo {
            generation_id: Uuid::new_v4(),
            config_sha256: "../config".into(),
        };
        assert!(invalid.validate().is_err());
        let overlay = HelperCommand::OverlayRuntimeGeneration {
            generation_id: Uuid::new_v4(),
            config_sha256: "../config".into(),
        };
        assert!(overlay.validate().is_err());
        assert!(HelperCommand::CollectServiceLogs { max_entries: 2_001 }
            .validate()
            .is_err());
    }

    #[test]
    fn start_side_tunnel_ipc_budget_tracks_command_timeout() {
        let command = HelperCommand::StartSideTunnel {
            driver: "openvpn".into(),
            client_id: Uuid::new_v4(),
            profile: "/tmp/profile.ovpn".into(),
            executable: None,
            auth_file: None,
            timeout_seconds: 45,
            pinned_remote: None,
            socks_proxy: None,
        };
        assert_eq!(
            helper_ipc_reply_timeout(&command),
            Duration::from_secs(45 + HELPER_IPC_SIDE_TUNNEL_MARGIN_SECS)
        );
    }

    #[test]
    fn routine_helper_commands_keep_short_ipc_budget() {
        assert_eq!(
            helper_ipc_reply_timeout(&HelperCommand::GetServiceStatus),
            Duration::from_secs(HELPER_IPC_FRAME_TIMEOUT_SECS)
        );
        assert_eq!(
            helper_ipc_reply_timeout(&HelperCommand::StopMihomo),
            Duration::from_secs(HELPER_IPC_FRAME_TIMEOUT_SECS)
        );
    }
}
