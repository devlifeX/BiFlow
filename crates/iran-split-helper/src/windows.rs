use super::{commands, HelperServiceError, HelperSettings, Supervisor};
use iran_split_helper_winacl::create_helper_server;
use iran_split_ipc::{HelloReply, HelperCommand, HelperError, HelperReply, PROTOCOL_VERSION};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tracing::{info, warn};

const PIPE_NAME: &str = r"\\.\pipe\iran-split-helper-v1";
const INSTALL_ROOT: &str = r"C:\ProgramData\iran-split";
const INSTALL_LOG: &str = r"C:\ProgramData\iran-split\install.log";
/// Builtin\Users. Medium-integrity desktop processes can then write generations
/// that the SYSTEM helper later publishes. `(OI)(CI)M` is modify, inherited.
const USERS_MODIFY_ACE: &str = "*S-1-5-32-545:(OI)(CI)M";
const TASK_NAME: &str = "BiFlowHelper";
const PIPE_READY_TIMEOUT: Duration = Duration::from_secs(15);
const PIPE_READY_POLL: Duration = Duration::from_millis(100);
/// A previous helper keeps its own `.exe` locked, so a reinstall waits this
/// long for the ended task to exit before copying over it.
const HELPER_STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// Defender and the previous helper can briefly retain the machine-wide
/// configuration during a reinstall. Keep the elevated install bounded while
/// allowing that transient sharing violation to clear.
const CONFIG_WRITE_ATTEMPTS: usize = 12;
/// `ERROR_FILE_NOT_FOUND`: the pipe object does not exist yet.
const ERROR_FILE_NOT_FOUND: i32 = 2;
/// `CREATE_NO_WINDOW`: the elevated installer is a GUI-subsystem binary
/// (ADR 0030), so a console child would flash a window over the UI.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Records why a GUI-subsystem helper install died. `Start-Process -Verb RunAs`
/// cannot capture elevated stderr, so the desktop reads this file after a
/// non-zero exit.
pub fn persist_install_error(message: &str) {
    let path = Path::new(INSTALL_LOG);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(error) = fs::write(path, format!("{}\n", super::single_line(message))) {
        warn!(
            event = "helper.install_log_failed",
            section = "helper_install",
            initiator = "elevated_installer",
            cause = %error,
            trace_route = "elevated_helper->install_log",
            "could not persist the helper install error"
        );
    }
}

/// Serves helper IPC on the versioned local named pipe.
///
/// # Errors
///
/// Returns an error when configuration cannot be loaded or the pipe cannot be
/// created.
pub async fn run_named_pipe(config_path: &Path) -> Result<(), HelperServiceError> {
    let settings = HelperSettings::load(config_path)?;
    let supervisor = Arc::new(Supervisor::new(settings));
    let mut server = create_helper_server(PIPE_NAME, true).map_err(HelperServiceError::Io)?;
    info!(
        event = "helper.listening",
        section = "helper_ipc",
        initiator = "helper_process",
        cause = "startup_complete",
        trace_route = "helper_process->named_pipe->ipc_listener",
        "helper listening"
    );

    loop {
        server.connect().await?;
        let connected = server;
        server = create_helper_server(PIPE_NAME, false).map_err(HelperServiceError::Io)?;
        let supervisor = Arc::clone(&supervisor);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(connected, supervisor).await {
                warn!(
                    event = "helper.connection_failed",
                    section = "helper_ipc",
                    initiator = "ipc_client",
                    cause = %error,
                    trace_route = "ipc_client->helper_connection->command_handler",
                    "helper connection closed with an error"
                );
            }
        });
    }
}

/// Copies the helper and Mihomo into `ProgramData` and starts a SYSTEM task.
///
/// # Errors
///
/// Returns an error when files cannot be copied, the configuration is unsafe,
/// or the scheduled task cannot be created.
pub fn install(
    mihomo_src: &Path,
    staging_dir: &Path,
    tun_name: &str,
) -> Result<(), HelperServiceError> {
    install_inner(mihomo_src, staging_dir, tun_name).inspect_err(|error| {
        persist_install_error(&error.to_string());
    })
}

