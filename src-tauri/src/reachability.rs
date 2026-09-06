use serde::Serialize;
use std::time::{Duration, Instant};
use tracing::info;

// A dead target should answer the operator quickly: production debug.log
// shows p95 pings pinned at the old 5s connect clamp, so the whole button
// press waited on the slowest row. 3s/6s keeps slow-but-alive Iranian links
// classified by SLOW_THRESHOLD while halving the worst-case wait.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
// Above this the site answered but a browser would feel it crawl; the UI turns
// the row yellow instead of green.
const SLOW_THRESHOLD: Duration = Duration::from_millis(2500);

/// Which network path a probe target is expected to take, mirroring the
/// generated Mihomo rules: `.ir` stays DIRECT, everything else is MATCH,VPN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbePath {
    Vpn,
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityStatus {
    Ok,
    Slow,
    Unreachable,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReachabilityResult {
    pub id: &'static str,
    pub domain: &'static str,
    pub path: ProbePath,
    /// Whether the VPN-path probes actually went through the Hiddify proxy.
    /// When the stack is down they fall back to a direct request, and the UI
    /// explains failures differently.
    pub via_proxy: bool,
    pub status: ReachabilityStatus,
    pub latency_ms: Option<u64>,
    pub detail: Option<String>,
}

struct Target {
    id: &'static str,
    domain: &'static str,
    url: &'static str,
    path: ProbePath,
}

// Fixed, well-known probe hosts — never user content, so ids may be logged.
const TARGETS: [Target; 3] = [
    Target {
        id: "google",
        domain: "google.com",
        url: "https://www.google.com/generate_204",
        path: ProbePath::Vpn,
    },
    Target {
        id: "facebook",
        domain: "facebook.com",
        url: "https://www.facebook.com/favicon.ico",
        path: ProbePath::Vpn,
    },
    Target {
        id: "iran",
        domain: "iran.ir",
        url: "https://iran.ir/",
        path: ProbePath::Direct,
    },
];

/// Local-proxy endpoints the desktop can speak SOCKS to.
///
/// VPN-path probes cannot use Mihomo's mixed port: the desktop process is
/// PROCESS-NAME-bypassed to DIRECT. `happ` is preferred for `google.com`
/// because that pin is commonly routed through Happ; other VPN probes use
/// the default (Hiddify) proxy and fall back to Happ.
#[derive(Debug, Clone, Default)]
pub struct VpnProxies {
    pub default: Option<(String, u16)>,
    pub happ: Option<(String, u16)>,
}

/// True for `google.com` and every subdomain. Production builds omit these
/// hosts from reachability rows and live-connection lists.
#[must_use]
pub fn is_google_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host == "google.com" || host.ends_with(".google.com")
}

/// Release packages hide Google probes and live hosts; debug/`./dev.sh` keeps them.
#[must_use]
pub const fn hide_google_in_this_build() -> bool {
    !cfg!(debug_assertions)
}

fn target_is_visible(target: &Target, hide_google: bool) -> bool {
    !(hide_google && target.id == "google")
}

fn proxy_for(target: &Target, proxies: &VpnProxies) -> Option<(String, u16)> {
    if target.id == "google" {
        proxies.happ.clone().or_else(|| proxies.default.clone())
    } else {
        proxies.default.clone().or_else(|| proxies.happ.clone())
    }
}

/// Probes the fixed reachability targets concurrently.
///
/// VPN-path targets go through a local-proxy SOCKS port when one is supplied
/// (the desktop process is PROCESS-NAME-bypassed in Mihomo, so probing the
/// mixed port would silently test the DIRECT path instead). DIRECT-path
/// targets and the no-proxy fallback use a plain client that ignores
/// environment proxies. `google.com` is probed only in debug builds.
pub async fn check_all(proxies: VpnProxies) -> Vec<ReachabilityResult> {
    let direct = plain_client();
    let default_proxied = proxies.default.as_ref().and_then(|(host, port)| {
        reqwest::Proxy::all(format!("socks5h://{host}:{port}"))
            .ok()
            .and_then(|proxy| base_client_builder().no_proxy().proxy(proxy).build().ok())
    });
    let happ_proxied = proxies.happ.as_ref().and_then(|(host, port)| {
        reqwest::Proxy::all(format!("socks5h://{host}:{port}"))
            .ok()
            .and_then(|proxy| base_client_builder().no_proxy().proxy(proxy).build().ok())
    });

    let probe = |target: &'static Target| {
        let hide_google = hide_google_in_this_build();
        let selected = proxy_for(target, &proxies);
        let use_happ = selected.is_some() && selected == proxies.happ;
        let socks = if use_happ {
            happ_proxied.clone()
        } else {
            default_proxied.clone().or_else(|| happ_proxied.clone())
        };
        let client = match (target.path, socks, &direct) {
            (ProbePath::Vpn, Some(client), _) => Some((client, true)),
            (_, _, Some(client)) => Some((client.clone(), false)),
            _ => None,
        };
        async move {
            if !target_is_visible(target, hide_google) {
                return None;
            }
            let Some((client, via_proxy)) = client else {
                return Some(ReachabilityResult {
                    id: target.id,
                    domain: target.domain,
                    path: target.path,
                    via_proxy: false,
                    status: ReachabilityStatus::Unreachable,
                    latency_ms: None,
                    detail: Some("probe client could not be built".into()),
                });
            };
            Some(probe_target(target, &client, via_proxy).await)
        }
    };

    let [first, second, third] = [&TARGETS[0], &TARGETS[1], &TARGETS[2]];
    let (first, second, third) = tokio::join!(probe(first), probe(second), probe(third));
    let results: Vec<ReachabilityResult> = [first, second, third].into_iter().flatten().collect();
    for result in &results {
        info!(
            event = "reachability.probe_completed",
            section = "network",
            initiator = "tauri_command",
            cause = "user_requested",
            trace_route = "tauri_command->network->reachability_probe",
            target_id = result.id,
            path = ?result.path,
            via_proxy = result.via_proxy,
            status = ?result.status,
            latency_ms = result.latency_ms,
            "reachability probe completed"
        );
    }
    results
}

