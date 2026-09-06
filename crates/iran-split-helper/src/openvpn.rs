//! Privileged supervision of an `OwnedSideTunnel` `OpenVPN` process.
//!
//! Invariants: `--route-noexec`, `--script-security 0` after `--config`,
//! helper-installed scoped routes only, reject `0.0.0.0/0`, Linux fwmark
//! policy table, Windows `interface-name` bind (no policy table).

use super::{redact, HelperServiceError, Supervisor};
use ipnet::IpNet;
use iran_split_clients::{audit_openvpn_profile, openvpn_arguments};
use iran_split_ipc::SideTunnelStatus;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};
use uuid::Uuid;

const DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const OPENVPN_STOP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MARK: u32 = 0x1780;
const DEFAULT_TABLE: u32 = 178;

/// One `StartSideTunnel` request, grouped so the privileged entry point keeps
/// a single argument instead of a long positional list.
#[derive(Debug, Clone, Copy)]
pub struct SideTunnelRequest<'a> {
    pub driver: &'a str,
    pub client_id: Uuid,
    pub profile: &'a Path,
    pub executable: Option<&'a Path>,
    pub auth_file: Option<&'a Path>,
    pub timeout_seconds: u64,
    pub pinned_remote: Option<(IpAddr, u16)>,
    pub socks_proxy: Option<(&'a str, u16)>,
}

#[derive(Debug)]
pub(crate) struct RunningSideTunnel {
    child: Child,
    device: String,
    routing_mark: u32,
    routing_table: u32,
    routes: Vec<IpNet>,
    policy_installed: bool,
}

impl Supervisor {
    /// Starts a side tunnel. `OpenVPN` is the only v1 driver.
    ///
    /// # Errors
    ///
    /// Returns [`HelperServiceError::SideTunnel`] when the profile is unsafe,
    /// the binary cannot be spawned, or the tunnel does not come up.
    pub async fn start_side_tunnel(
        &self,
        request: SideTunnelRequest<'_>,
    ) -> Result<SideTunnelStatus, HelperServiceError> {
        let SideTunnelRequest {
            driver,
            client_id,
            profile,
            executable,
            auth_file,
            timeout_seconds,
            pinned_remote,
            socks_proxy,
        } = request;
        if driver != "openvpn" {
            return Err(HelperServiceError::SideTunnel(
                "unsupported side-tunnel driver".into(),
            ));
        }
        let facts = audit_openvpn_profile(profile)
            .map_err(|error| HelperServiceError::SideTunnel(error.to_string()))?;
        self.stop_side_tunnel(client_id).await?;
        let binary = resolve_binary(executable)?;
        let device = format!("tun-{}", &client_id.to_string()[..8]);
        // Never spawn with an address the engine could not vouch for: a
        // poisoned or fake-ip answer must not become a --remote.
        if let Some((address, _)) = pinned_remote {
            if !iran_split_clients::is_routable_public(address) {
                return Err(HelperServiceError::SideTunnel(
                    "the pinned remote address is not routable".into(),
                ));
            }
        }
        let auth = auth_file.map(PathBuf::from);
        // Try the direct path first. A network that blocks the server by
        // address makes OpenVPN exit immediately; only then fall back to the
        // working client's SOCKS port, which costs a nested hop.
        // Both attempts share the caller's budget, so a retry can never push
        // the reply past the engine's IPC deadline.
        let attempt_seconds = if socks_proxy.is_some() {
            (timeout_seconds / 2).max(5)
        } else {
            timeout_seconds
        };
        let mut child = spawn_openvpn(
            &binary,
            &openvpn_arguments(profile, &device, auth.as_ref(), pinned_remote, None),
        )?;
        let mut outcome = wait_for_device(&mut child, &device, attempt_seconds).await;
        if let (Err(_), Some(proxy)) = (&outcome, socks_proxy) {
            self.push_log(
                "info",
                "side_tunnel_retry_via_proxy",
                BTreeMap::from([("driver".into(), "openvpn".into())]),
            )
            .await;
            let _ = child.start_kill();
            let args =
                openvpn_arguments(profile, &device, auth.as_ref(), pinned_remote, Some(proxy));
            child = spawn_openvpn(&binary, &args)?;
            outcome = wait_for_device(&mut child, &device, attempt_seconds).await;
        }
        outcome?;

        let mut routes = facts.server_networks;
        routes.retain(|network| network.prefix_len() > 0);
        if routes.iter().any(|network| network.prefix_len() == 0) {
            let _ = child.start_kill();
            return Err(HelperServiceError::SideTunnel(
                "default route is not allowed on a side tunnel".into(),
            ));
        }
        install_scoped_routes(&device, &routes).await?;
        revert_policy_routing(DEFAULT_MARK, DEFAULT_TABLE).await;
        let policy_installed = install_policy_routing(&device, DEFAULT_MARK, DEFAULT_TABLE).await?;
        self.side_tunnels.lock().await.insert(
            client_id,
            RunningSideTunnel {
                child,
                device: device.clone(),
                routing_mark: DEFAULT_MARK,
                routing_table: DEFAULT_TABLE,
                routes,
                policy_installed,
            },
        );
        self.push_log(
            "info",
            "side_tunnel_started",
            BTreeMap::from([
                ("driver".into(), "openvpn".into()),
                ("device".into(), device.clone()),
            ]),
        )
        .await;
        Ok(SideTunnelStatus {
            client_id: Some(client_id),
            driver: "openvpn".into(),
            running: true,
            device: Some(device),
            routing_mark: Some(DEFAULT_MARK),
        })
    }

