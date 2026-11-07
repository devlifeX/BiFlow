use iran_split_core::{
    proxy_endpoint_ready, proxy_restore_decision, CoreError, ProxyRestoreDecision,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::info;

/// `CREATE_NO_WINDOW`: `PowerShell` must not flash a console behind the UI.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub proxy_enable: u32,
    pub proxy_server: String,
    #[serde(default)]
    pub auto_config_url: Option<String>,
}

#[must_use]
pub fn points_at_hiddify(snapshot: &Snapshot, host: &str, port: u16) -> bool {
    if !iran_split_core::proxy_host_is_loopback(host) {
        return false;
    }
    if snapshot.proxy_enable == 0 {
        return false;
    }
    iran_split_core::proxy_server_matches(&snapshot.proxy_server, host, port)
}

pub fn snapshot_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("system-proxy-snapshot.json")
}

/// Clears `HKCU` Internet Settings when they point at Hiddify.
///
/// # Errors
///
/// Returns a platform error when `PowerShell` cannot read or write the keys.
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
        initiator = "windows_platform_backend",
        cause = "hiddify_endpoint",
        trace_route = "engine->windows_platform_backend->system_proxy",
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

fn unchanged_after_clear(original: &Snapshot, current: &Snapshot) -> bool {
    current.proxy_enable == 0
        && current.proxy_server.is_empty()
        && current.auto_config_url == original.auto_config_url
}

async fn read_current() -> Result<Option<Snapshot>, CoreError> {
    let output = powershell(
        "$ErrorActionPreference = 'Stop'; $p = Get-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings'; @{proxy_enable=[int]$p.ProxyEnable;proxy_server=[string]$p.ProxyServer;auto_config_url=$p.AutoConfigURL} | ConvertTo-Json -Compress",
    ).await?;
    serde_json::from_str(&output)
        .map(Some)
        .map_err(|_| CoreError::Platform("could not decode the Windows system proxy".into()))
}

async fn apply_disabled() -> Result<(), CoreError> {
    powershell(&write_script(0, "")).await?;
    Ok(())
}

async fn apply_snapshot(snapshot: &Snapshot) -> Result<(), CoreError> {
    powershell(&write_script(
        snapshot.proxy_enable,
        &snapshot.proxy_server.replace('\'', "''"),
    ))
    .await?;
    Ok(())
}

fn write_script(enable: u32, server: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'; Set-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name ProxyEnable -Value {enable}; Set-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name ProxyServer -Value '{server}'; Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public class W {{ [DllImport(\"wininet.dll\")] public static extern bool InternetSetOption(System.IntPtr h, int o, System.IntPtr b, int l); }}'; [W]::InternetSetOption([IntPtr]::Zero, 39, [IntPtr]::Zero, 0) | Out-Null; [W]::InternetSetOption([IntPtr]::Zero, 37, [IntPtr]::Zero, 0) | Out-Null"
    )
}

async fn powershell(script: &str) -> Result<String, CoreError> {
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .map_err(|error| CoreError::Platform(error.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(CoreError::Platform(
            "could not update the Windows system proxy".into(),
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

    #[test]
    fn restore_preserves_new_proxy_and_pac_choices() {
        let original = Snapshot {
            proxy_enable: 1,
            proxy_server: "127.0.0.1:12334".into(),
            auto_config_url: None,
        };
        let mut disabled = Snapshot::default();
        assert!(unchanged_after_clear(&original, &disabled));
        disabled.auto_config_url = Some("https://proxy.corp.example/config.pac".into());
        assert!(!unchanged_after_clear(&original, &disabled));
        disabled = original.clone();
        disabled.proxy_server = "proxy.corp.example:8080".into();
        assert!(!unchanged_after_clear(&original, &disabled));
    }

    #[tokio::test]
    async fn capture_before_start_cannot_be_overwritten_by_an_auto_started_client() {
        let directory = tempfile::tempdir().unwrap();
        let persist = snapshot_path(directory.path());
        let original = Snapshot::default();
        remember_first_snapshot(&persist, &original).unwrap();
        let started = Snapshot {
            proxy_enable: 1,
            proxy_server: "127.0.0.1:12334".into(),
            auto_config_url: None,
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
    fn matches_enabled_hiddify_endpoint() {
        let snapshot = Snapshot {
            proxy_enable: 1,
            auto_config_url: None,
            proxy_server: "127.0.0.1:12334".into(),
        };
        assert!(points_at_hiddify(&snapshot, "127.0.0.1", 12334));
        assert!(!points_at_hiddify(&snapshot, "10.0.0.1", 8080));
        assert!(points_at_hiddify(
            &Snapshot {
                proxy_enable: 1,
                auto_config_url: None,
                proxy_server: "localhost:10808".into(),
            },
            "127.0.0.1",
            10808
        ));
    }

    #[test]
    fn ignores_disabled_or_corporate_proxy() {
        assert!(!points_at_hiddify(
            &Snapshot {
                proxy_enable: 0,
                auto_config_url: None,
                proxy_server: "127.0.0.1:12334".into(),
            },
            "127.0.0.1",
            12334
        ));
        assert!(!points_at_hiddify(
            &Snapshot {
                proxy_enable: 1,
                auto_config_url: None,
                proxy_server: "proxy.corp.example:8080".into(),
            },
            "127.0.0.1",
            12334
        ));
    }
}
