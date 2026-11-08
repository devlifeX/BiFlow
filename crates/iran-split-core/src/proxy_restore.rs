//! Restore only a managed, reachable proxy after releasing the TUN.

use crate::CoreError;
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyRestoreDecision {
    Restore,
    SettingsChanged,
    NotManaged,
    ClientUnavailable,
}

impl ProxyRestoreDecision {
    #[must_use]
    pub const fn cause(self) -> &'static str {
        match self {
            Self::Restore => "saved_client_reachable",
            Self::SettingsChanged => "settings_changed_outside_biflow",
            Self::NotManaged => "baseline_not_managed",
            Self::ClientUnavailable => "saved_client_unavailable",
        }
    }
}

#[must_use]
pub const fn proxy_restore_decision(
    unchanged: bool,
    managed: bool,
    reachable: bool,
) -> ProxyRestoreDecision {
    if !managed {
        ProxyRestoreDecision::NotManaged
    } else if !unchanged {
        ProxyRestoreDecision::SettingsChanged
    } else if !reachable {
        ProxyRestoreDecision::ClientUnavailable
    } else {
        ProxyRestoreDecision::Restore
    }
}

/// Retire a completed handoff without losing a temporarily unavailable client.
///
/// # Errors
/// Returns an error and retains the baseline when the original client is down,
/// or when the completed record cannot be removed.
pub fn retire_proxy_snapshot(path: &Path, decision: ProxyRestoreDecision) -> Result<(), CoreError> {
    if decision == ProxyRestoreDecision::ClientUnavailable {
        tracing::warn!(
            event = "system_proxy.handoff_deferred",
            section = "system_proxy",
            initiator = "platform_backend",
            cause = decision.cause(),
            trace_route = "disconnect->restore_system_proxy->retain_baseline",
            "original proxy client is unavailable; retaining its baseline for retry"
        );
        return Err(CoreError::Platform(
            "BiFlow routing is detached, but the original proxy client is unavailable. Start that client and retry Disconnect or Quit to restore its proxy.".into(),
        ));
    }
    std::fs::remove_file(path)
        .map_err(|_| CoreError::Platform("could not retire the saved system proxy".into()))
}

/// Only local client endpoints belong to the system proxy handoff.
#[must_use]
pub fn proxy_host_is_loopback(host: &str) -> bool {
    super::proxy_match::is_loopback(host.trim_matches(['[', ']']))
}

/// Test the actual socket; a process name or socket-enumeration entry does not
/// prove that the saved local proxy can accept connections.
pub async fn proxy_endpoint_ready(host: &str, port: u16) -> bool {
    let host = host.trim_matches(['[', ']']);
    if !proxy_host_is_loopback(host) {
        return false;
    }
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_client_keeps_the_baseline_until_a_verified_retry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy.json");
        let baseline = b"original private proxy baseline";
        std::fs::write(&path, baseline).unwrap();
        assert!(retire_proxy_snapshot(&path, ProxyRestoreDecision::ClientUnavailable).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), baseline);
        retire_proxy_snapshot(&path, ProxyRestoreDecision::Restore).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn deliberate_settings_changes_and_unmanaged_baselines_can_be_retired() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy.json");
        for decision in [
            ProxyRestoreDecision::SettingsChanged,
            ProxyRestoreDecision::NotManaged,
        ] {
            std::fs::write(&path, b"baseline").unwrap();
            retire_proxy_snapshot(&path, decision).unwrap();
            assert!(!path.exists());
        }
        assert!(retire_proxy_snapshot(&path, ProxyRestoreDecision::Restore).is_err());
    }

    #[test]
    fn only_an_unchanged_managed_live_proxy_is_restored() {
        for unchanged in [false, true] {
            for managed in [false, true] {
                for reachable in [false, true] {
                    assert_eq!(
                        proxy_restore_decision(unchanged, managed, reachable)
                            == ProxyRestoreDecision::Restore,
                        unchanged && managed && reachable
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn actual_listener_is_required_and_remote_hosts_are_never_probed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(proxy_endpoint_ready("127.0.0.1", port).await);
        drop(listener);
        assert!(!proxy_endpoint_ready("127.0.0.1", port).await);
        assert!(!proxy_endpoint_ready("proxy.corp.example", port).await);
    }
}
