//! Per-user local IPC used by the phase-zero CLI/runtime ownership prototype.

use crate::{read_frame, write_frame, ProtocolError};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::path::PathBuf;
use std::{future::Future, io, time::Duration};
use tokio::time::timeout;
use uuid::Uuid;

pub const RUNTIME_PROTOCOL_VERSION: u16 = 1;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRequest {
    pub protocol_version: u16,
    pub request_id: Uuid,
    pub command: RuntimeCommand,
}

impl RuntimeRequest {
    #[must_use]
    pub fn new(command: RuntimeCommand) -> Self {
        Self {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            command,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum RuntimeCommand {
    Status,
    Connect { timeout_seconds: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeReply {
    pub protocol_version: u16,
    pub request_id: Uuid,
    pub result: Result<RuntimeResult, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum RuntimeResult {
    Status {
        phase: String,
        operation_id: Option<Uuid>,
    },
    ConnectAccepted {
        operation_id: Uuid,
    },
}

#[must_use]
pub fn reply(request: &RuntimeRequest, result: Result<RuntimeResult, String>) -> RuntimeReply {
    RuntimeReply {
        protocol_version: RUNTIME_PROTOCOL_VERSION,
        request_id: request.request_id,
        result,
    }
}

/// Sends one request to the user runtime and waits for its correlated reply.
///
/// # Errors
///
/// Returns an IPC error when the runtime is unavailable, times out, or returns
/// an invalid protocol version.
pub async fn request(command: RuntimeCommand) -> Result<RuntimeReply, ProtocolError> {
    #[cfg(unix)]
    let endpoint = socket_path()?.to_string_lossy().into_owned();
    #[cfg(windows)]
    let endpoint = pipe_name()?;
    request_at(&endpoint, command).await
}

/// Sends one runtime request to an explicit local endpoint.
///
/// # Errors
///
/// Returns an IPC error when the endpoint is unavailable, times out, or returns
/// an invalid protocol version.
pub async fn request_at(
    endpoint: &str,
    command: RuntimeCommand,
) -> Result<RuntimeReply, ProtocolError> {
    let request = RuntimeRequest::new(command);
    #[cfg(unix)]
    let mut stream = timeout(CONNECT_TIMEOUT, tokio::net::UnixStream::connect(endpoint))
        .await
        .map_err(|_| {
            ProtocolError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "runtime connect timed out",
            ))
        })?
        .map_err(|_| runtime_unavailable())?;
    #[cfg(windows)]
    let mut stream = connect_windows_pipe(endpoint).await?;

    timeout(CONNECT_TIMEOUT, write_frame(&mut stream, &request))
        .await
        .map_err(|_| {
            ProtocolError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "runtime write timed out",
            ))
        })??;
    let response: RuntimeReply = timeout(CONNECT_TIMEOUT, read_frame(&mut stream))
        .await
        .map_err(|_| {
            ProtocolError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "runtime reply timed out",
            ))
        })??;
    if response.protocol_version != RUNTIME_PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch {
            actual: response.protocol_version,
            expected: RUNTIME_PROTOCOL_VERSION,
        });
    }
    if response.request_id != request.request_id {
        return Err(ProtocolError::InvalidMessage(
            "runtime reply request id does not match".into(),
        ));
    }
    Ok(response)
}

/// Accepts user-runtime requests until the owning application exits.
///
/// The caller owns the application services and keeps this future running for
/// their lifetime. Each connection carries one bounded, correlated frame.
///
/// # Errors
///
/// Returns an IPC error if the endpoint cannot be secured or served.
pub async fn serve<F, Fut>(handler: F) -> Result<(), ProtocolError>
where
    F: Fn(RuntimeRequest) -> Fut,
    Fut: Future<Output = RuntimeReply>,
{
    #[cfg(unix)]
    let endpoint = socket_path()?.to_string_lossy().into_owned();
    #[cfg(windows)]
    let endpoint = pipe_name()?;
    serve_at(&endpoint, handler).await
}

