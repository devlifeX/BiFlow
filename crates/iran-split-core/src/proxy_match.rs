//! Decide whether an OS proxy entry belongs to a `BiFlow` local client.
//!
//! Corporate proxies stay untouched. `localhost`, `127.0.0.1`, and `::1` are
//! the same loopback host, because clients do not all write the same spelling.

/// `true` when `actual_host`:`actual_port` is the expected client endpoint.
#[must_use]
pub fn proxy_endpoint_matches(
    actual_host: &str,
    actual_port: &str,
    expected_host: &str,
    expected_port: u16,
) -> bool {
    actual_port.trim() == expected_port.to_string() && hosts_match(actual_host, expected_host)
}

/// `true` when a Windows `ProxyServer` string points at the expected endpoint.
///
/// Accepts `host:port`, `http=host:port;https=host:port`, and a scheme prefix.
#[must_use]
pub fn proxy_server_matches(server: &str, expected_host: &str, expected_port: u16) -> bool {
    server.split([';', ' ', ',']).any(|part| {
        let value = part
            .rsplit_once('=')
            .map_or(part, |(_, value)| value)
            .trim();
        let value = value
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("socks5://")
            .trim_start_matches("socks://");
        split_host_port(value).is_some_and(|(host, port)| {
            proxy_endpoint_matches(&host, &port, expected_host, expected_port)
        })
    })
}

fn hosts_match(actual: &str, expected: &str) -> bool {
    let actual = normalize_host(actual);
    let expected = normalize_host(expected);
    actual.eq_ignore_ascii_case(&expected) || (is_loopback(&actual) && is_loopback(&expected))
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_matches(|character| character == '[' || character == ']')
        .to_owned()
}

pub(crate) fn is_loopback(host: &str) -> bool {
    ["localhost", "127.0.0.1", "::1", "0:0:0:0:0:0:0:1"]
        .iter()
        .any(|candidate| host.eq_ignore_ascii_case(candidate))
}

fn split_host_port(value: &str) -> Option<(String, String)> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        return Some((host.to_owned(), port.to_owned()));
    }
    let (host, port) = value.rsplit_once(':')?;
    Some((host.to_owned(), port.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{proxy_endpoint_matches, proxy_server_matches};

    #[test]
    fn loopback_spellings_share_one_endpoint() {
        assert!(proxy_endpoint_matches(
            "localhost",
            "12334",
            "127.0.0.1",
            12_334
        ));
        assert!(proxy_endpoint_matches("::1", "10808", "127.0.0.1", 10_808));
        assert!(proxy_endpoint_matches(
            "[::1]",
            "10808",
            "localhost",
            10_808
        ));
    }

    #[test]
    fn corporate_and_other_ports_stay_unmatched() {
        assert!(!proxy_endpoint_matches(
            "proxy.corp.example",
            "8080",
            "127.0.0.1",
            8080
        ));
        assert!(!proxy_endpoint_matches(
            "127.0.0.1",
            "8080",
            "127.0.0.1",
            12_334
        ));
    }

    #[test]
    fn windows_proxy_server_matches_each_loopback_spelling() {
        assert!(proxy_server_matches(
            "http=127.0.0.1:12334;https=127.0.0.1:12334;socks=127.0.0.1:12334",
            "127.0.0.1",
            12_334
        ));
        assert!(proxy_server_matches("localhost:10808", "127.0.0.1", 10_808));
        assert!(proxy_server_matches(
            "http://[::1]:12334",
            "127.0.0.1",
            12_334
        ));
        assert!(!proxy_server_matches(
            "proxy.corp.example:8080",
            "127.0.0.1",
            8080
        ));
        assert!(!proxy_server_matches(
            "http=127.0.0.1:12334;https=10.1.1.1:8080",
            "10.1.1.1",
            12_334
        ));
    }
}
