use iran_split_core::{
    proxy_endpoint_ready, proxy_restore_decision, retire_proxy_snapshot, CoreError,
    ProxyRestoreDecision,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub mode: String,
    pub http_host: String,
    pub http_port: String,
    pub https_host: String,
    pub https_port: String,
    pub socks_host: String,
    pub socks_port: String,
}

#[must_use]
pub fn points_at_hiddify(snapshot: &Snapshot, host: &str, port: u16) -> bool {
    if !iran_split_core::proxy_host_is_loopback(host) {
        return false;
    }
    if matches!(snapshot.mode.as_str(), "none" | "0" | "") {
        return false;
    }
    iran_split_core::proxy_endpoint_matches(&snapshot.http_host, &snapshot.http_port, host, port)
        || iran_split_core::proxy_endpoint_matches(
            &snapshot.https_host,
            &snapshot.https_port,
            host,
            port,
        )
        || iran_split_core::proxy_endpoint_matches(
            &snapshot.socks_host,
            &snapshot.socks_port,
            host,
            port,
        )
}

pub fn snapshot_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("system-proxy-snapshot.json")
}

/// Clears GNOME/KDE system proxy when it points at Hiddify.
///
/// # Errors
///
/// Returns a platform error when a present desktop tool fails to write.
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
    apply_disabled().await?;
    info!(
        event = "system_proxy.cleared",
        section = "system_proxy",
        initiator = "linux_platform_backend",
        cause = "hiddify_endpoint",
        trace_route = "engine->linux_platform_backend->system_proxy",
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
            .is_some_and(|current| unchanged_after_clear(&snapshot, current)),
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
    retire_proxy_snapshot(persist, decision)?;
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

fn unchanged_after_clear(original: &Snapshot, current: &Snapshot) -> bool {
    let mut disabled = original.clone();
    disabled.mode.clone_from(&current.mode);
    matches!(current.mode.as_str(), "none" | "0" | "") && current == &disabled
}

async fn read_current() -> Result<Option<Snapshot>, CoreError> {
    if which("gsettings") {
        return Ok(Some(read_gnome().await?));
    }
    if which("kreadconfig5") {
        return Ok(Some(read_kde().await?));
    }
    Ok(None)
}

async fn apply_disabled() -> Result<(), CoreError> {
    if which("gsettings") {
        run_ok(
            "gsettings",
            &["set", "org.gnome.system.proxy", "mode", "none"],
        )
        .await?;
    }
    if which("kwriteconfig5") {
        kde_write("ProxyType", "0").await?;
    }
    Ok(())
}

async fn apply_snapshot(snapshot: &Snapshot) -> Result<(), CoreError> {
    if which("gsettings") {
        run_ok(
            "gsettings",
            &[
                "set",
                "org.gnome.system.proxy.http",
                "host",
                &snapshot.http_host,
            ],
        )
        .await?;
        run_ok(
            "gsettings",
            &[
                "set",
                "org.gnome.system.proxy.http",
                "port",
                &snapshot.http_port,
            ],
        )
        .await?;
        run_ok(
            "gsettings",
            &[
                "set",
                "org.gnome.system.proxy.https",
                "host",
                &snapshot.https_host,
            ],
        )
        .await?;
        run_ok(
            "gsettings",
            &[
                "set",
                "org.gnome.system.proxy.https",
                "port",
                &snapshot.https_port,
            ],
        )
        .await?;
        run_ok(
            "gsettings",
            &[
                "set",
                "org.gnome.system.proxy.socks",
                "host",
                &snapshot.socks_host,
            ],
        )
        .await?;
        run_ok(
            "gsettings",
            &[
                "set",
                "org.gnome.system.proxy.socks",
                "port",
                &snapshot.socks_port,
            ],
        )
        .await?;
        run_ok(
            "gsettings",
            &["set", "org.gnome.system.proxy", "mode", &snapshot.mode],
        )
        .await?;
    }
    if which("kwriteconfig5") {
        apply_kde_snapshot(snapshot).await?;
    }
    Ok(())
}

async fn read_kde() -> Result<Snapshot, CoreError> {
    let proxy_type = kde_read("ProxyType").await?;
    let http = parse_kde_proxy(&kde_read("httpProxy").await?);
    let https = parse_kde_proxy(&kde_read("httpsProxy").await?);
    let socks = parse_kde_proxy(&kde_read("socksProxy").await?);
    Ok(Snapshot {
        mode: if proxy_type == "1" {
            "manual".into()
        } else {
            "none".into()
        },
        http_host: http.0,
        http_port: http.1,
        https_host: https.0,
        https_port: https.1,
        socks_host: socks.0,
        socks_port: socks.1,
    })
}

async fn apply_kde_snapshot(snapshot: &Snapshot) -> Result<(), CoreError> {
    kde_write(
        "httpProxy",
        &format!("http://{}:{}", snapshot.http_host, snapshot.http_port),
    )
    .await?;
    kde_write(
        "httpsProxy",
        &format!("http://{}:{}", snapshot.https_host, snapshot.https_port),
    )
    .await?;
    kde_write(
        "socksProxy",
        &format!("socks://{}:{}", snapshot.socks_host, snapshot.socks_port),
    )
    .await?;
    kde_write(
        "ProxyType",
        if snapshot.mode == "manual" { "1" } else { "0" },
    )
    .await
}

async fn kde_read(key: &str) -> Result<String, CoreError> {
    let output = Command::new("kreadconfig5")
        .args([
            "--file",
            "kioslaverc",
            "--group",
            "Proxy Settings",
            "--key",
            key,
        ])
        .output()
        .await
        .map_err(|error| CoreError::Platform(error.to_string()))?;
    if output.status.success() {
        Ok(unquote(String::from_utf8_lossy(&output.stdout).trim()))
    } else {
        Err(CoreError::Platform(
            "could not read the desktop system proxy".into(),
        ))
    }
}

async fn kde_write(key: &str, value: &str) -> Result<(), CoreError> {
    run_ok(
        "kwriteconfig5",
        &[
            "--file",
            "kioslaverc",
            "--group",
            "Proxy Settings",
            "--key",
            key,
            value,
        ],
    )
    .await
}

fn parse_kde_proxy(value: &str) -> (String, String) {
    let trimmed = value.trim();
    let without_scheme = trimmed.rsplit("://").next().unwrap_or(trimmed);
    without_scheme
        .rsplit_once(':')
        .map_or((without_scheme.to_owned(), "0".into()), |(host, port)| {
            (host.to_owned(), port.to_owned())
        })
}

async fn read_gnome() -> Result<Snapshot, CoreError> {
    Ok(Snapshot {
        mode: gsettings_get(&["org.gnome.system.proxy", "mode"]).await?,
        http_host: gsettings_get(&["org.gnome.system.proxy.http", "host"]).await?,
        http_port: gsettings_get(&["org.gnome.system.proxy.http", "port"]).await?,
        https_host: gsettings_get(&["org.gnome.system.proxy.https", "host"]).await?,
        https_port: gsettings_get(&["org.gnome.system.proxy.https", "port"]).await?,
        socks_host: gsettings_get(&["org.gnome.system.proxy.socks", "host"]).await?,
        socks_port: gsettings_get(&["org.gnome.system.proxy.socks", "port"]).await?,
    })
}

async fn gsettings_get(keys: &[&str]) -> Result<String, CoreError> {
    let mut command = vec!["get"];
    command.extend_from_slice(keys);
    let output = Command::new("gsettings")
        .args(&command)
        .output()
        .await
        .map_err(|error| CoreError::Platform(error.to_string()))?;
    if !output.status.success() {
        return Err(CoreError::Platform(
            "could not read the desktop system proxy".into(),
        ));
    }
    Ok(unquote(
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_matches('\''),
    ))
}

async fn run_ok(program: &str, args: &[&str]) -> Result<(), CoreError> {
    let status = Command::new(program)
        .args(args)
        .status()
        .await
        .map_err(|error| CoreError::Platform(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(CoreError::Platform(
            "could not update the desktop system proxy".into(),
        ))
    }
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('\'').trim_matches('"').to_owned()
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

    #[test]
    fn restore_preserves_new_proxy_and_auto_configuration_choices() {
        let original = Snapshot {
            mode: "manual".into(),
            http_host: "127.0.0.1".into(),
            http_port: "12334".into(),
            https_port: "0".into(),
            socks_port: "0".into(),
            ..Snapshot::default()
        };
        let mut disabled = original.clone();
        disabled.mode = "none".into();
        assert!(unchanged_after_clear(&original, &disabled));
        disabled.mode = "auto".into();
        assert!(!unchanged_after_clear(&original, &disabled));
        disabled.mode = "none".into();
        disabled.http_host = "new.corp.example".into();
        assert!(!unchanged_after_clear(&original, &disabled));
    }

    #[tokio::test]
    async fn capture_before_start_cannot_be_overwritten_by_an_auto_started_client() {
        let directory = tempfile::tempdir().unwrap();
        let persist = snapshot_path(directory.path());
        let original = Snapshot::default();
        remember_first_snapshot(&persist, &original).unwrap();
        let started = Snapshot {
            mode: "manual".into(),
            http_host: "127.0.0.1".into(),
            http_port: "12334".into(),
            https_port: "0".into(),
            socks_port: "0".into(),
            ..Snapshot::default()
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

    #[test]
    fn matches_hiddify_http_or_socks() {
        let snapshot = Snapshot {
            mode: "manual".into(),
            http_host: "127.0.0.1".into(),
            http_port: "12334".into(),
            https_host: String::new(),
            https_port: "0".into(),
            socks_host: String::new(),
            socks_port: "0".into(),
        };
        assert!(points_at_hiddify(&snapshot, "127.0.0.1", 12334));
        assert!(!points_at_hiddify(&snapshot, "10.0.0.1", 8080));
        let localhost = Snapshot {
            mode: "manual".into(),
            http_host: "localhost".into(),
            http_port: "10808".into(),
            https_host: String::new(),
            https_port: "0".into(),
            socks_host: String::new(),
            socks_port: "0".into(),
        };
        assert!(points_at_hiddify(&localhost, "127.0.0.1", 10808));
    }

    #[test]
    fn disabled_gnome_mode_never_matches() {
        let snapshot = Snapshot {
            mode: "none".into(),
            http_host: "127.0.0.1".into(),
            http_port: "12334".into(),
            https_host: String::new(),
            https_port: "0".into(),
            socks_host: String::new(),
            socks_port: "0".into(),
        };
        assert!(!points_at_hiddify(&snapshot, "127.0.0.1", 12334));
    }

    #[test]
    fn ignores_unrelated_corporate_proxy() {
        let snapshot = Snapshot {
            mode: "manual".into(),
            http_host: "proxy.corp.example".into(),
            http_port: "8080".into(),
            https_host: "proxy.corp.example".into(),
            https_port: "8080".into(),
            socks_host: String::new(),
            socks_port: "0".into(),
        };
        assert!(!points_at_hiddify(&snapshot, "127.0.0.1", 12334));
    }
}