/// Serves the phase-zero protocol on a specific local endpoint.
///
/// Exposing the endpoint makes process-level tests independent of the user's
/// normal GUI runtime.
///
/// # Errors
///
/// Returns an IPC error if the endpoint cannot be secured or served.
pub async fn serve_at<F, Fut>(endpoint: &str, handler: F) -> Result<(), ProtocolError>
where
    F: Fn(RuntimeRequest) -> Fut,
    Fut: Future<Output = RuntimeReply>,
{
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = PathBuf::from(endpoint);
        secure_socket_parent(&path)?;
        remove_stale_socket(&path)?;
        let listener = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        loop {
            let (mut stream, _) = listener.accept().await?;
            let Ok(request) = read_frame::<_, RuntimeRequest>(&mut stream).await else {
                tracing::warn!(
                    event = "runtime_ipc.request_rejected",
                    section = "runtime_ipc",
                    initiator = "local_client",
                    cause = "invalid_or_disconnected_peer",
                    trace_route = "runtime_ipc_server->frame_decode",
                    "local runtime IPC request was rejected"
                );
                continue;
            };
            let response = if request.protocol_version == RUNTIME_PROTOCOL_VERSION {
                handler(request).await
            } else {
                tracing::warn!(
                    event = "runtime_ipc.version_rejected",
                    section = "runtime_ipc",
                    initiator = "local_client",
                    cause = "unsupported_protocol_version",
                    trace_route = "runtime_ipc_server->protocol_validation",
                    trace_id = %request.request_id,
                    "local runtime IPC protocol version was rejected"
                );
                reply(&request, Err("unsupported runtime protocol version".into()))
            };
            if write_frame(&mut stream, &response).await.is_err() {
                tracing::warn!(
                    event = "runtime_ipc.reply_failed",
                    section = "runtime_ipc",
                    initiator = "local_client",
                    cause = "client_disconnected_before_reply",
                    trace_route = "runtime_ipc_server->frame_write",
                    trace_id = %response.request_id,
                    "local runtime IPC reply could not be delivered"
                );
            }
        }
    }
    #[cfg(windows)]
    {
        loop {
            let mut stream = iran_split_helper_winacl::create_runtime_server(endpoint, true)?;
            stream.connect().await?;
            let Ok(request) = read_frame::<_, RuntimeRequest>(&mut stream).await else {
                tracing::warn!(
                    event = "runtime_ipc.request_rejected",
                    section = "runtime_ipc",
                    initiator = "local_client",
                    cause = "invalid_or_disconnected_peer",
                    trace_route = "runtime_ipc_server->frame_decode",
                    "local runtime IPC request was rejected"
                );
                continue;
            };
            let response = if request.protocol_version == RUNTIME_PROTOCOL_VERSION {
                handler(request).await
            } else {
                tracing::warn!(
                    event = "runtime_ipc.version_rejected",
                    section = "runtime_ipc",
                    initiator = "local_client",
                    cause = "unsupported_protocol_version",
                    trace_route = "runtime_ipc_server->protocol_validation",
                    trace_id = %request.request_id,
                    "local runtime IPC protocol version was rejected"
                );
                reply(&request, Err("unsupported runtime protocol version".into()))
            };
            if write_frame(&mut stream, &response).await.is_err() {
                tracing::warn!(
                    event = "runtime_ipc.reply_failed",
                    section = "runtime_ipc",
                    initiator = "local_client",
                    cause = "client_disconnected_before_reply",
                    trace_route = "runtime_ipc_server->frame_write",
                    trace_id = %response.request_id,
                    "local runtime IPC reply could not be delivered"
                );
            }
        }
    }
}

#[cfg(unix)]
fn socket_path() -> Result<PathBuf, ProtocolError> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        ProtocolError::InvalidMessage("XDG_RUNTIME_DIR is required for runtime IPC".into())
    })?;
    Ok(PathBuf::from(runtime_dir)
        .join("biflow")
        .join(if dev_profile() {
            "runtime-dev.sock"
        } else {
            "runtime.sock"
        }))
}

#[cfg(unix)]
fn secure_socket_parent(path: &std::path::Path) -> Result<(), ProtocolError> {
    use std::os::unix::fs::PermissionsExt;
    let parent = path
        .parent()
        .ok_or_else(|| ProtocolError::InvalidMessage("runtime socket has no parent".into()))?;
    std::fs::create_dir_all(parent)?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(unix)]
