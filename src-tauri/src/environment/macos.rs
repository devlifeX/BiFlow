//! Native DNS evidence on macOS, with addresses classified before logging.
use super::{classify_host, DnsServers, EnvironmentReport};

pub(super) fn parse_dns(text: &str, report: &mut EnvironmentReport) {
    let mut in_primary = false;
    let mut servers = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with("resolver #") {
            if in_primary {
                break;
            }
            in_primary = true;
        } else if in_primary && line.starts_with("nameserver[") {
            if let Some((_, address)) = line.split_once(':') {
                servers.push(classify_host(address.trim()));
            }
        }
    }
    report.primary_dns_present = Some(!servers.is_empty());
    report.dns_servers.push(DnsServers {
        interface: "primary_resolver".into(),
        servers,
    });
}

#[cfg(target_os = "macos")]
pub(super) async fn collect_platform(
    report: &mut EnvironmentReport,
    _tun_name: &str,
) -> super::PlatformData {
    match super::run_command("/usr/sbin/scutil", &["--dns"]).await {
        Ok(text) => parse_dns(&text, report),
        Err(error) => report.collection_errors.push(error),
    }
    super::PlatformData::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_dns_is_classified_without_logging_addresses_or_scoped_resolvers() {
        let mut report = EnvironmentReport::default();
        parse_dns("resolver #1\n nameserver[0] : 127.0.0.1\n nameserver[1] : 198.18.0.2\nresolver #2\n nameserver[0] : 192.0.2.53\n", &mut report);
        assert_eq!(report.primary_dns_present, Some(true));
        assert_eq!(report.dns_servers[0].servers, vec!["loopback", "fake_ip"]);
        report.biflow.stack_phase = "stopped".into();
        assert!(super::super::findings(&report, None).contains(&"leftover_fake_ip_dns".into()));
        let mut empty = EnvironmentReport::default();
        parse_dns("No DNS configuration available\n", &mut empty);
        empty.biflow.stack_phase = "stopped".into();
        assert_eq!(empty.primary_dns_present, Some(false));
        assert!(super::super::findings(&empty, None)
            .contains(&"stopped_without_primary_dns_resolver".into()));
    }
}
