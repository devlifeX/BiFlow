export type LifecycleBusy =
  | "connecting"
  | "disconnecting"
  | "pausing"
  | "resuming"
  | "reconciling"
  | "applying_rules";

export type OperationStage =
  | "preparing"
  | "starting_client"
  | "preparing_runtime"
  | "validating_config"
  | "starting_core"
  | "checking_readiness"
  | "stopping_core"
  | "stopping_proxy"
  | "cleaning_up"
  | "recovering";

export type StackPhase =
  | "uninitialized"
  | "stopped"
  | "starting_client"
  | "preparing_runtime"
  | "validating_config"
  | "starting_core"
  | "checking_readiness"
  | "running"
  | "paused"
  | "degraded"
  | "stopping"
  | "recovering"
  | "error";

export type ComponentPhase =
  | "unknown"
  | "checking"
  | "stopped"
  | "starting"
  | "running"
  | "degraded"
  | "unavailable"
  | "error";

export interface ComponentStatus {
  phase: ComponentPhase;
  message: string | null;
  since: string;
}

export interface ProviderSummary {
  ready: number;
  total: number;
  rules_loaded: number;
  last_refresh: string | null;
}

export interface AppError {
  code: string;
  message_key: string;
  retryable: boolean;
  remediation:
    | "retry"
    | "open_settings"
    | "install_helper"
    | "choose_hiddify_executable"
    | "install_dependency"
    | "run_diagnostics"
    | null;
  technical_details: string | null;
  correlation_id: string;
}

export interface OperationClient {
  preset: string;
  client_id: string;
}

export interface ClientComponentStatus {
  id: string;
  preset: string;
  enabled: boolean;
  status: ComponentStatus;
  exit_ip: string | null;
}

export interface StackSnapshot {
  revision: number;
  phase: StackPhase;
  busy?: LifecycleBusy | null;
  operation_stage?: OperationStage | null;
  operation_client?: OperationClient | null;
  operation_id: string | null;
  helper: ComponentStatus;
  clients: ClientComponentStatus[];
  mihomo: ComponentStatus;
  tun: ComponentStatus;
  dns: ComponentStatus;
  providers: ProviderSummary;
  exit_ip: string | null;
  backend: "external_hiddify";
  last_error: AppError | null;
  updated_at: string;
}

export interface OperationAccepted {
  operation_id: string;
  already_complete: boolean;
}

export type ExecutableSetting = "auto" | { path: string };
export type LogLevel = "error" | "warn" | "info" | "debug";
export type DirectDnsPreset =
  | "fake_ip"
  | "shecan"
  | "electro"
  | "radar"
  | "mokhaberat"
  | "custom";

export type DefaultRoute =
  | { kind: "direct" }
  | { kind: "client"; client_id: string };

export type ClientConfig =
  | {
      kind: "local_proxy";
      host: string;
      port: number;
      executable: ExecutableSetting;
      start_timeout_seconds: number;
      stop_with_stack: boolean;
    }
  | {
      kind: "owned_side_tunnel";
      profile_path: string | null;
      executable: ExecutableSetting;
      username: string | null;
      password: string | null;
      start_timeout_seconds: number;
    }
  | { kind: "unsupported" };

export interface ClientInstance {
  id: string;
  preset: string;
  enabled: boolean;
  allow_direct_when_down: boolean;
  config: ClientConfig;
}

export interface AppConfig {
  schema_version: number;
  revision: number;
  clients: ClientInstance[];
  default_route: DefaultRoute;
  mihomo: {
    controller_host: string;
    controller_port: number;
    controller_secret: string;
    mixed_port: number;
    dns_port: number;
    tun_name: string;
    log_level: LogLevel;
    direct_dns_preset: DirectDnsPreset;
    direct_dns_servers: string[];
  };
  rules: {
    refresh_interval_minutes: number;
    upstream_refresh_hours: number;
  };
  behavior: {
    launch_at_login: boolean;
    connect_at_launch: boolean;
    close_to_tray: boolean;
    fail_closed: boolean;
  };
}

export interface ValidationIssue {
  field: string;
  code: string;
  message: string;
}

export interface DirectTarget {
  kind: "domain" | "ip";
  value: string;
}

