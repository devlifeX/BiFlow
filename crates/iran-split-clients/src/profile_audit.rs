use ipnet::IpNet;
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};
use thiserror::Error;

/// Directives that let a profile run code, or steal the interface.
const FORBIDDEN_DIRECTIVES: [&str; 16] = [
    "up",
    "down",
    "up-restart",
    "route-up",
    "route-pre-down",
    "ipchange",
    "learn-address",
    "tls-verify",
    "auth-user-pass-verify",
    "client-connect",
    "client-disconnect",
    "plugin",
    "script-security",
    "daemon",
    "log",
    "log-append",
];

#[derive(Debug, Error)]
pub enum OpenVpnProfileError {
    #[error("profile is unreadable")]
    Unreadable,
    #[error("profile must be a regular, non-symlink file")]
    NotRegularFile,
    #[error("profile uses the '{0}' directive, which would run commands as root")]
    ForbiddenDirective(String),
    #[error("profile has an unterminated inline block")]
    UnterminatedBlock,
    #[error("profile declares no remote server")]
    NoRemote,
}

/// Facts the helper needs after a profile has been proven safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenVpnProfileFacts {
    pub remote_hosts: Vec<String>,
    /// Port of the first `remote` line, needed to pin a resolved address.
    pub remote_port: Option<u16>,
    pub server_networks: Vec<IpNet>,
}

/// Reads a `.ovpn` file and rejects anything that would run code as root.
///
/// # Errors
///
/// Returns [`OpenVpnProfileError`] when the file is missing, not a regular
/// file, carries a script directive, or declares no `remote`.
pub fn audit_openvpn_profile(path: &Path) -> Result<OpenVpnProfileFacts, OpenVpnProfileError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| OpenVpnProfileError::Unreadable)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(OpenVpnProfileError::NotRegularFile);
    }
    let text = std::fs::read_to_string(path).map_err(|_| OpenVpnProfileError::Unreadable)?;
    let mut remote_hosts = Vec::new();
    let mut remote_port = None;
    let mut inline_block: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(open) = inline_block.as_deref() {
            if line.eq_ignore_ascii_case(&format!("</{open}>")) {
                inline_block = None;
            }
            continue;
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(tag) = line
            .strip_prefix('<')
            .and_then(|rest| rest.strip_suffix('>'))
        {
            if !tag.starts_with('/') {
                inline_block = Some(tag.to_ascii_lowercase());
            }
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(directive) = parts.next() else {
            continue;
        };
        let directive = directive.trim_start_matches("--").to_ascii_lowercase();
        if FORBIDDEN_DIRECTIVES.contains(&directive.as_str()) {
            return Err(OpenVpnProfileError::ForbiddenDirective(directive));
        }
        if directive == "remote" {
            if let Some(host) = parts.next() {
                remote_hosts.push(host.to_owned());
                if remote_port.is_none() {
                    remote_port = parts.next().and_then(|port| port.parse::<u16>().ok());
                }
            }
        }
    }
    if inline_block.is_some() {
        return Err(OpenVpnProfileError::UnterminatedBlock);
    }
    if remote_hosts.is_empty() {
        return Err(OpenVpnProfileError::NoRemote);
    }
    let server_networks = remote_hosts
        .iter()
        .filter_map(|host| host.parse::<IpAddr>().ok())
        .filter_map(|address| match address {
            IpAddr::V4(address) => IpNet::new(address.into(), 32).ok(),
            IpAddr::V6(address) => IpNet::new(address.into(), 128).ok(),
        })
        .collect();
    Ok(OpenVpnProfileFacts {
        remote_hosts,
        remote_port,
        server_networks,
    })
}

