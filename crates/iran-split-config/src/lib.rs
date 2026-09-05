mod clients;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use thiserror::Error;

pub use clients::{
    is_custom_client_generation_file, ClientConfig, ClientId, ClientInstance, DefaultRoute,
    EgressKind, PresetId, PresetSpec, PresetStatus,
};

pub const CURRENT_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub revision: u64,
    pub clients: Vec<ClientInstance>,
    pub default_route: DefaultRoute,
    pub mihomo: MihomoConfig,
    pub rules: RulesConfig,
    pub behavior: BehaviorConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        let hiddify_id = ClientId::new();
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            revision: 0,
            clients: vec![ClientInstance::hiddify_default(hiddify_id)],
            default_route: DefaultRoute::client(hiddify_id),
            mihomo: MihomoConfig::default(),
            rules: RulesConfig::default(),
            behavior: BehaviorConfig::default(),
        }
    }
}

impl AppConfig {
    /// First enabled Hiddify instance, if the user still has one.
    #[must_use]
    pub fn hiddify_client(&self) -> Option<&ClientInstance> {
        self.clients
            .iter()
            .find(|client| client.preset == PresetId::Hiddify)
    }

    /// Enabled instances that should be started and emitted into Mihomo.
    #[must_use]
    pub fn enabled_clients(&self) -> Vec<&ClientInstance> {
        self.clients
            .iter()
            .filter(|client| client.enabled)
            .collect()
    }

    #[must_use]
    pub fn client(&self, id: ClientId) -> Option<&ClientInstance> {
        self.clients.iter().find(|client| client.id == id)
    }

    /// Loopback SOCKS endpoint of the Hiddify instance, or the catalog default.
    #[must_use]
    pub fn hiddify_endpoint(&self) -> (String, u16) {
        match self.hiddify_client().map(|client| &client.config) {
            Some(ClientConfig::LocalProxy { host, port, .. }) => (host.clone(), *port),
            _ => (
                PresetId::Hiddify.spec().default_host.into(),
                PresetId::Hiddify.spec().default_port.unwrap_or(12_334),
            ),
        }
    }

    #[must_use]
    pub fn hiddify_start_timeout(&self) -> u64 {
        match self.hiddify_client().map(|client| &client.config) {
            Some(ClientConfig::LocalProxy {
                start_timeout_seconds,
                ..
            }) => *start_timeout_seconds,
            _ => 45,
        }
    }

    #[must_use]
    pub fn hiddify_stop_with_stack(&self) -> bool {
        match self.hiddify_client().map(|client| &client.config) {
            Some(ClientConfig::LocalProxy {
                stop_with_stack, ..
            }) => *stop_with_stack,
            _ => true,
        }
    }

    #[must_use]
    pub fn hiddify_executable(&self) -> ExecutableSetting {
        match self.hiddify_client().map(|client| &client.config) {
            Some(ClientConfig::LocalProxy { executable, .. }) => executable.clone(),
            _ => ExecutableSetting::Auto,
        }
    }