    /// Stops one side tunnel and removes helper-owned routes.
    ///
    /// # Errors
    ///
    /// Returns [`HelperServiceError::SideTunnel`] when the process cannot be
    /// terminated.
    pub async fn stop_side_tunnel(
        &self,
        client_id: Uuid,
    ) -> Result<SideTunnelStatus, HelperServiceError> {
        let running = self.side_tunnels.lock().await.remove(&client_id);
        if let Some(mut running) = running {
            revert_scoped_routes(&running.device, &running.routes).await;
            if running.policy_installed {
                revert_policy_routing(running.routing_mark, running.routing_table).await;
            }
            terminate(&mut running.child).await?;
            self.push_log(
                "info",
                "side_tunnel_stopped",
                BTreeMap::from([("driver".into(), "openvpn".into())]),
            )
            .await;
        }
        Ok(SideTunnelStatus {
            client_id: Some(client_id),
            driver: "openvpn".into(),
            running: false,
            device: None,
            routing_mark: None,
        })
    }

    pub async fn stop_all_side_tunnels(&self) {
        let ids: Vec<Uuid> = self.side_tunnels.lock().await.keys().copied().collect();
        for id in ids {
            if let Err(error) = self.stop_side_tunnel(id).await {
                self.push_log(
                    "warn",
                    "side_tunnel_stop_failed",
                    BTreeMap::from([("cause".into(), redact(&error.to_string()))]),
                )
                .await;
            }
        }
    }
}

/// Last meaningful stderr line of an exited `OpenVPN`, redacted and bounded.
async fn last_stderr_line(child: &mut Child) -> String {
    use tokio::io::AsyncReadExt;

    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut buffer = String::new();
    // The process has exited, so the pipe is closed and this returns at once.
    if stderr.read_to_string(&mut buffer).await.is_err() {
        return String::new();
    }
    let Some(line) = buffer
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
    else {
        return String::new();
    };
    let line = redact(line);
    let line: String = line.chars().take(200).collect();
    format!(": {line}")
}

fn spawn_openvpn(binary: &Path, args: &[String]) -> Result<Child, HelperServiceError> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // Captured, not discarded: an OpenVPN option or file error is the
        // only place the real cause appears, and a bare "exit status: 1"
        // sends operators hunting the network instead of the message.
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    apply_openvpn_spawn(&mut command);
    command
        .spawn()
        .map_err(|error| HelperServiceError::SideTunnel(redact(&error.to_string())))
}

/// Waits for `OpenVPN` to bring `device` up, failing fast when it exits.
async fn wait_for_device(
    child: &mut Child,
    device: &str,
    timeout_seconds: u64,
) -> Result<(), HelperServiceError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_seconds.max(1));
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let detail = last_stderr_line(child).await;
            return Err(HelperServiceError::SideTunnel(format!(
                "openvpn exited early with status {status}{detail}"
            )));
        }
        if device_is_up(device).await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.start_kill();
            return Err(HelperServiceError::SideTunnel(
                "openvpn did not bring the tunnel up in time".into(),
            ));
        }
        tokio::time::sleep(DEVICE_POLL_INTERVAL).await;
    }
}

fn resolve_binary(configured: Option<&Path>) -> Result<PathBuf, HelperServiceError> {
    if let Some(path) = configured {
        // Same policy as the Mihomo binary: a configured executable must be a
        // regular, non-symlink file before the helper will spawn it as root.
        let metadata = std::fs::symlink_metadata(path).map_err(|_| {
            HelperServiceError::SideTunnel("the openvpn executable is unreadable".into())
        })?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(HelperServiceError::SideTunnel(
                "the openvpn executable must be a regular, non-symlink file".into(),
            ));
        }
        return Ok(path.to_path_buf());
    }
    candidate_binaries()
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            HelperServiceError::SideTunnel(
                "the openvpn binary was not found; install OpenVPN or set its path".into(),
            )
        })
}