export interface DirectRule {
  target: DirectTarget;
  resolved_ips: string[];
  created_at: string;
  refreshed_at: string | null;
}

export type Outbound =
  | { kind: "direct" }
  | { kind: "client"; client_id: string };

export interface PinnedRoute {
  target: DirectTarget;
  outbound: Outbound;
  list_id: string | null;
  resolved_ips: string[];
  created_at: string;
  refreshed_at: string | null;
}

export interface RuleListMeta {
  id: string;
  name: string;
  outbound: Outbound;
}

export interface ListCheckEntry {
  target: string;
  status: "ok" | "slow" | "fail" | "skipped";
  latency_ms: number | null;
  detail: string | null;
}

export interface DirectRulesDocument {
  revision: number;
  pins: PinnedRoute[];
  lists: RuleListMeta[];
}

export interface RouteTestResult {
  target: string;
  outbound: Outbound;
  reason: string;
  matched_rule: string | null;
  reachable: boolean | null;
  tested_at: string;
}

export type ReachabilityStatus = "ok" | "slow" | "unreachable";

export interface ReachabilityResult {
  id: string;
  domain: string;
  path: "vpn" | "direct";
  /** True when the probe actually went through the Hiddify proxy. */
  via_proxy: boolean;
  status: ReachabilityStatus;
  latency_ms: number | null;
  detail: string | null;
}

export interface DiagnosticStep {
  id: string;
  label: string;
  status: "pending" | "running" | "passed" | "failed" | "warning";
  detail: string | null;
  started_at: string | null;
  finished_at: string | null;
}

export interface DiagnosticsReport {
  operation_id: string;
  steps: DiagnosticStep[];
  finished: boolean;
}

export interface LogEntry {
  timestamp: string;
  level: string;
  event: string;
  fields: Record<string, string>;
}

export interface BootstrapResult {
  app_version: string;
  platform: string;
  mock_mode: boolean;
  snapshot: StackSnapshot;
  settings: AppConfig;
  direct_rules: DirectRulesDocument;
  cloud_rules: CloudRulesStatus;
  dependencies: DependencyStatus[];
  network_status: NetworkStatus;
}

export type InternetState = "checking" | "online" | "offline";

export interface TrafficTotals {
  sent: number;
  received: number;
}

export interface ActiveConnection {
  host: string;
  destination_ip: string;
  outbound: string;
  rule: string;
}

export interface NetworkStatus {
  state: InternetState;
  public_ip: string | null;
  country_code: string | null;
  city: string | null;
  checked_at: string;
  detail: string | null;
}

export interface ExportResult {
  path: string;
  files: string[];
}

export interface DebugLogStatus {
  path: string;
  size_bytes: number;
}

export interface FreshStartReport {
  data_dir: string;
  backup_dir: string;
  cleared: string[];
  preserved: string[];
  stopped: boolean;
  started: boolean;
}

export interface UpdateStatus {
  available: boolean;
  version: string | null;
  notes: string | null;
  app_available: boolean;
  rules_available: boolean;
  thirdparty_available: boolean;
}

export type UpdatePhase =
  | "idle"
  | "checking"
  | "current"
  | "available"
  | "downloading"
  | "installing"
  | "restarting"
  | "installed"
  | "failed";

export interface UpdateProgress {
  phase: UpdatePhase;
  percent: number | null;
  version: string | null;
  error: string | null;
  operation_id?: string | null;
  app_available?: boolean;
  rules_available?: boolean;
  thirdparty_available?: boolean;
}

export interface CloudRuleSetStatus {
  id: string;
  kind: "domain" | "ip_cidr";
  entry_count: number;
  source: string;
  sha256: string | null;
}

export interface CloudRulesStatus {
  domain_count: number;
  ip_count: number;
  last_synced_at: string | null;
  source: string;
  snapshot_revision: string | null;
  sets: CloudRuleSetStatus[];
}

export interface DependencyStatus {
  id: "hiddify" | "mihomo";
  name: string;
  installed: boolean;
  version: string | null;
  path: string | null;
}

export interface InstallGuide {
  id: string;
  title: string;
  download_url: string;
  steps: string[];
}

export interface InstallResult {
  id: string;
  installed: boolean;
  path: string | null;
  guide: InstallGuide;
}