    /// Falls back to [`DefaultRoute::Direct`] when the MATCH target is gone or disabled.
    pub fn sanitize_default_route(&mut self) -> bool {
        let DefaultRoute::Client { client_id } = self.default_route else {
            return false;
        };
        let usable = self
            .clients
            .iter()
            .any(|client| client.id == client_id && client.enabled);
        if usable {
            return false;
        }
        self.default_route = DefaultRoute::Direct;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutableSetting {
    #[default]
    Auto,
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MihomoConfig {
    pub controller_host: String,
    pub controller_port: u16,
    pub controller_secret: String,
    pub mixed_port: u16,
    pub dns_port: u16,
    pub tun_name: String,
    pub log_level: LogLevel,
    pub direct_dns_preset: DirectDnsPreset,
    /// Used when [`DirectDnsPreset::Custom`]. Named presets ignore this list.
    pub direct_dns_servers: Vec<String>,
}

impl Default for MihomoConfig {
    fn default() -> Self {
        Self {
            controller_host: "127.0.0.1".into(),
            controller_port: 19_090,
            controller_secret: generate_secret(),
            mixed_port: 17_890,
            dns_port: 1_053,
            tun_name: "clash-iran".into(),
            log_level: LogLevel::Info,
            direct_dns_preset: DirectDnsPreset::default(),
            direct_dns_servers: Vec::new(),
        }
    }
}

/// Resolvers for Iranian and user-pinned DIRECT domains (not VPN `DoH`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DirectDnsPreset {
    /// Mihomo fake-ip plus Cloudflare `DoH`. No Iranian nameserver policy.
    #[default]
    FakeIp,
    Shecan,
    Electro,
    Radar,
    Mokhaberat,
    Custom,
}

impl DirectDnsPreset {
    /// Built-in IPv4 resolvers. `FakeIp` and `Custom` have none.
    #[must_use]
    pub const fn servers(self) -> &'static [&'static str] {
        match self {
            Self::FakeIp | Self::Custom => &[],
            Self::Shecan => &["178.22.122.100", "185.51.200.2"],
            Self::Electro => &["78.157.42.100", "78.157.42.101"],
            Self::Radar => &["10.202.10.10", "10.202.10.11"],
            Self::Mokhaberat => &["5.200.200.200"],
        }
    }

    /// Whether DIRECT domains skip fake-ip and use these resolvers.
    #[must_use]
    pub const fn applies_direct_policy(self) -> bool {
        !matches!(self, Self::FakeIp)
    }
}

impl std::fmt::Display for DirectDnsPreset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::FakeIp => "fake_ip",
            Self::Shecan => "shecan",
            Self::Electro => "electro",
            Self::Radar => "radar",
            Self::Mokhaberat => "mokhaberat",
            Self::Custom => "custom",
        })
    }
}

