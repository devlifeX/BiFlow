//! macOS system proxy snapshot/restore via `networksetup`.
//!
//! Hiddify can publish an `HTTP`/`SOCKS` proxy on the macOS network services so
//! browsers bypass the TUN split routing. `BiFlow` snapshots that state, clears
//! it while connected, and restores it on disconnect — the same contract as
//! the GNOME/KDE backend on Linux, but driven by `networksetup` per network
//! service (Wi-Fi, Ethernet, …).

use iran_split_core::{
    proxy_endpoint_ready, proxy_restore_decision, CoreError, ProxyRestoreDecision,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::process::Command;
use tracing::{info, warn};

/// Absolute path: a GUI `PATH` can omit `/usr/sbin`, and then every
/// `networksetup` call silently looks like "no proxy configured".
const NETWORKSETUP: &str = "/usr/sbin/networksetup";

/// One network service's proxy configuration, as reported by `networksetup`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ServiceProxy {
    pub service: String,
    pub web_enabled: bool,
    pub secure_web_enabled: bool,
    pub socks_enabled: bool,
    pub http_host: String,
    pub http_port: String,
    pub https_host: String,
    pub https_port: String,
    pub socks_host: String,
    pub socks_port: String,
}

/// A snapshot of every network service's proxy state, used to restore later.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub mode: String,
    pub services: Vec<ServiceProxy>,
}

#[must_use]
pub fn points_at_hiddify(snapshot: &Snapshot, host: &str, port: u16) -> bool {
    if !iran_split_core::proxy_host_is_loopback(host) {
        return false;
    }
    if matches!(snapshot.mode.as_str(), "none" | "") {
        return false;
    }
    snapshot.services.iter().any(|service| {
        (service.web_enabled
            && iran_split_core::proxy_endpoint_matches(
                &service.http_host,
                &service.http_port,
                host,
                port,
            ))
            || (service.secure_web_enabled
                && iran_split_core::proxy_endpoint_matches(
                    &service.https_host,
                    &service.https_port,
                    host,
                    port,
                ))
            || (service.socks_enabled
                && iran_split_core::proxy_endpoint_matches(
                    &service.socks_host,
                    &service.socks_port,
                    host,
                    port,
                ))
    })
}

pub fn snapshot_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("system-proxy-snapshot.json")
}

/// Clears every network service's proxy when it points at Hiddify.
///
/// # Errors
///
/// Returns a platform error when `networksetup` cannot be read or written.
pub async fn clear_if_hiddify(
    host: &str,
    port: u16,
    persist: &Path,
) -> Result<Option<Snapshot>, CoreError> {
    let Some(snapshot) = read_current().await? else {
        return Ok(None);
    };
    if !points_at_hiddify(&snapshot, host, port) {
        return Ok(None);
    }
    remember_first_snapshot(persist, &snapshot)?;
    apply_disabled(&snapshot, host, port).await?;
    info!(
        event = "system_proxy.cleared",
        section = "system_proxy",
        initiator = "macos_platform_backend",
        cause = "hiddify_endpoint",
        trace_route = "engine->macos_platform_backend->system_proxy",
        "cleared a Hiddify system proxy without logging its endpoint"
    );
    Ok(Some(snapshot))
}

/// Capture once before starting clients, including an originally disabled proxy.
/// A disabled baseline prevents a newly launched client from becoming the
/// apparent owner of the user's preexisting network settings.
pub async fn capture(persist: &Path, endpoints: &[(String, u16)]) -> Result<(), CoreError> {
    if read_snapshot(persist)?.is_none() {
        if let Some(snapshot) = read_current().await? {
            let baseline = if endpoints
                .iter()
                .any(|(host, port)| points_at_hiddify(&snapshot, host, *port))
            {
                snapshot
            } else {
                Snapshot::default()
            };
            write_snapshot(persist, &baseline)?;
            info!(
                event = "system_proxy.baseline_captured",
                section = "system_proxy",
                initiator = "platform_backend",
                cause = "before_client_start",
                trace_route = "connect->capture_system_proxy->snapshot",
                "captured the original desktop proxy without logging its settings"
            );
        }
    }
    Ok(())
}

pub fn saved_proxy_uses(persist: &Path, host: &str, port: u16) -> Result<bool, CoreError> {
    Ok(read_snapshot(persist)?.is_some_and(|snapshot| points_at_hiddify(&snapshot, host, port)))
}

