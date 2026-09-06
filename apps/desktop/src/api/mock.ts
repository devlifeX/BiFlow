import { APP_VERSION } from "../version";
import { INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT } from "../lib/sideTunnelConnect";
import type {
  AppConfig,
  BootstrapResult,
  CloudRulesStatus,
  DependencyStatus,
  DiagnosticStep,
  DiagnosticsReport,
  DebugLogStatus,
  DirectRulesDocument,
  ListCheckEntry,
  Outbound,
  PinnedRoute,
  ExportResult,
  FreshStartReport,
  InstallGuide,
  InstallResult,
  LogEntry,
  NetworkStatus,
  OperationAccepted,
  ReachabilityResult,
  RouteTestResult,
  LifecycleBusy,
  OperationStage,
  StackPhase,
  StackSnapshot,
  UpdateProgress,
  TrafficTotals,
  ActiveConnection,
  UpdateStatus,
  ValidationIssue,
} from "./models";
import { validateDirectDns } from "../lib/directDns";
import { sanitizeDefaultRoute } from "../lib/clients";
import { MOCK_HIDDIFY_ID, outboundFromKey } from "../lib/outbound";

const now = () => new Date().toISOString();
const component = (
  phase: StackSnapshot["mihomo"]["phase"],
  message: string,
) => ({
  phase,
  message,
  since: now(),
});

function initialSnapshot(): StackSnapshot {
  const helperMissing =
    typeof sessionStorage !== "undefined" &&
    sessionStorage.getItem("biflow-mock-force-missing-helper") === "1";
  return {
    revision: 1,
    phase: "stopped",
    busy: null,
    operation_stage: null,
    operation_id: null,
    helper: helperMissing
      ? {
          phase: "unavailable",
          message: "Helper service is not installed or running",
          since: now(),
        }
      : {
          phase: "running",
          message: "Mock helper is ready",
          since: now(),
        },
    clients: [
      {
        id: MOCK_HIDDIFY_ID,
        preset: "hiddify",
        enabled: true,
        status: component("stopped", "Hiddify proxy is not listening"),
        exit_ip: null,
      },
    ],
    mihomo: component("stopped", "Mihomo controller is not listening"),
    tun: component("stopped", "TUN interface is absent"),
    dns: component("stopped", "DNS listener is inactive"),
    providers: { ready: 0, total: 0, rules_loaded: 0, last_refresh: null },
    exit_ip: null,
    backend: "external_hiddify",
    last_error: null,
    updated_at: now(),
  };
}

function initialSettings(): AppConfig {
  return {
    schema_version: 3,
    revision: 0,
    clients: [
      {
        id: MOCK_HIDDIFY_ID,
        preset: "hiddify",
        enabled: true,
        allow_direct_when_down: false,
        config: {
          kind: "local_proxy",
          host: "127.0.0.1",
          port: 12334,
          executable: "auto",
          start_timeout_seconds: 45,
          stop_with_stack: true,
        },
      },
    ],
    default_route: { kind: "client", client_id: MOCK_HIDDIFY_ID },
    mihomo: {
      controller_host: "127.0.0.1",
      controller_port: 19090,
      controller_secret: "[managed by desktop core]",
      mixed_port: 17890,
      dns_port: 1053,
      tun_name: "clash-iran",
      log_level: "info",
      direct_dns_preset: "fake_ip",
      direct_dns_servers: [],
    },
    rules: { refresh_interval_minutes: 15, upstream_refresh_hours: 24 },
    behavior: {
      launch_at_login: false,
      connect_at_launch: false,
      close_to_tray: true,
      fail_closed: true,
    },
  };
}

const MOCK_DIRECT_LIST_ID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

function initialDirectRules(): DirectRulesDocument {
  return {
    revision: 1,
    pins: [
      {
        target: { kind: "domain", value: "example.ir" },
        outbound: { kind: "direct" },
        list_id: MOCK_DIRECT_LIST_ID,
        resolved_ips: ["203.0.113.8"],
        created_at: now(),
        refreshed_at: now(),
      },
    ],
    lists: [
      {
        id: MOCK_DIRECT_LIST_ID,
        name: "Direct",
        outbound: { kind: "direct" },
      },
    ],
  };
}

function mockUuid(): string {
  return "xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx".replaceAll(/x/g, () =>
    Math.floor(Math.random() * 16).toString(16),
  );
}

function ensureListFor(outbound: Outbound): string {
  const existing = directRules.lists.find(
    (list) => JSON.stringify(list.outbound) === JSON.stringify(outbound),
  );
  if (existing) return existing.id;
  const id = mockUuid();
  directRules = {
    ...directRules,
    lists: [
      ...directRules.lists,
      {
        id,
        name: outbound.kind === "direct" ? "Direct" : "Client pins",
        outbound,
      },
    ],
  };
  return id;
}

function initialCloudRules(): CloudRulesStatus {
  return {
    domain_count: 62_828,
    ip_count: 2_906,
    last_synced_at: null,
    source: "bundled",
    snapshot_revision: null,
    sets: [
      {
        id: "iran-domains",
        kind: "domain",
        entry_count: 62_828,
        source: "bundled",
        sha256: null,
      },
      {
        id: "iran-networks",
        kind: "ip_cidr",
        entry_count: 2_888,
        source: "bundled",
        sha256: null,
      },
      {
        id: "private",
        kind: "ip_cidr",
        entry_count: 18,
        source: "bundled",
        sha256: null,
      },
    ],
  };
}