impl MihomoConfig {
    /// Addresses written into Mihomo `direct-nameserver` and nameserver-policy.
    #[must_use]
    pub fn direct_dns_resolvers(&self) -> Vec<String> {
        if self.direct_dns_preset == DirectDnsPreset::Custom {
            return self
                .direct_dns_servers
                .iter()
                .map(|server| server.trim().to_owned())
                .filter(|server| !server.is_empty())
                .collect();
        }
        self.direct_dns_preset
            .servers()
            .iter()
            .map(|server| (*server).to_owned())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RulesConfig {
    pub refresh_interval_minutes: u64,
    pub upstream_refresh_hours: u64,
}

impl Default for RulesConfig {
    fn default() -> Self {
        Self {
            refresh_interval_minutes: 15,
            upstream_refresh_hours: 24,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
    pub launch_at_login: bool,
    pub connect_at_launch: bool,
    pub close_to_tray: bool,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            connect_at_launch: false,
            close_to_tray: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub field: String,
    pub code: String,
    pub message: String,
}

impl AppConfig {
    #[must_use]
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        validate_loopback(
            "mihomo.controller_host",
            &self.mihomo.controller_host,
            &mut issues,
        );

        if self.mihomo.controller_secret.trim().len() < 32 {
            issues.push(issue(
                "mihomo.controller_secret",
                "SECRET_TOO_SHORT",
                "controller secret must contain at least 32 characters",
            ));
        }
        if self.rules.refresh_interval_minutes == 0 || self.rules.upstream_refresh_hours == 0 {
            issues.push(issue(
                "rules",
                "OUT_OF_RANGE",
                "refresh intervals must be non-zero",
            ));
        }
        if self.mihomo.tun_name.trim().is_empty() || self.mihomo.tun_name.len() > 64 {
            issues.push(issue(
                "mihomo.tun_name",
                "INVALID_TUN_NAME",
                "TUN name must contain between 1 and 64 characters",
            ));
        }
        validate_direct_dns(&self.mihomo, &mut issues);
        clients::validate_clients(&self.clients, self.default_route, &mut issues);

        let mut ports = vec![
            (
                "mihomo.controller_port".to_owned(),
                self.mihomo.controller_port,
            ),
            ("mihomo.mixed_port".to_owned(), self.mihomo.mixed_port),
            ("mihomo.dns_port".to_owned(), self.mihomo.dns_port),
        ];
        for (index, client) in self.clients.iter().enumerate() {
            if let ClientConfig::LocalProxy { port, .. } = client.config {
                ports.push((format!("clients.{index}.port"), port));
            }
        }
        for (index, (field, port)) in ports.iter().enumerate() {
            if *port == 0 {
                issues.push(issue(field, "INVALID_PORT", "port cannot be zero"));
            }
            if ports[..index].iter().any(|(_, previous)| previous == port) {
                issues.push(issue(
                    field,
                    "PORT_CONFLICT",
                    "configured ports must be unique",
                ));
            }
        }
        issues
    }

    #[must_use]
    pub fn redacted(&self) -> Self {
        let mut value = self.clone();
        value.mihomo.controller_secret = "[REDACTED]".into();
        for client in &mut value.clients {
            client.config = client.config.redacted();
        }
        value
    }
}

fn validate_direct_dns(mihomo: &MihomoConfig, issues: &mut Vec<ValidationIssue>) {
    if !mihomo.direct_dns_preset.applies_direct_policy() {
        return;
    }
    let servers = mihomo.direct_dns_resolvers();
    if servers.is_empty() {
        issues.push(issue(
            "mihomo.direct_dns_servers",
            "DIRECT_DNS_REQUIRED",
            "DIRECT DNS needs at least one resolver address",
        ));
        return;
    }
    if servers.len() > 4 {
        issues.push(issue(
            "mihomo.direct_dns_servers",
            "DIRECT_DNS_TOO_MANY",
            "DIRECT DNS accepts at most four resolver addresses",
        ));
    }
    for server in servers {
        match server.parse::<IpAddr>() {
            Ok(address) if is_usable_direct_dns(address) => {}
            Ok(_) => issues.push(issue(
                "mihomo.direct_dns_servers",
                "DIRECT_DNS_INVALID",
                "DIRECT DNS cannot be loopback, unspecified, multicast, or fake-ip",
            )),
            Err(_) => issues.push(issue(
                "mihomo.direct_dns_servers",
                "DIRECT_DNS_INVALID",
                "DIRECT DNS resolvers must be IP addresses, not host names",
            )),
        }
    }
}

fn is_usable_direct_dns(address: IpAddr) -> bool {
    if address.is_loopback() || address.is_unspecified() || address.is_multicast() {
        return false;
    }
    match address {
        IpAddr::V4(value) => {
            let octets = value.octets();
            !(octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        }
        IpAddr::V6(_) => true,
    }
}

pub(crate) fn validate_loopback(field: &str, value: &str, issues: &mut Vec<ValidationIssue>) {
    match value.parse::<IpAddr>() {
        Ok(address) if address.is_loopback() => {}
        _ => issues.push(issue(
            field,
            "LOOPBACK_REQUIRED",
            "address must be a numeric loopback address",
        )),
    }
}

pub(crate) fn issue(field: &str, code: &str, message: &str) -> ValidationIssue {
    ValidationIssue {
        field: field.into(),
        code: code.into(),
        message: message.into(),
    }
}

fn generate_secret() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("configuration is not valid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("configuration could not be serialized: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("unsupported configuration schema version {0}")]
    UnsupportedSchema(u32),
    #[error("configuration validation failed")]
    Validation(Vec<ValidationIssue>),
    #[error("configuration revision conflict: expected {expected}, found {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("atomic configuration publication failed: {0}")]
    Persist(#[from] tempfile::PersistError),
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Loads the stored configuration, creating a validated default when absent.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the file cannot be read, parsed, migrated,
    /// validated, or atomically persisted.
    pub fn load_or_create(&self) -> Result<AppConfig, ConfigError> {
        if !self.path.exists() {
            let config = AppConfig::default();
            self.write_atomic(&config)?;
            return Ok(config);
        }
        self.load()
    }

    /// Loads, migrates, and validates the stored configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for I/O or TOML failures, unsupported schema
    /// versions, failed migrations, or invalid configuration values.
    pub fn load(&self) -> Result<AppConfig, ConfigError> {
        let source = fs::read_to_string(&self.path)?;
        let mut value: toml::Value = toml::from_str(&source)?;
        let schema = value
            .get("schema_version")
            .and_then(toml::Value::as_integer)
            .unwrap_or(0);
        let schema = u32::try_from(schema).unwrap_or(u32::MAX);
        if schema > CURRENT_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchema(schema));
        }
        if schema < CURRENT_SCHEMA_VERSION {
            self.backup()?;
            migrate(&mut value, schema)?;
        }
        let config: AppConfig = value.try_into()?;
        let issues = config.validate();
        if !issues.is_empty() {
            return Err(ConfigError::Validation(issues));
        }
        if schema < CURRENT_SCHEMA_VERSION {
            self.write_atomic(&config)?;
        }
        Ok(config)
    }

    /// Validates and atomically saves a configuration at the expected revision.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the current configuration cannot be loaded,
    /// the revision conflicts, validation fails, or atomic persistence fails.
    pub fn save(
        &self,
        mut config: AppConfig,
        expected_revision: u64,
    ) -> Result<AppConfig, ConfigError> {
        let current = self.load()?;
        if current.revision != expected_revision {
            return Err(ConfigError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let issues = config.validate();
        if !issues.is_empty() {
            return Err(ConfigError::Validation(issues));
        }
        config.schema_version = CURRENT_SCHEMA_VERSION;
        config.revision = current.revision.saturating_add(1);
        self.write_atomic(&config)?;
        Ok(config)
    }

    fn backup(&self) -> Result<(), ConfigError> {
        let backup = self.path.with_extension("toml.bak");
        fs::copy(&self.path, backup)?;
        Ok(())
    }

    fn write_atomic(&self, config: &AppConfig) -> Result<(), ConfigError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let content = toml::to_string_pretty(config)?;
        let mut temporary = NamedTempFile::new_in(parent)?;
        temporary.write_all(content.as_bytes())?;
        temporary.as_file().sync_all()?;
        #[cfg(unix)]
        set_private_permissions(temporary.path())?;
        #[cfg(not(unix))]
        set_private_permissions(temporary.path());
        temporary.persist(&self.path)?;
        #[cfg(unix)]
        sync_directory(parent)?;
        #[cfg(not(unix))]
        sync_directory(parent);
        Ok(())
    }
}

fn migrate(value: &mut toml::Value, from: u32) -> Result<(), ConfigError> {
    if from >= CURRENT_SCHEMA_VERSION {
        return Err(ConfigError::UnsupportedSchema(from));
    }
    let table = value
        .as_table_mut()
        .ok_or_else(|| ConfigError::UnsupportedSchema(from))?;
    if from == 0 {
        table.entry("revision").or_insert(toml::Value::Integer(0));
    }
    if from < 2 {
        if let Some(mihomo) = table.get_mut("mihomo").and_then(toml::Value::as_table_mut) {
            if mihomo
                .get("direct_dns_preset")
                .and_then(toml::Value::as_str)
                == Some("shecan")
            {
                mihomo.insert(
                    "direct_dns_preset".into(),
                    toml::Value::String("fake_ip".into()),
                );
            }
        }
    }
    if from < 3 {
        migrate_clients_v3(table);
    }
    table.insert(
        "schema_version".into(),
        toml::Value::Integer(i64::from(CURRENT_SCHEMA_VERSION)),
    );
    Ok(())
}

fn migrate_clients_v3(table: &mut toml::map::Map<String, toml::Value>) {
    if table
        .get("clients")
        .and_then(toml::Value::as_array)
        .is_some()
    {
        table.remove("hiddify");
        table.remove("openvpn");
        if !table.contains_key("default_route") {
            table.insert("default_route".into(), default_route_direct());
        }
        return;
    }

    let mut clients = Vec::new();
    let mut default_route = default_route_direct();

    if let Some(hiddify) = table.remove("hiddify") {
        let id = ClientId::new();
        clients.push(toml::Value::Table(local_proxy_client_table(
            id,
            "hiddify",
            hiddify.as_table(),
        )));
        default_route = default_route_client(id);
    }

    if let Some(openvpn) = table.remove("openvpn") {
        let id = ClientId::new();
        clients.push(toml::Value::Table(side_tunnel_client_table(
            id,
            "openvpn",
            openvpn.as_table(),
        )));
        if clients.len() == 1 {
            default_route = default_route_client(id);
        }
    }

    if clients.is_empty() {
        let id = ClientId::new();
        clients.push(toml::Value::Table(local_proxy_client_table(
            id, "hiddify", None,
        )));
        default_route = default_route_client(id);
    }

    table.insert("clients".into(), toml::Value::Array(clients));
    table.insert("default_route".into(), default_route);
}

fn default_route_direct() -> toml::Value {
    let mut table = toml::map::Map::new();
    table.insert("kind".into(), toml::Value::String("direct".into()));
    toml::Value::Table(table)
}

fn default_route_client(id: ClientId) -> toml::Value {
    let mut table = toml::map::Map::new();
    table.insert("kind".into(), toml::Value::String("client".into()));
    table.insert("client_id".into(), toml::Value::String(id.as_hyphenated()));
    toml::Value::Table(table)
}

fn local_proxy_client_table(
    id: ClientId,
    preset: &str,
    source: Option<&toml::map::Map<String, toml::Value>>,
) -> toml::map::Map<String, toml::Value> {
    let spec = PresetId::Hiddify.spec();
    let mut config = toml::map::Map::new();
    config.insert("kind".into(), toml::Value::String("local_proxy".into()));
    config.insert(
        "host".into(),
        source
            .and_then(|table| table.get("host").cloned())
            .unwrap_or_else(|| toml::Value::String(spec.default_host.into())),
    );
    config.insert(
        "port".into(),
        source
            .and_then(|table| table.get("port").cloned())
            .unwrap_or_else(|| toml::Value::Integer(i64::from(spec.default_port.unwrap_or(1080)))),
    );
    config.insert(
        "executable".into(),
        source
            .and_then(|table| table.get("executable").cloned())
            .unwrap_or_else(|| toml::Value::String("auto".into())),
    );
    config.insert(
        "start_timeout_seconds".into(),
        source
            .and_then(|table| table.get("start_timeout_seconds").cloned())
            .unwrap_or(toml::Value::Integer(45)),
    );
    config.insert(
        "stop_with_stack".into(),
        source
            .and_then(|table| table.get("stop_with_stack").cloned())
            .unwrap_or(toml::Value::Boolean(true)),
    );
    client_table(id, preset, config)
}

fn side_tunnel_client_table(
    id: ClientId,
    preset: &str,
    source: Option<&toml::map::Map<String, toml::Value>>,
) -> toml::map::Map<String, toml::Value> {
    let mut config = toml::map::Map::new();
    config.insert(
        "kind".into(),
        toml::Value::String("owned_side_tunnel".into()),
    );
    if let Some(path) = source.and_then(|table| table.get("profile_path").cloned()) {
        config.insert("profile_path".into(), path);
    }
    config.insert(
        "executable".into(),
        source
            .and_then(|table| table.get("executable").cloned())
            .unwrap_or_else(|| toml::Value::String("auto".into())),
    );
    config.insert(
        "start_timeout_seconds".into(),
        source
            .and_then(|table| table.get("start_timeout_seconds").cloned())
            .unwrap_or(toml::Value::Integer(45)),
    );
    client_table(id, preset, config)
}

fn client_table(
    id: ClientId,
    preset: &str,
    config: toml::map::Map<String, toml::Value>,
) -> toml::map::Map<String, toml::Value> {
    let mut table = toml::map::Map::new();
    table.insert("id".into(), toml::Value::String(id.as_hyphenated()));
    table.insert("preset".into(), toml::Value::String(preset.into()));
    table.insert("enabled".into(), toml::Value::Boolean(true));
    table.insert("config".into(), toml::Value::Table(config));
    table
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) {}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), std::io::Error> {
    let directory = fs::OpenOptions::new().read(true).open(path)?;
    directory.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_secret_is_random() {
        let first = AppConfig::default();
        let second = AppConfig::default();
        assert!(first.validate().is_empty());
        assert_ne!(
            first.mihomo.controller_secret,
            second.mihomo.controller_secret
        );
        assert_eq!(first.mihomo.controller_secret.len(), 64);
        assert_eq!(first.mihomo.direct_dns_preset, DirectDnsPreset::FakeIp);
        assert_eq!(first.mihomo.direct_dns_preset.to_string(), "fake_ip");
        assert!(first.mihomo.direct_dns_resolvers().is_empty());
    }

    #[test]
    fn mokhaberat_and_radar_presets_are_valid_addresses() {
        let mut config = AppConfig::default();
        config.mihomo.direct_dns_preset = DirectDnsPreset::Mokhaberat;
        assert!(config.validate().is_empty());
        assert_eq!(config.mihomo.direct_dns_resolvers(), ["5.200.200.200"]);
        config.mihomo.direct_dns_preset = DirectDnsPreset::Radar;
        assert!(config.validate().is_empty());
        assert_eq!(
            config.mihomo.direct_dns_resolvers(),
            ["10.202.10.10", "10.202.10.11"]
        );
    }

    #[test]
    fn custom_direct_dns_rejects_empty_and_loopback() {
        let mut config = AppConfig::default();
        config.mihomo.direct_dns_preset = DirectDnsPreset::Custom;
        assert!(config
            .validate()
            .iter()
            .any(|item| item.code == "DIRECT_DNS_REQUIRED"));
        config.mihomo.direct_dns_servers = vec!["127.0.0.1".into()];
        assert!(config
            .validate()
            .iter()
            .any(|item| item.code == "DIRECT_DNS_INVALID"));
        config.mihomo.direct_dns_servers = vec!["5.200.200.200".into(), "1.1.1.1".into()];
        assert!(config.validate().is_empty());
    }

    #[test]
    fn schema_v1_implicit_shecan_migrates_to_fake_ip() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("config.toml");
        let mut config = AppConfig {
            schema_version: 1,
            ..AppConfig::default()
        };
        config.mihomo.direct_dns_preset = DirectDnsPreset::Shecan;
        fs::write(&path, toml::to_string(&config).expect("toml")).expect("write");
        let loaded = ConfigStore::new(&path).load().expect("load");
        assert_eq!(loaded.schema_version, 3);
        assert_eq!(loaded.mihomo.direct_dns_preset, DirectDnsPreset::FakeIp);
        assert_eq!(loaded.clients.len(), 1);
        assert_eq!(loaded.clients[0].preset, PresetId::Hiddify);
        assert!(matches!(loaded.default_route, DefaultRoute::Client { .. }));
    }

    #[test]
    fn schema_v1_mokhaberat_survives_direct_dns_migration() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("config.toml");
        let mut config = AppConfig {
            schema_version: 1,
            ..AppConfig::default()
        };
        config.mihomo.direct_dns_preset = DirectDnsPreset::Mokhaberat;
        fs::write(&path, toml::to_string(&config).expect("toml")).expect("write");
        let loaded = ConfigStore::new(&path).load().expect("load");
        assert_eq!(loaded.schema_version, 3);
        assert_eq!(loaded.mihomo.direct_dns_preset, DirectDnsPreset::Mokhaberat);
    }

