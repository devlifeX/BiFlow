//! Address-free evidence from the host resolver while capture is inactive.
use super::classify_host;
use serde::Serialize;
use std::{collections::BTreeSet, future::Future, io, net::IpAddr, time::Duration};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct DnsProbe {
    pub status: &'static str,
    pub answer_classes: Vec<&'static str>,
}

pub(super) async fn collect() -> DnsProbe {
    // A fixed reserved example domain, independent of user rules or diagnostics
    // targets. Query only the host resolver; never pin an address or send HTTP.
    let lookup = async {
        tokio::net::lookup_host(("example.com", 443))
            .await
            .map(|addresses| addresses.map(|address| address.ip()).collect())
    };
    let probe = collect_with(lookup, Duration::from_secs(2)).await;
    tracing::info!(
        event = "environment.dns_probe_completed",
        section = "environment",
        initiator = "environment_collector",
        cause = probe.status,
        trace_id = %Uuid::new_v4(),
        trace_route = "environment_snapshot->inactive_dns_probe->host_resolver",
        answer_classes = ?probe.answer_classes,
        "checked host resolution without logging targets or addresses"
    );
    probe
}

async fn collect_with<F>(lookup: F, budget: Duration) -> DnsProbe
where
    F: Future<Output = io::Result<Vec<IpAddr>>>,
{
    let (status, addresses) = match tokio::time::timeout(budget, lookup).await {
        Ok(Ok(addresses)) if addresses.is_empty() => ("empty", Vec::new()),
        Ok(Ok(addresses)) => ("resolved", addresses),
        Ok(Err(_)) => ("failed", Vec::new()),
        Err(_) => ("timed_out", Vec::new()),
    };
    let answer_classes = addresses
        .into_iter()
        .map(|address| {
            if address.is_multicast() {
                "multicast"
            } else if matches!(address, IpAddr::V4(ip) if ip.is_broadcast()) {
                "broadcast"
            } else {
                classify_host(&address.to_string())
            }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    DnsProbe {
        status,
        answer_classes,
    }
}

pub(super) fn findings(probe: Option<&DnsProbe>, findings: &mut Vec<String>) {
    let Some(probe) = probe else { return };
    if probe.status != "resolved" {
        findings.push(format!("stopped_public_dns_{}", probe.status));
    } else if probe
        .answer_classes
        .iter()
        .any(|class| *class != "public_ip")
    {
        // This is evidence about resolution, not proof of an app leftover or
        // permission to replace the user's DNS, corporate proxy, or firewall.
        findings.push("stopped_public_dns_non_public_answer".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn non_public_answers_are_visible_without_recording_addresses() {
        let addresses = [
            "10.0.0.7",
            "127.0.0.1",
            "198.18.0.7",
            "100.64.0.7",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "fe80::1",
            "10.0.0.7",
        ];
        let probe = collect_with(
            async {
                Ok(addresses
                    .iter()
                    .map(|value| value.parse().unwrap())
                    .collect())
            },
            Duration::from_secs(1),
        )
        .await;
        let mut found = Vec::new();
        findings(Some(&probe), &mut found);
        assert_eq!(found, ["stopped_public_dns_non_public_answer"]);
        assert_eq!(
            probe.answer_classes,
            [
                "broadcast",
                "cgnat",
                "fake_ip",
                "link_local",
                "loopback",
                "multicast",
                "private"
            ]
        );
        let json = serde_json::to_string(&probe).unwrap();
        for address in addresses {
            assert!(!json.contains(address));
        }
    }

    #[tokio::test]
    async fn public_answers_do_not_claim_http_reachability_or_raise_a_dns_failure() {
        let probe = collect_with(
            async {
                Ok(vec![
                    "192.0.2.7".parse().unwrap(),
                    "2001:db8::7".parse().unwrap(),
                ])
            },
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.status, "resolved");
        assert_eq!(probe.answer_classes, ["public_ip"]);
        let mut found = Vec::new();
        findings(Some(&probe), &mut found);
        findings(None, &mut found);
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn lookup_failure_empty_answer_and_timeout_are_distinct_and_redacted() {
        let probes = [
            collect_with(
                async { Err(io::Error::other("private resolver failure payload")) },
                Duration::from_secs(1),
            )
            .await,
            collect_with(async { Ok(Vec::new()) }, Duration::from_secs(1)).await,
            collect_with(std::future::pending(), Duration::ZERO).await,
        ];
        let mut found = Vec::new();
        for probe in &probes {
            assert!(probe.answer_classes.is_empty());
            assert!(!serde_json::to_string(probe).unwrap().contains("payload"));
            findings(Some(probe), &mut found);
        }
        assert_eq!(
            found,
            [
                "stopped_public_dns_failed",
                "stopped_public_dns_empty",
                "stopped_public_dns_timed_out"
            ]
        );
    }
}