/// Restore after detaching the TUN, only if the saved local client is alive
/// and the settings are still in the state `BiFlow` left them.
pub async fn restore(persist: &Path, endpoints: &[(String, u16)]) -> Result<(), CoreError> {
    let Some(snapshot) = read_snapshot(persist)? else {
        return Ok(());
    };
    let mut managed = false;
    let mut reachable = true;
    for (host, port) in endpoints {
        if points_at_hiddify(&snapshot, host, *port) {
            managed = true;
            reachable &= proxy_endpoint_ready(host, *port).await;
        }
    }
    let current = read_current().await?;
    let decision = proxy_restore_decision(
        current
            .as_ref()
            .is_some_and(|current| unchanged_after_clear(&snapshot, current, endpoints)),
        managed,
        reachable,
    );
    if decision == ProxyRestoreDecision::Restore {
        apply_snapshot(&snapshot).await?;
        if read_current().await?.as_ref() != Some(&snapshot) {
            return Err(CoreError::Platform(
                "restored system proxy could not be verified".into(),
            ));
        }
    }
    std::fs::remove_file(persist)
        .map_err(|_| CoreError::Platform("could not retire the saved system proxy".into()))?;
    info!(
        event = "system_proxy.handoff_completed",
        section = "system_proxy",
        initiator = "platform_backend",
        cause = decision.cause(),
        trace_route = "disconnect->restore_system_proxy->verify",
        restored = decision == ProxyRestoreDecision::Restore,
        "completed desktop proxy handoff without logging settings"
    );
    Ok(())
}

fn cleared_snapshot(snapshot: &Snapshot, host: &str, port: u16) -> Snapshot {
    let mut cleared = snapshot.clone();
    for service in &mut cleared.services {
        if iran_split_core::proxy_endpoint_matches(
            &service.http_host,
            &service.http_port,
            host,
            port,
        ) {
            service.web_enabled = false;
        }
        if iran_split_core::proxy_endpoint_matches(
            &service.https_host,
            &service.https_port,
            host,
            port,
        ) {
            service.secure_web_enabled = false;
        }
        if iran_split_core::proxy_endpoint_matches(
            &service.socks_host,
            &service.socks_port,
            host,
            port,
        ) {
            service.socks_enabled = false;
        }
    }
    cleared.mode =
        if cleared.services.iter().any(|service| {
            service.web_enabled || service.secure_web_enabled || service.socks_enabled
        }) {
            "manual"
        } else {
            "none"
        }
        .into();
    cleared
}

fn unchanged_after_clear(
    original: &Snapshot,
    current: &Snapshot,
    endpoints: &[(String, u16)],
) -> bool {
    let mut disabled = original.clone();
    for (host, port) in endpoints {
        disabled = cleared_snapshot(&disabled, host, *port);
    }
    current == &disabled
}

async fn read_current() -> Result<Option<Snapshot>, CoreError> {
    if !Path::new(NETWORKSETUP).is_file() {
        return Ok(None);
    }
    let services = list_services().await?;
    let mut captured = Vec::with_capacity(services.len());
    let mut any_enabled = false;
    for service in services {
        let proxy = read_service(&service).await?;
        if proxy.web_enabled || proxy.secure_web_enabled || proxy.socks_enabled {
            any_enabled = true;
        }
        captured.push(proxy);
    }
    Ok(Some(Snapshot {
        mode: if any_enabled {
            "manual".into()
        } else {
            "none".into()
        },
        services: captured,
    }))
}

async fn apply_disabled(snapshot: &Snapshot, host: &str, port: u16) -> Result<(), CoreError> {
    let cleared = cleared_snapshot(snapshot, host, port);
    for (before, after) in snapshot.services.iter().zip(&cleared.services) {
        if before.web_enabled != after.web_enabled {
            set_proxy_state(&before.service, ProxyKind::Web, false).await?;
        }
        if before.secure_web_enabled != after.secure_web_enabled {
            set_proxy_state(&before.service, ProxyKind::SecureWeb, false).await?;
        }
        if before.socks_enabled != after.socks_enabled {
            set_proxy_state(&before.service, ProxyKind::Socks, false).await?;
        }
    }
    Ok(())
}

async fn apply_snapshot(snapshot: &Snapshot) -> Result<(), CoreError> {
    for service in &snapshot.services {
        if service.web_enabled {
            set_web_proxy(&service.service, &service.http_host, &service.http_port).await?;
        }
        set_proxy_state(&service.service, ProxyKind::Web, service.web_enabled).await?;
        if service.secure_web_enabled {
            set_secure_web_proxy(&service.service, &service.https_host, &service.https_port)
                .await?;
        }
        set_proxy_state(
            &service.service,
            ProxyKind::SecureWeb,
            service.secure_web_enabled,
        )
        .await?;
        if service.socks_enabled {
            set_socks_proxy(&service.service, &service.socks_host, &service.socks_port).await?;
        }
        set_proxy_state(&service.service, ProxyKind::Socks, service.socks_enabled).await?;
    }
    Ok(())
}

