use super::services;
use iran_split_core::PlatformBackend;
use serde::Serialize;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Manager, Runtime};
use tokio::process::Command;
#[cfg(target_os = "windows")]
use tracing::warn;
use tracing::{error, info};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
const LINUX_HELPER_ROOT: &str = "/usr/lib/biflow";
#[cfg(target_os = "linux")]
const PKEXEC: &str = "/usr/bin/pkexec";
/// `install-helper.sh` refuses `--authorized-uid 0`, so a root-owned UI can only
/// ever fail. Say why instead of forwarding that exit code as a generic failure.
#[cfg(target_os = "linux")]
const ROOT_APP_REJECTION: &str = "BiFlow is running as root, so the helper cannot be installed. The helper authorizes one non-root user. Quit BiFlow, start it as your normal user without sudo, then install the helper again.";
/// Keeps a runaway script error out of the dialog while preserving the reason.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
const MAX_DETAIL_CHARS: usize = 200;
/// `ERROR_CANCELLED`: the operator refused the UAC prompt.
#[cfg(any(target_os = "windows", test))]
const UAC_CANCELLED: i32 = 1223;
/// `CREATE_NO_WINDOW`: keeps a spawned console program from flashing a window
/// over the GUI (ADR 0030).
#[cfg(target_os = "windows")]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(target_os = "windows")]
const WINDOWS_INSTALL_LOG: &str = r"C:\ProgramData\iran-split\install.log";
/// Machine-wide generation root. NSIS `perMachine` `$LOCALAPPDATA` is
/// `C:\ProgramData`, not the user's profile, so a user-profile staging path
/// recorded at install time never matches what the desktop writes later.
#[cfg(any(target_os = "windows", test))]
pub(crate) const WINDOWS_HELPER_STAGING: &str = r"C:\ProgramData\iran-split\staging";

#[derive(Debug, Clone, Serialize)]
pub struct InstallHelperResult {
    pub installed: bool,
}

/// Installs and starts the privileged helper for the current packaged app.
///
/// This is the **only** privileged provisioning entry point. Every route that
/// can rewrite the installed helper — the Install button, the Connect flow's
/// missing-helper branch, and its version-mismatch reinstall — goes through
/// here, so the development-profile refusal lives here rather than in any
/// single caller (ADR 0115).
///
/// # Errors
///
/// Returns an error when this is a development run, when bundled files are
/// missing, when elevation fails, or when the helper does not become
/// reachable.
pub async fn install_helper<R: Runtime>(app: &AppHandle<R>) -> Result<InstallHelperResult, String> {
    // A development run gets its transient helper from `dev.sh`. Running the
    // production installer would reconfigure the machine-wide system helper
    // with development paths and break the installed app. This check is
    // deliberately inside the installer, not at the call sites: guarding only
    // the Tauri command left the Connect path free to reach it.
    crate::profile::ensure_provisioning_allowed("helper_install::install_helper")?;
    let services = services(app)?;
    let resource_root = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    let exe_dir = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .ok_or_else(|| "application executable has no parent directory".to_owned())?
        .to_path_buf();
    let tun_name = services
        .config_store
        .load_or_create()
        .map_err(|error| error.to_string())?
        .mihomo
        .tun_name;
    info!(
        event = "helper.install_started",
        section = "helper_install",
        initiator = "tauri_command",
        cause = "user_requested",
        trace_route = "ui->tauri_command->install_helper",
        "privileged helper installation requested"
    );
    #[cfg(target_os = "linux")]
    {
        let staging_dir = services.paths.data.join("runtime").join("generations");
        fs::create_dir_all(&staging_dir).map_err(|error| error.to_string())?;
        let payload_dir = services.paths.data.join("runtime").join("helper-install");
        install_linux(
            &resource_root,
            &exe_dir,
            &payload_dir,
            &staging_dir,
            &tun_name,
        )
        .await?;
    }
    #[cfg(target_os = "windows")]
    {
        // The elevated installer records this same root in `helper.toml`, so
        // SYSTEM can publish the generation. A development run never gets this
        // far, but resolve it through the same policy as the backend rather
        // than hardcoding it, so the two cannot diverge.
        let staging_dir = crate::platform_paths::windows_generation_staging(&services.paths.data);
        install_windows(&resource_root, &exe_dir, &staging_dir, &tun_name).await?;
    }
    #[cfg(target_os = "macos")]
    {
        let staging_dir = services.paths.data.join("runtime").join("generations");
        fs::create_dir_all(&staging_dir).map_err(|error| error.to_string())?;
        let payload_dir = services.paths.data.join("runtime").join("helper-install");
        install_macos(
            &resource_root,
            &exe_dir,
            &payload_dir,
            &staging_dir,
            &tun_name,
        )
        .await?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = (resource_root, exe_dir, tun_name);
        return Err("helper installation is not supported on this platform".into());
    }
    wait_for_helper(app).await?;
    info!(
        event = "helper.install_completed",
        section = "helper_install",
        initiator = "tauri_command",
        cause = "none",
        trace_route = "ui->tauri_command->install_helper->helper_ready",
        "privileged helper installation completed"
    );
    Ok(InstallHelperResult { installed: true })
}

