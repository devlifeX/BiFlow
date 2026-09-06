//! Resolve side-tunnel server names over `DoH` through an already-connected
//! client.
//!
//! A filtered network can answer plain DNS for a VPN server with an
//! unroutable address (observed in Iran: `ber-449.whiskergalaxy.com` ->
//! `10.10.34.36`), so `OpenVPN` never reaches the server and exits early.
//! Resolving over `DoH` *through a proxy that already works* — the operator's
//! Hiddify, say — returns the real address, which the helper then pins with
//! `--remote`. The profile's `verify-x509-name` still authenticates the
//! server by name, so pinning an address never weakens TLS.

use std::{net::IpAddr, time::Duration};

/// `DoH` endpoints tried in order. Each answers the JSON API.
const DOH_ENDPOINTS: &[&str] = &[
    "https://1.1.1.1/dns-query",
    "https://dns.google/resolve",
    "https://9.9.9.9:5053/dns-query",
];

#[derive(Debug, thiserror::Error)]
pub enum RemoteResolveError {
    #[error("no usable proxy to resolve through")]
    NoProxy,
    #[error("every DoH endpoint failed: {0}")]
    AllEndpointsFailed(String),
    #[error("DoH returned no address that can be routed to")]
    NoRoutableAddress,
}

/// Addresses that must never be pinned: a poisoned or fake answer.
///
/// `198.18.0.0/15` is the benchmark range Mihomo hands out in fake-ip mode,
/// so accepting it would pin the app's own placeholder instead of the server.
#[must_use]
pub fn is_routable_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                // Shared address space (CGNAT) and the benchmark range.
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (18..20).contains(&b)))
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || v6.is_unique_local())
        }
    }
}

/// Resolves `host` over `DoH` through `proxy` (a `host:port` SOCKS5 endpoint of
/// a client that is already serving traffic).
///
/// # Errors
///
/// Returns [`RemoteResolveError`] when no endpoint answers or every answer is
/// an address that cannot be routed to.
pub async fn resolve_through_proxy(
    host: &str,
    proxy: (&str, u16),
    timeout: Duration,
) -> Result<Vec<IpAddr>, RemoteResolveError> {
    // An address literal in the profile needs no resolution at all.
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(vec![address]);
    }
    let (proxy_host, proxy_port) = proxy;
    let proxy_url = format!("socks5h://{proxy_host}:{proxy_port}");
    let proxy = reqwest::Proxy::all(&proxy_url).map_err(|_| RemoteResolveError::NoProxy)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(proxy)
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .map_err(|_| RemoteResolveError::NoProxy)?;

    let mut last_error = String::from("no endpoint was tried");
    for endpoint in DOH_ENDPOINTS {
        match query_endpoint(&client, endpoint, host).await {
            Ok(addresses) => {
                let routable: Vec<IpAddr> = addresses
                    .into_iter()
                    .filter(|a| is_routable_public(*a))
                    .collect();
                if routable.is_empty() {
                    last_error = "answer held no routable address".into();
                    continue;
                }
                return Ok(routable);
            }
            Err(error) => last_error = error,
        }
    }
    if last_error.contains("routable") {
        return Err(RemoteResolveError::NoRoutableAddress);
    }
    Err(RemoteResolveError::AllEndpointsFailed(last_error))
}

async fn query_endpoint(
    client: &reqwest::Client,
    endpoint: &str,
    host: &str,
) -> Result<Vec<IpAddr>, String> {
    let response = client
        .get(endpoint)
        .query(&[("name", host), ("type", "A")])
        .header("accept", "application/dns-json")
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!("DoH status {}", response.status()));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let answers = body
        .get("Answer")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "DoH answer was empty".to_owned())?;
    Ok(answers
        .iter()
        .filter_map(|answer| answer.get("data")?.as_str()?.parse::<IpAddr>().ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_poisoned_and_fake_answers() {
        // What the filtered network actually returned for a Windscribe node.
        assert!(!is_routable_public("10.10.34.36".parse().expect("ip")));
        // Mihomo's fake-ip range: pinning it would point at the app itself.
        assert!(!is_routable_public("198.18.0.24".parse().expect("ip")));
        assert!(!is_routable_public("127.0.0.1".parse().expect("ip")));
        assert!(!is_routable_public("192.168.1.1".parse().expect("ip")));
        assert!(!is_routable_public("100.64.0.1".parse().expect("ip")));
        assert!(!is_routable_public("169.254.1.1".parse().expect("ip")));
        // The real addresses of the same node.
        assert!(is_routable_public("152.233.20.207".parse().expect("ip")));
        assert!(is_routable_public("213.177.229.162".parse().expect("ip")));
    }

    #[tokio::test]
    async fn address_literals_skip_resolution() {
        let resolved =
            resolve_through_proxy("203.0.113.10", ("127.0.0.1", 1), Duration::from_millis(50))
                .await
                .expect("literal");
        assert_eq!(resolved, ["203.0.113.10".parse::<IpAddr>().expect("ip")]);
    }
}