#[derive(Copy, Clone)]
enum ProxyKind {
    Web,
    SecureWeb,
    Socks,
}

impl ProxyKind {
    const fn state_flag(self) -> &'static str {
        match self {
            Self::Web => "webproxystate",
            Self::SecureWeb => "securewebproxystate",
            Self::Socks => "socksfirewallproxystate",
        }
    }
}

async fn set_proxy_state(service: &str, kind: ProxyKind, enabled: bool) -> Result<(), CoreError> {
    let flag = format!("-set{}", kind.state_flag());
    run_ok(
        NETWORKSETUP,
        &[&flag, service, if enabled { "on" } else { "off" }],
    )
    .await
}

async fn set_web_proxy(service: &str, host: &str, port: &str) -> Result<(), CoreError> {
    run_ok(NETWORKSETUP, &["-setwebproxy", service, host, port]).await
}

async fn set_secure_web_proxy(service: &str, host: &str, port: &str) -> Result<(), CoreError> {
    run_ok(NETWORKSETUP, &["-setsecurewebproxy", service, host, port]).await
}

async fn set_socks_proxy(service: &str, host: &str, port: &str) -> Result<(), CoreError> {
    run_ok(
        NETWORKSETUP,
        &["-setsocksfirewallproxy", service, host, port],
    )
    .await
}

/// Lists network services, skipping the header line `networksetup` prints.
async fn list_services() -> Result<Vec<String>, CoreError> {
    let output = Command::new(NETWORKSETUP)
        .arg("-listallnetworkservices")
        .output()
        .await
        .map_err(|error| CoreError::Platform(error.to_string()))?;
    if !output.status.success() {
        return Err(CoreError::Platform(
            "could not list macOS network services".into(),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut services = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("An asterisk") {
            continue;
        }
        services.push(trimmed.to_owned());
    }
    Ok(services)
}

async fn read_service(service: &str) -> Result<ServiceProxy, CoreError> {
    let web = read_proxy(service, "-getwebproxy").await?;
    let secure_web = read_proxy(service, "-getsecurewebproxy").await?;
    let socks = read_proxy(service, "-getsocksfirewallproxy").await?;
    Ok(ServiceProxy {
        service: service.to_owned(),
        web_enabled: web.0,
        secure_web_enabled: secure_web.0,
        socks_enabled: socks.0,
        http_host: web.1,
        http_port: web.2,
        https_host: secure_web.1,
        https_port: secure_web.2,
        socks_host: socks.1,
        socks_port: socks.2,
    })
}

/// Parses `networksetup -get<kind> <service>` output:
/// ```text
/// Enabled: No
/// Server: 127.0.0.1
/// Port: 12334
/// Authenticated Proxy Enabled: 0
/// ```
async fn read_proxy(service: &str, kind: &str) -> Result<(bool, String, String), CoreError> {
    let output = Command::new(NETWORKSETUP)
        .args([kind, service])
        .output()
        .await
        .map_err(|error| CoreError::Platform(error.to_string()))?;
    if !output.status.success() {
        warn!(
            event = "system_proxy.read_failed",
            section = "system_proxy",
            initiator = "macos_platform_backend",
            cause = "networksetup_failed",
            trace_route = "engine->macos_platform_backend->system_proxy",
            service,
            kind,
            "networksetup could not read a proxy entry"
        );
        return Ok((false, String::new(), String::new()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut enabled = false;
    let mut host = String::new();
    let mut port = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Enabled:") {
            enabled = matches!(value.trim(), "Yes" | "1" | "true");
        } else if let Some(value) = trimmed.strip_prefix("Server:") {
            value.trim().clone_into(&mut host);
        } else if let Some(value) = trimmed.strip_prefix("Port:") {
            value.trim().clone_into(&mut port);
        }
    }
    Ok((enabled, host, port))
}

async fn run_ok(program: &str, args: &[&str]) -> Result<(), CoreError> {
    let status = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|error| CoreError::Platform(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(CoreError::Platform(
            "could not update the macOS system proxy".into(),
        ))
    }
}

fn remember_first_snapshot(persist: &Path, snapshot: &Snapshot) -> Result<(), CoreError> {
    if read_snapshot(persist)?.is_none() {
        write_snapshot(persist, snapshot)?;
    }
    Ok(())
}

fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<(), CoreError> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::Platform("saved proxy has no parent directory".into()))?;
    std::fs::create_dir_all(parent)
        .map_err(|_| CoreError::Platform("could not prepare the saved proxy directory".into()))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| CoreError::Platform("could not stage the proxy baseline".into()))?;
    let bytes = serde_json::to_vec(snapshot)
        .map_err(|_| CoreError::Platform("could not encode the proxy baseline".into()))?;
    staged
        .write_all(&bytes)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|_| CoreError::Platform("could not write the proxy baseline".into()))?;
    staged
        .persist(path)
        .map_err(|_| CoreError::Platform("could not publish the proxy baseline".into()))?;
    Ok(())
}

