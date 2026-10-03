use super::{HelperServiceError, HelperSettings};
use std::{process::Stdio, time::Duration};
use tokio::process::Command;
use tracing::{info, warn};
use uuid::Uuid;

/// Reclaim only a process launched with this helper's binary and runtime.
/// A name-only kill would also terminate the production/dev profile or an
/// unrelated proxy client. Exit 1 from Unix pkill means no process matched.
pub(crate) async fn cleanup(
    settings: &HelperSettings,
    trace_id: Uuid,
) -> Result<(), HelperServiceError> {
    info!(
        event = "helper.orphan_cleanup_started",
        section = "helper_process",
        initiator = "helper_process",
        cause = "mihomo_start",
        trace_route = "helper_command->start_mihomo->owned_orphan_cleanup",
        %trace_id,
        "checking for a Mihomo orphan in this helper's runtime"
    );
    let mut command = cleanup_command(settings);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let outcome = tokio::time::timeout(Duration::from_secs(3), command.status()).await;
    let result = match outcome {
        Ok(Ok(status)) if status.success() || (cfg!(unix) && status.code() == Some(1)) => Ok(()),
        Ok(Ok(_)) => Err(HelperServiceError::Process(
            "owned Mihomo orphan cleanup failed".into(),
        )),
        Ok(Err(_)) => Err(HelperServiceError::Process(
            "owned Mihomo orphan cleanup could not start".into(),
        )),
        Err(_) => Err(HelperServiceError::Process(
            "owned Mihomo orphan cleanup timed out".into(),
        )),
    };
    if result.is_ok() {
        info!(
            event = "helper.orphan_cleanup_completed",
            section = "helper_process",
            initiator = "helper_process",
            cause = "owned_scope_checked",
            trace_route = "helper_command->start_mihomo->owned_orphan_cleanup",
            %trace_id,
            "finished scoped Mihomo orphan cleanup"
        );
    } else {
        warn!(
            event = "helper.orphan_cleanup_failed",
            section = "helper_process",
            initiator = "helper_process",
            cause = "cleanup_command_failed",
            trace_route = "helper_command->start_mihomo->owned_orphan_cleanup",
            %trace_id,
            "Mihomo start stopped because orphan cleanup failed"
        );
    }
    result
}

fn command_pattern(settings: &HelperSettings) -> String {
    let binary = regex::escape(&settings.mihomo_binary.to_string_lossy());
    let root = regex::escape(&settings.runtime_dir.join("generations").to_string_lossy());
    let separator = regex::escape(std::path::MAIN_SEPARATOR_STR);
    let generation = format!("{root}{separator}[0-9a-f-]{{36}}");
    let config = format!("{generation}{separator}config\\.yaml");
    let argument = |value: &str| {
        if cfg!(windows) {
            format!("(\"{value}\"|{value})")
        } else {
            value.to_owned()
        }
    };
    format!(
        "^{} -d {} -f {}$",
        argument(&binary),
        argument(&generation),
        argument(&config)
    )
}

#[cfg(unix)]
fn cleanup_command(settings: &HelperSettings) -> Command {
    let mut command = Command::new("/usr/bin/pkill");
    command
        .args(["-KILL", "-f", &command_pattern(settings)])
        .kill_on_drop(true);
    command
}

#[cfg(windows)]
fn cleanup_command(settings: &HelperSettings) -> Command {
    let binary = settings.mihomo_binary.to_string_lossy().replace('\'', "''");
    let pattern = command_pattern(settings).replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         Get-CimInstance Win32_Process | Where-Object {{ \
         $_.ExecutablePath -ieq '{binary}' -and $_.CommandLine -match '{pattern}' \
         }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop }}"
    );
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .kill_on_drop(true);
    command
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::cleanup;
    use super::command_pattern;
    use crate::HelperSettings;
    use regex::Regex;
    use std::path::Path;
    use uuid::Uuid;

    fn settings(root: &Path) -> HelperSettings {
        HelperSettings {
            authorized_uid: 1_000,
            authorized_gid: 1_000,
            socket_path: root.join("helper.sock"),
            staging_dir: root.join("staging"),
            runtime_dir: root.join("runtime"),
            mihomo_binary: root.join("bin with spaces/mihomo"),
            mihomo_sha256: "a".repeat(64),
            tun_name: "biflow-test".into(),
        }
    }

    #[test]
    fn matching_image_names_do_not_cross_helper_runtimes() {
        let directory = tempfile::tempdir().expect("tempdir");
        let owned = settings(directory.path());
        let pattern = Regex::new(&command_pattern(&owned)).expect("scoped regex");
        let id = Uuid::new_v4();
        let generation = owned.runtime_dir.join("generations").join(id.to_string());
        let command = |binary: &Path, root: &Path| {
            format!(
                "{} -d {} -f {}",
                binary.display(),
                root.display(),
                root.join("config.yaml").display()
            )
        };
        assert!(pattern.is_match(&command(&owned.mihomo_binary, &generation)));
        assert!(!pattern.is_match(&command(
            &directory.path().join("another/mihomo"),
            &generation
        )));
        assert!(!pattern.is_match(&command(
            &owned.mihomo_binary,
            &directory
                .path()
                .join("other-runtime/generations")
                .join(id.to_string())
        )));
        assert!(!pattern.is_match(&command(&owned.mihomo_binary, &generation.join("extra"))));
        if cfg!(windows) {
            assert!(pattern.is_match(&format!(
                "\"{}\" -d \"{}\" -f \"{}\"",
                owned.mihomo_binary.display(),
                generation.display(),
                generation.join("config.yaml").display()
            )));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cleanup_preserves_a_process_using_the_same_executable() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut owned = settings(directory.path());
        owned.mihomo_binary = "/bin/sleep".into();
        let mut other = tokio::process::Command::new(&owned.mihomo_binary)
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .expect("unrelated process");
        cleanup(&owned, Uuid::new_v4())
            .await
            .expect("no owned orphan");
        assert!(other.try_wait().expect("process status").is_none());
        other.kill().await.expect("fixture cleanup");
    }
}