async fn probe_target(
    target: &Target,
    client: &reqwest::Client,
    via_proxy: bool,
) -> ReachabilityResult {
    let started = Instant::now();
    // Any HTTP status counts as reachable: the point is whether the TLS
    // handshake survives, which is exactly what SNI filtering kills.
    let outcome = client.get(target.url).send().await;
    let elapsed = started.elapsed();
    let (status, latency_ms, detail) = match outcome {
        Ok(_) => (
            classify(elapsed),
            Some(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)),
            None,
        ),
        Err(error) => (
            ReachabilityStatus::Unreachable,
            None,
            Some(error.without_url().to_string()),
        ),
    };
    ReachabilityResult {
        id: target.id,
        domain: target.domain,
        path: target.path,
        via_proxy,
        status,
        latency_ms,
        detail,
    }
}

fn classify(elapsed: Duration) -> ReachabilityStatus {
    if elapsed > SLOW_THRESHOLD {
        ReachabilityStatus::Slow
    } else {
        ReachabilityStatus::Ok
    }
}

fn base_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("BiFlow/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(4))
}

fn plain_client() -> Option<reqwest::Client> {
    base_client_builder().no_proxy().build().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_response_is_ok_and_slow_response_is_slow() {
        assert_eq!(classify(Duration::from_millis(300)), ReachabilityStatus::Ok);
        assert_eq!(
            classify(SLOW_THRESHOLD + Duration::from_millis(1)),
            ReachabilityStatus::Slow
        );
    }

    #[test]
    fn targets_cover_both_paths_with_fixed_domains() {
        assert_eq!(TARGETS.len(), 3);
        assert!(TARGETS
            .iter()
            .any(|target| target.domain == "iran.ir" && target.path == ProbePath::Direct));
        assert!(
            TARGETS
                .iter()
                .filter(|target| target.path == ProbePath::Vpn)
                .count()
                == 2
        );
    }

    #[test]
    fn google_host_matches_the_root_and_subdomains() {
        assert!(is_google_host("google.com"));
        assert!(is_google_host("www.google.com"));
        assert!(is_google_host("gemini.google.com."));
        assert!(!is_google_host("notgoogle.com"));
        assert!(!is_google_host("facebook.com"));
    }

    #[test]
    fn google_probe_is_omitted_only_when_hidden() {
        assert!(target_is_visible(&TARGETS[0], false));
        assert!(!target_is_visible(&TARGETS[0], true));
        assert!(target_is_visible(&TARGETS[1], true));
        assert!(target_is_visible(&TARGETS[2], true));
    }

    #[test]
    fn google_probe_prefers_happ_over_the_default_proxy() {
        let proxies = VpnProxies {
            default: Some(("127.0.0.1".into(), 12_334)),
            happ: Some(("127.0.0.1".into(), 10_808)),
        };
        assert_eq!(proxy_for(&TARGETS[0], &proxies), proxies.happ);
        assert_eq!(proxy_for(&TARGETS[1], &proxies), proxies.default);
    }

    #[test]
    fn vpn_probes_fall_back_to_happ_when_hiddify_is_absent() {
        // The operator removed Hiddify and Happ is the only local proxy:
        // every VPN-path probe must ride the remaining SOCKS endpoint.
        let proxies = VpnProxies {
            default: None,
            happ: Some(("127.0.0.1".into(), 10_808)),
        };
        assert_eq!(proxy_for(&TARGETS[0], &proxies), proxies.happ);
        assert_eq!(proxy_for(&TARGETS[1], &proxies), proxies.happ);
    }

    #[test]
    fn result_serializes_snake_case_for_the_ui() {
        let value = serde_json::to_value(ReachabilityResult {
            id: "google",
            domain: "google.com",
            path: ProbePath::Vpn,
            via_proxy: true,
            status: ReachabilityStatus::Unreachable,
            latency_ms: None,
            detail: Some("tls closed".into()),
        })
        .expect("serialize");
        assert_eq!(value["path"], "vpn");
        assert_eq!(value["status"], "unreachable");
        assert_eq!(value["via_proxy"], true);
    }
}