fn read_snapshot(path: &Path) -> Result<Option<Snapshot>, CoreError> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|error| CoreError::Platform(error.to_string()))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| CoreError::Platform(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn capture_before_start_cannot_be_overwritten_by_an_auto_started_client() {
        let directory = tempfile::tempdir().unwrap();
        let persist = snapshot_path(directory.path());
        let original = Snapshot::default();
        remember_first_snapshot(&persist, &original).unwrap();
        let started = Snapshot {
            mode: "manual".into(),
            services: vec![service("Wi-Fi", "127.0.0.1", "12334")],
        };
        remember_first_snapshot(&persist, &started).unwrap();
        capture(&persist, &[("127.0.0.1".into(), 12334)])
            .await
            .unwrap();
        assert_eq!(read_snapshot(&persist).unwrap(), Some(original));
        assert!(!saved_proxy_uses(&persist, "127.0.0.1", 12334).unwrap());
        write_snapshot(&persist, &started).unwrap();
        assert!(saved_proxy_uses(&persist, "127.0.0.1", 12334).unwrap());
    }

    fn service(name: &str, host: &str, port: &str) -> ServiceProxy {
        ServiceProxy {
            service: name.into(),
            web_enabled: true,
            secure_web_enabled: false,
            socks_enabled: false,
            http_host: host.into(),
            http_port: port.into(),
            https_host: String::new(),
            https_port: String::new(),
            socks_host: String::new(),
            socks_port: String::new(),
        }
    }

    #[test]
    fn matches_hiddify_web_proxy() {
        let snapshot = Snapshot {
            mode: "manual".into(),
            services: vec![service("Wi-Fi", "127.0.0.1", "12334")],
        };
        assert!(points_at_hiddify(&snapshot, "127.0.0.1", 12334));
        assert!(!points_at_hiddify(&snapshot, "10.0.0.1", 8080));
        let localhost = Snapshot {
            mode: "manual".into(),
            services: vec![service("Wi-Fi", "localhost", "10808")],
        };
        assert!(points_at_hiddify(&localhost, "127.0.0.1", 10808));
    }

    #[test]
    fn disabled_mode_never_matches() {
        let snapshot = Snapshot {
            mode: "none".into(),
            services: vec![service("Wi-Fi", "127.0.0.1", "12334")],
        };
        assert!(!points_at_hiddify(&snapshot, "127.0.0.1", 12334));
    }

    #[test]
    fn ignores_unrelated_corporate_proxy() {
        let snapshot = Snapshot {
            mode: "manual".into(),
            services: vec![service("Wi-Fi", "proxy.corp.example", "8080")],
        };
        assert!(!points_at_hiddify(&snapshot, "127.0.0.1", 12334));
    }

    #[test]
    fn clear_preserves_unrelated_services_and_restore_detects_user_edits() {
        let original = Snapshot {
            mode: "manual".into(),
            services: vec![
                service("Wi-Fi", "127.0.0.1", "12334"),
                service("Corporate", "proxy.corp.example", "8080"),
            ],
        };
        let cleared = cleared_snapshot(&original, "127.0.0.1", 12334);
        assert!(!cleared.services[0].web_enabled);
        assert!(cleared.services[1].web_enabled);
        let endpoints = [("127.0.0.1".into(), 12334)];
        assert!(unchanged_after_clear(&original, &cleared, &endpoints));
        let mut edited = cleared;
        edited.services[1].http_host = "new.corp.example".into();
        assert!(!unchanged_after_clear(&original, &edited, &endpoints));
    }

    #[test]
    fn round_trips_snapshot_json() {
        let snapshot = Snapshot {
            mode: "manual".into(),
            services: vec![service("Wi-Fi", "127.0.0.1", "12334")],
        };
        let bytes = serde_json::to_vec(&snapshot).expect("serialize");
        let back: Snapshot = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(snapshot, back);
    }
}