    #[test]
    fn rejects_remote_controller_and_conflicting_ports() {
        let mut config = AppConfig::default();
        config.mihomo.controller_host = "0.0.0.0".into();
        config.mihomo.mixed_port = config.mihomo.controller_port;
        let issues = config.validate();
        assert!(issues.iter().any(|item| item.code == "LOOPBACK_REQUIRED"));
        assert!(issues.iter().any(|item| item.code == "PORT_CONFLICT"));
    }

    #[test]
    fn persists_atomically_and_checks_revision() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("config.toml");
        let store = ConfigStore::new(&path);
        let mut config = store.load_or_create().expect("create config");
        config.behavior.connect_at_launch = true;
        let saved = store.save(config.clone(), 0).expect("save config");
        assert_eq!(saved.revision, 1);
        assert!(store.save(config, 0).is_err());
        assert!(store.load().expect("reload").behavior.connect_at_launch);
    }

    #[test]
    fn redaction_removes_secret_and_parent_path() {
        let mut config = AppConfig::default();
        if let Some(client) = config.clients.first_mut() {
            client.config = ClientConfig::LocalProxy {
                host: "127.0.0.1".into(),
                port: 12_334,
                executable: ExecutableSetting::Path(PathBuf::from("/home/alice/Hiddify.AppImage")),
                start_timeout_seconds: 45,
                stop_with_stack: true,
            };
        }
        let redacted = config.redacted();
        assert_eq!(redacted.mihomo.controller_secret, "[REDACTED]");
        assert_eq!(
            redacted.clients[0].config,
            ClientConfig::LocalProxy {
                host: "127.0.0.1".into(),
                port: 12_334,
                executable: ExecutableSetting::Path(PathBuf::from("Hiddify.AppImage")),
                start_timeout_seconds: 45,
                stop_with_stack: true,
            }
        );
    }

    #[test]
    fn schema_v2_hiddify_migrates_to_a_client_match_default() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("config.toml");
        let contents = r#"
