//! Install a verified DMG synchronously; hand only the relaunch to a user process.

use crate::helper_install::macos_shell_quote;
use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(target_os = "macos")]
use tracing::{info, warn};

fn destination(current_exe: &Path) -> Result<PathBuf, String> {
    let bundle = current_exe
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or("the running executable is not inside an application bundle")?;
    if bundle.starts_with("/Volumes")
        || bundle
            .components()
            .any(|part| part.as_os_str() == "AppTranslocation")
    {
        return Ok(PathBuf::from("/Applications/BiFlow.app"));
    }
    if !bundle.starts_with("/") {
        return Err("the installed application location must be absolute".into());
    }
    Ok(bundle.to_path_buf())
}

fn install_script(source: &Path, target: &Path, version: &str) -> String {
    format!(
        r#"#!/bin/sh
set -eu
source={source}
target={target}
version={version}
validate() {{
    [ -d "$1" ] && [ ! -L "$1" ] &&
    [ -x "$1/Contents/MacOS/iran-split-desktop" ] &&
    [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$1/Contents/Info.plist")" = 'iran-split-desktop' ] &&
    [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$1/Contents/Info.plist")" = 'app.biflow.desktop' ] &&
    [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$1/Contents/Info.plist")" = "$version" ]
}}
validate "$source" || exit 65
[ ! -L "$target" ] || exit 66
parent=$(/usr/bin/dirname "$target")
[ -d "$parent" ] || exit 66
work=$(/usr/bin/mktemp -d "$parent/.biflow-update.XXXXXX")
backup="$work/previous.app"
committed=0
cleanup() {{
    result=$?
    trap - EXIT HUP INT TERM
    if [ "$committed" = 1 ]; then
        /bin/rm -rf "$work" || exit 74
    elif [ -e "$backup" ]; then
        # Never delete the only recoverable copy if either rollback rename fails.
        if [ -e "$target" ]; then
            /bin/mv "$target" "$work/failed.app" || exit 74
        fi
        /bin/mv "$backup" "$target" || exit 74
        /bin/rm -rf "$work" || exit 74
    else
        /bin/rm -rf "$work" || exit 74
    fi
    exit "$result"
}}
trap cleanup EXIT
trap 'exit 74' HUP INT TERM
/usr/bin/ditto "$source" "$work/BiFlow.app"
validate "$work/BiFlow.app" || exit 65
if [ -e "$target" ]; then /bin/mv "$target" "$backup"; fi
/bin/mv "$work/BiFlow.app" "$target"
validate "$target" || exit 65
committed=1
"#,
        source = macos_shell_quote(&source.to_string_lossy()),
        target = macos_shell_quote(&target.to_string_lossy()),
        version = macos_shell_quote(version),
    )
}

fn relaunch_script(pid: u32, target: &Path, log: &Path, trace_id: uuid::Uuid) -> String {
    format!(
        r#"#!/bin/sh
target={target}
log={log}
audit() {{
    level=ERROR
    if [ "$1" = launch_requested ]; then level=INFO; fi
    printf '{{"timestamp":"%s","level":"%s","event":"update.relaunch_result","section":"updates","initiator":"user_relaunch_helper","cause":"%s","trace_route":"update->wait_old_pid->open","trace_id":"{trace_id}"}}\n' "$(/bin/date -u +'%Y-%m-%dT%H:%M:%SZ')" "$level" "$1" >> "$log"
}}
count=0
while /bin/kill -0 {pid} 2>/dev/null; do
    count=$((count + 1))
    if [ "$count" -ge 120 ]; then audit old_process_still_running; exit 1; fi
    /bin/sleep 1
done
if /usr/bin/open -n "$target"; then audit launch_requested; else audit launch_failed; exit 1; fi
"#,
        target = macos_shell_quote(&target.to_string_lossy()),
        log = macos_shell_quote(&log.to_string_lossy()),
    )
}

