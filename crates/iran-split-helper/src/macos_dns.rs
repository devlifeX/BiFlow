//! Keep the original system DNS until every restore has succeeded. The
//! recovery file is private, root-owned, and never copied into diagnostics.
use super::HelperServiceError;
use std::{collections::BTreeMap, fs, io::Write, net::IpAddr, path::Path};

type Snapshot = BTreeMap<String, Vec<String>>;
const SNAPSHOT_FILE: &str = "system-dns.toml";

#[cfg(target_os = "macos")]
pub(crate) fn apply(runtime: &Path) -> Result<(), HelperServiceError> {
    run_action(runtime, true)
}

#[cfg(target_os = "macos")]
pub(crate) fn restore(runtime: &Path) -> Result<(), HelperServiceError> {
    run_action(runtime, false)
}

#[cfg(target_os = "macos")]
fn run_action(runtime: &Path, apply: bool) -> Result<(), HelperServiceError> {
    let trace_id = uuid::Uuid::new_v4();
    let operation = if apply { "apply" } else { "restore" };
    tracing::info!(event = "helper.dns_started", section = "helper_dns",
        initiator = "helper_process", cause = "dns_lifecycle", %trace_id, operation,
        trace_route = "helper_process->macos_dns->networksetup", "updating macOS system DNS");
    let result = if apply {
        apply_with(runtime, &mut networksetup)
    } else {
        restore_with(runtime, &mut networksetup)
    };
    match &result {
        Ok(services) => tracing::info!(event = "helper.dns_completed", section = "helper_dns",
            initiator = "helper_process", cause = "dns_update_succeeded", %trace_id, operation, services,
            trace_route = "helper_process->macos_dns->networksetup", "macOS system DNS updated"),
        Err(_) => tracing::warn!(event = "helper.dns_failed", section = "helper_dns",
            initiator = "helper_process", cause = "networksetup_or_recovery_file_failed", %trace_id, operation,
            trace_route = "helper_process->macos_dns->networksetup", "macOS DNS recovery remains pending"),
    }
    result.map(|_| ())
}