fn install_inner(
    mihomo_src: &Path,
    requested_staging_dir: &Path,
    tun_name: &str,
) -> Result<(), HelperServiceError> {
    let helper_src = std::env::current_exe()?;
    let root = PathBuf::from(INSTALL_ROOT);
    let bin = root.join("bin");
    let helper_dest = bin.join("iran-split-helper.exe");
    let mihomo_dest = bin.join("mihomo.exe");
    let config_dest = install_config_path(&root);
    // NSIS perMachine `SetShellVarContext all` makes the local-appdata shell
    // variable expand to `C:\ProgramData`, and an elevated in-app Install can
    // record an admin profile. The helper always stages beside `runtime`, never
    // inside a user profile and never inside `runtime_dir` (those two must not
    // nest).
    let staging_dir = root.join("staging");
    if requested_staging_dir != staging_dir {
        warn!(
            event = "helper.staging_dir_overridden",
            section = "helper_install",
            initiator = "elevated_installer",
            cause = "machine_wide_staging",
            trace_route = "elevated_helper->helper.toml",
            "Windows helper staging is always ProgramData\\iran-split\\staging"
        );
    }
    fs::create_dir_all(&bin)?;
    fs::create_dir_all(root.join("runtime"))?;
    fs::create_dir_all(&staging_dir)?;
    grant_users_modify(&staging_dir)?;
    stop_previous_helper();
    super::copy_file_unless_same(&helper_src, &helper_dest)?;
    super::copy_file_unless_same(mihomo_src, &mihomo_dest)?;
    if let Some(wintun_src) = mihomo_src.parent().map(|parent| parent.join("wintun.dll")) {
        if wintun_src.is_file() {
            super::copy_file_unless_same(&wintun_src, &bin.join("wintun.dll"))?;
        }
    }
    let mihomo_sha256 = sha256_file(&mihomo_dest)?;
    let settings = HelperSettings {
        authorized_uid: 0,
        authorized_gid: 0,
        socket_path: PathBuf::from(PIPE_NAME),
        staging_dir,
        runtime_dir: root.join("runtime"),
        mihomo_binary: mihomo_dest,
        mihomo_sha256,
        tun_name: tun_name.to_owned(),
    };
    settings.validate()?;
    let config = format!(
        "authorized_uid = 0\nauthorized_gid = 0\nsocket_path = \"{}\"\nstaging_dir = \"{}\"\nruntime_dir = \"{}\"\nmihomo_binary = \"{}\"\nmihomo_sha256 = \"{}\"\ntun_name = \"{}\"\n",
        escape_toml(&settings.socket_path),
        escape_toml(&settings.staging_dir),
        escape_toml(&settings.runtime_dir),
        escape_toml(&settings.mihomo_binary),
        settings.mihomo_sha256,
        settings.tun_name
    );
    write_config_with_retry(&config_dest, &config)?;
    register_and_start_task(&helper_dest, &config_dest)?;
    info!(
        event = "helper.installed",
        section = "helper_install",
        initiator = "elevated_installer",
        cause = "user_requested",
        trace_route = "desktop->elevated_helper->scheduled_task",
        "windows helper scheduled task installed"
    );
    Ok(())
}