#[cfg(target_os = "macos")]
pub async fn install(
    package: &Path,
    current_exe: &Path,
    version: &str,
    trace_id: uuid::Uuid,
) -> Result<(), String> {
    let target = destination(current_exe)?;
    let log = PathBuf::from(crate::diagnostics::status()?.path);
    let directory = tempfile::Builder::new()
        .prefix("biflow-update-")
        .tempdir()
        .map_err(|_| "could not create a private update directory")?;
    let mount = directory.path().join("image");
    std::fs::create_dir(&mount).map_err(|_| "could not prepare the disk image mount")?;
    // A dropped install future must not make TempDir recurse into a mounted
    // image. Opt out before starting attach, including cancellation paths.
    let directory = directory.keep();
    info!(
        trace_id = %trace_id,
        event = "update.dmg_mount_started",
        section = "updates",
        initiator = "install_update",
        cause = "verified_package",
        trace_route = "update->hdiutil->install_bundle",
        "mounting verified update image"
    );
    let attached = attach_image(package, &mount).await;
    // Even an attach failure can leave a mounted device. Never recursively
    // remove the mount root unless detach has positively succeeded.
    let result = match attached {
        Ok(output) if output.status.success() => {
            replace_bundle(&directory, &mount.join("BiFlow.app"), &target, version).await
        }
        failure => {
            warn!(trace_id = %trace_id, event = "update.dmg_mount_failed",
                section = "updates", initiator = "install_update",
                cause = if failure.is_err() { "attach_execution_failed" } else { "attach_rejected" },
                exit_code = failure.ok().and_then(|output| output.status.code()),
                trace_route = "update->hdiutil->attach",
                "could not attach the verified disk image");
            Err("could not mount the verified update disk image".into())
        }
    };
    cleanup_mount(&directory, &mount, trace_id).await;
    result?;
    schedule_relaunch(&target, &log, trace_id)
}

#[cfg(target_os = "macos")]
async fn attach_image(package: &Path, mount: &Path) -> std::io::Result<std::process::Output> {
    let mut command = tokio::process::Command::new("/usr/bin/hdiutil");
    command
        .args([
            "attach",
            "-readonly",
            "-nobrowse",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(mount)
        .arg(package)
        .kill_on_drop(true);
    disk_command(command.output(), Duration::from_secs(60)).await
}

async fn disk_command<T>(
    command: impl std::future::Future<Output = std::io::Result<T>>,
    budget: Duration,
) -> std::io::Result<T> {
    tokio::time::timeout(budget, command).await.map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "disk image command timed out")
    })?
}

#[cfg(target_os = "macos")]
async fn cleanup_mount(directory: &Path, mount: &Path, trace_id: uuid::Uuid) {
    let mut command = tokio::process::Command::new("/usr/bin/hdiutil");
    command.arg("detach").arg(mount).kill_on_drop(true);
    let detached = disk_command(command.output(), Duration::from_secs(20)).await;
    if detached.is_ok_and(|output| output.status.success()) {
        if std::fs::remove_dir_all(directory).is_err() {
            warn!(trace_id = %trace_id, event = "update.staging_cleanup_failed",
                section = "updates", initiator = "install_update",
                cause = "unmounted_staging_retained", trace_route = "update->detach->cleanup",
                "could not remove unmounted update staging");
        }
    } else {
        warn!(trace_id = %trace_id, event = "update.dmg_detach_failed",
            section = "updates", initiator = "install_update",
            cause = "mount_retained", trace_route = "update->hdiutil->detach",
            "update mount retained because detachment was not confirmed");
    }
}

