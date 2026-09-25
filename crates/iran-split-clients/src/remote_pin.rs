//! Remember a side-tunnel server address without editing the imported profile.
//!
//! The operator's `.ovpn` stays as they exported it. The first successful
//! lookup through a working client is stored under the app data directory and
//! reused when that client is not running.

use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    audit_openvpn_profile,
    remote_resolver::{is_routable_public, resolve_through_proxy, RemoteResolveError},
};

const CACHE_FILE: &str = "side-tunnel-remotes.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StoredPin {
    host: String,
    port: u16,
    address: IpAddr,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PinDocument {
    pins: Vec<StoredPin>,
}

/// A resolved server address and whether it was written for the next start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedRemote {
    pub address: IpAddr,
    pub port: u16,
    pub persisted: bool,
}

/// Resolves the profile hostname once through `proxy` and stores the address.
/// Later starts reuse that store when `proxy` is absent. The profile file is
/// only read.
///
/// # Errors
///
/// Returns a profile error when the file has no remote, and a resolve error
/// when a proxy is up, lookup fails, and nothing usable is stored yet.
pub async fn pin_profile_remote(
    profile: &Path,
    proxy: Option<(&str, u16)>,
    cache_dir: &Path,
) -> Result<Option<PinnedRemote>, String> {
    let facts = audit_openvpn_profile(profile).map_err(|error| error.to_string())?;
    let host = facts
        .remote_hosts
        .first()
        .cloned()
        .ok_or_else(|| "openvpn profile has no remote host".to_owned())?;
    let port = facts
        .remote_port
        .ok_or_else(|| "openvpn profile has no remote port".to_owned())?;
    if host.parse::<IpAddr>().is_ok() {
        return Ok(None);
    }
    let cached = recall_remote_pin(cache_dir, &host, port);
    let resolved = match proxy {
        Some((proxy_host, proxy_port)) => Some(
            resolve_through_proxy(
                &host,
                (proxy_host, proxy_port),
                std::time::Duration::from_secs(8),
            )
            .await
            .and_then(|addresses| {
                addresses
                    .into_iter()
                    .find(|address| is_routable_public(*address))
                    .ok_or(RemoteResolveError::NoRoutableAddress)
            }),
        ),
        None => None,
    };
    let address = select_remote_pin(resolved, cached).map_err(|error| {
        format!(
            "could not resolve the side-tunnel server through the running client ({error}); its DNS name is filtered"
        )
    })?;
    let Some(address) = address else {
        return Ok(None);
    };
    let persisted = remember_remote_pin(cache_dir, &host, port, address).is_ok();
    Ok(Some(PinnedRemote {
        address,
        port,
        persisted,
    }))
}

/// Picks the address `OpenVPN` should dial.
///
/// `resolved` is `None` when no proxy is up to look through. A failed lookup
/// falls back to the stored address so the tunnel still starts without Hiddify.
///
/// # Errors
///
/// Returns the lookup error when nothing usable is stored yet.
pub fn select_remote_pin(
    resolved: Option<Result<IpAddr, RemoteResolveError>>,
    cached: Option<IpAddr>,
) -> Result<Option<IpAddr>, RemoteResolveError> {
    match resolved {
        Some(Ok(address)) => Ok(Some(address)),
        Some(Err(error)) => cached.map(Some).ok_or(error),
        None => Ok(cached),
    }
}

/// Reads a previously stored address for `host` and `port`.
#[must_use]
pub fn recall_remote_pin(cache_dir: &Path, host: &str, port: u16) -> Option<IpAddr> {
    load(cache_dir).pins.into_iter().find_map(|pin| {
        (pin.port == port && pin.host.eq_ignore_ascii_case(host) && is_routable_public(pin.address))
            .then_some(pin.address)
    })
}

/// Stores `address` for the next start. The imported profile is not touched.
///
/// # Errors
///
/// Returns an I/O error when the cache file cannot be written.
pub fn remember_remote_pin(
    cache_dir: &Path,
    host: &str,
    port: u16,
    address: IpAddr,
) -> std::io::Result<()> {
    if !is_routable_public(address) {
        return Ok(());
    }
    let mut document = load(cache_dir);
    document
        .pins
        .retain(|pin| !(pin.port == port && pin.host.eq_ignore_ascii_case(host)));
    document.pins.push(StoredPin {
        host: host.to_owned(),
        port,
        address,
    });
    let path = cache_path(cache_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    std::fs::write(path, bytes)
}

fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(CACHE_FILE)
}

fn load(cache_dir: &Path) -> PinDocument {
    let Ok(bytes) = std::fs::read(cache_path(cache_dir)) else {
        return PinDocument::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_address_is_reused_without_a_proxy_and_the_profile_stays_put() {
        let dir = tempfile::tempdir().expect("temp");
        let profile = dir.path().join("office.ovpn");
        let original = "remote vpn.example.com 443\n";
        std::fs::write(&profile, original).expect("profile");
        remember_remote_pin(
            dir.path(),
            "vpn.example.com",
            443,
            "8.8.8.8".parse().unwrap(),
        )
        .expect("store");
        assert_eq!(std::fs::read_to_string(&profile).unwrap(), original);
        assert_eq!(
            recall_remote_pin(dir.path(), "VPN.Example.com", 443),
            Some("8.8.8.8".parse().unwrap())
        );
        let selected =
            select_remote_pin(None, recall_remote_pin(dir.path(), "vpn.example.com", 443))
                .expect("cache");
        assert_eq!(selected, Some("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn a_failed_lookup_uses_the_stored_address() {
        let cached = Some("8.8.8.8".parse().unwrap());
        let selected = select_remote_pin(Some(Err(RemoteResolveError::NoRoutableAddress)), cached)
            .expect("fallback");
        assert_eq!(selected, cached);
    }

    #[test]
    fn a_failed_lookup_without_a_store_is_an_error() {
        let selected = select_remote_pin(Some(Err(RemoteResolveError::NoRoutableAddress)), None);
        assert!(selected.is_err());
    }

    #[test]
    fn poisoned_cache_entries_are_ignored() {
        let dir = tempfile::tempdir().expect("temp");
        remember_remote_pin(
            dir.path(),
            "vpn.example.com",
            443,
            "10.10.34.36".parse().unwrap(),
        )
        .expect("store");
        assert_eq!(recall_remote_pin(dir.path(), "vpn.example.com", 443), None);
    }
}