fn remove_stale_socket(path: &std::path::Path) -> Result<(), ProtocolError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    match std::fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_socket()
                && metadata.uid()
                    == path
                        .parent()
                        .and_then(|parent| std::fs::metadata(parent).ok())
                        .map_or(u32::MAX, |parent| parent.uid()) =>
        {
            std::fs::remove_file(path)?;
            Ok(())
        }
        Ok(_) => Err(ProtocolError::InvalidMessage(
            "runtime endpoint exists and is not an owned socket".into(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn pipe_name() -> Result<String, ProtocolError> {
    let user = std::env::var("USERNAME").map_err(|_| {
        ProtocolError::InvalidMessage("USERNAME is required for runtime IPC".into())
    })?;
    let user = user
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                char::from(byte)
            } else {
                '_'
            }
        })
        .collect::<String>();
    Ok(format!(
        r"\\.\pipe\biflow-runtime-v1-{user}{}",
        if dev_profile() { "-dev" } else { "" }
    ))
}

fn dev_profile() -> bool {
    std::env::var_os("BIFLOW_DEV_PROFILE").is_some()
}

fn runtime_unavailable() -> ProtocolError {
    ProtocolError::InvalidMessage(
        "BiFlow runtime is unavailable; start the desktop application first".into(),
    )
}

#[cfg(windows)]
async fn connect_windows_pipe(
    endpoint: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, ProtocolError> {
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
            Ok(stream) => return Ok(stream),
            Err(error) if error.raw_os_error() == Some(231) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(ProtocolError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "runtime pipe remained busy",
                    )));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(_) => return Err(runtime_unavailable()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn runtime_request_has_correlated_id_and_current_protocol() {
        let request = RuntimeRequest::new(RuntimeCommand::Connect {
            timeout_seconds: 60,
        });
        let decoded: RuntimeRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).expect("encode request"))
                .expect("decode request");
        assert_eq!(decoded, request);
        let response = reply(
            &request,
            Ok(RuntimeResult::ConnectAccepted {
                operation_id: Uuid::new_v4(),
            }),
        );
        assert_eq!(response.request_id, request.request_id);
        assert_eq!(response.protocol_version, RUNTIME_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn client_disconnect_does_not_cancel_connect_or_kill_the_owner() {
        #[cfg(unix)]
        let directory = tempfile::tempdir().expect("temporary directory");
        #[cfg(unix)]
        let endpoint = directory
            .path()
            .join("runtime.sock")
            .to_string_lossy()
            .into_owned();
        #[cfg(windows)]
        let endpoint = format!(r"\\.\pipe\biflow-runtime-test-{}", Uuid::new_v4());

        let connect_count = Arc::new(AtomicUsize::new(0));
        let handler_count = Arc::clone(&connect_count);
        let server_endpoint = endpoint.clone();
        let server = tokio::spawn(async move {
            serve_at(&server_endpoint, move |request| {
                let connect_count = Arc::clone(&handler_count);
                async move {
                    let result = match request.command {
                        RuntimeCommand::Connect { .. } => {
                            tokio::time::sleep(Duration::from_millis(80)).await;
                            connect_count.fetch_add(1, Ordering::SeqCst);
                            Ok(RuntimeResult::ConnectAccepted {
                                operation_id: Uuid::new_v4(),
                            })
                        }
                        RuntimeCommand::Status => Ok(RuntimeResult::Status {
                            phase: "connecting".into(),
                            operation_id: Some(Uuid::new_v4()),
                        }),
                    };
                    reply(&request, result)
                }
            })
            .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;

        let client_endpoint = endpoint.clone();
        let client = tokio::spawn(async move {
            request_at(
                &client_endpoint,
                RuntimeCommand::Connect {
                    timeout_seconds: 60,
                },
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        client.abort();
        let _ = client.await;

        let status = tokio::time::timeout(
            Duration::from_secs(2),
            request_at(&endpoint, RuntimeCommand::Status),
        )
        .await
        .expect("owner serves a reconnecting client")
        .expect("status response");
        match status.result.expect("status payload") {
            RuntimeResult::Status {
                phase,
                operation_id,
            } => {
                assert_eq!(phase, "connecting");
                assert!(operation_id.is_some());
            }
            RuntimeResult::ConnectAccepted { .. } => panic!("expected status response"),
        }
        assert_eq!(connect_count.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
    }
}