function initialDependencies(): DependencyStatus[] {
  if (
    typeof sessionStorage !== "undefined" &&
    sessionStorage.getItem("biflow-mock-force-missing-deps") === "1"
  ) {
    return missingDependencies();
  }
  return mergeInstalled(detectedDependencies(), loadSavedDependencies());
}

function persistMockDependencies() {
  try {
    localStorage.setItem(
      "biflow-mock-installed-deps",
      JSON.stringify(dependencies),
    );
  } catch {
    // Ignore quota / private-mode failures.
  }
}

function loadSavedDependencies(): DependencyStatus[] | null {
  try {
    const raw = localStorage.getItem("biflow-mock-installed-deps");
    if (!raw) return null;
    const parsed = JSON.parse(raw) as DependencyStatus[];
    return Array.isArray(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function mergeInstalled(
  base: DependencyStatus[],
  saved: DependencyStatus[] | null,
): DependencyStatus[] {
  if (!saved) return base;
  return base.map((item) => {
    const extra = saved.find((entry) => entry.id === item.id);
    return extra?.installed
      ? { ...item, installed: true, path: extra.path ?? item.path }
      : item;
  });
}

function missingDependencies(): DependencyStatus[] {
  return [
    {
      id: "hiddify",
      name: "Hiddify",
      installed: false,
      version: null,
      path: null,
    },
    {
      id: "mihomo",
      name: "Mihomo",
      installed: false,
      version: null,
      path: null,
    },
  ];
}

function detectedDependencies(): DependencyStatus[] {
  const hiddify = Boolean(__MOCK_HIDDIFY_INSTALLED__);
  const mihomo = Boolean(__MOCK_MIHOMO_INSTALLED__);
  return [
    {
      id: "hiddify",
      name: "Hiddify",
      installed: hiddify,
      version: null,
      path: hiddify ? "detected" : null,
    },
    {
      id: "mihomo",
      name: "Mihomo",
      installed: mihomo,
      version: null,
      path: mihomo ? "detected" : null,
    },
  ];
}

let snapshot = initialSnapshot();
let trafficTotals: TrafficTotals = { sent: 1_048_576, received: 2_097_152 };
let lastSessionSent = 0;
let lastSessionReceived = 0;
let settings = initialSettings();
let directRules = initialDirectRules();
let cloudRules = initialCloudRules();
let dependencies = initialDependencies();

function isPrivateHost(value: string): boolean {
  return (
    /^127\./.test(value) ||
    /^10\./.test(value) ||
    /^192\.168\./.test(value) ||
    /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(value) ||
    value === "::1"
  );
}

const PRIVATE_SUFFIXES = ["github.io"];
const IRAN_BUSINESS_DOMAINS = [
  "technolife.com",
  "azkivam.com",
  "azkisarmayeh.com",
  "nextpay.com",
  "payping.io",
  "tomanpay.com",
  "kifpool.me",
  "safarmarket.com",
  "arazcloud.com",
  "excoino.com",
  "hitobit.com",
  "karboom.io",
  "kavenegar.com",
  "ewano.app",
];

function canonicalTarget(input: string): {
  kind: "ip" | "domain";
  value: string;
} {
  const value = input.trim().toLowerCase().replace(/\.$/u, "");
  if (/^\d{1,3}(\.\d{1,3}){3}$/u.test(value) || value.includes(":")) {
    return { kind: "ip", value };
  }
  const labels = value.split(".").filter(Boolean);
  if (labels.length < 2) {
    throw new Error("domain must have a registrable root");
  }
  const lastTwo = labels.slice(-2).join(".");
  if (PRIVATE_SUFFIXES.includes(lastTwo) && labels.length < 3) {
    throw new Error("public suffixes cannot be pinned");
  }
  // Pins stay exactly as typed: a root covers every subdomain, and a more
  // specific subdomain pin can live in another list and win.
  return { kind: "domain", value: labels.join(".") };
}

function pinSpecificity(pin: string): number {
  return pin.split(".").length;
}

function bestPinMatch(
  target: string,
  candidates: PinnedRoute[],
): PinnedRoute | undefined {
  let best: PinnedRoute | undefined;
  for (const item of candidates) {
    if (!pinMatchesHost(item, target)) continue;
    if (
      !best ||
      pinSpecificity(item.target.value) > pinSpecificity(best.target.value)
    ) {
      best = item;
    }
  }
  return best;
}

function domainMatchesPin(host: string, pin: string): boolean {
  return host === pin || host.endsWith(`.${pin}`);
}

function pinMatchesHost(item: PinnedRoute, host: string): boolean {
  if (item.target.kind === "ip") {
    return item.target.value === host;
  }
  return domainMatchesPin(host, item.target.value);
}

function enabledClientIds(): Set<string> {
  return new Set(
    settings.clients
      .filter((client) => client.enabled)
      .map((client) => client.id),
  );
}

function matchOutbound(): Outbound {
  const route = settings.default_route;
  if (route.kind === "direct") return { kind: "direct" };
  if (enabledClientIds().has(route.client_id)) return route;
  return { kind: "direct" };
}

function route(
  target: string,
  outbound: Outbound,
  reason: string,
  matched: string,
): RouteTestResult {
  return {
    target,
    outbound,
    reason,
    matched_rule: matched,
    reachable: true,
    tested_at: now(),
  };
}

function mockNetworkStatus(): NetworkStatus {
  return {
    state: "online",
    public_ip: "198.51.100.24",
    country_code: "IR",
    city: "Tehran",
    checked_at: now(),
    detail: "Internet is reachable",
  };
}

function guideFor(id: string): InstallGuide {
  const linux =
    typeof navigator === "undefined" || !/win/i.test(navigator.platform);
  if (id === "mihomo") {
    return {
      id,
      title: linux ? "Install Mihomo on Linux" : "Install Mihomo on Windows",
      download_url: "https://github.com/MetaCubeX/mihomo/releases/latest",
      steps: linux
        ? [
            "Download mihomo-linux-amd64 gzip from the MetaCubeX GitHub release.",
            "Decompress it into ~/.local/share/biflow/bin/mihomo",
            "chmod +x ~/.local/share/biflow/bin/mihomo",
            "Restart BiFlow and press Connect.",
          ]
        : [
            "Download the Windows zip from the MetaCubeX GitHub release.",
            "Extract mihomo.exe into %LOCALAPPDATA%\\biflow\\bin\\mihomo.exe",
            "Restart BiFlow and press Connect.",
          ],
    };
  }
  return {
    id,
    title: linux ? "Install Hiddify on Linux" : "Install Hiddify on Windows",
    download_url: "https://github.com/hiddify/hiddify-app/releases/latest",
    steps: linux
      ? [
          "Download Hiddify-Linux-x64-AppImage.AppImage from the official GitHub release.",
          "chmod +x Hiddify-Linux-x64-AppImage.AppImage",
          "Move it to ~/.local/share/biflow/apps/Hiddify.AppImage",
          "Restart BiFlow and press Connect.",
        ]
      : [
          "Download Hiddify-Windows-Setup-x64.exe from the official GitHub release.",
          "Run the installer and accept the permission prompt.",
          "Restart BiFlow and press Connect.",
        ],
  };
}

const logs: LogEntry[] = [
  {
    timestamp: now(),
    level: "info",
    event: "mock_transport_ready",
    fields: { mode: "development" },
  },
];
let debugLogSize = 48_512;

function mockDebugLogStatus(): DebugLogStatus {
  return {
    path: "/home/user/.local/share/biflow/debug.log",
    size_bytes: debugLogSize,
  };
}

const listeners = new Set<(next: StackSnapshot) => void>();
const updateListeners = new Set<(progress: UpdateProgress) => void>();
let lastUpdateProgress: UpdateProgress = {
  phase: "idle",
  percent: null,
  version: null,
  error: null,
  operation_id: null,
};

function mockUpdateAvailable(): boolean {
  return (
    typeof sessionStorage !== "undefined" &&
    sessionStorage.getItem("biflow-mock-update-available") === "1"
  );
}

function mockUpdateShouldFail(): boolean {
  return (
    typeof sessionStorage !== "undefined" &&
    sessionStorage.getItem("biflow-mock-update-fail") === "1"
  );
}

function emitUpdateProgress(progress: UpdateProgress) {
  lastUpdateProgress = structuredClone(progress);
  for (const listener of updateListeners) {
    listener(structuredClone(progress));
  }
}

async function simulateInstallProgress(version: string) {
  for (const percent of [0, 35, 70, 100]) {
    emitUpdateProgress({
      phase: "downloading",
      percent,
      version,
      error: null,
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  emitUpdateProgress({
    phase: "installing",
    percent: 100,
    version,
    error: null,
  });
  await new Promise((resolve) => setTimeout(resolve, 20));
  emitUpdateProgress({
    phase: "restarting",
    percent: 100,
    version,
    error: null,
  });
}

let lifecycleBusy: LifecycleBusy | null = null;
let mockSideTunnelConnectTimeout: number = INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT;

function sideTunnelStatusForTimeout(
  timeoutSeconds: number,
  preset: string,
): ReturnType<typeof component> {
  if (timeoutSeconds >= 30) {
    return component("running", `${preset} side tunnel is ready`);
  }
  return component(
    "stopped",
    "OpenVPN did not come up before the connect timeout",
  );
}

function clientsWithMockSideTunnelOutcomes(timeoutSeconds: number) {
  return settings.clients.map((client) => {
    const existing = snapshot.clients.find((item) => item.id === client.id);
    const base = {
      id: client.id,
      preset: client.preset,
      enabled: client.enabled,
      exit_ip: existing?.exit_ip ?? null,
    };
    if (client.config.kind === "owned_side_tunnel" && client.enabled) {
      return {
        ...base,
        status: sideTunnelStatusForTimeout(timeoutSeconds, client.preset),
      };
    }
    return {
      ...base,
      status: component("running", `${client.preset} is ready`),
    };
  });
}

function emit(
  phase: StackPhase,
  operationId: string | null,
  busy: LifecycleBusy | null = lifecycleBusy,
  operationStage: OperationStage | null = null,
) {
  snapshot = {
    ...snapshot,
    revision: snapshot.revision + 1,
    phase,
    busy,
    operation_stage: operationStage,
    operation_client:
      operationStage === "starting_client"
        ? { preset: "hiddify", client_id: MOCK_HIDDIFY_ID }
        : null,
    operation_id: operationId,
    updated_at: now(),
  };
  for (const listener of listeners) listener(structuredClone(snapshot));
}

function assertIdle(): void {
  if (lifecycleBusy) {
    throw new Error("operation is already in progress");
  }
}

function begin(busy: LifecycleBusy): void {
  assertIdle();
  lifecycleBusy = busy;
}

function operation(): OperationAccepted {
  return { operation_id: crypto.randomUUID(), already_complete: false };
}

async function runStart(accepted: OperationAccepted) {
  const phases: Array<[StackPhase, OperationStage]> = [
    ["starting_client", "starting_client"],
    ["preparing_runtime", "preparing_runtime"],
    ["validating_config", "validating_config"],
    ["starting_core", "starting_core"],
    ["checking_readiness", "checking_readiness"],
  ];

  snapshot = {
    ...snapshot,
    helper: component("checking", "Checking helper service"),
  };
  emit(snapshot.phase, accepted.operation_id, lifecycleBusy, "preparing");
  await new Promise((resolve) => setTimeout(resolve, 80));
  snapshot = {
    ...snapshot,
    helper: component("running", "Mock helper is ready"),
  };
  for (const listener of listeners) listener(structuredClone(snapshot));

  snapshot = {
    ...snapshot,
    clients: snapshot.clients.map((client) =>
      client.enabled
        ? {
            ...client,
            status: component("starting", `${client.preset} is starting`),
          }
        : client,
    ),
  };
  emit(
    "starting_client",
    accepted.operation_id,
    lifecycleBusy,
    "starting_client",
  );
  await new Promise((resolve) => setTimeout(resolve, 100));
  snapshot = {
    ...snapshot,
    clients: clientsWithMockSideTunnelOutcomes(mockSideTunnelConnectTimeout),
  };
  for (const listener of listeners) listener(structuredClone(snapshot));

  for (const [phase, stage] of phases.slice(1)) {
    if (phase === "starting_core") {
      snapshot = {
        ...snapshot,
        mihomo: component("starting", "Mihomo controller is starting"),
        tun: component("starting", "TUN interface is starting"),
        dns: component("starting", "DNS listener is starting"),
      };
    }
    emit(phase, accepted.operation_id, lifecycleBusy, stage);
    await new Promise((resolve) => setTimeout(resolve, 100));
    if (phase === "starting_core") {
      snapshot = {
        ...snapshot,
        mihomo: component("running", "Mihomo controller is ready"),
        dns: component("running", "DNS listener is active"),
      };
      for (const listener of listeners) listener(structuredClone(snapshot));
    }
    if (phase === "checking_readiness") {
      snapshot = {
        ...snapshot,
        tun: component("running", "TUN interface is active"),
        providers: {
          ready: 6,
          total: 6,
          rules_loaded: 184203,
          last_refresh: now(),
        },
      };
      for (const listener of listeners) listener(structuredClone(snapshot));
    }
  }

  snapshot = {
    ...snapshot,
    clients: clientsWithMockSideTunnelOutcomes(mockSideTunnelConnectTimeout),
    exit_ip: "203.0.113.42",
  };
  lifecycleBusy = null;
  emit("running", null, null, null);
  logs.push({
    timestamp: now(),
    level: "info",
    event: "stack_running",
    fields: {},
  });
}

export const mockApi = {
  async bootstrap(): Promise<BootstrapResult> {
    return {
      app_version: APP_VERSION,
      platform: navigator.platform,
      mock_mode: true,
      snapshot: structuredClone(snapshot),
      settings: structuredClone(settings),
      direct_rules: structuredClone(directRules),
      cloud_rules: structuredClone(cloudRules),
      dependencies: structuredClone(dependencies),
      network_status: mockNetworkStatus(),
    };
  },
  async getSnapshot() {
    return structuredClone(snapshot);
  },
  async getNetworkStatus() {
    return mockNetworkStatus();
  },
  async getTrafficTotals(): Promise<TrafficTotals> {
    const connected =
      snapshot.phase === "running" || snapshot.phase === "degraded";
    if (!connected) {
      lastSessionSent = 0;
      lastSessionReceived = 0;
      return { ...trafficTotals };
    }
    const sessionSent = lastSessionSent + 4_096;
    const sessionReceived = lastSessionReceived + 8_192;
    trafficTotals = {
      sent: trafficTotals.sent + (sessionSent - lastSessionSent),
      received:
        trafficTotals.received + (sessionReceived - lastSessionReceived),
    };
    lastSessionSent = sessionSent;
    lastSessionReceived = sessionReceived;
    return { ...trafficTotals };
  },
  async listActiveConnections(): Promise<ActiveConnection[]> {
    if (snapshot.phase !== "running" && snapshot.phase !== "degraded") {
      return [];
    }
    return [
      {
        host: "digikala.ir",
        destination_ip: "5.22.12.1",
        outbound: "direct",
        rule: "iran-domains",
      },
      {
        host: "openai.com",
        destination_ip: "104.18.1.1",
        outbound: MOCK_HIDDIFY_ID,
        rule: "MATCH",
      },
    ];
  },
  async start(sideTunnelTimeoutSeconds?: number) {
    mockSideTunnelConnectTimeout =
      sideTunnelTimeoutSeconds ?? INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT;
    if (lifecycleBusy && lifecycleBusy !== "connecting") {
      throw new Error("operation is already in progress");
    }
    if (snapshot.phase === "running") {
      return { operation_id: crypto.randomUUID(), already_complete: true };
    }
    begin("connecting");
    const accepted = operation();
    emit(snapshot.phase, accepted.operation_id, "connecting", "preparing");
    void runStart(accepted);
    return accepted;
  },
  async retrySideTunnels(sideTunnelTimeoutSeconds: number): Promise<boolean> {
    if (!["running", "degraded"].includes(snapshot.phase)) {
      throw new Error("side tunnel retry requires an active stack");
    }
    mockSideTunnelConnectTimeout = sideTunnelTimeoutSeconds;
    await new Promise((resolve) => setTimeout(resolve, 120));
    snapshot = {
      ...snapshot,
      revision: snapshot.revision + 1,
      clients: clientsWithMockSideTunnelOutcomes(sideTunnelTimeoutSeconds),
      updated_at: now(),
    };
    for (const listener of listeners) listener(structuredClone(snapshot));
    return sideTunnelTimeoutSeconds >= 30;
  },
  async stop(): Promise<OperationAccepted> {
    if (lifecycleBusy && lifecycleBusy !== "disconnecting") {
      throw new Error("operation is already in progress");
    }
    if (snapshot.phase === "stopped") {
      return { operation_id: crypto.randomUUID(), already_complete: true };
    }
    begin("disconnecting");
    const accepted = operation();
    emit("stopping", accepted.operation_id, "disconnecting", "stopping_core");
    window.setTimeout(() => {
      emit(
        "stopping",
        accepted.operation_id,
        "disconnecting",
        "stopping_proxy",
      );
    }, 120);
    window.setTimeout(() => {
      emit("stopping", accepted.operation_id, "disconnecting", "cleaning_up");
    }, 220);
    window.setTimeout(() => {
      snapshot = {
        ...snapshot,
        clients: snapshot.clients.map((client) => ({
          ...client,
          status: component("stopped", `${client.preset} is stopped`),
        })),
        mihomo: component("stopped", "Mihomo controller is not listening"),
        tun: component("stopped", "TUN interface is absent"),
        dns: component("stopped", "DNS listener is inactive"),
        providers: { ready: 0, total: 0, rules_loaded: 0, last_refresh: null },
        exit_ip: null,
      };
      lifecycleBusy = null;
      emit("stopped", null, null, null);
    }, 350);
    return accepted;
  },
  async pause(): Promise<OperationAccepted> {
    if (lifecycleBusy && lifecycleBusy !== "pausing") {
      throw new Error("operation is already in progress");
    }
    if (snapshot.phase === "paused") {
      return { operation_id: crypto.randomUUID(), already_complete: true };
    }
    if (snapshot.phase !== "running" && snapshot.phase !== "degraded") {
      return { operation_id: crypto.randomUUID(), already_complete: true };
    }
    begin("pausing");
    const accepted = operation();
    emit("stopping", accepted.operation_id, "pausing", "stopping_core");
    window.setTimeout(() => {
      emit("stopping", accepted.operation_id, "pausing", "cleaning_up");
    }, 160);
    window.setTimeout(() => {
      snapshot = {
        ...snapshot,
        clients: snapshot.clients.map((client) => ({
          ...client,
          status: component("running", `${client.preset} is ready`),
        })),
        mihomo: component("stopped", "Mihomo controller is not listening"),
        tun: component("stopped", "TUN interface is absent"),
        dns: component("stopped", "DNS listener is inactive"),
        providers: { ready: 0, total: 0, rules_loaded: 0, last_refresh: null },
        exit_ip: null,
      };
      lifecycleBusy = null;
      emit("paused", null, null, null);
    }, 350);
    return accepted;
  },
  async resume(): Promise<OperationAccepted> {
    if (lifecycleBusy && lifecycleBusy !== "resuming") {
      throw new Error("operation is already in progress");
    }
    if (snapshot.phase === "running") {
      return { operation_id: crypto.randomUUID(), already_complete: true };
    }
    if (snapshot.phase !== "paused") {
      return { operation_id: crypto.randomUUID(), already_complete: true };
    }
    begin("resuming");
    const accepted = operation();
    emit(snapshot.phase, accepted.operation_id, "resuming", "preparing");
    void runStart(accepted);
    return accepted;
  },
  async cancel(operationId: string) {
    if (snapshot.operation_id === operationId) {
      lifecycleBusy = null;
      emit("stopped", null, null, null);
      return true;
    }
    return false;
  },
  async getSettings() {
    return structuredClone(settings);
  },
  async validateSettings(draft: AppConfig): Promise<ValidationIssue[]> {
    const ports = [
      ...draft.clients.flatMap((client) =>
        client.config.kind === "local_proxy" ? [client.config.port] : [],
      ),
      draft.mihomo.controller_port,
      draft.mihomo.mixed_port,
      draft.mihomo.dns_port,
    ];
    const issues: ValidationIssue[] = [];
    if (new Set(ports).size !== ports.length) {
      issues.push({
        field: "ports",
        code: "PORT_CONFLICT",
        message: "Ports must be unique",
      });
    }
    issues.push(...validateDirectDns(draft.mihomo));
    return issues;
  },
  async saveSettings(draft: AppConfig, expectedRevision: number) {
    if (expectedRevision !== settings.revision)
      throw new Error("Settings changed in another window");
    const next = sanitizeDefaultRoute(structuredClone(draft));
    settings = { ...next, revision: settings.revision + 1 };
    snapshot = {
      ...snapshot,
      revision: snapshot.revision + 1,
      clients: settings.clients.map((client) => {
        const existing = snapshot.clients.find((item) => item.id === client.id);
        return {
          id: client.id,
          preset: client.preset,
          enabled: client.enabled,
          status:
            existing?.status ??
            component("stopped", `${client.preset} is stopped`),
          exit_ip: existing?.exit_ip ?? null,
        };
      }),
      updated_at: now(),
    };
    for (const listener of listeners) listener(structuredClone(snapshot));
    return structuredClone(settings);
  },
  async listRules() {
    return structuredClone(directRules);
  },
  async addRule(input: string, expectedRevision: number) {
    return mockApi.pinRoute(input, "direct", expectedRevision);
  },
  async pinRoute(input: string, outbound: string, expectedRevision: number) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    const parsed = canonicalTarget(input);
    const route = outboundFromKey(outbound);
    if (route.kind === "client" && isPrivateHost(parsed.value)) {
      throw new Error(
        "private, loopback, and carrier-grade NAT addresses cannot be sent through a local proxy",
      );
    }
    const listId = ensureListFor(route);
    const pin: PinnedRoute = {
      target: parsed,
      outbound: route,
      list_id: listId,
      resolved_ips: parsed.kind === "ip" ? [parsed.value] : [],
      created_at: now(),
      refreshed_at: now(),
    };
    const pins = directRules.pins.filter(
      (item) =>
        !(
          item.target.kind === parsed.kind && item.target.value === parsed.value
        ),
    );
    pins.push(pin);
    directRules = {
      ...directRules,
      revision: directRules.revision + 1,
      pins,
    };
    return structuredClone(directRules);
  },
  async discardClientPins(id: string, expectedRevision: number) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    directRules = {
      revision: directRules.revision + 1,
      pins: directRules.pins.filter(
        (pin) =>
          !(pin.outbound.kind === "client" && pin.outbound.client_id === id),
      ),
      lists: directRules.lists.filter(
        (list) =>
          !(list.outbound.kind === "client" && list.outbound.client_id === id),
      ),
    };
    return structuredClone(directRules);
  },
  async reassignClientPins(from: string, to: string, expectedRevision: number) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    const outbound = outboundFromKey(to);
    directRules = {
      revision: directRules.revision + 1,
      pins: directRules.pins.map((pin) =>
        pin.outbound.kind === "client" && pin.outbound.client_id === from
          ? { ...pin, outbound }
          : pin,
      ),
      lists: directRules.lists.map((list) =>
        list.outbound.kind === "client" && list.outbound.client_id === from
          ? { ...list, outbound }
          : list,
      ),
    };
    return structuredClone(directRules);
  },
  async removeRule(input: string, expectedRevision: number) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    directRules = {
      ...directRules,
      revision: directRules.revision + 1,
      pins: directRules.pins.filter((item) => item.target.value !== input),
    };
    return structuredClone(directRules);
  },
  async createRuleList(
    name: string,
    outbound: string,
    expectedRevision: number,
  ) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    const trimmed = name.trim();
    if (!trimmed || trimmed.length > 60)
      throw new Error("list name must be 1-60 characters");
    directRules = {
      ...directRules,
      revision: directRules.revision + 1,
      lists: [
        ...directRules.lists,
        { id: mockUuid(), name: trimmed, outbound: outboundFromKey(outbound) },
      ],
    };
    return structuredClone(directRules);
  },
  async renameRuleList(listId: string, name: string, expectedRevision: number) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    const trimmed = name.trim();
    if (!trimmed || trimmed.length > 60)
      throw new Error("list name must be 1-60 characters");
    directRules = {
      ...directRules,
      revision: directRules.revision + 1,
      lists: directRules.lists.map((list) =>
        list.id === listId ? { ...list, name: trimmed } : list,
      ),
    };
    return structuredClone(directRules);
  },
  async deleteRuleList(listId: string, expectedRevision: number) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    directRules = {
      revision: directRules.revision + 1,
      pins: directRules.pins.filter((pin) => pin.list_id !== listId),
      lists: directRules.lists.filter((list) => list.id !== listId),
    };
    return structuredClone(directRules);
  },
  async setRuleListOutbound(
    listId: string,
    outbound: string,
    expectedRevision: number,
  ) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    const route = outboundFromKey(outbound);
    if (
      route.kind === "client" &&
      directRules.pins.some(
        (pin) =>
          pin.list_id === listId &&
          pin.target.kind === "ip" &&
          isPrivateHost(pin.target.value),
      )
    ) {
      throw new Error(
        "private, loopback, and carrier-grade NAT addresses cannot be sent through a local proxy",
      );
    }
    directRules = {
      revision: directRules.revision + 1,
      pins: directRules.pins.map((pin) =>
        pin.list_id === listId ? { ...pin, outbound: route } : pin,
      ),
      lists: directRules.lists.map((list) =>
        list.id === listId ? { ...list, outbound: route } : list,
      ),
    };
    return structuredClone(directRules);
  },
  async pinToRuleList(input: string, listId: string, expectedRevision: number) {
    if (expectedRevision !== directRules.revision)
      throw new Error("Rules changed in another window");
    const list = directRules.lists.find((item) => item.id === listId);
    if (!list) throw new Error("unknown list");
    const parsed = canonicalTarget(input);
    if (list.outbound.kind === "client" && isPrivateHost(parsed.value)) {
      throw new Error(
        "private, loopback, and carrier-grade NAT addresses cannot be sent through a local proxy",
      );
    }
    const pins = directRules.pins.filter(
      (item) =>
        !(
          item.target.kind === parsed.kind && item.target.value === parsed.value
        ),
    );
    pins.push({
      target: parsed,
      outbound: list.outbound,
      list_id: listId,
      resolved_ips: parsed.kind === "ip" ? [parsed.value] : [],
      created_at: now(),
      refreshed_at: now(),
    });
    directRules = {
      ...directRules,
      revision: directRules.revision + 1,
      pins,
    };
    return structuredClone(directRules);
  },
  async clientBinaryInstalled(preset: string): Promise<boolean> {
    // Side tunnels need the system OpenVPN binary; the mock reports it
    // missing so the download link stays visible in development.
    if (preset === "openvpn" || preset === "windscribe") {
      return sessionStorage.getItem("biflow-mock-openvpn-installed") === "1";
    }
    return true;
  },
  async checkRuleList(listId: string): Promise<ListCheckEntry[]> {
    const domains = directRules.pins
      .filter((pin) => pin.list_id === listId && pin.target.kind === "domain")
      .slice(0, 3);
    await new Promise((resolve) => setTimeout(resolve, 400));
    return domains.map((pin, index) => ({
      target: pin.target.value,
      status: index === 2 ? "slow" : "ok",
      latency_ms: 120 + index * 340,
      detail: null,
    }));
  },
  async refreshRules() {
    directRules = {
      ...directRules,
      revision: directRules.revision + 1,
      pins: directRules.pins.map((item) => ({ ...item, refreshed_at: now() })),
    };
    return structuredClone(directRules);
  },
  async getCloudRules() {
    return structuredClone(cloudRules);
  },
  async syncCloudRules() {
    cloudRules = {
      ...cloudRules,
      last_synced_at: now(),
      source: "devlifeX/BiFlow",
      snapshot_revision: "767ef8bf5673",
      domain_count: 63_104,
      ip_count: 2_912,
    };
    return structuredClone(cloudRules);
  },
  async listDependencies() {
    return structuredClone(dependencies);
  },
  async installDependency(id: string): Promise<InstallResult> {
    await new Promise((resolve) => setTimeout(resolve, 250));
    dependencies = dependencies.map((item) =>
      item.id === id
        ? { ...item, installed: true, path: `/tmp/biflow/${id}` }
        : item,
    );
    persistMockDependencies();
    return {
      id,
      installed: true,
      path: `/tmp/biflow/${id}`,
      guide: guideFor(id),
    };
  },
  async freshHiddifyStart(): Promise<FreshStartReport> {
    await new Promise((resolve) => setTimeout(resolve, 150));
    return {
      data_dir: "/home/user/.local/share/hiddify",
      backup_dir:
        "/home/user/.local/share/biflow/backups/hiddify-20260815-120000",
      cleared: ["configs", "data", "app.log"],
      preserved: ["db.sqlite", "shared_preferences.json"],
      stopped: true,
      started: true,
    };
  },
  async installHelper(): Promise<{ installed: boolean }> {
    await new Promise((resolve) => setTimeout(resolve, 150));
    snapshot = {
      ...snapshot,
      revision: snapshot.revision + 1,
      helper: {
        phase: "running",
        message: "Mock helper is ready",
        since: now(),
      },
      updated_at: now(),
    };
    for (const listener of listeners) listener(structuredClone(snapshot));
    return { installed: true };
  },
  async getInstallGuide(id: string) {
    return guideFor(id);
  },
  async openUrl(_url: string) {
    return undefined;
  },
  async pickProfileFile(): Promise<string | null> {
    const override =
      typeof window !== "undefined"
        ? window.__BIFLOW_NEXT_PROFILE_PATH__
        : undefined;
    if (override === null) return null;
    if (typeof override === "string" && override.length > 0) {
      return override;
    }
    return "/tmp/biflow-mock-profile.ovpn";
  },
  async applyLiveSettings(): Promise<void> {
    return undefined;
  },
  async testRoute(target: string): Promise<RouteTestResult> {
    // Mirrors RuleSet::decide: private, enabled client pins, DIRECT pins,
    // bundled Iran list, then MATCH default_route.
    if (isPrivateHost(target)) {
      return route(target, { kind: "direct" }, "private_or_local", target);
    }
    const enabled = enabledClientIds();
    // Longest matching pin wins across lists and outbounds.
    const pin = bestPinMatch(
      target,
      directRules.pins.filter(
        (item) =>
          item.outbound.kind === "direct" ||
          enabled.has(item.outbound.client_id),
      ),
    );
    if (pin) {
      return route(
        target,
        pin.outbound,
        pin.outbound.kind === "direct" ? "custom_rule" : "vpn_rule",
        pin.target.value,
      );
    }
    if (target.endsWith(".ir") || target === "ir") {
      return route(target, { kind: "direct" }, "iran_domain", "ir");
    }
    const business = IRAN_BUSINESS_DOMAINS.find((pin) =>
      domainMatchesPin(target, pin),
    );
    if (business) {
      return route(target, { kind: "direct" }, "iran_domain", business);
    }
    return route(target, matchOutbound(), "default_proxy", "MATCH");
  },
  async checkReachability(): Promise<ReachabilityResult[]> {
    await new Promise((resolve) => setTimeout(resolve, 150));
    // Mirrors the real probe: VPN domains only pass while the stack runs;
    // iran.ir is DIRECT and reachable either way.
    const connected =
      snapshot.phase === "running" || snapshot.phase === "degraded";
    const vpnStatus = (domain: string, id: string): ReachabilityResult => ({
      id,
      domain,
      path: "vpn",
      via_proxy: connected,
      status: connected ? "ok" : "unreachable",
      latency_ms: connected ? 420 : null,
      detail: connected ? null : "connection closed before TLS finished",
    });
    return [
      vpnStatus("google.com", "google"),
      vpnStatus("facebook.com", "facebook"),
      {
        id: "iran",
        domain: "iran.ir",
        path: "direct",
        via_proxy: false,
        status: "ok",
        latency_ms: 95,
        detail: null,
      },
    ];
  },
  async diagnostics(): Promise<DiagnosticsReport> {
    const steps: DiagnosticStep[] = [
      "Helper authorization",
      "Hiddify listener",
      "Mihomo controller",
      "Providers",
      "Owned TUN state",
      "Foreign egress",
    ].map((label, index) => ({
      id: String(index),
      label,
      status:
        index === 2 && snapshot.phase === "stopped" ? "warning" : "passed",
      detail:
        index === 2 && snapshot.phase === "stopped"
          ? "Stack is disconnected"
          : null,
      started_at: now(),
      finished_at: now(),
    }));
    return { operation_id: crypto.randomUUID(), steps, finished: true };
  },
  async queryLogs() {
    return structuredClone(logs);
  },
  async debugLogStatus(): Promise<DebugLogStatus> {
    return mockDebugLogStatus();
  },
  async revealDebugLog(): Promise<DebugLogStatus> {
    return mockDebugLogStatus();
  },
  async deleteDebugLog(): Promise<DebugLogStatus> {
    debugLogSize = 512;
    return mockDebugLogStatus();
  },
  async exportBundle(): Promise<ExportResult> {
    return {
      path: "/tmp/biflow-support-mock.json",
      files: [
        "versions.json",
        "config-redacted.json",
        "snapshot.json",
        "debug.log",
      ],
    };
  },
  async getUpdateState(): Promise<UpdateProgress> {
    return structuredClone(lastUpdateProgress);
  },
  async checkUpdate(): Promise<UpdateStatus> {
    if (mockUpdateShouldFail()) {
      throw new Error("Malformed update manifest");
    }
    if (mockUpdateAvailable()) {
      return {
        available: true,
        version: "9.9.9",
        notes: "Mock GitHub Release",
        app_available: true,
        rules_available: false,
        thirdparty_available: false,
      };
    }
    return {
      available: false,
      version: null,
      notes: null,
      app_available: false,
      rules_available: false,
      thirdparty_available: false,
    };
  },
  async installUpdate(): Promise<OperationAccepted> {
    if (mockUpdateShouldFail()) {
      emitUpdateProgress({
        phase: "failed",
        percent: null,
        version: "9.9.9",
        error: "Signature verification failed",
      });
      throw new Error("Signature verification failed");
    }
    if (!mockUpdateAvailable()) {
      throw new Error("no update is available");
    }
    const accepted = operation();
    await simulateInstallProgress("9.9.9");
    return accepted;
  },
  subscribeUpdateProgress(listener: (progress: UpdateProgress) => void) {
    updateListeners.add(listener);
    return () => updateListeners.delete(listener);
  },
  subscribe(listener: (next: StackSnapshot) => void) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
};

export function resetMockState() {
  try {
    sessionStorage.setItem("biflow-mock-force-missing-deps", "1");
    localStorage.removeItem("biflow-mock-installed-deps");
  } catch {
    // jsdom and Playwright always provide web storage.
  }
  lifecycleBusy = null;
  snapshot = initialSnapshot();
  trafficTotals = { sent: 1_048_576, received: 2_097_152 };
  lastSessionSent = 0;
  lastSessionReceived = 0;
  settings = initialSettings();
  directRules = initialDirectRules();
  cloudRules = initialCloudRules();
  dependencies = missingDependencies();
  logs.length = 0;
  debugLogSize = 48_512;
  logs.push({
    timestamp: now(),
    level: "info",
    event: "mock_transport_ready",
    fields: { mode: "development" },
  });
  listeners.clear();
  updateListeners.clear();
  if (typeof window !== "undefined") {
    delete window.__BIFLOW_NEXT_PROFILE_PATH__;
  }
}

if (typeof window !== "undefined") {
  window.__BIFLOW_RESET_MOCK = resetMockState;
}