fn write_config_with_retry(path: &Path, contents: &str) -> Result<(), HelperServiceError> {
    let mut last_error = None;
    for attempt in 0..CONFIG_WRITE_ATTEMPTS {
        match fs::write(path, contents) {
            Ok(()) => return Ok(()),
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => {
                last_error = Some(error);
                if attempt + 1 >= CONFIG_WRITE_ATTEMPTS {
                    break;
                }
                warn!(
                    event = "helper.config_write_retry",
                    section = "helper_install",
                    initiator = "elevated_helper",
                    cause = "windows_file_lock",
                    trace_route = "elevated_helper->helper.toml",
                    attempt = attempt + 1,
                    "helper configuration is temporarily locked; retrying"
                );
                thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(HelperServiceError::Io(error)),
        }
    }
    let error = last_error.expect("config write retry must retain the last error");
    Err(HelperServiceError::Install(format!(
        "helper configuration write failed for {}: {error}",
        path.display()
    )))
}

fn install_config_path(root: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    root.join("runtime")
        .join(format!("helper-{timestamp}-{}.toml", std::process::id()))
}

/// Lets the unelevated desktop write generation files that SYSTEM later copies
/// into `runtime_dir`. `icacls /grant` adds Builtin\Users modify without
/// replacing SYSTEM or Administrators inherited from `ProgramData`.
fn grant_users_modify(path: &Path) -> Result<(), HelperServiceError> {
    let output = Command::new("icacls")
        .arg(path)
        .arg("/grant")
        .arg(USERS_MODIFY_ACE)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if output.status.success() {
        info!(
            event = "helper.staging_acl_granted",
            section = "helper_install",
            initiator = "elevated_installer",
            cause = "users_modify",
            trace_route = "elevated_helper->staging_acl",
            "Users can write helper staging generations"
        );
        return Ok(());
    }
    let mut detail = super::single_line(&super::decode_console_output(&output.stderr));
    if detail.is_empty() {
        detail = super::single_line(&super::decode_console_output(&output.stdout));
    }
    Err(HelperServiceError::Install(format!(
        "could not grant Users modify on the helper staging directory: {}",
        detail.chars().take(300).collect::<String>()
    )))
}

fn register_and_start_task(helper: &Path, config: &Path) -> Result<(), HelperServiceError> {
    // Keep the task manifest paired with the unique config. A previous NSIS
    // install can leave the legacy helper-task.xml open while Defender scans
    // the machine-wide directory.
    let xml_path = config.with_extension("xml");
    write_utf16_le_bom(&xml_path, &super::scheduled_task_xml(helper, config))?;
    let xml = xml_path.to_string_lossy();
    run_schtasks(&["/Create", "/TN", TASK_NAME, "/XML", xml.as_ref(), "/F"])?;
    run_schtasks(&["/Run", "/TN", TASK_NAME])?;
    wait_until_pipe_ready()
}

fn write_utf16_le_bom(path: &Path, text: &str) -> Result<(), HelperServiceError> {
    let mut bytes = Vec::with_capacity(2 + text.len().saturating_mul(2));
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn wait_until_pipe_ready() -> Result<(), HelperServiceError> {
    let deadline = Instant::now() + PIPE_READY_TIMEOUT;
    while Instant::now() < deadline {
        if pipe_is_serving() {
            return Ok(());
        }
        thread::sleep(PIPE_READY_POLL);
    }
    Err(HelperServiceError::Install(pipe_missing_reason()))
}

/// `Path::exists` cannot answer this. `\\.\pipe\…` is an NPFS object with no
/// file attributes to query, so `fs::metadata` fails while the helper is
/// serving perfectly well and every install would time out. Open the pipe the
/// way the desktop does instead, and treat only `ERROR_FILE_NOT_FOUND` as "not
/// created yet": a busy instance or a denied ACL still proves it exists, and
/// failing open keeps an unexpected error from condemning a healthy install.
fn pipe_is_serving() -> bool {
    match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE_NAME)
    {
        Ok(_) => true,
        Err(error) => error.raw_os_error() != Some(ERROR_FILE_NOT_FOUND),
    }
}

/// The scheduled helper records its own startup failure in `install.log`, so
/// prefer that. When it is empty the process never ran at all, and the action
/// Task Scheduler actually stored is the one fact that separates a malformed
/// registration from a helper that started and died.
fn pipe_missing_reason() -> String {
    let logged = fs::read_to_string(INSTALL_LOG).unwrap_or_default();
    if let Some(line) = logged.lines().map(str::trim).find(|line| !line.is_empty()) {
        return line.to_owned();
    }
    format!(
        "scheduled task ran but never opened the helper pipe; action: {}",
        stored_task_action()
    )
}

fn stored_task_action() -> String {
    let Ok(output) = schtasks(&["/Query", "/TN", TASK_NAME, "/XML"]) else {
        return "unreadable".into();
    };
    let xml = super::decode_console_output(&output.stdout);
    let Some(command) = super::xml_element(&xml, "Command") else {
        return "absent".into();
    };
    let arguments = super::xml_element(&xml, "Arguments").unwrap_or_default();
    super::single_line(&format!("{command} {arguments}"))
}

/// A reinstall cannot copy over a helper that is still running — Windows
/// answers `ERROR_SHARING_VIOLATION` for the executable's own image — so end
/// the previous task and let it release the file first.
fn stop_previous_helper() {
    if run_schtasks(&["/End", "/TN", TASK_NAME]).is_err() {
        return;
    }
    let deadline = Instant::now() + HELPER_STOP_TIMEOUT;
    while Instant::now() < deadline && pipe_is_serving() {
        thread::sleep(PIPE_READY_POLL);
    }
}

/// Stops and removes the packaged Windows helper task.
///
/// # Errors
///
/// Returns an error when `schtasks` fails for a reason other than a missing
/// task.
pub fn uninstall() -> Result<(), HelperServiceError> {
    uninstall_inner().inspect_err(|error| {
        persist_install_error(&error.to_string());
    })
}

fn uninstall_inner() -> Result<(), HelperServiceError> {
    let _ = run_schtasks(&["/End", "/TN", TASK_NAME]);
    match run_schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]) {
        Ok(()) => Ok(()),
        Err(error) if error.to_string().contains("cannot find the file") => Ok(()),
        Err(error) => Err(error),
    }
}