async fn wait_for_helper<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let services = services(app)?;
    let attempts = if cfg!(target_os = "windows") { 80 } else { 50 };
    for _ in 0..attempts {
        match services.backend.helper_status().await {
            Ok(status) if status.available => {
                services.engine.refresh_health().await;
                return Ok(());
            }
            Ok(_) | Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    error!(
        event = "helper.install_timeout",
        section = "helper_install",
        initiator = "tauri_command",
        cause = "helper_not_ready",
        trace_route = "ui->tauri_command->install_helper->wait",
        "installed helper did not become ready"
    );
    #[cfg(target_os = "windows")]
    {
        let detail = windows_install_detail(&[]);
        if !detail.is_empty() {
            return Err(format!(
                "the helper was installed but is not reachable yet: {detail}"
            ));
        }
    }
    Err("the helper was installed but is not reachable yet".into())
}

#[cfg(target_os = "linux")]
async fn install_linux(
    resource_root: &Path,
    exe_dir: &Path,
    payload_dir: &Path,
    staging_dir: &Path,
    tun_name: &str,
) -> Result<(), String> {
    let (uid, gid) = current_linux_ids()?;
    if let Some(rejection) = root_install_rejection(uid) {
        error!(
            event = "helper.install_rejected",
            section = "helper_install",
            initiator = "tauri_command",
            cause = "app_running_as_root",
            trace_route = "ui->tauri_command->install_helper->install_linux",
            "refusing to authorize the helper for root"
        );
        return Err(rejection.to_owned());
    }
    let helper_src = first_existing_file(&helper_binary_candidates(resource_root, exe_dir))
        .ok_or_else(|| "packaged helper binary is missing".to_owned())?;
    let mihomo_src = first_existing_file(&mihomo_candidates(resource_root, exe_dir))
        .ok_or_else(|| "packaged Mihomo binary is missing".to_owned())?;
    let script = first_existing_file(&install_script_candidates(resource_root, exe_dir))
        .ok_or_else(|| "helper install script is missing".to_owned())?;
    let unit = first_existing_file(&unit_candidates(resource_root, exe_dir));
    let helper_sha256 = sha256_file(&helper_src)?;
    let mihomo_sha256 = sha256_file(&mihomo_src)?;
    if !Path::new(PKEXEC).is_file() {
        return Err("pkexec is not installed; install policykit-1 and retry".into());
    }
    let payload = stage_privileged_payload(
        payload_dir,
        &script,
        &helper_src,
        &mihomo_src,
        unit.as_deref(),
    )?;
    if payload.staged {
        verify_staged_payload(&payload, &helper_sha256, &mihomo_sha256).inspect_err(|_| {
            discard_staged_payload(&payload);
        })?;
    }
    let mut command = Command::new(PKEXEC);
    command
        .arg(&payload.script)
        .arg("--authorized-uid")
        .arg(uid.to_string())
        .arg("--authorized-gid")
        .arg(gid.to_string())
        .arg("--staging-dir")
        .arg(staging_dir)
        .arg("--helper-src")
        .arg(&payload.helper)
        .arg("--mihomo-src")
        .arg(&payload.mihomo)
        .arg("--helper-sha256")
        .arg(&helper_sha256)
        .arg("--mihomo-sha256")
        .arg(&mihomo_sha256)
        .arg("--tun-name")
        .arg(tun_name);
    if let Some(unit) = payload.unit.as_ref() {
        command.arg("--unit-src").arg(unit);
    }
    let elevated = command.output().await.map_err(|error| error.to_string());
    discard_staged_payload(&payload);
    let output = elevated?;
    if output.status.success() {
        return Ok(());
    }
    // pkexec exits 126 only when the operator dismissed the polkit dialog. 127
    // means it could not run the script at all, which is a real failure.
    if output.status.code() == Some(126) {
        return Err("helper installation was cancelled".into());
    }
    let detail = last_error_line(&output.stderr);
    error!(
        event = "helper.install_failed",
        section = "helper_install",
        initiator = "tauri_command",
        cause = "install_script_failed",
        trace_route = "ui->tauri_command->install_helper->install_linux",
        exit_code = output.status.code().unwrap_or(-1),
        staged = payload.staged,
        detail = %detail,
        "privileged helper installation failed"
    );
    if detail.is_empty() {
        Err("privileged helper installation failed".into())
    } else {
        Err(format!("privileged helper installation failed: {detail}"))
    }
}

/// Absolute paths the elevated script reads. They are copies whenever the
/// packaged originals live somewhere root cannot follow.
#[cfg(target_os = "linux")]
pub(crate) struct PrivilegedPayload {
    pub script: PathBuf,
    pub helper: PathBuf,
    pub mihomo: PathBuf,
    pub unit: Option<PathBuf>,
    pub staged: bool,
    root: PathBuf,
}

/// An `AppImage` mounts itself through FUSE as the calling user, and without
/// `allow_other` that mount denies every other uid — root included. `pkexec`
/// then fails with `Error accessing …: Permission denied` before the script
/// runs, so copy the payload onto a normal filesystem root can read.
///
/// A `.deb` install already sits in root-owned `/usr/lib/biflow`, and running
/// that copy keeps the polkit action's `exec.path` annotation matching, so it
/// is used as-is.
#[cfg(target_os = "linux")]
fn stage_privileged_payload(
    payload_dir: &Path,
    script: &Path,
    helper: &Path,
    mihomo: &Path,
    unit: Option<&Path>,
) -> Result<PrivilegedPayload, String> {
    if !needs_staging(script) {
        return Ok(PrivilegedPayload {
            script: script.to_path_buf(),
            helper: helper.to_path_buf(),
            mihomo: mihomo.to_path_buf(),
            unit: unit.map(Path::to_path_buf),
            staged: false,
            root: payload_dir.to_path_buf(),
        });
    }
    if payload_dir.exists() {
        fs::remove_dir_all(payload_dir).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(payload_dir).map_err(|error| error.to_string())?;
    // 0700 keeps another unprivileged user from swapping the payload between
    // the polkit prompt and the elevated read.
    fs::set_permissions(payload_dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    Ok(PrivilegedPayload {
        script: copy_payload_file(script, payload_dir, 0o755)?,
        helper: copy_payload_file(helper, payload_dir, 0o755)?,
        mihomo: copy_payload_file(mihomo, payload_dir, 0o755)?,
        unit: unit
            .map(|unit| copy_payload_file(unit, payload_dir, 0o644))
            .transpose()?,
        staged: true,
        root: payload_dir.to_path_buf(),
    })
}

/// `/usr/lib/biflow` is written by the `.deb` and owned by root.
#[cfg(target_os = "linux")]
#[must_use]
pub(crate) fn needs_staging(script: &Path) -> bool {
    !script.starts_with(LINUX_HELPER_ROOT)
}

#[cfg(target_os = "linux")]
fn copy_payload_file(source: &Path, directory: &Path, mode: u32) -> Result<PathBuf, String> {
    let name = source
        .file_name()
        .ok_or_else(|| format!("{} has no file name", source.display()))?;
    let destination = directory.join(name);
    fs::copy(source, &destination)
        .map_err(|error| format!("cannot stage {}: {error}", source.display()))?;
    fs::set_permissions(&destination, fs::Permissions::from_mode(mode))
        .map_err(|error| error.to_string())?;
    Ok(destination)
}

/// The hashes come from the packaged originals, so a copy that does not match
/// them is rejected before the operator is asked to authenticate. The elevated
/// script verifies the same digests again.
#[cfg(target_os = "linux")]
fn verify_staged_payload(
    payload: &PrivilegedPayload,
    helper_sha256: &str,
    mihomo_sha256: &str,
) -> Result<(), String> {
    if sha256_file(&payload.helper)? != helper_sha256 {
        return Err("staged helper binary failed checksum verification".into());
    }
    if sha256_file(&payload.mihomo)? != mihomo_sha256 {
        return Err("staged Mihomo binary failed checksum verification".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn discard_staged_payload(payload: &PrivilegedPayload) {
    if !payload.staged {
        return;
    }
    if let Err(error) = fs::remove_dir_all(&payload.root) {
        error!(
            event = "helper.install_staging_cleanup_failed",
            section = "helper_install",
            initiator = "tauri_command",
            cause = %error,
            trace_route = "ui->tauri_command->install_helper->install_linux",
            "could not remove the staged helper payload"
        );
    }
}

/// The helper authorizes exactly one non-root uid, so a root-owned UI can never
/// be its client. Returns the operator-facing reason when `uid` is not usable.
#[cfg(target_os = "linux")]
#[must_use]
pub(crate) fn root_install_rejection(uid: u32) -> Option<&'static str> {
    if uid == 0 {
        Some(ROOT_APP_REJECTION)
    } else {
        None
    }
}

/// `install-helper.sh` and `PowerShell` both report why they stopped on their
/// last stderr line.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
#[must_use]
pub(crate) fn last_error_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let Some(line) = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
    else {
        return String::new();
    };
    if line.chars().count() <= MAX_DETAIL_CHARS {
        return line.to_owned();
    }
    line.chars().take(MAX_DETAIL_CHARS).collect::<String>() + "…"
}

#[cfg(target_os = "windows")]
async fn install_windows(
    resource_root: &Path,
    exe_dir: &Path,
    staging_dir: &Path,
    tun_name: &str,
) -> Result<(), String> {
    let helper_src = first_existing_file(&windows_helper_candidates(resource_root, exe_dir))
        .ok_or_else(|| "packaged helper binary is missing".to_owned())?;
    let mihomo_src = first_existing_file(&windows_mihomo_candidates(resource_root, exe_dir))
        .ok_or_else(|| "packaged Mihomo binary is missing".to_owned())?;
    let helper = helper_src.to_string_lossy().into_owned();
    let mihomo = mihomo_src.to_string_lossy().into_owned();
    let staging = staging_dir.to_string_lossy().into_owned();
    let script = elevate_script(
        &helper,
        &[
            "--install",
            "--mihomo",
            &mihomo,
            "--staging-dir",
            &staging,
            "--tun-name",
            tun_name,
        ],
    );
    // The elevated helper overwrites install.log only when it reaches its own
    // error path. If it dies before that — a blocked or missing exe — a stale
    // file would report the *previous* attempt's reason, which sends the
    // operator after the wrong problem. Clear it so whatever remains is ours.
    discard_stale_install_log();
    // The desktop app is a `windows` subsystem binary (ADR 0030); without this
    // flag the elevation helper flashes a console window over the UI.
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let code = output.status.code().unwrap_or(-1);
    if code == UAC_CANCELLED {
        return Err("helper installation was cancelled".into());
    }
    let detail = windows_install_detail(&output.stderr);
    error!(
        event = "helper.install_failed",
        section = "helper_install",
        initiator = "tauri_command",
        cause = "elevated_helper_failed",
        trace_route = "ui->tauri_command->install_helper->install_windows",
        exit_code = code,
        detail = %detail,
        "privileged helper installation failed"
    );
    if detail.is_empty() {
        Err(format!(
            "privileged helper installation failed (exit code {code})"
        ))
    } else {
        Err(format!("privileged helper installation failed: {detail}"))
    }
}

/// `Start-Process -Wait` alone reports `PowerShell`'s own exit code, so a helper
/// that failed — or a dismissed UAC prompt — still looked like success and the
/// install only surfaced later as an unreachable-helper timeout. Re-raise the
/// elevated process's exit code, and map a refused prompt to `ERROR_CANCELLED`.
///
/// Pass one Windows-quoted `lpParameters` string. An `-ArgumentList` array is
/// concatenated without quoting, so `C:\Program Files\…` becomes two argv
/// tokens and clap exits 2 before `install.log` is written.
#[cfg(any(target_os = "windows", test))]
#[must_use]
pub(crate) fn elevate_script(program: &str, arguments: &[&str]) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'; \
         try {{ $process = Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -Wait -PassThru }} \
         catch {{ Write-Error $_.Exception.Message; exit {UAC_CANCELLED} }}; \
         exit $process.ExitCode",
        powershell_quote(program),
        powershell_quote(&windows_command_line(arguments)),
    )
}

#[cfg(any(target_os = "windows", test))]
fn windows_command_line(arguments: &[&str]) -> String {
    arguments
        .iter()
        .map(|argument| windows_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quote one argv token the way `CommandLineToArgvW` (and clap) expect.
#[cfg(any(target_os = "windows", test))]
fn windows_quote(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".into();
    }
    let needs_quotes = value
        .chars()
        .any(|character| character == ' ' || character == '\t' || character == '"');
    if !needs_quotes {
        return value.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0_usize;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes.saturating_mul(2).saturating_add(1)));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes.saturating_mul(2)));
    quoted.push('"');
    quoted
}