/// Helper-owned `OpenVPN` argv. Always includes `--route-noexec` and pins
/// `--script-security 0` after `--config`.
///
/// `pinned_remote` goes **before** `--config` so it becomes the first entry in
/// `OpenVPN`'s connection list and wins over the profile's own `remote`. The
/// profile's `verify-x509-name` still authenticates the server by name, so
/// pinning an address never weakens TLS; it only removes the dependency on a
/// system resolver that a filtered network can poison.
#[must_use]
pub fn openvpn_arguments(
    profile: &Path,
    device: &str,
    auth_file: Option<&PathBuf>,
    pinned_remote: Option<(IpAddr, u16)>,
    socks_proxy: Option<(&str, u16)>,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some((host, port)) = socks_proxy {
        args.push("--socks-proxy".into());
        args.push(host.to_owned());
        args.push(port.to_string());
    }
    if let Some((address, port)) = pinned_remote {
        args.push("--remote".into());
        args.push(address.to_string());
        args.push(port.to_string());
    }
    args.extend([
        "--config".into(),
        profile.to_string_lossy().into_owned(),
        "--script-security".into(),
        "0".into(),
        "--route-noexec".into(),
        "--dev".into(),
        device.into(),
        "--dev-type".into(),
        "tun".into(),
    ]);
    if let Some(auth) = auth_file {
        args.push("--auth-user-pass".into());
        args.push(auth.to_string_lossy().into_owned());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn audit_refuses_script_directives() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("bad.ovpn");
        let mut file = std::fs::File::create(&path).expect("create");
        writeln!(file, "client").expect("write");
        writeln!(file, "remote 203.0.113.10 1194").expect("write");
        writeln!(file, "up /tmp/evil.sh").expect("write");
        let error = audit_openvpn_profile(&path).expect_err("script");
        assert!(matches!(error, OpenVpnProfileError::ForbiddenDirective(_)));
    }

    #[test]
    fn audit_accepts_a_plain_remote_and_emits_a_slash32() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("ok.ovpn");
        std::fs::write(&path, "client\nremote 203.0.113.10 1194\n").expect("write");
        let facts = audit_openvpn_profile(&path).expect("audit");
        assert_eq!(facts.remote_hosts, ["203.0.113.10"]);
        assert_eq!(facts.server_networks.len(), 1);
        assert_eq!(facts.server_networks[0].to_string(), "203.0.113.10/32");
    }

    #[test]
    fn arguments_pin_script_security_after_config() {
        let args = openvpn_arguments(Path::new("/tmp/office.ovpn"), "tun-ovpn", None, None, None);
        let config = args
            .iter()
            .position(|arg| arg == "--config")
            .expect("config");
        let script = args
            .iter()
            .position(|arg| arg == "--script-security")
            .expect("script");
        assert!(config < script);
        assert!(args.iter().any(|arg| arg == "--route-noexec"));
    }

    #[test]
    fn pinned_remote_and_proxy_come_before_config() {
        let args = openvpn_arguments(
            Path::new("/tmp/office.ovpn"),
            "tun-ovpn",
            None,
            Some(("152.233.20.207".parse().expect("ip"), 587)),
            Some(("127.0.0.1", 12_334)),
        );
        let config = args
            .iter()
            .position(|arg| arg == "--config")
            .expect("config");
        let remote = args
            .iter()
            .position(|arg| arg == "--remote")
            .expect("remote");
        let socks = args
            .iter()
            .position(|arg| arg == "--socks-proxy")
            .expect("socks");
        // OpenVPN takes the first entry of its connection list, so a pinned
        // address only wins when it precedes the profile.
        assert!(remote < config, "--remote must precede --config");
        assert!(socks < config);
        assert_eq!(args[remote + 1], "152.233.20.207");
        assert_eq!(args[remote + 2], "587");
        assert_eq!(args[socks + 1], "127.0.0.1");
        assert_eq!(args[socks + 2], "12334");
    }

    #[test]
    fn audit_reports_the_remote_port_for_pinning() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("ok.ovpn");
        let mut file = std::fs::File::create(&path).expect("create");
        writeln!(file, "client").expect("write");
        writeln!(file, "remote vpn.example.com 587").expect("write");
        let facts = audit_openvpn_profile(&path).expect("audit");
        assert_eq!(facts.remote_hosts, ["vpn.example.com"]);
        assert_eq!(facts.remote_port, Some(587));
        // A hostname yields no scoped route; only literals do.
        assert!(facts.server_networks.is_empty());
    }
}
