use super::{HelperServiceError, Supervisor};
use iran_split_ipc::{
    read_frame, validate_envelope, write_frame, Envelope, HelperCommand, HelperError, HelperReply,
    ServiceStatus, PROTOCOL_VERSION,
};
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};
use tracing::{error, info};

const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn execute_audited(
    supervisor: &Supervisor,
    request: Envelope<HelperCommand>,
    peer: &str,
) -> Envelope<HelperReply> {
    info!(
        event = "helper.command_received",
        section = "helper_command",
        initiator = "authorized_ipc_peer",
        cause = "ipc_request",
        trace_id = %request.request_id,
        trace_route = "desktop->helper_ipc->command_handler",
        peer,
        request_id = %request.request_id,
        command = request.payload.audit_name(),
        "authorized helper command"
    );
    let request_id = request.request_id;
    let protocol_version = request.protocol_version;
    let reply = execute(supervisor, request.payload).await;
    if let HelperReply::Error(reply_error) = &reply {
        error!(
            event = "helper.command_failed",
            section = "helper_command",
            initiator = "command_handler",
            cause = %reply_error.message,
            trace_id = %request_id,
            trace_route = "desktop->helper_ipc->command_handler->error_reply",
            error_code = reply_error.code,
            retryable = reply_error.retryable,
            "helper command failed"
        );
    } else {
        info!(
            event = "helper.command_completed",
            section = "helper_command",
            initiator = "command_handler",
            cause = "none",
            trace_id = %request_id,
            trace_route = "desktop->helper_ipc->command_handler->reply",
            "helper command completed"
        );
    }
    Envelope {
        protocol_version,
        request_id,
        payload: reply,
    }
}

async fn execute(supervisor: &Supervisor, command: HelperCommand) -> HelperReply {
    let result: Result<HelperReply, HelperServiceError> = async {
        Ok(match command {
            HelperCommand::Hello { .. } => HelperReply::Error(HelperError {
                code: "HELLO_ALREADY_COMPLETED".into(),
                message: "protocol negotiation is already complete".into(),
                retryable: false,
            }),
            HelperCommand::GetServiceStatus => HelperReply::ServiceStatus(ServiceStatus {
                helper_version: env!("CARGO_PKG_VERSION").into(),
                protocol_version: PROTOCOL_VERSION,
                authorized: true,
                active_generation: supervisor.status().await?.generation_id,
            }),
            HelperCommand::RegisterRuntimeGeneration {
                generation_id,
                config_sha256,
            } => {
                supervisor
                    .register_generation(generation_id, &config_sha256)
                    .await?;
                HelperReply::GenerationRegistered { generation_id }
            }
            HelperCommand::StartMihomo {
                generation_id,
                config_sha256,
            } => HelperReply::ProcessStatus(supervisor.start(generation_id, &config_sha256).await?),
            HelperCommand::StopMihomo => HelperReply::ProcessStatus(supervisor.stop().await?),
            HelperCommand::RestartMihomo {
                generation_id,
                config_sha256,
            } => {
                supervisor.stop().await?;
                HelperReply::ProcessStatus(supervisor.start(generation_id, &config_sha256).await?)
            }
            HelperCommand::GetMihomoProcessStatus => {
                HelperReply::ProcessStatus(supervisor.status().await?)
            }
            HelperCommand::CleanupOwnedNetworkState => {
                HelperReply::CleanupReport(supervisor.cleanup().await?)
            }
            HelperCommand::CollectServiceLogs { max_entries } => {
                HelperReply::Logs(supervisor.logs(usize::from(max_entries)).await)
            }
            HelperCommand::PrepareForUpdate => {
                let report = supervisor.cleanup().await?;
                if !report.clean() {
                    return Err(HelperServiceError::Process(
                        "owned network state remains before update".into(),
                    ));
                }
                HelperReply::ReadyForUpdate
            }
            HelperCommand::StartSideTunnel {
                driver,
                client_id,
                profile,
                executable,
                auth_file,
                timeout_seconds,
                pinned_remote,
                socks_proxy,
            } => HelperReply::SideTunnel(
                supervisor
                    .start_side_tunnel(crate::openvpn::SideTunnelRequest {
                        driver: &driver,
                        client_id,
                        profile: &profile,
                        executable: executable.as_deref(),
                        auth_file: auth_file.as_deref(),
                        timeout_seconds,
                        pinned_remote,
                        socks_proxy: socks_proxy
                            .as_ref()
                            .map(|(host, port)| (host.as_str(), *port)),
                    })
                    .await?,
            ),
            HelperCommand::StopSideTunnel { client_id } => {
                HelperReply::SideTunnel(supervisor.stop_side_tunnel(client_id).await?)
            }
        })
    }
    .await;
    result.unwrap_or_else(|error| {
        HelperReply::Error(HelperError {
            code: helper_error_code(&error).into(),
            message: error.to_string(),
            retryable: matches!(
                error,
                HelperServiceError::Io(_) | HelperServiceError::Process(_)
            ),
        })
    })
}

fn helper_error_code(error: &HelperServiceError) -> &'static str {
    match error {
        HelperServiceError::InvalidGeneration(_) => "INVALID_GENERATION",
        HelperServiceError::BinaryIntegrity => "BINARY_INTEGRITY_FAILED",
        HelperServiceError::Process(_) => "PROCESS_FAILED",
        HelperServiceError::Install(_) => "INSTALL_FAILED",
        HelperServiceError::UnsafeConfig(_) | HelperServiceError::Toml(_) => {
            "HELPER_CONFIG_INVALID"
        }
        HelperServiceError::Io(_) => "IO_FAILED",
        HelperServiceError::Protocol(_) => "PROTOCOL_ERROR",
        HelperServiceError::SideTunnel(_) => "SIDE_TUNNEL_FAILED",
    }
}

pub(crate) async fn read_request<S>(
    stream: &mut S,
) -> Result<Envelope<HelperCommand>, HelperServiceError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = timeout(IO_TIMEOUT, read_frame(stream))
        .await
        .map_err(|_| {
            HelperServiceError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "helper request timed out",
            ))
        })??;
    validate_envelope(&request)?;
    Ok(request)
}

pub(crate) async fn send_reply<S>(
    stream: &mut S,
    reply: Envelope<HelperReply>,
) -> Result<(), HelperServiceError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    timeout(IO_TIMEOUT, write_frame(stream, &reply))
        .await
        .map_err(|_| {
            HelperServiceError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "helper response timed out",
            ))
        })??;
    Ok(())
}