#[cfg(target_os = "linux")]
fn helper_binary_candidates(resource_root: &Path, exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from(LINUX_HELPER_ROOT).join("iran-split-helper"),
        resource_root.join("helper/iran-split-helper"),
        exe_dir.join("helper/iran-split-helper"),
        resource_root.join("_up_/resources/helper/iran-split-helper"),
    ]
}

#[cfg(target_os = "linux")]
fn mihomo_candidates(resource_root: &Path, exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from(LINUX_HELPER_ROOT).join("mihomo"),
        resource_root.join("dependencies/mihomo"),
        exe_dir.join("dependencies/mihomo"),
        resource_root.join("_up_/resources/dependencies/mihomo"),
    ]
}

#[cfg(target_os = "linux")]
fn install_script_candidates(resource_root: &Path, exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from(LINUX_HELPER_ROOT).join("install-helper.sh"),
        resource_root.join("helper/install-helper.sh"),
        exe_dir.join("helper/install-helper.sh"),
        resource_root.join("_up_/resources/helper/install-helper.sh"),
    ]
}

#[cfg(target_os = "linux")]
fn unit_candidates(resource_root: &Path, exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from(LINUX_HELPER_ROOT).join("iran-split-helper.service"),
        resource_root.join("helper/iran-split-helper.service"),
        exe_dir.join("helper/iran-split-helper.service"),
        resource_root.join("_up_/resources/helper/iran-split-helper.service"),
    ]
}

