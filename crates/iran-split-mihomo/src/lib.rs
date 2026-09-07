use futures_util::StreamExt;
use iran_split_clients::{
    process_bypass_union, synthesized_egress_handle, DriverPlatform, EgressHandle,
};
use iran_split_config::{AppConfig, DefaultRoute, EgressKind};
use iran_split_rules::{DirectTarget, RoutePinsDocument};
use reqwest::{header, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tokio::{process::Command, sync::mpsc};
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Error)]
pub enum MihomoError {
    #[error("Mihomo configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("Mihomo configuration serialization failed: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Mihomo controller request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Hiddify egress probe failed: {0}")]
    EgressProbe(String),
    #[error("Mihomo controller returned HTTP {0}")]
    UnexpectedStatus(StatusCode),
    #[error("Mihomo controller rejected the configuration ({status}): {detail}")]
    ReloadRejected { status: StatusCode, detail: String },
    #[error("Mihomo validation process failed: {0}")]
    ValidationProcess(std::io::Error),
    #[error("Mihomo rejected the generated configuration: {0}")]
    ValidationRejected(String),
    #[error("Mihomo readiness check timed out: {0}")]
    ReadinessTimeout(String),
    #[error("Mihomo controller rejected the secret; another instance may already own this port")]
    Unauthorized,
    #[error("operation was cancelled")]
    Cancelled,
    #[error("log WebSocket failed: {0}")]
    WebSocket(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub private_networks: PathBuf,
    pub iran_domains: PathBuf,
    pub iran_business_domains: PathBuf,
    pub iran_networks: PathBuf,
    pub iran_cdn_networks: PathBuf,
    pub custom_direct_domains: PathBuf,
    pub custom_direct_ips: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedConfig {
    pub yaml: String,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct MihomoConfigDocument {
    mixed_port: u16,
    allow_lan: bool,
    bind_address: String,
    mode: String,
    log_level: String,
    external_controller: String,
    secret: String,
    ipv6: bool,
    find_process_mode: String,
    tun: TunConfig,
    dns: DnsConfig,
    sniffer: SnifferConfig,
    proxies: Vec<ProxyConfig>,
    proxy_groups: Vec<ProxyGroup>,
    rule_providers: BTreeMap<String, RuleProvider>,
    rules: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "this serializable DTO mirrors Mihomo's TUN configuration schema"
)]
struct TunConfig {
    enable: bool,
    stack: String,
    device: String,
    auto_route: bool,
    auto_redirect: bool,
    auto_detect_interface: bool,
    strict_route: bool,
    dns_hijack: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct DnsConfig {
    enable: bool,
    listen: String,
    ipv6: bool,
    enhanced_mode: String,
    fake_ip_range: String,
    fake_ip_filter: Vec<String>,
    default_nameserver: Vec<String>,
    nameserver: Vec<String>,
    proxy_server_nameserver: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    direct_nameserver: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    nameserver_policy: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "this serializable DTO mirrors Mihomo's sniffer configuration schema"
)]
struct SnifferConfig {
    enable: bool,
    force_dns_mapping: bool,
    parse_pure_ip: bool,
    override_destination: bool,
    sniff: BTreeMap<String, SniffPorts>,
}

#[derive(Debug, Serialize)]
struct SniffPorts {
    ports: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProxyConfig {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    udp: bool,
    #[serde(rename = "interface-name", skip_serializing_if = "Option::is_none")]
    interface_name: Option<String>,
    #[serde(rename = "routing-mark", skip_serializing_if = "Option::is_none")]
    routing_mark: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ProxyGroup {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    proxies: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RuleProvider {
    #[serde(rename = "type")]
    kind: String,
    behavior: String,
    format: String,
    path: String,
}

/// Generates a validated Mihomo YAML document and its SHA-256 digest.
///
/// # Errors
///
/// Returns [`MihomoError::InvalidConfig`] when application or custom-rule
/// settings are invalid, or [`MihomoError::Yaml`] when serialization fails.
pub fn generate_config(
    app: &AppConfig,
    platform: Platform,
    paths: &RuntimePaths,
    custom_rules: &RoutePinsDocument,
) -> Result<GeneratedConfig, MihomoError> {
    generate_config_with_handles(app, platform, paths, custom_rules, &[])
}

/// Generates YAML from ready client handles. Empty `handles` synthesizes
/// `LocalProxy` endpoints from config so unit tests stay hermetic.
///
/// # Errors
///
/// Returns [`MihomoError::InvalidConfig`] when application or custom-rule
/// settings are invalid, or [`MihomoError::Yaml`] when serialization fails.
#[expect(
    clippy::too_many_lines,
    reason = "the function assembles one declarative Mihomo configuration document"
)]
pub fn generate_config_with_handles(
    app: &AppConfig,
    platform: Platform,
    _paths: &RuntimePaths,
    custom_rules: &RoutePinsDocument,
    handles: &[EgressHandle],
) -> Result<GeneratedConfig, MihomoError> {
    let issues = app.validate();
    if !issues.is_empty() {
        return Err(MihomoError::InvalidConfig(
            issues
                .into_iter()
                .map(|issue| format!("{}: {}", issue.field, issue.message))
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    if app.mihomo.controller_secret.trim().is_empty() {
        return Err(MihomoError::InvalidConfig(
            "controller secret must not be empty".into(),
        ));
    }

    let live = live_handles(app, handles);
    let routing = routing_handles(app, &live);
    let match_target = match_group(app, &live);
    let mut rules = process_bypass_rules(app, platform);
    rules.extend([
        "DOMAIN-SUFFIX,localhost,DIRECT".into(),
        "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve".into(),
        "IP-CIDR6,::1/128,DIRECT,no-resolve".into(),
    ]);
    for exclude in live.iter().flat_map(|handle| &handle.transport_excludes) {
        let flag = if exclude.addr().is_ipv6() {
            "IP-CIDR6"
        } else {
            "IP-CIDR"
        };
        rules.push(format!("{flag},{exclude},DIRECT,no-resolve"));
    }
    // Domain pins are emitted inline, most specific first, so the longest
    // matching pin wins across lists and outbounds: a developer.google.com
    // pin overrides a google.com pin even when they route differently.
    rules.extend(ordered_domain_pin_rules(app, custom_rules, &live, &routing));
    for client in app
        .enabled_clients()
        .into_iter()
        .filter(|client| client.spec().kind == EgressKind::OwnedSideTunnel)
    {
        let id = client.id.as_hyphenated();
        let target = client_rule_target(app, client, &live, &routing);
        rules.push(format!("RULE-SET,custom-{id}-ips,{target},no-resolve"));
    }
    rules.push("RULE-SET,private-networks,DIRECT,no-resolve".into());
    for client in app
        .enabled_clients()
        .into_iter()
        .filter(|client| client.spec().kind == EgressKind::LocalProxy)
    {
        let id = client.id.as_hyphenated();
        let target = client_rule_target(app, client, &live, &routing);
        rules.push(format!("RULE-SET,custom-{id}-ips,{target},no-resolve"));
    }
    rules.extend([
        "RULE-SET,custom-direct-ips,DIRECT,no-resolve".into(),
        "RULE-SET,iran-domains,DIRECT".into(),
        "RULE-SET,iran-business-domains,DIRECT".into(),
        "RULE-SET,iran-networks,DIRECT,no-resolve".into(),
        "RULE-SET,iran-cdn-networks,DIRECT,no-resolve".into(),
        "AND,((NETWORK,udp),(DST-PORT,443)),REJECT".into(),
        format!("MATCH,{match_target}"),
    ]);

    let direct_dns = app.mihomo.direct_dns_resolvers();
    // `apply_direct_dns` only decides whether to pin DIRECT domains to an
    // Iranian resolver (`nameserver-policy`/`direct-nameserver`). It must NOT
    // gate `fake-ip-filter`: DIRECT domains have to skip fake-ip regardless of
    // the resolver, otherwise the 198.18.0.0/15 fake IP collides with
    // `private.txt` and the browser never reaches the real address. See
    // ADR 0058 (witness: console.kavenegar.com) and ADR 0061.
    let apply_direct_dns =
        app.mihomo.direct_dns_preset.applies_direct_policy() && !direct_dns.is_empty();
    let fake_ip_filter = vec![
        "+.lan".into(),
        "+.local".into(),
        "localhost.ptlogin2.qq.com".into(),
        "rule-set:custom-direct-domains".into(),
        "rule-set:iran-domains".into(),
        "rule-set:iran-business-domains".into(),
    ];
    let document = MihomoConfigDocument {
        mixed_port: app.mihomo.mixed_port,
        allow_lan: false,
        bind_address: "127.0.0.1".into(),
        mode: "rule".into(),
        log_level: app.mihomo.log_level.to_string(),
        external_controller: format!(
            "{}:{}",
            app.mihomo.controller_host, app.mihomo.controller_port
        ),
        secret: app.mihomo.controller_secret.clone(),
        ipv6: platform != Platform::Windows,
        find_process_mode: "always".into(),
        tun: TunConfig {
            enable: true,
            stack: "mixed".into(),
            device: app.mihomo.tun_name.clone(),
            auto_route: true,
            auto_redirect: false,
            auto_detect_interface: true,
            strict_route: platform == Platform::Windows,
            dns_hijack: vec!["any:53".into(), "tcp://any:53".into()],
        },
        dns: DnsConfig {
            enable: true,
            listen: format!("127.0.0.1:{}", app.mihomo.dns_port),
            ipv6: platform != Platform::Windows,
            enhanced_mode: "fake-ip".into(),
            fake_ip_range: "198.18.0.1/16".into(),
            fake_ip_filter,
            default_nameserver: vec!["1.1.1.1".into(), "8.8.8.8".into()],
            nameserver: nameservers(&match_target),
            proxy_server_nameserver: vec!["8.8.8.8".into(), "1.1.1.1".into()],
            direct_nameserver: if apply_direct_dns {
                direct_dns.clone()
            } else {
                Vec::new()
            },
            nameserver_policy: if apply_direct_dns {
                direct_nameserver_policy(&direct_dns)
            } else {
                BTreeMap::new()
            },
        },
        sniffer: SnifferConfig {
            enable: true,
            force_dns_mapping: true,
            parse_pure_ip: true,
            override_destination: true,
            sniff: BTreeMap::from([
                (
                    "HTTP".into(),
                    SniffPorts {
                        ports: vec!["80".into(), "8080-8880".into()],
                    },
                ),
                (
                    "TLS".into(),
                    SniffPorts {
                        ports: vec!["443".into(), "8443".into()],
                    },
                ),
                (
                    "QUIC".into(),
                    SniffPorts {
                        ports: vec!["443".into(), "8443".into()],
                    },
                ),
            ]),
        },
        proxies: routing
            .iter()
            .filter_map(|handle| handle.outbound.as_ref())
            .map(|outbound| ProxyConfig {
                name: outbound.name.clone(),
                kind: outbound.kind.clone(),
                server: outbound.server.clone(),
                port: outbound.port,
                udp: outbound.udp,
                interface_name: outbound.interface_name.clone(),
                routing_mark: outbound.routing_mark,
            })
            .collect(),
        proxy_groups: routing
            .iter()
            .filter_map(|handle| handle.outbound.as_ref())
            .map(|outbound| ProxyGroup {
                name: outbound.group_name.clone(),
                kind: "select".into(),
                proxies: vec![outbound.name.clone()],
            })
            .collect(),
        rule_providers: providers(app),
        rules,
    };
    validate_custom_rules(custom_rules)?;
    let yaml = serde_yaml::to_string(&document)?;
    let sha256 = hex::encode(Sha256::digest(yaml.as_bytes()));
    Ok(GeneratedConfig { yaml, sha256 })
}

fn nameservers(match_target: &str) -> Vec<String> {
    // Pin DoH to the MATCH group. Unpinned Cloudflare / Google DoH is often
    // blocked on the Iranian WAN. When MATCH is DIRECT (or fail-closed
    // REJECT, which is not a proxy group) the hash is omitted.
    if match_target == "DIRECT" || match_target == "REJECT" {
        return vec![
            "https://1.1.1.1/dns-query".into(),
            "https://8.8.8.8/dns-query".into(),
        ];
    }
    vec![
        format!("https://1.1.1.1/dns-query#{match_target}"),
        format!("https://8.8.8.8/dns-query#{match_target}"),
    ]
}

/// Inline `DOMAIN-SUFFIX` rules for every user domain pin, ordered by
/// specificity (label count, descending) so the longest matching pin wins.
/// Pins of disabled clients are skipped, matching the "disable keeps pins
/// but does not emit them" contract. Clash `DOMAIN-SUFFIX,google.com`
/// matches the apex and every subdomain.
fn ordered_domain_pin_rules(
    app: &AppConfig,
    custom_rules: &RoutePinsDocument,
    live: &[EgressHandle],
    routing: &[EgressHandle],
) -> Vec<String> {
    let mut pins: Vec<(&str, String)> = Vec::new();
    for pin in &custom_rules.pins {
        let iran_split_rules::DirectTarget::Domain(domain) = &pin.target else {
            continue;
        };
        let target = match pin.outbound {
            iran_split_rules::Outbound::Direct => "DIRECT".into(),
            iran_split_rules::Outbound::Client { client_id } => {
                let Some(client) = app
                    .enabled_clients()
                    .into_iter()
                    .find(|client| client.id == client_id)
                else {
                    continue;
                };
                client_rule_target(app, client, live, routing)
            }
        };
        pins.push((domain.as_str(), target));
    }
    pins.sort_by(|left, right| {
        iran_split_rules::domain_specificity(right.0)
            .cmp(&iran_split_rules::domain_specificity(left.0))
            .then_with(|| left.0.cmp(right.0))
    });
    pins.into_iter()
        .map(|(domain, target)| format!("DOMAIN-SUFFIX,{domain},{target}"))
        .collect()
}

/// Rule target for one enabled client. User pins keep the client group even
/// when that egress is not live yet, so a Windscribe pin is not silently
/// rewritten to DIRECT/REJECT (ADR 0082). MATCH still fail-closes separately.
fn client_rule_target(
    app: &AppConfig,
    client: &iran_split_config::ClientInstance,
    live: &[EgressHandle],
    routing: &[EgressHandle],
) -> String {
    let group = routing
        .iter()
        .find(|handle| handle.client_id == client.id)
        .and_then(|handle| handle.outbound.as_ref())
        .map_or_else(
            || client.group_name(),
            |outbound| outbound.group_name.clone(),
        );
    if live.iter().any(|handle| handle.client_id == client.id) {
        return group;
    }
    if app.behavior.fail_closed && !client.allow_direct_when_down {
        group
    } else {
        "DIRECT".into()
    }
}

fn live_handles(app: &AppConfig, handles: &[EgressHandle]) -> Vec<EgressHandle> {
    if handles.is_empty() {
        return app
            .enabled_clients()
            .into_iter()
            .filter_map(synthesized_egress_handle)
            .collect();
    }
    handles
        .iter()
        .filter(|handle| handle.ready && !handle.degraded)
        .cloned()
        .collect()
}

fn routing_handles(app: &AppConfig, live: &[EgressHandle]) -> Vec<EgressHandle> {
    let mut routing = live.to_vec();
    for client in app.enabled_clients() {
        if routing.iter().any(|handle| handle.client_id == client.id) {
            continue;
        }
        if let Some(stub) = synthesized_egress_handle(client) {
            routing.push(stub);
        }
    }
    routing
}

fn match_group(app: &AppConfig, ready: &[EgressHandle]) -> String {
    match app.default_route {
        DefaultRoute::Direct => "DIRECT".into(),
        DefaultRoute::Client { client_id } => {
            if let Some(group) = ready
                .iter()
                .find(|handle| handle.client_id == client_id)
                .and_then(|handle| handle.outbound.as_ref())
                .map(|outbound| outbound.group_name.clone())
            {
                return group;
            }
            // The chosen default egress is down. Fail closed: unmatched
            // traffic is rejected locally instead of leaking over DIRECT.
            let excluded = app
                .client(client_id)
                .is_some_and(|client| client.allow_direct_when_down);
            if app.behavior.fail_closed && !excluded {
                "REJECT".into()
            } else {
                "DIRECT".into()
            }
        }
    }
}

fn direct_nameserver_policy(resolvers: &[String]) -> BTreeMap<String, Vec<String>> {
    [
        "custom-direct-domains",
        "iran-domains",
        "iran-business-domains",
    ]
    .into_iter()
    .map(|name| (format!("rule-set:{name}"), resolvers.to_vec()))
    .collect()
}

fn process_bypass_rules(app: &AppConfig, platform: Platform) -> Vec<String> {
    let driver_platform = match platform {
        Platform::Linux => DriverPlatform::Linux,
        Platform::Windows => DriverPlatform::Windows,
    };
    let mut rules: Vec<String> = process_bypass_union(&app.clients, driver_platform)
        .into_iter()
        .map(|bypass| {
            if bypass.wildcard {
                format!("PROCESS-NAME-WILDCARD,{},DIRECT", bypass.name)
            } else {
                format!("PROCESS-NAME,{},DIRECT", bypass.name)
            }
        })
        .collect();
    match platform {
        Platform::Linux => {
            rules.extend([
                "PROCESS-NAME,tailscaled,DIRECT".into(),
                "PROCESS-NAME,iran-split-desktop,DIRECT".into(),
                "PROCESS-NAME,iran-split-desk,DIRECT".into(),
                "PROCESS-NAME,BiFlow,DIRECT".into(),
            ]);
        }
        Platform::Windows => {
            rules.extend([
                "PROCESS-NAME,tailscaled.exe,DIRECT".into(),
                "PROCESS-NAME,iran-split-desktop.exe,DIRECT".into(),
                "PROCESS-NAME,BiFlow.exe,DIRECT".into(),
            ]);
        }
    }
    rules
}

fn providers(app: &AppConfig) -> BTreeMap<String, RuleProvider> {
    let mut map = BTreeMap::new();
    for (name, behavior, path) in [
        ("private-networks", "ipcidr", "private.txt"),
        ("iran-domains", "domain", "iran-domains.txt"),
        (
            "iran-business-domains",
            "domain",
            "iran-business-domains.txt",
        ),
        ("iran-networks", "ipcidr", "iran-networks.txt"),
        ("iran-cdn-networks", "ipcidr", "iran-cdn-networks.txt"),
        (
            "custom-direct-domains",
            "domain",
            "custom-direct-domains.txt",
        ),
        ("custom-direct-ips", "ipcidr", "custom-direct-ips.txt"),
    ] {
        map.insert(
            name.into(),
            RuleProvider {
                kind: "file".into(),
                behavior: behavior.into(),
                format: "text".into(),
                path: path.into(),
            },
        );
    }
    for client in app.enabled_clients() {
        let id = client.id.as_hyphenated();
        map.insert(
            format!("custom-{id}-domains"),
            RuleProvider {
                kind: "file".into(),
                behavior: "domain".into(),
                format: "text".into(),
                path: format!("custom-{id}-domains.txt"),
            },
        );
        map.insert(
            format!("custom-{id}-ips"),
            RuleProvider {
                kind: "file".into(),
                behavior: "ipcidr".into(),
                format: "text".into(),
                path: format!("custom-{id}-ips.txt"),
            },
        );
    }
    map
}

fn optional_rule_provider(name: &str) -> bool {
    name == "custom-direct-domains"
        || name == "custom-direct-ips"
        || iran_split_config::is_custom_client_generation_file(&format!("{name}.txt"))
}

fn summarize_rule_providers(value: &Value) -> Result<ProviderStatus, MihomoError> {
    let providers = value
        .get("providers")
        .and_then(Value::as_object)
        .ok_or_else(|| MihomoError::InvalidConfig("controller omitted providers map".into()))?;
    let total = u32::try_from(providers.len()).unwrap_or(u32::MAX);
    let ready = providers
        .iter()
        .filter(|(name, provider)| rule_provider_is_ready(name, provider))
        .count();
    let rules_loaded = providers
        .values()
        .filter_map(|provider| provider.get("ruleCount").and_then(Value::as_u64))
        .sum();
    Ok(ProviderStatus {
        ready: u32::try_from(ready).unwrap_or(u32::MAX),
        total,
        rules_loaded,
    })
}

fn rule_provider_is_ready(name: &str, provider: &Value) -> bool {
    if !provider.get("error").is_none_or(Value::is_null) {
        return false;
    }
    if optional_rule_provider(name) {
        return true;
    }
    provider
        .get("ruleCount")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0
}

fn validate_custom_rules(document: &RoutePinsDocument) -> Result<(), MihomoError> {
    for rule in &document.pins {
        if let DirectTarget::Domain(domain) = &rule.target {
            if domain.contains(',') || domain.contains('\n') || domain.contains('\r') {
                return Err(MihomoError::InvalidConfig(
                    "custom domain contains a provider control character".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Asks a Mihomo binary to validate a generated configuration file.
///
/// # Errors
///
/// Returns an error when the validation process cannot run, times out, or
/// rejects the configuration.
pub async fn validate_with_binary(
    binary: &Path,
    config_path: &Path,
    timeout: Duration,
) -> Result<(), MihomoError> {
    let workdir = config_path.parent().ok_or_else(|| {
        MihomoError::InvalidConfig("configuration path must include a parent directory".into())
    })?;
    let output = tokio::time::timeout(
        timeout,
        Command::new(binary)
            .arg("-t")
            .arg("-d")
            .arg(workdir)
            .arg("-f")
            .arg(config_path)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        MihomoError::ReadinessTimeout("configuration validation process timed out".into())
    })?
    .map_err(MihomoError::ValidationProcess)?;
    if !output.status.success() {
        let details = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let details = details.trim();
        let message = if details.is_empty() {
            "Mihomo rejected the configuration without details".into()
        } else {
            details.chars().take(4_096).collect()
        };
        return Err(MihomoError::ValidationRejected(message));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ControllerClient {
    base_url: String,
    secret: String,
    client: reqwest::Client,
}

impl ControllerClient {
    /// Creates a client restricted to a loopback Mihomo controller.
    ///
    /// # Errors
    ///
    /// Returns an error when `host` is not a loopback IP address, the secret is
    /// empty, or the HTTP client cannot be constructed.
    pub fn new(host: &str, port: u16, secret: impl Into<String>) -> Result<Self, MihomoError> {
        let address: IpAddr = host.parse().map_err(|_| {
            MihomoError::InvalidConfig("controller host must be an IP address".into())
        })?;
        if !address.is_loopback() {
            return Err(MihomoError::InvalidConfig(
                "controller must use a loopback address".into(),
            ));
        }
        let secret = secret.into();
        if secret.trim().is_empty() {
            return Err(MihomoError::InvalidConfig(
                "controller secret must not be empty".into(),
            ));
        }
        let host = match address {
            IpAddr::V4(_) => address.to_string(),
            IpAddr::V6(_) => format!("[{address}]"),
        };
        Ok(Self {
            base_url: format!("http://{host}:{port}"),
            secret,
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                // Loopback controller traffic must never follow HTTP_PROXY /
                // HTTPS_PROXY / ALL_PROXY. Hiddify and other local proxies
                // intercept those and return connection errors or HTTP 500.
                .no_proxy()
                .build()?,
        })
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(&self.secret)
    }

    /// Reads the running Mihomo version.
    ///
    /// # Errors
    ///
    /// Returns an error when the controller request or response decoding fails.
    pub async fn version(&self) -> Result<VersionResponse, MihomoError> {
        Ok(self
            .get("/version")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Reads session upload and download totals from the controller.
    ///
    /// # Errors
    ///
    /// Returns an error when the controller request or response decoding fails.
    pub async fn connection_totals(&self) -> Result<(u64, u64), MihomoError> {
        let snapshot = self.connections_snapshot().await?;
        Ok((snapshot.upload_total, snapshot.download_total))
    }

    /// Reads live connections from the controller.
    ///
    /// # Errors
    ///
    /// Returns an error when the controller request or response decoding fails.
    pub async fn active_connections(&self) -> Result<Vec<ActiveConnection>, MihomoError> {
        let snapshot = self.connections_snapshot().await?;
        Ok(snapshot
            .connections
            .into_iter()
            .map(ActiveConnection::from)
            .collect())
    }

    async fn connections_snapshot(&self) -> Result<ConnectionsSnapshot, MihomoError> {
        Ok(self
            .get("/connections")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Reads the active Mihomo configuration from the controller.
    ///
    /// # Errors
    ///
    /// Returns an error when the controller request or response decoding fails.
    pub async fn configs(&self) -> Result<Value, MihomoError> {
        Ok(self
            .get("/configs")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Summarizes the readiness and rule count of configured rule providers.
    ///
    /// # Errors
    ///
    /// Returns an error when the controller request fails, its response cannot
    /// be decoded, or it omits the provider map.
    pub async fn provider_summary(&self) -> Result<ProviderStatus, MihomoError> {
        let value = self
            .get("/providers/rules")
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        summarize_rule_providers(&value)
    }

    /// Replaces the active Mihomo configuration without restarting the process.
    ///
    /// Overlay must already have copied the new generation into the running
    /// `-d` workdir. Meta 1.19+ rejects a relative `path` on this endpoint
    /// (`path is not a absolute path`) and also rejects a sibling directory,
    /// so the body leaves `path` empty and reloads the process default file.
    ///
    /// # Errors
    ///
    /// Returns an error when the controller request fails or does not return
    /// HTTP 204 No Content.
    pub async fn hot_reload(&self) -> Result<(), MihomoError> {
        let response = self
            .client
            .put(format!("{}/configs?force=true", self.base_url))
            .timeout(Duration::from_secs(20))
            .bearer_auth(&self.secret)
            .json(&serde_json::json!({
                "path": "",
                "payload": "",
            }))
            .send()
            .await?;
        if response.status() != StatusCode::NO_CONTENT {
            return Err(controller_reload_error(response).await);
        }
        Ok(())
    }

    /// Closes live connections whose host or destination matches `pin`.
    ///
    /// Used after a pin apply so the moved name reconnects on the new
    /// outbound without dropping unrelated sites. Does not log the pin.
    ///
    /// # Errors
    ///
    /// Returns an error when listing connections fails. Individual close
    /// failures are skipped so a vanished id cannot fail the apply.
    pub async fn close_connections_matching(&self, pin: &str) -> Result<usize, MihomoError> {
        let snapshot = self.connections_snapshot().await?;
        let mut closed = 0_usize;
        for entry in snapshot.connections {
            if entry.id.is_empty() {
                continue;
            }
            let host = if entry.metadata.host.is_empty() {
                entry.metadata.destination_ip.as_str()
            } else {
                entry.metadata.host.as_str()
            };
            if !pin_matches_connection(pin, host, &entry.metadata.destination_ip) {
                continue;
            }
            let response = self
                .client
                .delete(format!("{}/connections/{}", self.base_url, entry.id))
                .bearer_auth(&self.secret)
                .send()
                .await?;
            if response.status().is_success() || response.status() == StatusCode::NO_CONTENT {
                closed += 1;
            }
        }
        Ok(closed)
    }

    /// Closes live connections for a pin apply. `google.com` also closes
    /// Search companion hosts so stale MATCH sockets cannot keep the page on
    /// the previous outbound.
    ///
    /// # Errors
    ///
    /// Returns an error when listing connections fails.
    pub async fn close_connections_for_pin_apply(&self, pin: &str) -> Result<usize, MihomoError> {
        let mut closed = 0_usize;
        for host in iran_split_rules::rebind_hosts_for_pin(pin) {
            closed += self.close_connections_matching(&host).await?;
        }
        Ok(closed)
    }

    /// Waits until Mihomo and every rule provider are ready.
    ///
    /// # Errors
    ///
    /// Returns [`MihomoError::Cancelled`] when cancelled,
    /// [`MihomoError::Unauthorized`] when the controller rejects the secret, or
    /// [`MihomoError::ReadinessTimeout`] after the supplied timeout.
    pub async fn wait_until_ready(
        &self,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<ProviderStatus, MihomoError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last_status;
        loop {
            if cancel.is_cancelled() {
                return Err(MihomoError::Cancelled);
            }
            match (self.version().await, self.provider_summary().await) {
                (Ok(_), Ok(providers)) => {
                    last_status = format!(
                        "providers {}/{} ready, {} rules loaded",
                        providers.ready, providers.total, providers.rules_loaded
                    );
                    if providers.total > 0 && providers.ready == providers.total {
                        return Ok(providers);
                    }
                }
                (Err(error), _) if controller_rejected_secret(&error) => {
                    return Err(MihomoError::Unauthorized);
                }
                (Ok(_), Err(error)) if controller_rejected_secret(&error) => {
                    return Err(MihomoError::Unauthorized);
                }
                (Err(error), _) => {
                    last_status = format!("controller unavailable: {error}");
                }
                (Ok(_), Err(error)) => {
                    last_status = format!("provider status unavailable: {error}");
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(MihomoError::ReadinessTimeout(last_status));
            }
            tokio::select! {
                () = cancel.cancelled() => return Err(MihomoError::Cancelled),
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    }

    /// Opens a controller WebSocket and returns its asynchronous log receiver.
    #[must_use]
    pub fn stream_logs(&self, level: &str) -> mpsc::Receiver<Result<MihomoLog, MihomoError>> {
        let (sender, receiver) = mpsc::channel(128);
        let url = format!(
            "{}/logs?level={level}",
            self.base_url.replacen("http", "ws", 1)
        );
        let secret = self.secret.clone();
        tokio::spawn(async move {
            let mut request = match url.into_client_request() {
                Ok(request) => request,
                Err(error) => {
                    let _ = sender
                        .send(Err(MihomoError::WebSocket(error.to_string())))
                        .await;
                    return;
                }
            };
            let value = match format!("Bearer {secret}").parse() {
                Ok(value) => value,
                Err(error) => {
                    let _ = sender
                        .send(Err(MihomoError::WebSocket(format!(
                            "invalid authorization header: {error}"
                        ))))
                        .await;
                    return;
                }
            };
            request.headers_mut().insert(header::AUTHORIZATION, value);
            let (mut stream, _) = match connect_async(request).await {
                Ok(connection) => connection,
                Err(error) => {
                    let _ = sender
                        .send(Err(MihomoError::WebSocket(error.to_string())))
                        .await;
                    return;
                }
            };
            while let Some(message) = stream.next().await {
                let result = message
                    .map_err(|error| MihomoError::WebSocket(error.to_string()))
                    .and_then(|message| {
                        serde_json::from_slice::<MihomoLog>(&message.into_data())
                            .map_err(|error| MihomoError::WebSocket(error.to_string()))
                    });
                if sender.send(result).await.is_err() {
                    break;
                }
            }
        });
        receiver
    }
}

fn controller_rejected_secret(error: &MihomoError) -> bool {
    match error {
        MihomoError::Unauthorized => true,
        MihomoError::UnexpectedStatus(status) | MihomoError::ReloadRejected { status, .. } => {
            *status == StatusCode::UNAUTHORIZED
        }
        MihomoError::Http(inner) => inner.status() == Some(StatusCode::UNAUTHORIZED),
        _ => false,
    }
}

async fn controller_reload_error(response: reqwest::Response) -> MihomoError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<Value>(&body).ok().and_then(|value| {
        value
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(|message| message.chars().take(200).collect::<String>())
    });
    match detail {
        Some(detail) => MihomoError::ReloadRejected { status, detail },
        None => MihomoError::UnexpectedStatus(status),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VersionResponse {
    pub version: String,
    #[serde(default)]
    pub meta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatus {
    pub ready: u32,
    pub total: u32,
    pub rules_loaded: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MihomoLog {
    #[serde(rename = "type")]
    pub level: String,
    pub payload: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExitIpResponse {
    pub ip: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ConnectionsSnapshot {
    #[serde(rename = "uploadTotal", default)]
    upload_total: u64,
    #[serde(rename = "downloadTotal", default)]
    download_total: u64,
    #[serde(default)]
    connections: Vec<ConnectionEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct ConnectionEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    metadata: ConnectionMetadata,
    #[serde(default)]
    chains: Vec<String>,
    #[serde(default)]
    rule: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ConnectionMetadata {
    #[serde(default)]
    host: String,
    #[serde(rename = "destinationIP", default)]
    destination_ip: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActiveConnection {
    pub host: String,
    pub destination_ip: String,
    pub outbound: String,
    pub rule: String,
}

impl From<ConnectionEntry> for ActiveConnection {
    fn from(entry: ConnectionEntry) -> Self {
        let host = if entry.metadata.host.is_empty() {
            entry.metadata.destination_ip.clone()
        } else {
            entry.metadata.host
        };
        Self {
            destination_ip: entry.metadata.destination_ip,
            outbound: classify_outbound(&entry.chains, &entry.rule),
            rule: entry.rule,
            host,
        }
    }
}

/// True when a live connection belongs to the pin the operator just moved.
///
/// Domain pins cover the exact name and every subdomain. IP pins match the
/// destination address. `notgoogle.com` does not match `google.com`.
#[must_use]
pub fn pin_matches_connection(pin: &str, host: &str, destination_ip: &str) -> bool {
    let pin = pin.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if pin.is_empty() {
        return false;
    }
    if pin.parse::<IpAddr>().is_ok() {
        return destination_ip.eq_ignore_ascii_case(&pin) || host == pin;
    }
    host == pin || host.ends_with(&format!(".{pin}"))
}

fn classify_outbound(chains: &[String], rule: &str) -> String {
    if let Some(last) = chains.last() {
        return if last.eq_ignore_ascii_case("DIRECT") {
            "direct".into()
        } else {
            last.clone()
        };
    }
    if rule.eq_ignore_ascii_case("DIRECT") {
        "direct".into()
    } else {
        rule.to_owned()
    }
}

/// Resolves the public egress IP through the configured Hiddify SOCKS proxy.
///
/// # Errors
///
/// Returns an error when the proxy/client cannot be configured, the request
/// fails, or the service returns an invalid IP address.
const EGRESS_PROBE_URLS: &[&str] = &[
    "https://cp.cloudflare.com/generate_204",
    "http://cp.cloudflare.com/generate_204",
    "https://www.gstatic.com/generate_204",
];

/// Resolves the public egress IP through the configured Hiddify proxy.
///
/// Tries SOCKS then HTTP CONNECT on the mixed port, and more than one
/// `generate_204` host, so a single blocked Google URL does not fail Connect.
///
/// # Errors
///
/// Returns [`MihomoError::EgressProbe`] when every proxy/host combination
/// fails, or the client cannot be built.
pub async fn probe_hiddify_egress(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<String, MihomoError> {
    let mut last = String::from("no probe ran");
    for proxy_kind in ["socks5h", "http"] {
        let proxy = reqwest::Proxy::all(format!("{proxy_kind}://{host}:{port}"))
            .map_err(|error| MihomoError::EgressProbe(error.without_url().to_string()))?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(proxy)
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|error| MihomoError::EgressProbe(error.to_string()))?;
        for url in EGRESS_PROBE_URLS {
            match client.get(*url).send().await {
                Ok(response)
                    if response.status().as_u16() == 204 || response.status().is_success() =>
                {
                    return Ok(optional_exit_ip(&client).await);
                }
                Ok(response) => {
                    last = format!("{proxy_kind} HTTP {}", response.status().as_u16());
                }
                Err(error) => {
                    last = format!("{proxy_kind} {}", error.without_url());
                }
            }
        }
    }
    Err(MihomoError::EgressProbe(last))
}

async fn optional_exit_ip(client: &reqwest::Client) -> String {
    let Ok(response) = client.get("https://api.ipify.org?format=json").send().await else {
        return "unknown".into();
    };
    if !response.status().is_success() {
        return "unknown".into();
    }
    let Ok(payload) = response.json::<ExitIpResponse>().await else {
        return "unknown".into();
    };
    if payload.ip.parse::<IpAddr>().is_ok() {
        payload.ip
    } else {
        "unknown".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iran_split_clients::{synthesized_local_handle, MihomoOutbound};
    use iran_split_config::{ClientInstance, EgressKind, PresetId};
    use iran_split_rules::{Outbound, PinnedRoute, RoutePinsDocument};
    use std::net::Ipv4Addr;

    fn paths() -> RuntimePaths {
        RuntimePaths {
            private_networks: "/runtime/private.txt".into(),
            iran_domains: "/runtime/iran-domains.txt".into(),
            iran_business_domains: "/runtime/iran-business-domains.txt".into(),
            iran_networks: "/runtime/iran-networks.txt".into(),
            iran_cdn_networks: "/runtime/iran-cdn-networks.txt".into(),
            custom_direct_domains: "/runtime/custom-direct-domains.txt".into(),
            custom_direct_ips: "/runtime/custom-direct-ips.txt".into(),
        }
    }

    fn match_needle(app: &AppConfig) -> String {
        format!("MATCH,{}", app.clients[0].group_name())
    }

    #[test]
    fn generated_config_is_loopback_secret_and_precedence_safe() {
        let app = AppConfig::default();
        let custom = RoutePinsDocument {
            revision: 1,
            pins: vec![PinnedRoute {
                target: DirectTarget::Ip(Ipv4Addr::new(203, 0, 113, 1).into()),
                outbound: Outbound::Direct,
                list_id: None,
                resolved_ips: vec![],
                created_at: chrono::Utc::now(),
                refreshed_at: None,
            }],
            lists: vec![],
        };
        let generated = generate_config(&app, Platform::Linux, &paths(), &custom).expect("config");
        let match_line = match_needle(&app);
        assert!(generated
            .yaml
            .contains("external-controller: 127.0.0.1:19090"));
        assert!(generated.yaml.contains("secret:"));
        assert!(generated.yaml.contains("PROCESS-NAME,hiddify,DIRECT"));
        assert!(generated
            .yaml
            .contains("PROCESS-NAME-WILDCARD,*Hiddify*,DIRECT"));
        assert!(generated
            .yaml
            .contains("PROCESS-NAME,iran-split-desk,DIRECT"));
        assert!(generated.yaml.contains(&match_line));
        let quic_reject = generated
            .yaml
            .find("AND,((NETWORK,udp),(DST-PORT,443)),REJECT")
            .expect("QUIC reject rule");
        let iran_networks = generated
            .yaml
            .find("RULE-SET,iran-networks,DIRECT")
            .expect("iran-networks rule");
        let iran_cdn_networks = generated
            .yaml
            .find("RULE-SET,iran-cdn-networks,DIRECT")
            .expect("iran-cdn-networks rule");
        let match_vpn = generated.yaml.find(&match_line).expect("match rule");
        assert!(
            iran_networks < iran_cdn_networks
                && iran_cdn_networks < quic_reject
                && quic_reject < match_vpn
        );
        assert!(generated.yaml.contains("find-process-mode: always"));
        assert!(generated.yaml.contains("ipv6: true"));
        assert!(generated
            .yaml
            .contains(&format!("dns-query#{}", app.clients[0].group_name())));
        assert!(generated.yaml.contains("path: private.txt"));
        assert!(generated.yaml.contains("path: iran-business-domains.txt"));
        assert!(generated.yaml.contains("path: iran-cdn-networks.txt"));
        assert!(generated
            .yaml
            .contains("RULE-SET,iran-business-domains,DIRECT"));
        assert!(generated.yaml.contains("RULE-SET,iran-cdn-networks,DIRECT"));
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&generated.yaml).expect("generated yaml");
        let dns = parsed.get("dns").expect("dns");
        // fake-ip default keeps Cloudflare DoH: no Iranian resolver policy...
        assert!(dns.get("nameserver-policy").is_none());
        assert!(dns.get("direct-nameserver").is_none());
        // ...but DIRECT domains must still skip fake-ip so the 198.18/15 fake
        // IP never collides with private.txt (ADR 0058 / ADR 0061).
        let filter = dns
            .get("fake-ip-filter")
            .and_then(serde_yaml::Value::as_sequence)
            .expect("fake-ip-filter");
        for key in [
            "rule-set:custom-direct-domains",
            "rule-set:iran-domains",
            "rule-set:iran-business-domains",
        ] {
            assert!(
                filter.iter().any(|item| item.as_str() == Some(key)),
                "fake-ip-filter missing {key}"
            );
        }
        assert!(
            !filter
                .iter()
                .any(|item| item.as_str() == Some("rule-set:iran-cdn-networks")),
            "CIDR providers must not enter fake-ip-filter"
        );
        assert!(!generated.yaml.contains("178.22.122.100"));
        assert!(!generated.yaml.contains("/runtime/"));
        assert_eq!(generated.sha256.len(), 64);
    }

    #[test]
    fn windows_enables_strict_route_and_hiddify_bypass() {
        let generated = generate_config(
            &AppConfig::default(),
            Platform::Windows,
            &paths(),
            &RoutePinsDocument::default(),
        )
        .expect("config");
        assert!(generated.yaml.contains("strict-route: true"));
        assert!(generated.yaml.contains("find-process-mode: always"));
        assert!(generated.yaml.contains("auto-redirect: false"));
        assert!(generated.yaml.contains("ipv6: false"));
        assert!(generated.yaml.contains("dns-query#client-"));
        assert!(generated.yaml.contains("PROCESS-NAME,Hiddify.exe,DIRECT"));
        assert!(generated
            .yaml
            .contains("PROCESS-NAME-WILDCARD,*Hiddify*,DIRECT"));
        assert!(generated.yaml.contains("PROCESS-NAME,BiFlow.exe,DIRECT"));
    }

    #[test]
    fn generated_config_uses_the_selected_direct_dns_preset() {
        let mut app = AppConfig::default();
        app.mihomo.direct_dns_preset = iran_split_config::DirectDnsPreset::Mokhaberat;
        let generated = generate_config(
            &app,
            Platform::Linux,
            &paths(),
            &RoutePinsDocument::default(),
        )
        .expect("config");
        assert!(generated.yaml.contains("5.200.200.200"));
        assert!(!generated.yaml.contains("178.22.122.100"));
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&generated.yaml).expect("generated yaml");
        let policy = parsed
            .get("dns")
            .and_then(|dns| dns.get("nameserver-policy"))
            .expect("nameserver-policy");
        for key in [
            "rule-set:custom-direct-domains",
            "rule-set:iran-domains",
            "rule-set:iran-business-domains",
        ] {
            assert!(policy.get(key).is_some(), "DIRECT DNS policy missing {key}");
        }
    }

    #[test]
    fn fake_ip_default_still_excludes_direct_domains_from_fake_ip() {
        // Regression for ADR 0061: the fake_ip default must not gate
        // fake-ip-filter. Iranian/custom DIRECT domains have to resolve to a
        // real address (witness: console.kavenegar.com, iran.ir), even though
        // this preset keeps Cloudflare DoH and emits no nameserver-policy.
        let app = AppConfig::default();
        assert_eq!(
            app.mihomo.direct_dns_preset,
            iran_split_config::DirectDnsPreset::FakeIp
        );
        let generated = generate_config(
            &app,
            Platform::Linux,
            &paths(),
            &RoutePinsDocument::default(),
        )
        .expect("config");
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&generated.yaml).expect("generated yaml");
        let dns = parsed.get("dns").expect("dns");
        assert!(dns.get("nameserver-policy").is_none());
        assert!(dns.get("direct-nameserver").is_none());
        let filter = dns
            .get("fake-ip-filter")
            .and_then(serde_yaml::Value::as_sequence)
            .expect("fake-ip-filter");
        for key in [
            "rule-set:custom-direct-domains",
            "rule-set:iran-domains",
            "rule-set:iran-business-domains",
        ] {
            assert!(
                filter.iter().any(|item| item.as_str() == Some(key)),
                "fake-ip-filter missing {key}"
            );
        }
    }

    #[test]
    fn generation_emits_one_group_per_ready_client_and_match_can_be_direct() {
        let mut app = AppConfig::default();
        let happ =
            iran_split_config::ClientInstance::from_preset(iran_split_config::PresetId::Happ);
        let happ_id = happ.id;
        app.clients.push(happ);
        let generated = generate_config(
            &app,
            Platform::Linux,
            &paths(),
            &RoutePinsDocument::default(),
        )
        .expect("config");
        assert!(generated.yaml.contains(&app.clients[0].group_name()));
        assert!(generated
            .yaml
            .contains(&format!("client-{}", happ_id.as_hyphenated())));
        assert!(generated
            .yaml
            .contains(&format!("custom-{}-domains.txt", happ_id.as_hyphenated())));
        // The Debian Happ daemon is lowercase `happd`; PROCESS-NAME is
        // case-sensitive on Linux, so the bypass must name it exactly or
        // the daemon's upstream traffic loops into the TUN.
        assert!(generated.yaml.contains("PROCESS-NAME,happd,DIRECT"));
        assert!(generated.yaml.contains("PROCESS-NAME,happ,DIRECT"));
        assert!(generated
            .yaml
            .contains("PROCESS-NAME-WILDCARD,*happ*,DIRECT"));
        app.default_route = DefaultRoute::Direct;
        let direct = generate_config(
            &app,
            Platform::Linux,
            &paths(),
            &RoutePinsDocument::default(),
        )
        .expect("direct match");
        assert!(direct.yaml.contains("MATCH,DIRECT"));
        assert!(!direct.yaml.contains("dns-query#client-"));
    }

    fn pin(domain: &str, outbound: Outbound) -> PinnedRoute {
        PinnedRoute {
            target: DirectTarget::Domain(domain.into()),
            outbound,
            list_id: None,
            resolved_ips: vec![],
            created_at: chrono::Utc::now(),
            refreshed_at: None,
        }
    }

    #[test]
    fn match_routes_to_a_non_hiddify_primary_without_hiddify_present() {
        // The operator removed Hiddify and promoted Happ to the default
        // route: MATCH and DoH must follow the Happ group.
        let mut app = AppConfig::default();
        app.clients.clear();
        let happ =
            iran_split_config::ClientInstance::from_preset(iran_split_config::PresetId::Happ);
        app.default_route = DefaultRoute::client(happ.id);
        app.clients.push(happ);
        let handle = synthesized_local_handle(&app.clients[0]).expect("happ handle");
        let group = app.clients[0].group_name();
        let generated = generate_config_with_handles(
            &app,
            Platform::Linux,
            &paths(),
            &RoutePinsDocument::default(),
            &[handle],
        )
        .expect("config");
        assert!(generated.yaml.contains(&format!("MATCH,{group}")));
        assert!(generated.yaml.contains(&format!("dns-query#{group}")));
        assert!(!generated.yaml.contains("MATCH,REJECT"));
        assert!(!generated.yaml.contains("MATCH,DIRECT"));
    }

    fn ready_side_tunnel_handle(client: &ClientInstance) -> EgressHandle {
        EgressHandle {
            client_id: client.id,
            preset: client.preset,
            kind: EgressKind::OwnedSideTunnel,
            ready: true,
            degraded: false,
            outbound: Some(MihomoOutbound {
                name: client.proxy_name(),
                group_name: client.group_name(),
                kind: "direct".into(),
                server: None,
                port: None,
                udp: true,
                interface_name: Some("tun-windscribe".into()),
                routing_mark: Some(200),
            }),
            transport_excludes: Vec::new(),
        }
    }

    #[test]
    fn dead_secondary_keeps_match_on_the_healthy_primary() {
        // Hiddify stays primary; a second client (Happ) is enabled but its
        // egress never came up. MATCH stays on the primary. The Happ pin
        // still names the Happ group (stub proxy) instead of REJECT.
        let mut app = AppConfig::default();
        let happ = ClientInstance::from_preset(PresetId::Happ);
        let happ_id = happ.id;
        app.clients.push(happ);
        let hiddify_group = app.clients[0].group_name();
        let happ_group = app.clients[1].group_name();
        let hiddify_handle = synthesized_local_handle(&app.clients[0]).expect("hiddify handle");
        let pinned = RoutePinsDocument {
            revision: 1,
            pins: vec![
                pin("pinned-to-happ.example", Outbound::client(happ_id)),
                pin(
                    "pinned-to-hiddify.example",
                    Outbound::client(app.clients[0].id),
                ),
            ],
            lists: vec![],
        };
        let generated = generate_config_with_handles(
            &app,
            Platform::Linux,
            &paths(),
            &pinned,
            &[hiddify_handle],
        )
        .expect("config");
        assert!(generated.yaml.contains(&format!("MATCH,{hiddify_group}")));
        assert!(generated.yaml.contains(&format!(
            "DOMAIN-SUFFIX,pinned-to-happ.example,{happ_group}"
        )));
        assert!(!generated
            .yaml
            .contains("DOMAIN-SUFFIX,pinned-to-happ.example,REJECT"));
        assert!(generated.yaml.contains(&format!(
            "DOMAIN-SUFFIX,pinned-to-hiddify.example,{hiddify_group}"
        )));
        assert!(generated.yaml.contains(&happ_group));
    }

    #[test]
    fn three_ready_clients_route_their_own_pins() {
        // Hiddify primary plus two more local proxies, all serving: every
        // pin follows its own client group and MATCH stays on the primary.
        let mut app = AppConfig::default();
        for preset in [
            iran_split_config::PresetId::Happ,
            iran_split_config::PresetId::V2rayn,
        ] {
            app.clients
                .push(iran_split_config::ClientInstance::from_preset(preset));
        }
        // Happ and v2rayN share the catalog default 10808; generation
        // enforces unique local ports, so the third client moves off it.
        if let iran_split_config::ClientConfig::LocalProxy { port, .. } = &mut app.clients[2].config
        {
            *port = 10_809;
        }
        let handles: Vec<EgressHandle> = app
            .clients
            .iter()
            .map(|client| synthesized_local_handle(client).expect("handle"))
            .collect();
        let pinned = RoutePinsDocument {
            revision: 1,
            pins: app
                .clients
                .iter()
                .enumerate()
                .map(|(index, client)| {
                    pin(
                        &format!("client-{index}.example"),
                        Outbound::client(client.id),
                    )
                })
                .collect(),
            lists: vec![],
        };
        let generated =
            generate_config_with_handles(&app, Platform::Linux, &paths(), &pinned, &handles)
                .expect("config");
        for (index, client) in app.clients.iter().enumerate() {
            assert!(
                generated.yaml.contains(&format!(
                    "DOMAIN-SUFFIX,client-{index}.example,{}",
                    client.group_name()
                )),
                "pin {index} must route to its own client group"
            );
        }
        assert!(generated.yaml.contains(&match_needle(&app)));
    }

    #[test]
    fn domain_pins_emit_most_specific_first_across_outbounds() {
        let app = AppConfig::default();
        let id = app.clients[0].id;
        let now = chrono::Utc::now();
        let pinned = RoutePinsDocument {
            revision: 1,
            pins: vec![
                PinnedRoute {
                    target: DirectTarget::Domain("google.com".into()),
                    outbound: Outbound::Direct,
                    list_id: None,
                    resolved_ips: vec![],
                    created_at: now,
                    refreshed_at: None,
                },
                PinnedRoute {
                    target: DirectTarget::Domain("developer.google.com".into()),
                    outbound: Outbound::client(id),
                    list_id: None,
                    resolved_ips: vec![],
                    created_at: now,
                    refreshed_at: None,
                },
            ],
            lists: vec![],
        };
        let generated = generate_config(&app, Platform::Linux, &paths(), &pinned).expect("config");
        let group = app.clients[0].group_name();
        let specific = generated
            .yaml
            .find(&format!("DOMAIN-SUFFIX,developer.google.com,{group}"))
            .expect("specific pin");
        let root = generated
            .yaml
            .find("DOMAIN-SUFFIX,google.com,DIRECT")
            .expect("root pin");
        assert!(
            specific < root,
            "the more specific pin must be evaluated first"
        );
    }

    #[test]
    fn fail_closed_rejects_match_when_the_default_client_is_down() {
        let app = AppConfig::default();
        assert!(app.behavior.fail_closed);
        let id = app.clients[0].id;
        let group = app.clients[0].group_name();
        // A degraded handle filters out of the live set, so the default
        // client is "down" while still enabled.
        let dead = EgressHandle {
            client_id: id,
            preset: app.clients[0].preset,
            kind: EgressKind::LocalProxy,
            ready: false,
            degraded: true,
            outbound: None,
            transport_excludes: Vec::new(),
        };
        let pinned = RoutePinsDocument {
            revision: 1,
            pins: vec![PinnedRoute {
                target: DirectTarget::Domain("office.example".into()),
                outbound: Outbound::client(id),
                list_id: None,
                resolved_ips: vec![],
                created_at: chrono::Utc::now(),
                refreshed_at: None,
            }],
            lists: vec![],
        };
        let generated =
            generate_config_with_handles(&app, Platform::Linux, &paths(), &pinned, &[dead.clone()])
                .expect("config");
        assert!(generated.yaml.contains("MATCH,REJECT"));
        assert!(generated
            .yaml
            .contains(&format!("DOMAIN-SUFFIX,office.example,{group}")));
        assert!(!generated
            .yaml
            .contains("DOMAIN-SUFFIX,office.example,REJECT"));
        assert!(generated.yaml.contains(&format!(
            "RULE-SET,custom-{}-ips,{group}",
            id.as_hyphenated()
        )));
        // REJECT is not a proxy group, so DoH must not be pinned to it.
        assert!(!generated.yaml.contains("dns-query#REJECT"));

        // Per-client exclusion downgrades MATCH to DIRECT.
        let mut excluded = app.clone();
        excluded.clients[0].allow_direct_when_down = true;
        let generated = generate_config_with_handles(
            &excluded,
            Platform::Linux,
            &paths(),
            &RoutePinsDocument::default(),
            &[dead.clone()],
        )
        .expect("config");
        assert!(generated.yaml.contains("MATCH,DIRECT"));
        assert!(!generated.yaml.contains("MATCH,REJECT"));

        // Disabling the global switch restores the old DIRECT fallback.
        let mut open = app;
        open.behavior.fail_closed = false;
        let generated = generate_config_with_handles(
            &open,
            Platform::Linux,
            &paths(),
            &RoutePinsDocument::default(),
            &[dead],
        )
        .expect("config");
        assert!(generated.yaml.contains("MATCH,DIRECT"));
    }

    #[test]
    fn windscribe_google_pin_covers_the_apex_and_keeps_the_client_group() {
        let mut app = AppConfig::default();
        let windscribe = ClientInstance::from_preset(PresetId::Windscribe);
        let windscribe_id = windscribe.id;
        app.clients.push(windscribe);
        let windscribe_group = app.clients[1].group_name();
        let hiddify_handle = synthesized_local_handle(&app.clients[0]).expect("hiddify");
        let windscribe_handle = ready_side_tunnel_handle(&app.clients[1]);
        let pinned = RoutePinsDocument {
            revision: 1,
            pins: vec![pin("google.com", Outbound::client(windscribe_id))],
            lists: vec![],
        };
        let generated = generate_config_with_handles(
            &app,
            Platform::Linux,
            &paths(),
            &pinned,
            &[hiddify_handle, windscribe_handle],
        )
        .expect("config");
        assert!(generated
            .yaml
            .contains(&format!("DOMAIN-SUFFIX,google.com,{windscribe_group}")));
        assert!(!generated.yaml.contains("DOMAIN-SUFFIX,google.com,DIRECT"));
        assert!(!generated.yaml.contains("DOMAIN-SUFFIX,google.com,REJECT"));
        assert!(generated.yaml.contains("interface-name: tun-windscribe"));
        assert!(generated
            .yaml
            .contains(&format!("MATCH,{}", app.clients[0].group_name())));
    }

    #[test]
    fn windscribe_google_pin_keeps_the_group_when_the_tunnel_is_down() {
        let mut app = AppConfig::default();
        let windscribe = ClientInstance::from_preset(PresetId::Windscribe);
        let windscribe_id = windscribe.id;
        app.clients.push(windscribe);
        let windscribe_group = app.clients[1].group_name();
        let hiddify_handle = synthesized_local_handle(&app.clients[0]).expect("hiddify");
        let pinned = RoutePinsDocument {
            revision: 1,
            pins: vec![
                pin("google.com", Outbound::client(windscribe_id)),
                PinnedRoute {
                    target: DirectTarget::Ip("203.0.113.8".parse().expect("ip")),
                    outbound: Outbound::client(windscribe_id),
                    list_id: None,
                    resolved_ips: vec![],
                    created_at: chrono::Utc::now(),
                    refreshed_at: None,
                },
            ],
            lists: vec![],
        };
        let generated = generate_config_with_handles(
            &app,
            Platform::Linux,
            &paths(),
            &pinned,
            &[hiddify_handle],
        )
        .expect("config");
        assert!(generated
            .yaml
            .contains(&format!("DOMAIN-SUFFIX,google.com,{windscribe_group}")));
        assert!(generated.yaml.contains(&format!(
            "RULE-SET,custom-{}-ips,{windscribe_group}",
            windscribe_id.as_hyphenated()
        )));
        assert!(!generated.yaml.contains("DOMAIN-SUFFIX,google.com,REJECT"));
        assert!(!generated.yaml.contains("DOMAIN-SUFFIX,google.com,DIRECT"));
        assert!(generated.yaml.contains(&windscribe_group));
        assert!(generated
            .yaml
            .contains(&format!("MATCH,{}", app.clients[0].group_name())));
    }

    #[test]
    fn controller_rejects_remote_binding_and_empty_secret() {
        assert!(ControllerClient::new("0.0.0.0", 9090, "secret").is_err());
        assert!(ControllerClient::new("127.0.0.1", 9090, "").is_err());
    }

    #[tokio::test]
    async fn wait_until_ready_fails_immediately_on_unauthorized() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0_u8; 1024];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            let response =
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response).await;
        });
        let client = ControllerClient::new(
            "127.0.0.1",
            port,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("client");
        let started = tokio::time::Instant::now();
        let error = client
            .wait_until_ready(Duration::from_secs(20), CancellationToken::new())
            .await
            .expect_err("401");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "HTTP 401 must not wait out the readiness budget"
        );
        assert!(
            matches!(error, MihomoError::Unauthorized),
            "expected Unauthorized, got {error}"
        );
        server.await.expect("server");
    }

    #[test]
    fn controller_client_bypasses_environment_proxies() {
        let source = include_str!("lib.rs");
        let constructor = source
            .split("impl ControllerClient")
            .nth(1)
            .and_then(|rest| rest.split("fn get(").next())
            .expect("ControllerClient::new");
        assert!(
            constructor.contains(".no_proxy()"),
            "loopback controller requests must ignore HTTP_PROXY"
        );
    }

    #[test]
    fn hiddify_egress_probe_tries_socks_and_http_without_env_proxy() {
        let source = include_str!("lib.rs");
        let probe = source
            .split("const EGRESS_PROBE_URLS")
            .nth(1)
            .and_then(|rest| rest.split("async fn optional_exit_ip").next())
            .expect("probe_hiddify_egress");
        assert!(
            probe.contains("socks5h") && probe.contains("\"http\""),
            "Hiddify mixed port may speak HTTP CONNECT instead of SOCKS"
        );
        assert!(
            probe.contains(".no_proxy()"),
            "egress probe must ignore HTTP_PROXY so it talks to Hiddify directly"
        );
        assert!(
            probe.contains("cp.cloudflare.com") && probe.contains("gstatic.com"),
            "one blocked generate_204 host must not fail Connect"
        );
        assert!(
            source.contains("Hiddify egress probe failed"),
            "probe failures must not reuse the Mihomo controller error"
        );
    }

    #[test]
    fn connection_totals_read_clash_field_names() {
        let parsed: ConnectionsSnapshot = serde_json::from_value(serde_json::json!({
            "uploadTotal": 11,
            "downloadTotal": 29
        }))
        .expect("connections snapshot");
        assert_eq!(parsed.upload_total, 11);
        assert_eq!(parsed.download_total, 29);
        assert!(parsed.connections.is_empty());
    }

    #[test]
    fn active_connections_classify_last_chain() {
        let parsed: ConnectionsSnapshot = serde_json::from_value(serde_json::json!({
            "uploadTotal": 1,
            "downloadTotal": 2,
            "connections": [
                {
                    "metadata": { "host": "digikala.ir", "destinationIP": "5.22.12.1" },
                    "chains": ["Iran", "DIRECT"],
                    "rule": "RuleSet"
                },
                {
                    "metadata": { "host": "openai.com", "destinationIP": "104.18.1.1" },
                    "chains": ["Hiddify", "PROXY"],
                    "rule": "MATCH"
                }
            ]
        }))
        .expect("connections snapshot");
        let rows: Vec<ActiveConnection> = parsed.connections.into_iter().map(Into::into).collect();
        assert_eq!(
            rows[0],
            ActiveConnection {
                host: "digikala.ir".into(),
                destination_ip: "5.22.12.1".into(),
                outbound: "direct".into(),
                rule: "RuleSet".into(),
            }
        );
        assert_eq!(rows[1].outbound, "PROXY");
        assert_eq!(rows[1].host, "openai.com");
    }

    #[test]
    fn pin_matches_connection_covers_subdomains_and_ips() {
        assert!(pin_matches_connection(
            "google.com",
            "google.com",
            "142.250.1.1"
        ));
        assert!(pin_matches_connection(
            "google.com",
            "www.google.com",
            "142.250.1.1"
        ));
        assert!(pin_matches_connection(
            "google.com",
            "gemini.google.com.",
            "142.250.1.1"
        ));
        assert!(!pin_matches_connection(
            "google.com",
            "notgoogle.com",
            "1.1.1.1"
        ));
        assert!(pin_matches_connection("8.8.8.8", "", "8.8.8.8"));
        assert!(!pin_matches_connection("8.8.8.8", "dns.google", "1.1.1.1"));
    }

    #[tokio::test]
    async fn close_connections_matching_deletes_only_the_pinned_host() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            let mut deleted = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut buf = [0_u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                let request = String::from_utf8_lossy(&buf);
                let (status, body) = if request.starts_with("GET /connections") {
                    (
                        "HTTP/1.1 200 OK",
                        concat!(
                            r#"{"uploadTotal":0,"downloadTotal":0,"connections":["#,
                            r#"{"id":"aaa","metadata":{"host":"www.google.com","destinationIP":"1.1.1.1"},"chains":["PROXY"],"rule":"MATCH"},"#,
                            r#"{"id":"bbb","metadata":{"host":"openai.com","destinationIP":"2.2.2.2"},"chains":["PROXY"],"rule":"MATCH"}"#,
                            r#"]}"#
                        ),
                    )
                } else if request.starts_with("DELETE /connections/aaa") {
                    deleted.push("aaa");
                    ("HTTP/1.1 204 No Content", "")
                } else {
                    deleted.push("other");
                    ("HTTP/1.1 204 No Content", "")
                };
                let response = if body.is_empty() {
                    format!("{status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                } else {
                    format!(
                        "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
            deleted
        });
        let client = ControllerClient::new(
            "127.0.0.1",
            port,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("client");
        let closed = client
            .close_connections_matching("google.com")
            .await
            .expect("close");
        assert_eq!(closed, 1);
        let deleted = server.await.expect("server");
        assert_eq!(deleted, vec!["aaa"]);
    }

    #[tokio::test]
    async fn hot_reload_reloads_the_running_workdir_default() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0_u8; 2048];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            let request = String::from_utf8_lossy(&buf);
            let response =
                b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response).await;
            request.contains("PUT /configs?force=true")
                && request.contains(r#""path":"""#)
                && request.contains(r#""payload":"""#)
                && !request.contains("config.yaml")
        });
        let client = ControllerClient::new(
            "127.0.0.1",
            port,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("client");
        client.hot_reload().await.expect("reload");
        assert!(server.await.expect("server"));
    }

    #[tokio::test]
    async fn hot_reload_includes_controller_message_on_400() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0_u8; 2048];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            let body = r#"{"message":"path is not a absolute path"}"#;
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
        });
        let client = ControllerClient::new(
            "127.0.0.1",
            port,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("client");
        let error = client.hot_reload().await.expect_err("400");
        let text = error.to_string();
        assert!(text.contains("400"), "{text}");
        assert!(text.contains("path is not a absolute path"), "{text}");
        server.await.expect("server");
    }

    #[test]
    fn empty_custom_providers_count_as_ready() {
        let summary = summarize_rule_providers(&serde_json::json!({
            "providers": {
                "custom-direct-domains": { "ruleCount": 0 },
                "custom-direct-ips": { "ruleCount": 0 },
                "iran-domains": { "ruleCount": 12 },
                "iran-networks": { "ruleCount": 4 },
                "private-networks": { "ruleCount": 8 }
            }
        }))
        .expect("summary");
        assert_eq!(summary.ready, 5);
        assert_eq!(summary.total, 5);
        assert_eq!(summary.rules_loaded, 24);
    }

    #[test]
    fn bundled_provider_without_rules_is_not_ready() {
        let summary = summarize_rule_providers(&serde_json::json!({
            "providers": {
                "custom-direct-domains": { "ruleCount": 0 },
                "iran-domains": { "ruleCount": 0 },
                "iran-networks": { "ruleCount": 4 },
                "iran-cdn-networks": { "ruleCount": 0 },
                "private-networks": { "ruleCount": 8 }
            }
        }))
        .expect("summary");
        assert_eq!(summary.ready, 3);
        assert_eq!(summary.total, 5);
    }

    #[test]
    fn provider_error_is_not_ready() {
        let summary = summarize_rule_providers(&serde_json::json!({
            "providers": {
                "custom-direct-domains": { "error": "read failed", "ruleCount": 0 },
                "iran-domains": { "ruleCount": 12 }
            }
        }))
        .expect("summary");
        assert_eq!(summary.ready, 1);
        assert_eq!(summary.total, 2);
    }

    #[tokio::test]
    async fn validates_generated_config_with_vendored_mihomo_binary() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root");
        let (mihomo, platform) = if cfg!(windows) {
            (
                workspace.join("vendor/mihomo/windows-x86_64/mihomo.exe"),
                Platform::Windows,
            )
        } else {
            (
                workspace.join("vendor/mihomo/linux-x86_64/mihomo"),
                Platform::Linux,
            )
        };
        if !mihomo.is_file() {
            eprintln!("skipping vendored Mihomo validation: {}", mihomo.display());
            return;
        }

        let generation = tempfile::tempdir().expect("tempdir");
        let rules = workspace.join("resources/rules");
        for name in [
            "private.txt",
            "iran-domains.txt",
            "iran-networks.txt",
            "iran-business-domains.txt",
            "iran-cdn-networks.txt",
        ] {
            std::fs::copy(rules.join(name), generation.path().join(name)).expect("rule file");
        }
        std::fs::write(generation.path().join("custom-direct-domains.txt"), "")
            .expect("custom domains");
        std::fs::write(generation.path().join("custom-direct-ips.txt"), "").expect("custom ips");
        let app = AppConfig::default();
        for name in app.clients[0].provider_files() {
            std::fs::write(generation.path().join(name), "").expect("client provider");
        }

        let generated = generate_config(&app, platform, &paths(), &RoutePinsDocument::default())
            .expect("config");
        let config_path = generation.path().join("config.yaml");
        std::fs::write(&config_path, generated.yaml.as_bytes()).expect("config yaml");

        validate_with_binary(&mihomo, &config_path, Duration::from_secs(30))
            .await
            .expect("mihomo should accept the generated configuration");
    }
}