schema_version = 2
revision = 4

[hiddify]
host = "127.0.0.1"
port = 12334
executable = "auto"
start_timeout_seconds = 45
stop_with_stack = true

[mihomo]
controller_host = "127.0.0.1"
controller_port = 19090
controller_secret = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
mixed_port = 17890
dns_port = 1053
tun_name = "clash-iran"
log_level = "info"
direct_dns_preset = "fake_ip"
direct_dns_servers = []

[rules]
refresh_interval_minutes = 15
upstream_refresh_hours = 24

[behavior]
launch_at_login = false
connect_at_launch = false
close_to_tray = true
"#;
        fs::write(&path, contents).expect("write");
        let loaded = ConfigStore::new(&path).load().expect("load");
        assert_eq!(loaded.schema_version, 3);
        assert_eq!(loaded.clients.len(), 1);
        assert_eq!(loaded.clients[0].preset, PresetId::Hiddify);
        assert!(loaded.clients[0].enabled);
        let DefaultRoute::Client { client_id } = loaded.default_route else {
            panic!("expected MATCH to the migrated Hiddify instance");
        };
        assert_eq!(client_id, loaded.clients[0].id);
    }

    #[test]
    fn schema_v2_openvpn_blob_becomes_a_side_tunnel_instance() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("config.toml");
        let contents = r#"