#[cfg(target_os = "macos")]
fn networksetup(args: &[String]) -> Result<String, HelperServiceError> {
    let output = std::process::Command::new("/usr/sbin/networksetup")
        .args(args)
        .env("LC_ALL", "C")
        .output()?;
    if !output.status.success() {
        return Err(HelperServiceError::Process(
            "macOS DNS command failed".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn apply_with(
    runtime: &Path,
    command: &mut impl FnMut(&[String]) -> Result<String, HelperServiceError>,
) -> Result<usize, HelperServiceError> {
    let output = command(&["-listallnetworkservices".into()])?;
    let services = parse_services(&output);
    if services.is_empty() {
        return Err(HelperServiceError::Process(
            "no active macOS network services".into(),
        ));
    }
    // Reuse the pre-takeover values after a repeated start/helper restart.
    let mut snapshot = read_snapshot(runtime)?.unwrap_or_default();
    for service in &services {
        if !snapshot.contains_key(service) {
            let output = command(&["-getdnsservers".into(), service.clone()])?;
            snapshot.insert(service.clone(), parse_dns(&output)?);
        }
    }
    write_snapshot(runtime, &snapshot)?;
    for service in &services {
        if let Err(error) = command(&dns_arguments(service, &["127.0.0.1".into()])) {
            if restore_with(runtime, command).is_err() {
                return Err(HelperServiceError::Process(
                    "macOS DNS apply failed; recovery remains pending".into(),
                ));
            }
            return Err(error);
        }
    }
    Ok(services.len())
}

fn restore_with(
    runtime: &Path,
    command: &mut impl FnMut(&[String]) -> Result<String, HelperServiceError>,
) -> Result<usize, HelperServiceError> {
    let Some(snapshot) = read_snapshot(runtime)? else {
        return Ok(0);
    };
    let mut first_error = None;
    for (service, servers) in &snapshot {
        if let Err(error) = command(&dns_arguments(service, servers)) {
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    fs::remove_file(runtime.join(SNAPSHOT_FILE))?;
    #[cfg(unix)]
    fs::File::open(runtime)?.sync_all()?;
    Ok(snapshot.len())
}

fn parse_services(output: &str) -> Vec<String> {
    output
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('*'))
        .map(str::to_owned)
        .collect()
}

fn parse_dns(output: &str) -> Result<Vec<String>, HelperServiceError> {
    if output
        .trim()
        .starts_with("There aren't any DNS Servers set")
    {
        return Ok(Vec::new());
    }
    let servers: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if servers.is_empty()
        || servers
            .iter()
            .any(|server| server.parse::<IpAddr>().is_err())
    {
        return Err(HelperServiceError::Process(
            "unrecognized macOS DNS response".into(),
        ));
    }
    Ok(servers)
}

fn dns_arguments(service: &str, servers: &[String]) -> Vec<String> {
    let mut args = vec!["-setdnsservers".into(), service.into()];
    if servers.is_empty() {
        args.push("Empty".into());
    } else {
        args.extend_from_slice(servers);
    }
    args
}

fn read_snapshot(runtime: &Path) -> Result<Option<Snapshot>, HelperServiceError> {
    let text = match fs::read_to_string(runtime.join(SNAPSHOT_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let snapshot: Snapshot = toml::from_str(&text)
        .map_err(|_| HelperServiceError::Process("invalid macOS DNS recovery file".into()))?;
    if snapshot.is_empty()
        || snapshot.iter().any(|(service, servers)| {
            service.is_empty()
                || servers
                    .iter()
                    .any(|server| server.parse::<IpAddr>().is_err())
        })
    {
        return Err(HelperServiceError::Process(
            "invalid macOS DNS recovery values".into(),
        ));
    }
    Ok(Some(snapshot))
}

fn write_snapshot(runtime: &Path, snapshot: &Snapshot) -> Result<(), HelperServiceError> {
    fs::create_dir_all(runtime)?;
    let text = toml::to_string(snapshot)
        .map_err(|_| HelperServiceError::Process("could not encode macOS DNS recovery".into()))?;
    let mut file = tempfile::NamedTempFile::new_in(runtime)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(runtime.join(SNAPSHOT_FILE))
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(runtime)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_with, dns_arguments, parse_dns, parse_services, read_snapshot, restore_with,
        SNAPSHOT_FILE,
    };
    use crate::HelperServiceError;
    #[cfg(unix)]
    use std::fs;

    fn network_response(args: &[String]) -> Result<String, HelperServiceError> {
        match args[0].as_str() {
            "-listallnetworkservices" => {
                Ok("An asterisk denotes a disabled service.\nWi-Fi\n".into())
            }
            "-getdnsservers" => Ok("There aren't any DNS Servers set on Wi-Fi.\n".into()),
            "-setdnsservers" => Ok(String::new()),
            _ => Err(HelperServiceError::Process(
                "unexpected fixture command".into(),
            )),
        }
    }

    #[test]
    fn dhcp_dns_is_restored_with_empty_and_the_snapshot_is_private() {
        let root = tempfile::tempdir().expect("tempdir");
        apply_with(root.path(), &mut network_response).expect("apply");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.path().join(SNAPSHOT_FILE))
                    .expect("snapshot")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let mut calls = Vec::new();
        restore_with(root.path(), &mut |args| {
            calls.push(args.to_vec());
            Ok(String::new())
        })
        .expect("restore");
        assert_eq!(calls, vec![vec!["-setdnsservers", "Wi-Fi", "Empty"]]);
        assert!(!root.path().join(SNAPSHOT_FILE).exists());
    }

    #[test]
    fn failed_restore_keeps_recovery_for_a_new_helper() {
        let root = tempfile::tempdir().expect("tempdir");
        apply_with(root.path(), &mut network_response).expect("apply");
        assert!(
            restore_with(root.path(), &mut |_| Err(HelperServiceError::Process(
                "fixture".into()
            )))
            .is_err()
        );
        assert!(root.path().join(SNAPSHOT_FILE).is_file());
        restore_with(root.path(), &mut network_response).expect("retry");
        assert!(!root.path().join(SNAPSHOT_FILE).exists());
    }

    #[test]
    fn repeated_start_preserves_the_original_resolvers() {
        let root = tempfile::tempdir().expect("tempdir");
        apply_with(root.path(), &mut |args| {
            if args[0] == "-getdnsservers" {
                Ok("8.8.8.8\n2001:4860:4860::8888\n".into())
            } else {
                network_response(args)
            }
        })
        .expect("first start");
        apply_with(root.path(), &mut |args| {
            assert_ne!(args[0], "-getdnsservers", "must not recapture loopback DNS");
            network_response(args)
        })
        .expect("restarted helper");
        let snapshot = read_snapshot(root.path())
            .expect("snapshot")
            .expect("present");
        assert_eq!(snapshot["Wi-Fi"], vec!["8.8.8.8", "2001:4860:4860::8888"]);
        restore_with(root.path(), &mut network_response).expect("restore");
    }

    #[test]
    fn partial_apply_rolls_back_every_service() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut restores = Vec::new();
        let result = apply_with(root.path(), &mut |args| {
            if args[0] == "-listallnetworkservices" {
                return Ok("header\nWi-Fi\nEthernet\n".into());
            }
            if args[0] == "-setdnsservers" && args[2] == "127.0.0.1" && args[1] == "Ethernet" {
                return Err(HelperServiceError::Process("fixture".into()));
            }
            if args[0] == "-setdnsservers" && args[2] == "Empty" {
                restores.push(args[1].clone());
            }
            network_response(args)
        });
        assert!(result.is_err());
        assert_eq!(restores, vec!["Ethernet", "Wi-Fi"]);
        assert!(!root.path().join(SNAPSHOT_FILE).exists());
    }

    #[test]
    fn invalid_resolver_output_stops_before_any_mutation() {
        let root = tempfile::tempdir().expect("tempdir");
        assert!(apply_with(root.path(), &mut |args| {
            assert_ne!(args[0], "-setdnsservers");
            if args[0] == "-getdnsservers" {
                Ok("unexpected error text".into())
            } else {
                network_response(args)
            }
        })
        .is_err());
        assert!(!root.path().join(SNAPSHOT_FILE).exists());
        assert_eq!(
            parse_services("header\nWi-Fi\n*Disabled service\n\n"),
            vec!["Wi-Fi"]
        );
        assert!(parse_dns("invalid").is_err());
        assert_eq!(
            dns_arguments("Wi-Fi", &["1.1.1.1".into()]),
            vec!["-setdnsservers", "Wi-Fi", "1.1.1.1"]
        );
    }
}