#[cfg(target_os = "macos")]
fn schedule_relaunch(target: &Path, log: &Path, trace_id: uuid::Uuid) -> Result<(), String> {
    // This stays in the desktop user's session, never in the elevated shell.
    let mut launcher = tokio::process::Command::new("/usr/bin/nohup");
    launcher.args(["/bin/sh", "-c"]).arg(relaunch_script(
        std::process::id(),
        target,
        log,
        trace_id,
    ));
    launcher
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    launcher.spawn().map_err(|_| {
        "the new application is installed, but relaunch could not start; quit and reopen BiFlow"
    })?;
    info!(
        trace_id = %trace_id,
        event = "update.bundle_installed",
        section = "updates",
        initiator = "install_update",
        cause = "bundle_verified_relaunch_scheduled",
        trace_route = "update->replace_bundle->user_relaunch",
        "application replacement verified; relaunch waits for the old process to exit"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
async fn replace_bundle(
    directory: &Path,
    source: &Path,
    target: &Path,
    version: &str,
) -> Result<(), String> {
    let script = directory.join("install.sh");
    std::fs::write(&script, install_script(source, target, version))
        .map_err(|_| "could not stage the application installer")?;
    let output = tokio::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(crate::helper_install::macos_admin_applescript(&script))
        .output()
        .await
        .map_err(|_| "could not start application installation authorization")?;
    if output.status.success() {
        Ok(())
    } else if String::from_utf8_lossy(&output.stderr).contains("(-128)") {
        Err("application installation was cancelled".into())
    } else {
        Err("application replacement failed; the previous copy was preserved for recovery".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disk_command_preserves_failures_and_bounds_a_stalled_operation() {
        let result = disk_command(
            async { Ok::<_, std::io::Error>(42) },
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(result.unwrap(), 42);
        let refused = disk_command(
            async { Err::<(), _>(std::io::Error::from(std::io::ErrorKind::PermissionDenied)) },
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(
            refused.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let stalled = disk_command(
            std::future::pending::<std::io::Result<()>>(),
            Duration::ZERO,
        )
        .await;
        assert_eq!(stalled.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn installed_location_is_preserved_but_disk_images_are_not_destinations() {
        for (executable, expected) in [
            ("/Applications/BiFlow.app/Contents/MacOS/iran-split-desktop", "/Applications/BiFlow.app"),
            ("/Users/example/My Apps/BiFlow.app/Contents/MacOS/iran-split-desktop", "/Users/example/My Apps/BiFlow.app"),
            ("/Volumes/BiFlow/BiFlow.app/Contents/MacOS/iran-split-desktop", "/Applications/BiFlow.app"),
            ("/private/var/folders/example/AppTranslocation/token/d/BiFlow.app/Contents/MacOS/iran-split-desktop", "/Applications/BiFlow.app"),
        ] {
            assert_eq!(destination(Path::new(executable)).unwrap(), Path::new(expected));
        }
        assert!(destination(Path::new("/tmp/iran-split-desktop")).is_err());
    }

    #[test]
    fn scripts_quote_user_paths_and_wait_for_the_old_process() {
        let script = install_script(
            Path::new("/tmp/BiFlow's image/BiFlow.app"),
            Path::new("/Applications/BiFlow.app"),
            "6.2.82",
        );
        assert!(script.contains("BiFlow'\\''s image"));
        let relaunch = relaunch_script(
            42,
            Path::new("/Users/example/My Apps/BiFlow.app"),
            Path::new("/tmp/debug.log"),
            uuid::Uuid::nil(),
        );
        assert!(
            relaunch.find("/bin/kill -0 42").unwrap() < relaunch.find("/usr/bin/open -n").unwrap()
        );
        assert!(relaunch.contains("launch_failed"));
        assert!(!relaunch.contains("administrator privileges"));
    }

    #[cfg(target_os = "macos")]
    fn bundle(root: &Path, name: &str, version: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let app = root.join(name);
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let executable = app.join("Contents/MacOS/iran-split-desktop");
        std::fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), format!(r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>iran-split-desktop</string><key>CFBundleIdentifier</key><string>app.biflow.desktop</string><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>"#)).unwrap();
        app
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_install_validates_before_replacing_and_rolls_back_failed_swaps() {
        let directory = tempfile::tempdir().unwrap();
        let source = bundle(directory.path(), "BiFlow's source.app", "6.2.82");
        let target = bundle(directory.path(), "Installed BiFlow.app", "6.2.81");
        std::fs::write(target.join("old-marker"), "old").unwrap();
        let run = |script: &str| {
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .output()
                .unwrap()
        };
        let script = install_script(&source, &target, "0.0.0");
        assert!(!run(&script).status.success());
        assert!(target.join("old-marker").exists());
        let script = install_script(&source, &target, "6.2.82");
        let failed = script.replace("/bin/mv \"$work/BiFlow.app\" \"$target\"", "false");
        assert!(!run(&failed).status.success());
        assert!(
            target.join("old-marker").exists(),
            "the previous app must be restored"
        );
        assert!(run(&script).status.success());
        assert!(!target.join("old-marker").exists());
        assert!(source.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_failed_rollback_retains_the_only_previous_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let source = bundle(directory.path(), "Source.app", "6.2.82");
        let target = bundle(directory.path(), "Target.app", "6.2.81");
        std::fs::write(target.join("old-marker"), "old").unwrap();
        let script = install_script(&source, &target, "6.2.82")
            .replace("/bin/mv \"$work/BiFlow.app\" \"$target\"", "false")
            .replace("/bin/mv \"$backup\" \"$target\"", "false");
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let retained = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".biflow-update.")
            })
            .unwrap();
        assert!(retained.path().join("previous.app/old-marker").exists());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn native_disk_image_installs_exact_bundle_and_detaches() {
        let fixtures = tempfile::tempdir().unwrap();
        let payload = fixtures.path().join("payload");
        std::fs::create_dir(&payload).unwrap();
        bundle(&payload, "BiFlow.app", "6.2.82");
        let image = fixtures.path().join("update.dmg");
        let volume_name = format!("BiFlow-test-{}", uuid::Uuid::new_v4());
        let created = std::process::Command::new("/usr/bin/hdiutil")
            .args([
                "create",
                "-fs",
                "HFS+",
                "-volname",
                &volume_name,
                "-format",
                "UDZO",
                "-srcfolder",
            ])
            .arg(&payload)
            .arg(&image)
            .output()
            .unwrap();
        assert!(
            created.status.success(),
            "fixture disk image creation failed: status={:?}, stderr={}",
            created.status.code(),
            String::from_utf8_lossy(&created.stderr)
        );
        let staging = tempfile::tempdir().unwrap().keep();
        let mount = staging.join("image");
        std::fs::create_dir(&mount).unwrap();
        let attached = attach_image(&image, &mount).await;
        let target = fixtures.path().join("Installed BiFlow.app");
        let installed = if attached
            .as_ref()
            .is_ok_and(|output| output.status.success())
        {
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(install_script(&mount.join("BiFlow.app"), &target, "6.2.82"))
                .output()
                .map(|output| output.status.success())
        } else {
            Ok(false)
        };
        cleanup_mount(&staging, &mount, uuid::Uuid::nil()).await;
        let attached = attached.expect("fixture disk image attach could not complete");
        assert!(
            attached.status.success(),
            "fixture disk image attachment failed: status={:?}, stderr={}",
            attached.status.code(),
            String::from_utf8_lossy(&attached.stderr)
        );
        assert!(
            installed.expect("fixture installer could not complete"),
            "the mounted application's replacement must succeed"
        );
        assert!(
            !staging.exists(),
            "successful detachment must release staging"
        );
        assert!(target.join("Contents/MacOS/iran-split-desktop").exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_relaunch_waits_and_records_open_failures_without_user_paths() {
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("debug.log");
        let marker = directory.path().join("launched");
        let mut old = std::process::Command::new("/bin/sleep")
            .arg("0.3")
            .spawn()
            .unwrap();
        let script = relaunch_script(
            old.id(),
            Path::new("/Applications/BiFlow.app"),
            &log,
            uuid::Uuid::nil(),
        );
        let open = format!(
            "/usr/bin/touch {}",
            macos_shell_quote(&marker.to_string_lossy())
        );
        let mut watcher = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script.replace("/usr/bin/open -n \"$target\"", &open))
            .spawn()
            .unwrap();
        assert!(!marker.exists());
        assert!(old.wait().unwrap().success());
        assert!(watcher.wait().unwrap().success());
        assert!(marker.exists());
        let script = relaunch_script(
            99_999_999,
            Path::new("/private/secret/My App.app"),
            &log,
            uuid::Uuid::nil(),
        )
        .replace("/usr/bin/open -n \"$target\"", "false");
        assert!(!std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .status()
            .unwrap()
            .success());
        let events = std::fs::read_to_string(log).unwrap();
        let causes: Vec<_> = events
            .lines()
            .map(|line| {
                let event: serde_json::Value = serde_json::from_str(line).unwrap();
                assert!(event["timestamp"].as_str().unwrap().ends_with('Z'));
                assert_eq!(event["event"], "update.relaunch_result");
                event["cause"].as_str().unwrap().to_owned()
            })
            .collect();
        assert_eq!(causes, ["launch_requested", "launch_failed"]);
        assert!(!events.contains("secret"));
    }
}