schema_version = 2
revision = 1

[hiddify]
host = "127.0.0.1"
port = 12334
executable = "auto"
start_timeout_seconds = 45
stop_with_stack = true

[openvpn]
profile_path = "/tmp/office.ovpn"

[mihomo]
controller_host = "127.0.0.1"
controller_port = 19090
controller_secret = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
mixed_port = 17890
dns_port = 1053
tun_name = "clash-iran"
log_level = "info"
direct_dns_preset = "fake_ip"

[rules]
refresh_interval_minutes = 15
upstream_refresh_hours = 24

[behavior]
close_to_tray = true
"#;
        fs::write(&path, contents).expect("write");
        let loaded = ConfigStore::new(&path).load().expect("load");
        assert_eq!(loaded.clients.len(), 2);
        assert!(loaded
            .clients
            .iter()
            .any(|client| client.preset == PresetId::Openvpn));
        assert!(matches!(
            loaded
                .clients
                .iter()
                .find(|client| client.preset == PresetId::Openvpn)
                .map(|client| &client.config),
            Some(ClientConfig::OwnedSideTunnel { .. })
        ));
        assert!(matches!(loaded.default_route, DefaultRoute::Client { .. }));
    }

    #[test]
    fn disabling_the_match_client_falls_back_to_direct() {
        let mut config = AppConfig::default();
        let id = config.clients[0].id;
        assert_eq!(config.default_route, DefaultRoute::client(id));
        config.clients[0].enabled = false;
        assert!(config.sanitize_default_route());
        assert_eq!(config.default_route, DefaultRoute::Direct);
    }

    #[cfg(unix)]
    #[test]
    fn private_permissions_restrict_to_owner_read_write() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("secret.toml");
        fs::write(&path, "x").expect("write");
        set_private_permissions(&path).expect("chmod");
        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(not(unix))]
    #[test]
    fn private_permissions_are_a_noop_off_unix() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("secret.toml");
        fs::write(&path, "x").expect("write");
        set_private_permissions(&path);
        assert!(path.is_file());
    }
}