/// Root-owned helper + Mihomo location on macOS. A launchd daemon runs as root
/// and reads its configuration from here.
#[cfg(target_os = "macos")]
const MACOS_HELPER_ROOT: &str = "/Library/Application Support/BiFlow";
/// The launchd daemon plist lives in the system domain.
#[cfg(target_os = "macos")]
const MACOS_PLIST_PATH: &str = "/Library/LaunchDaemons/app.biflow.helper.plist";
/// The daemon label launchd uses to track the helper.
#[cfg(target_os = "macos")]
const MACOS_HELPER_LABEL: &str = "app.biflow.helper";

#[cfg(target_os = "macos")]
fn macos_helper_candidates(resource_root: &Path, exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        resource_root.join("helper/iran-split-helper"),
        exe_dir.join("helper/iran-split-helper"),
        exe_dir.join("iran-split-helper"),
        resource_root.join("_up_/resources/helper/iran-split-helper"),
    ]
}

#[cfg(target_os = "macos")]
fn macos_mihomo_candidates(resource_root: &Path, exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        resource_root.join("dependencies/mihomo"),
        exe_dir.join("dependencies/mihomo"),
        resource_root.join("_up_/resources/dependencies/mihomo"),
    ]
}

/// Installs the privileged helper on macOS via a launchd daemon.
///
/// The desktop cannot write under `/Library` without elevation, so it stages
/// the helper binary, Mihomo, a launchd plist, and an install shell script
/// into the user's profile, then runs the script through `osascript` with
/// administrator privileges. The script copies the payload into
/// `/Library/Application Support/BiFlow`, writes `helper.toml` (root-owned),
/// installs the plist, and bootstraps the daemon with `launchctl`.
#[cfg(target_os = "macos")]
async fn install_macos(
    resource_root: &Path,
    exe_dir: &Path,
    payload_dir: &Path,
    staging_dir: &Path,
    tun_name: &str,
) -> Result<(), String> {
    let (uid, gid) = current_macos_ids()?;
    let helper_src = first_existing_file(&macos_helper_candidates(resource_root, exe_dir))
        .ok_or_else(|| "packaged helper binary is missing".to_owned())?;
    let mihomo_src = first_existing_file(&macos_mihomo_candidates(resource_root, exe_dir))
        .ok_or_else(|| "packaged Mihomo binary is missing".to_owned())?;

    if payload_dir.exists() {
        fs::remove_dir_all(payload_dir).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(payload_dir).map_err(|error| error.to_string())?;
    // 0700 keeps another unprivileged user from swapping the payload before
    // the elevated install reads it.
    fs::set_permissions(payload_dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let helper_stage = copy_payload_file_macos(&helper_src, payload_dir, 0o755)?;
    let mihomo_stage = copy_payload_file_macos(&mihomo_src, payload_dir, 0o755)?;

    // The helper validates `mihomo_sha256` against the installed Mihomo
    // binary on startup and refuses to serve with an empty hash
    // (`UnsafeConfig("mihomo_sha256 must be lowercase SHA-256")`). Compute
    // the real digest of the staged Mihomo copy so the daemon can boot.
    let mihomo_sha256 = sha256_file(&mihomo_stage)?;
    let helper_toml = format!(
        "authorized_uid = {uid}\nauthorized_gid = {gid}\nsocket_path = \
         \"{socket_path}\"\nstaging_dir = \
         \"{staging_dir}\"\nruntime_dir = \
         \"{runtime_dir}\"\nmihomo_binary = \
         \"{mihomo_binary}\"\nmihomo_sha256 = \
         \"{mihomo_sha256}\"\ntun_name = \"{tun_name}\"\n",
        socket_path = MACOS_HELPER_ROOT.to_owned() + "/helper.sock",
        staging_dir = staging_dir.display(),
        runtime_dir = MACOS_HELPER_ROOT.to_owned() + "/runtime",
        mihomo_binary = MACOS_HELPER_ROOT.to_owned() + "/mihomo",
    );
    let helper_toml_stage = payload_dir.join("helper.toml");
    fs::write(&helper_toml_stage, helper_toml).map_err(|error| error.to_string())?;

    let plist = launchd_plist();
    let plist_stage = payload_dir.join("app.biflow.helper.plist");
    fs::write(&plist_stage, plist).map_err(|error| error.to_string())?;

    let install_script = payload_dir.join("install-helper.sh");
    let script = format!(
        "set -e\n\
         mkdir -p '{MACOS_HELPER_ROOT}' '{MACOS_HELPER_ROOT}/runtime'\n\
         cp -f {helper} '{MACOS_HELPER_ROOT}/iran-split-helper'\n\
         cp -f {mihomo} '{MACOS_HELPER_ROOT}/mihomo'\n\
         config_tmp=$(mktemp '{MACOS_HELPER_ROOT}/helper.toml.XXXXXX')\n\
         trap 'rm -f \"$config_tmp\"' EXIT\n\
         cp -f {toml} \"$config_tmp\"\n\
         chmod 600 \"$config_tmp\"\n\
         chown root:wheel \"$config_tmp\"\n\
         mv -f \"$config_tmp\" '{MACOS_HELPER_ROOT}/helper.toml'\n\
         chmod 755 '{MACOS_HELPER_ROOT}/iran-split-helper' \
         '{MACOS_HELPER_ROOT}/mihomo'\n\
         chown root:wheel '{MACOS_HELPER_ROOT}/iran-split-helper' \
         '{MACOS_HELPER_ROOT}/mihomo' '{MACOS_HELPER_ROOT}/helper.toml'\n\
         cp -f {plist} '{MACOS_PLIST_PATH}'\n\
         chown root:wheel '{MACOS_PLIST_PATH}'\n\
         chmod 644 '{MACOS_PLIST_PATH}'\n\
         launchctl bootout system/{MACOS_HELPER_LABEL} 2>/dev/null || true\n\
         launchctl bootstrap system '{MACOS_PLIST_PATH}'\n\
         launchctl enable system/{MACOS_HELPER_LABEL}\n",
        helper = macos_shell_quote(&helper_stage.to_string_lossy()),
        mihomo = macos_shell_quote(&mihomo_stage.to_string_lossy()),
        toml = macos_shell_quote(&helper_toml_stage.to_string_lossy()),
        plist = macos_shell_quote(&plist_stage.to_string_lossy()),
    );
    fs::write(&install_script, script).map_err(|error| error.to_string())?;
    fs::set_permissions(&install_script, fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;

    // The payload path lives under `~/Library/Application Support/biflow/...`,
    // which contains a space. `do shell script` runs through sh, so the
    // path must be single-quoted or sh splits it at the space and tries to
    // execute `/Users/.../Library/Application` (which does not exist).
    let apple_script = macos_admin_applescript(&install_script);
    let output = Command::new("osascript")
        .args(["-e", &apple_script])
        .output()
        .await
        .map_err(|error| error.to_string())?;
    // Discard the staged payload regardless of outcome.
    if let Err(error) = fs::remove_dir_all(payload_dir) {
        tracing::warn!(event = "helper.payload_cleanup_failed", section = "helper_install",
            initiator = "tauri_command", cause = ?error.kind(),
            trace_route = "ui->install_helper->install_macos->payload_cleanup",
            "helper payload cleanup failed after installation");
    }
    if output.status.success() {
        return Ok(());
    }
    // osascript exits -128 when the operator dismissed the admin prompt.
    if output.status.code() == Some(-128) {
        return Err("helper installation was cancelled".into());
    }
    let detail = last_error_line(&output.stderr);
    error!(
        event = "helper.install_failed",
        section = "helper_install",
        initiator = "tauri_command",
        cause = "install_script_failed",
        trace_route = "ui->tauri_command->install_helper->install_macos",
        exit_code = output.status.code().unwrap_or(-1),
        "privileged helper installation failed"
    );
    if detail.is_empty() {
        Err("privileged helper installation failed".into())
    } else {
        Err(detail)
    }
}

/// Builds the `do shell script ... with administrator privileges` `AppleScript`
/// that elevates the staged install script. The payload path lives under
/// `~/Library/Application Support/biflow/...`, which contains a space, so the
/// path must be single-quoted or sh splits it at the space and tries to
/// execute `/Users/.../Library/Application` (which does not exist).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_admin_applescript(script_path: &Path) -> String {
    let command = format!("sh {}", macos_shell_quote(&script_path.to_string_lossy()));
    let literal = serde_json::to_string(&command).expect("serializing a string cannot fail");
    format!("do shell script {literal} with administrator privileges")
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn copy_payload_file_macos(source: &Path, directory: &Path, mode: u32) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let name = source
        .file_name()
        .ok_or_else(|| format!("{} has no file name", source.display()))?;
    let destination = directory.join(name);
    fs::copy(source, &destination)
        .map_err(|error| format!("cannot stage {}: {error}", source.display()))?;
    fs::set_permissions(&destination, fs::Permissions::from_mode(mode))
        .map_err(|error| error.to_string())?;
    Ok(destination)
}

#[cfg(target_os = "macos")]
fn launchd_plist() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key><string>{MACOS_HELPER_LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{MACOS_HELPER_ROOT}/iran-split-helper</string>\n\
         \t\t<string>--config</string>\n\
         \t\t<string>{MACOS_HELPER_ROOT}/helper.toml</string>\n\
         \t</array>\n\
         \t<key>RunAtLoad</key><true/>\n\
         \t<key>KeepAlive</key><true/>\n\
         \t<key>StandardOutPath</key><string>{MACOS_HELPER_ROOT}/helper.log</string>\n\
         \t<key>StandardErrorPath</key><string>{MACOS_HELPER_ROOT}/helper.log</string>\n\
         </dict>\n\
         </plist>\n",
    )
}

/// Returns the current user's UID and primary GID on macOS. The desktop runs
/// as the normal user, so these are the credentials the helper authorizes.
#[cfg(target_os = "macos")]
fn current_macos_ids() -> Result<(u32, u32), String> {
    let uid = id_output("-u")?;
    let gid = id_output("-g")?;
    Ok((uid, gid))
}

#[cfg(target_os = "macos")]
fn id_output(flag: &str) -> Result<u32, String> {
    let output = std::process::Command::new("id")
        .arg(flag)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("could not read the current user id".into());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|_| "current user id is not a number".to_owned())
}