fn schtasks(args: &[&str]) -> Result<std::process::Output, HelperServiceError> {
    Ok(Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?)
}

fn run_schtasks(args: &[&str]) -> Result<(), HelperServiceError> {
    let output = schtasks(args)?;
    if output.status.success() {
        return Ok(());
    }
    // `schtasks` reports refusals on stderr but falls back to stdout for a few
    // of them, so an empty stderr must not become an empty reason.
    let mut detail = super::single_line(&super::decode_console_output(&output.stderr));
    if detail.is_empty() {
        detail = super::single_line(&super::decode_console_output(&output.stdout));
    }
    Err(HelperServiceError::Install(format!(
        "schtasks {} failed: {}",
        args.first().copied().unwrap_or_default(),
        detail.chars().take(300).collect::<String>()
    )))
}

fn sha256_file(path: &Path) -> Result<String, HelperServiceError> {
    let bytes = fs::read(path)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn escape_toml(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

async fn handle_connection(
    mut stream: NamedPipeServer,
    supervisor: Arc<Supervisor>,
) -> Result<(), HelperServiceError> {
    let hello = commands::read_request(&mut stream).await?;
    let HelperCommand::Hello {
        supported_protocols,
        ..
    } = &hello.payload
    else {
        commands::send_reply(
            &mut stream,
            hello.reply(HelperReply::Error(HelperError {
                code: "HELLO_REQUIRED".into(),
                message: "the first command must negotiate the protocol".into(),
                retryable: false,
            })),
        )
        .await?;
        return Ok(());
    };
    if !supported_protocols.contains(&PROTOCOL_VERSION) {
        commands::send_reply(
            &mut stream,
            hello.reply(HelperReply::Error(HelperError {
                code: "PROTOCOL_MISMATCH".into(),
                message: "no supported protocol version overlaps".into(),
                retryable: false,
            })),
        )
        .await?;
        return Ok(());
    }
    commands::send_reply(
        &mut stream,
        hello.reply(HelperReply::Hello(HelloReply {
            helper_version: env!("CARGO_PKG_VERSION").into(),
            selected_protocol: PROTOCOL_VERSION,
            capabilities: vec![
                "runtime_generation_v1".into(),
                "owned_network_cleanup_v1".into(),
                "bounded_logs_v1".into(),
            ],
        })),
    )
    .await?;

    loop {
        let request = match commands::read_request(&mut stream).await {
            Ok(request) => request,
            Err(HelperServiceError::Protocol(iran_split_ipc::ProtocolError::Io(error)))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        request.payload.validate()?;
        commands::send_reply(
            &mut stream,
            commands::execute_audited(&supervisor, request, "named_pipe").await,
        )
        .await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_writer_creates_the_helper_configuration() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("helper.toml");

        write_config_with_retry(&path, "tun_name = \"clash-iran\"\n")
            .expect("configuration should be written");

        assert_eq!(
            fs::read_to_string(path).expect("configuration should be readable"),
            "tun_name = \"clash-iran\"\n"
        );
    }

    #[test]
    fn config_writer_preserves_the_path_when_windows_shares_are_locked() {
        use std::os::windows::fs::OpenOptionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("locked-helper.toml");
        let _lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(&path)
            .expect("locked configuration");

        let error = write_config_with_retry(&path, "tun_name = \"clash-iran\"\n")
            .expect_err("the locked file should fail after bounded retries");
        let message = error.to_string();
        assert!(message.contains("locked-helper.toml"));
        assert!(message.contains("os error 32") || message.contains("os error 5"));
    }

    #[test]
    fn install_config_path_does_not_reuse_the_locked_default_file() {
        let root = Path::new(r"C:\ProgramData\iran-split");
        let path = install_config_path(root);

        assert_eq!(path.parent(), Some(root.join("runtime").as_path()));
        assert_ne!(path, root.join("helper.toml"));
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("toml")
        );
    }

    #[test]
    fn task_manifest_follows_the_unique_config_path() {
        let config = Path::new(r"C:\ProgramData\iran-split\runtime\helper-1-2.toml");
        assert_eq!(
            config.with_extension("xml"),
            Path::new(r"C:\ProgramData\iran-split\runtime\helper-1-2.xml")
        );
    }
}