#[cfg(unix)]
fn candidate_binaries() -> Vec<PathBuf> {
    [
        "/usr/sbin/openvpn",
        "/usr/bin/openvpn",
        "/sbin/openvpn",
        "/bin/openvpn",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

#[cfg(windows)]
fn candidate_binaries() -> Vec<PathBuf> {
    [
        r"C:\Program Files\OpenVPN\bin\openvpn.exe",
        r"C:\Program Files (x86)\OpenVPN\bin\openvpn.exe",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

#[cfg(unix)]
#[allow(clippy::unused_async)] // Windows twin awaits netsh; keep one async call site.
async fn device_is_up(device: &str) -> bool {
    Path::new("/sys/class/net").join(device).exists()
}

#[cfg(windows)]
async fn device_is_up(device: &str) -> bool {
    run_capture(
        "netsh",
        &["interface", "show", "interface", &format!("name={device}")],
    )
    .await
    .is_some()
}

#[cfg(unix)]
async fn install_scoped_routes(device: &str, routes: &[IpNet]) -> Result<(), HelperServiceError> {
    for network in routes {
        let target = network.to_string();
        if ip_command(&["route", "replace", &target, "dev", device])
            .await
            .is_none()
        {
            return Err(HelperServiceError::SideTunnel(
                "scoped route could not be added".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn install_scoped_routes(device: &str, routes: &[IpNet]) -> Result<(), HelperServiceError> {
    for network in routes {
        let family = if network.addr().is_ipv4() {
            "ipv4"
        } else {
            "ipv6"
        };
        if run_capture(
            "netsh",
            &[
                "interface",
                family,
                "add",
                "route",
                &network.to_string(),
                &format!("interface={device}"),
                "store=active",
            ],
        )
        .await
        .is_none()
        {
            return Err(HelperServiceError::SideTunnel(
                "scoped route could not be added".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn install_policy_routing(
    device: &str,
    mark: u32,
    table: u32,
) -> Result<bool, HelperServiceError> {
    let table = table.to_string();
    let mark = format!("{mark:#x}");
    let installed = ip_command(&[
        "route", "replace", "default", "dev", device, "table", &table,
    ])
    .await
    .is_some()
        && ip_command(&[
            "rule", "add", "fwmark", &mark, "lookup", &table, "priority", "17800",
        ])
        .await
        .is_some();
    if !installed {
        return Err(HelperServiceError::SideTunnel(
            "the side-tunnel policy-routing table could not be installed".into(),
        ));
    }
    Ok(true)
}

#[cfg(windows)]
#[allow(clippy::unused_async)]
async fn install_policy_routing(
    _device: &str,
    _mark: u32,
    _table: u32,
) -> Result<bool, HelperServiceError> {
    Ok(false)
}

#[cfg(unix)]
async fn revert_scoped_routes(device: &str, routes: &[IpNet]) {
    for network in routes {
        drop(ip_command(&["route", "del", &network.to_string(), "dev", device]).await);
    }
    if Path::new("/sys/class/net").join(device).exists() {
        drop(ip_command(&["link", "delete", "dev", device]).await);
    }
}

#[cfg(windows)]
async fn revert_scoped_routes(device: &str, routes: &[IpNet]) {
    for network in routes {
        let family = if network.addr().is_ipv4() {
            "ipv4"
        } else {
            "ipv6"
        };
        drop(
            run_capture(
                "netsh",
                &[
                    "interface",
                    family,
                    "delete",
                    "route",
                    &network.to_string(),
                    &format!("interface={device}"),
                ],
            )
            .await,
        );
    }
}

#[cfg(unix)]
async fn revert_policy_routing(mark: u32, table: u32) {
    let table = table.to_string();
    let mark = format!("{mark:#x}");
    drop(ip_command(&["rule", "del", "fwmark", &mark, "lookup", &table]).await);
    drop(ip_command(&["route", "flush", "table", &table]).await);
}

#[cfg(windows)]
#[allow(clippy::unused_async)]
async fn revert_policy_routing(_mark: u32, _table: u32) {}

#[cfg(unix)]
async fn ip_command(args: &[&str]) -> Option<String> {
    let binary = [Path::new("/usr/sbin/ip"), Path::new("/usr/bin/ip")]
        .into_iter()
        .find(|path| path.is_file())?;
    let output = Command::new(binary)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(windows)]
async fn run_capture(binary: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(unix)]
fn apply_openvpn_spawn(command: &mut Command) {
    command.env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
}

#[cfg(windows)]
fn apply_openvpn_spawn(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let system_root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| r"C:\Windows".into());
    let system_drive = std::env::var("SYSTEMDRIVE").unwrap_or_else(|_| r"C:".into());
    let path = format!(r"{system_root}\System32;{system_root}");
    command
        .env("SYSTEMROOT", &system_root)
        .env("SystemRoot", &system_root)
        .env("WINDIR", &system_root)
        .env("SYSTEMDRIVE", system_drive)
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .env("PATH", path);
    // tokio's Command exposes creation_flags directly on Windows.
    command.creation_flags(CREATE_NO_WINDOW);
}

async fn terminate(child: &mut Child) -> Result<(), HelperServiceError> {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return Ok(());
    }
    child
        .start_kill()
        .map_err(|error| HelperServiceError::SideTunnel(error.to_string()))?;
    tokio::time::timeout(OPENVPN_STOP_TIMEOUT, child.wait())
        .await
        .map_err(|_| HelperServiceError::SideTunnel("openvpn did not stop in time".into()))?
        .map_err(|error| HelperServiceError::SideTunnel(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the unix end-to-end test builds a Supervisor.
    #[cfg(unix)]
    use crate::HelperSettings;
    use std::fs;

    fn write_regular_file(directory: &Path, name: &str, contents: &str) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, contents).expect("fixture file");
        path
    }

    #[test]
    fn resolve_binary_accepts_configured_regular_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let binary = write_regular_file(directory.path(), "openvpn", "not really a binary");
        let resolved = resolve_binary(Some(&binary)).expect("regular file accepted");
        assert_eq!(resolved, binary);
    }

    #[test]
    fn resolve_binary_rejects_missing_configured_executable() {
        let directory = tempfile::tempdir().expect("tempdir");
        let missing = directory.path().join("does-not-exist");
        let error = resolve_binary(Some(&missing)).expect_err("missing file rejected");
        assert!(matches!(error, HelperServiceError::SideTunnel(_)));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_binary_rejects_symlinked_executable() {
        let directory = tempfile::tempdir().expect("tempdir");
        let target = write_regular_file(directory.path(), "real-openvpn", "target");
        let link = directory.path().join("openvpn");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let error = resolve_binary(Some(&link)).expect_err("symlink rejected");
        assert!(matches!(error, HelperServiceError::SideTunnel(_)));
        assert!(error.to_string().contains("regular, non-symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_binary_rejects_directory_executable() {
        let directory = tempfile::tempdir().expect("tempdir");
        let error = resolve_binary(Some(directory.path())).expect_err("directory rejected");
        assert!(matches!(error, HelperServiceError::SideTunnel(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_side_tunnel_rejects_symlinked_executable_without_spawning() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("tempdir");
        let profile = write_regular_file(
            directory.path(),
            "profile.ovpn",
            "client\nremote 192.0.2.10 1194\ndev tun\n",
        );
        // The symlink points at a script that records execution, so the test
        // proves the helper failed before spawning anything.
        let marker = directory.path().join("executed-marker");
        let script = write_regular_file(
            directory.path(),
            "fake-openvpn.sh",
            &format!("#!/bin/sh\ntouch {}\n", marker.display()),
        );
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        let link = directory.path().join("openvpn");
        std::os::unix::fs::symlink(&script, &link).expect("symlink");

        let supervisor = Supervisor::new(HelperSettings {
            authorized_uid: 1_000,
            authorized_gid: 1_000,
            socket_path: directory.path().join("helper.sock"),
            staging_dir: directory.path().join("staging"),
            runtime_dir: directory.path().join("runtime"),
            mihomo_binary: directory.path().join("mihomo"),
            mihomo_sha256: "0".repeat(64),
            tun_name: "biflow-tun".into(),
        });
        let error = supervisor
            .start_side_tunnel(SideTunnelRequest {
                driver: "openvpn",
                client_id: Uuid::new_v4(),
                profile: &profile,
                executable: Some(&link),
                auth_file: None,
                timeout_seconds: 1,
                pinned_remote: None,
                socks_proxy: None,
            })
            .await
            .expect_err("symlinked executable rejected");
        assert!(matches!(error, HelperServiceError::SideTunnel(_)));
        assert!(error.to_string().contains("regular, non-symlink"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!marker.exists(), "helper spawned the symlinked executable");
    }
}