#[cfg(target_os = "windows")]
fn windows_helper_candidates(resource_root: &Path, exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        resource_root.join("helper").join("iran-split-helper.exe"),
        exe_dir.join("helper").join("iran-split-helper.exe"),
        exe_dir.join("iran-split-helper.exe"),
    ]
}

#[cfg(target_os = "windows")]
fn windows_mihomo_candidates(resource_root: &Path, exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        resource_root.join("dependencies").join("mihomo.exe"),
        exe_dir.join("dependencies").join("mihomo.exe"),
    ]
}

#[must_use]
pub(crate) fn first_existing_file(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[cfg(target_os = "linux")]
fn current_linux_ids() -> Result<(u32, u32), String> {
    let status = fs::read_to_string("/proc/self/status").map_err(|error| error.to_string())?;
    parse_proc_status_ids(&status).ok_or_else(|| "could not read process uid/gid".into())
}

#[cfg(target_os = "linux")]
#[must_use]
pub(crate) fn parse_proc_status_ids(status: &str) -> Option<(u32, u32)> {
    let mut uid = None;
    let mut gid = None;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            uid = rest.split_whitespace().next()?.parse().ok();
        }
        if let Some(rest) = line.strip_prefix("Gid:") {
            gid = rest.split_whitespace().next()?.parse().ok();
        }
    }
    Some((uid?, gid?))
}

#[cfg(any(target_os = "windows", test))]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// GUI-subsystem helpers leave stderr empty. Prefer the last line of
/// Removes a previous attempt's install log so a stale line is never reported
/// as the reason for this one.
#[cfg(target_os = "windows")]
fn discard_stale_install_log() {
    match fs::remove_file(WINDOWS_INSTALL_LOG) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!(
            event = "helper.install_log_stale",
            section = "helper_install",
            initiator = "tauri_command",
            cause = %error,
            trace_route = "ui->tauri_command->install_helper->install_windows",
            "could not clear the previous install log; a stale reason may be reported"
        ),
    }
}

/// `install.log` written by the elevated process.
#[cfg(target_os = "windows")]
fn windows_install_detail(stderr: &[u8]) -> String {
    let from_log = fs::read_to_string(WINDOWS_INSTALL_LOG)
        .ok()
        .map(|text| last_error_line(text.as_bytes()))
        .unwrap_or_default();
    if from_log.is_empty() {
        last_error_line(stderr)
    } else {
        from_log
    }
}

#[cfg(test)]
mod tests {
    use super::first_existing_file;
    use std::fs;

    use super::{elevate_script, UAC_CANCELLED};
    #[cfg(target_os = "linux")]
    use super::{
        helper_binary_candidates, needs_staging, parse_proc_status_ids, root_install_rejection,
        stage_privileged_payload, LINUX_HELPER_ROOT,
    };
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use super::{last_error_line, MAX_DETAIL_CHARS};
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(target_os = "linux")]
    use std::path::{Path, PathBuf};

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_proc_status_ids_reads_real_and_effective() {
        let sample = "Uid:\t1000\t1000\t1000\t1000\nGid:\t1001\t1001\t1001\t1001\n";
        assert_eq!(parse_proc_status_ids(sample), Some((1000, 1001)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn helper_candidates_prefer_system_install_root() {
        let resource = PathBuf::from("/tmp/resources");
        let exe = PathBuf::from("/tmp/app");
        let candidates = helper_binary_candidates(&resource, &exe);
        assert_eq!(
            candidates[0],
            PathBuf::from(LINUX_HELPER_ROOT).join("iran-split-helper")
        );
        assert!(candidates
            .iter()
            .any(|path| path.ends_with("helper/iran-split-helper")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn root_cannot_be_the_authorized_helper_user() {
        let rejection = root_install_rejection(0).expect("root is rejected");
        assert!(rejection.contains("running as root"));
        assert!(rejection.contains("without sudo"));
        assert!(root_install_rejection(1000).is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_a_root_owned_script_skips_staging() {
        assert!(!needs_staging(Path::new(
            "/usr/lib/biflow/install-helper.sh"
        )));
        assert!(needs_staging(Path::new(
            "/tmp/.mount_BiFlowAgjuzm/usr/lib/BiFlow/helper/install-helper.sh"
        )));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn staging_copies_the_payload_out_of_an_unreadable_mount() {
        let source = tempfile::tempdir().expect("tempdir");
        let destination = tempfile::tempdir().expect("tempdir");
        let payload_dir = destination.path().join("helper-install");
        for (name, body) in [
            ("install-helper.sh", "#!/bin/sh\n"),
            ("iran-split-helper", "helper"),
            ("mihomo", "mihomo"),
            ("iran-split-helper.service", "[Unit]\n"),
        ] {
            fs::write(source.path().join(name), body).expect("write");
        }
        let unit = source.path().join("iran-split-helper.service");
        let payload = stage_privileged_payload(
            &payload_dir,
            &source.path().join("install-helper.sh"),
            &source.path().join("iran-split-helper"),
            &source.path().join("mihomo"),
            Some(&unit),
        )
        .expect("stage");

        assert!(payload.staged);
        assert_eq!(payload.script, payload_dir.join("install-helper.sh"));
        assert_eq!(
            payload.unit,
            Some(payload_dir.join("iran-split-helper.service"))
        );
        assert_eq!(
            fs::read_to_string(&payload.helper).expect("read staged helper"),
            "helper"
        );
        let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode(&payload_dir), 0o700);
        assert_eq!(mode(&payload.script), 0o755);
        assert_eq!(mode(&payload.mihomo), 0o755);
        assert_eq!(mode(payload.unit.as_ref().expect("unit")), 0o644);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_deb_install_runs_the_root_owned_script_in_place() {
        let destination = tempfile::tempdir().expect("tempdir");
        let script = PathBuf::from(LINUX_HELPER_ROOT).join("install-helper.sh");
        let payload = stage_privileged_payload(
            &destination.path().join("helper-install"),
            &script,
            &PathBuf::from(LINUX_HELPER_ROOT).join("iran-split-helper"),
            &PathBuf::from(LINUX_HELPER_ROOT).join("mihomo"),
            None,
        )
        .expect("stage");

        assert!(!payload.staged);
        assert_eq!(payload.script, script);
        assert!(payload.unit.is_none());
        assert!(!destination.path().join("helper-install").exists());
    }

    #[test]
    fn elevation_quotes_program_files_as_one_command_line() {
        let script = elevate_script(
            r"C:\Program Files\BiFlow\helper\iran-split-helper.exe",
            &[
                "--install",
                "--mihomo",
                r"C:\Program Files\BiFlow\dependencies\mihomo.exe",
                "--staging-dir",
                r"C:\Users\name\AppData\Local\biflow\runtime\generations",
                "--tun-name",
                "clash-iran",
            ],
        );
        assert!(script.contains("-PassThru"));
        assert!(script.contains("exit $process.ExitCode"));
        assert!(script.contains(&format!("exit {UAC_CANCELLED}")));
        assert!(script.contains("$ErrorActionPreference = 'Stop'"));
        assert!(!script.contains("-ArgumentList @("));
        assert!(
            script.contains(r#""C:\Program Files\BiFlow\dependencies\mihomo.exe""#),
            "spaced path must appear Windows-quoted inside the command line: {script}"
        );
        assert!(
            script.contains("-ArgumentList '--install --mihomo "),
            "Start-Process must receive one quoted command line: {script}"
        );
        assert!(script.contains(r"'C:\Program Files\BiFlow\helper\iran-split-helper.exe'"));
    }

    #[test]
    fn elevation_escapes_single_quotes_in_paths() {
        let script = elevate_script(r"C:\Users\o'brien\helper.exe", &["--install"]);
        assert!(script.contains(r"'C:\Users\o''brien\helper.exe'"));
    }

    #[test]
    fn windows_elevation_sources_are_packaged_not_programdata() {
        let source = include_str!("helper_install.rs");
        let helper_start = source
            .find("fn windows_helper_candidates")
            .expect("helper candidates");
        let mihomo_start = source
            .find("fn windows_mihomo_candidates")
            .expect("mihomo candidates");
        let first_existing = source
            .find("pub(crate) fn first_existing_file")
            .expect("first_existing_file");
        let helpers = &source[helper_start..mihomo_start];
        let mihomo = &source[mihomo_start..first_existing];
        assert!(!helpers.contains("WINDOWS_PROGRAMDATA_HELPER"));
        assert!(!helpers.contains(r"C:\\ProgramData"));
        assert!(!mihomo.contains(r"C:\\ProgramData"));
        let elevate_start = source.find("fn elevate_script").expect("elevate_script");
        let elevate_end = source[elevate_start..]
            .find("fn windows_command_line")
            .expect("windows_command_line")
            + elevate_start;
        let elevate = &source[elevate_start..elevate_end];
        assert!(
            !elevate.contains("-ArgumentList @("),
            "elevate_script must not pass a PowerShell argument array"
        );
        assert!(elevate.contains("windows_command_line"));
    }

    #[test]
    fn nsis_hook_stages_into_programdata_iran_split() {
        let hook = include_str!("../../packaging/windows/installer-hooks.nsh");
        assert!(hook.contains(r"$PROGRAMDATA\iran-split\staging"));
        assert_eq!(
            super::WINDOWS_HELPER_STAGING,
            r"C:\ProgramData\iran-split\staging"
        );
        assert!(!hook.contains(r"$LOCALAPPDATA\biflow\runtime\generations"));
        assert!(!hook.contains("$COMMONPROGRAMDATA"));
        let source = include_str!("helper_install.rs");
        assert!(source.contains("PathBuf::from(WINDOWS_HELPER_STAGING)"));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn last_error_line_reports_the_final_script_message() {
        assert_eq!(
            last_error_line(b"install -d ...\nauthorized-uid must not be root\n\n"),
            "authorized-uid must not be root"
        );
        assert_eq!(last_error_line(b"   \n\n"), "");
        let long = last_error_line(&vec![b'x'; MAX_DETAIL_CHARS + 50]);
        assert_eq!(long.chars().count(), MAX_DETAIL_CHARS + 1);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn first_existing_file_skips_missing_paths() {
        let directory = tempfile::tempdir().expect("tempdir");
        let missing = directory.path().join("missing");
        let present = directory.path().join("present");
        fs::write(&present, b"ok").expect("write");
        assert_eq!(
            first_existing_file(&[missing, present.clone()]),
            Some(present)
        );
    }

    #[test]
    fn macos_admin_applescript_quotes_spaced_paths() {
        // The payload path lives under `~/Library/Application Support/biflow/...`
        // which contains a space. The AppleScript must single-quote it or sh
        // splits the path at the space and the helper install fails with
        // `sh: /Users/.../Library/Application: No such file or directory`.
        let script = super::macos_admin_applescript(std::path::Path::new(
            "/Users/omid/Library/Application Support/biflow/runtime/helper-install/install-helper.sh",
        ));
        assert!(
            script.contains("sh '/Users/omid/Library/Application Support/biflow/runtime/helper-install/install-helper.sh'"),
            "spaced payload path must be single-quoted inside `do shell script`: {script}"
        );
        assert!(script.contains("with administrator privileges"));
    }

    #[test]
    fn macos_elevation_escapes_shell_and_applescript_metacharacters() {
        let path =
            std::path::Path::new("/Users/owner's profile/quoted\"name/back\\slash/install.sh");
        let script = super::macos_admin_applescript(path);
        let literal = script
            .strip_prefix("do shell script ")
            .expect("command")
            .strip_suffix(" with administrator privileges")
            .expect("elevation");
        let shell: String = serde_json::from_str(literal).expect("escaped AppleScript string");
        assert_eq!(
            shell,
            "sh '/Users/owner'\\''s profile/quoted\"name/back\\slash/install.sh'"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_install_script_uses_modern_launchctl_service_targets() {
        // Modern `launchctl` (macOS 10.10+) takes a service-target of the
        // form `<domain-target>/<service-id>` (e.g. `system/app.biflow.helper`),
        // not `<domain> <service-id>`. The space form fails with
        // `Usage: launchctl enable <service-target>`.
        let source = include_str!("helper_install.rs");
        let script_start = source
            .find("let script = format!(\n        \"set -e\\n\\")
            .expect("install script template");
        let script_end = source[script_start..]
            .find("fs::write(&install_script")
            .expect("end of script template")
            + script_start;
        let script = &source[script_start..script_end];
        assert!(
            script.contains("launchctl bootout system/{MACOS_HELPER_LABEL}"),
            "bootout must use the system/<service-id> service-target form"
        );
        assert!(
            script.contains("launchctl enable system/{MACOS_HELPER_LABEL}"),
            "enable must use the system/<service-id> service-target form"
        );
        assert!(
            !script.contains("launchctl enable system '{MACOS_HELPER_LABEL}'"),
            "enable must not use the legacy `system <service-id>` space form"
        );
    }
}
