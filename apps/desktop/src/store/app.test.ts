import { beforeEach, describe, expect, it, vi } from "vitest";
import { desktop } from "../api/desktop";
import type { BootstrapResult } from "../api/models";
import { useAppStore } from "./app";

vi.mock("../api/desktop", () => ({
  desktop: {
    bootstrap: vi.fn(),
    subscribe: vi.fn(async () => () => undefined),
    subscribeUpdateProgress: vi.fn(async () => () => undefined),
    start: vi.fn(),
    stop: vi.fn(),
    pause: vi.fn(),
    resume: vi.fn(),
    installDependency: vi.fn(),
    installHelper: vi.fn(),
    listDependencies: vi.fn(),
    getInstallGuide: vi.fn(),
    getSnapshot: vi.fn(),
    syncCloudRules: vi.fn(),
    getNetworkStatus: vi.fn(),
    getTrafficTotals: vi.fn(),
    addRule: vi.fn(),
    pinRoute: vi.fn(),
    checkUpdate: vi.fn(),
    getUpdateState: vi.fn(),
    installUpdate: vi.fn(),
    openUrl: vi.fn(),
  },
}));

const boot = {
  app_version: "1.0.0",
  platform: "linux",
  mock_mode: true,
  snapshot: {
    revision: 1,
    phase: "stopped",
    operation_id: null,
    helper: { phase: "running", message: "Helper is ready", since: "now" },
    clients: [
      {
        id: "11111111-1111-1111-1111-111111111111",
        preset: "hiddify",
        enabled: true,
        status: { phase: "stopped", message: null, since: "now" },
      },
    ],
    mihomo: { phase: "stopped", message: null, since: "now" },
    tun: { phase: "stopped", message: null, since: "now" },
    dns: { phase: "stopped", message: null, since: "now" },
    providers: { ready: 0, total: 0, rules_loaded: 0, last_refresh: null },
    exit_ip: null,
    backend: "external_hiddify",
    last_error: null,
    updated_at: "now",
  },
  settings: { revision: 0 },
  direct_rules: { revision: 1, pins: [], lists: [] },
  cloud_rules: {
    domain_count: 10,
    ip_count: 4,
    last_synced_at: null,
    source: "bundled",
    snapshot_revision: null,
    sets: [],
  },
  dependencies: [
    {
      id: "hiddify",
      name: "Hiddify",
      installed: false,
      version: null,
      path: null,
    },
  ],
  network_status: {
    state: "checking",
    public_ip: null,
    country_code: null,
    city: null,
    checked_at: "now",
    detail: null,
  },
} as unknown as BootstrapResult;

describe("app store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAppStore.setState({
      loading: true,
      actionPending: false,
      installingId: null,
      page: "dashboard",
      boot: null,
      snapshot: null,
      settings: null,
      rules: null,
      cloudRules: null,
      dependencies: [],
      networkStatus: null,
      trafficTotals: { sent: 0, received: 0 },
      trafficRefreshing: false,
      diagnostics: null,
      error: null,
      installGuide: null,
      update: {
        phase: "idle",
        percent: null,
        version: null,
        error: null,
      },
    });
  });

  it("loads bootstrap data including cloud rules and dependencies", async () => {
    vi.mocked(desktop.bootstrap).mockResolvedValue(boot);
    vi.mocked(desktop.getNetworkStatus).mockResolvedValue({
      state: "online",
      public_ip: "203.0.113.8",
      country_code: "IR",
      city: "Tehran",
      checked_at: "now",
      detail: null,
    });
    vi.mocked(desktop.getTrafficTotals).mockResolvedValue({
      sent: 2048,
      received: 4096,
    });
    await useAppStore.getState().initialize();
    const state = useAppStore.getState();
    expect(state.loading).toBe(false);
    expect(state.cloudRules?.domain_count).toBe(10);
    expect(state.dependencies[0]?.id).toBe("hiddify");
    expect(desktop.getNetworkStatus).toHaveBeenCalledOnce();
    expect(desktop.getTrafficTotals).toHaveBeenCalledOnce();
    expect(state.trafficTotals).toEqual({ sent: 2048, received: 4096 });
  });

  it("keeps lifetime traffic totals when a later probe fails", async () => {
    useAppStore.setState({
      trafficTotals: { sent: 9_000, received: 8_000 },
      trafficRefreshing: false,
    });
    vi.mocked(desktop.getTrafficTotals).mockRejectedValue(
      new Error("controller offline"),
    );
    await useAppStore.getState().refreshTrafficTotals();
    expect(useAppStore.getState().trafficTotals).toEqual({
      sent: 9_000,
      received: 8_000,
    });
    expect(useAppStore.getState().trafficRefreshing).toBe(false);
  });

  it("ignores a second traffic refresh while one is in flight", async () => {
    useAppStore.setState({ trafficRefreshing: true });
    await useAppStore.getState().refreshTrafficTotals();
    expect(desktop.getTrafficTotals).not.toHaveBeenCalled();
  });

  it("starts only one backend operation when Connect is clicked twice", async () => {
    vi.mocked(desktop.start).mockResolvedValue({
      operation_id: "op",
      already_complete: false,
    });
    useAppStore.setState({
      snapshot: boot.snapshot,
      actionPending: false,
      dependencies: [
        {
          id: "hiddify",
          name: "Hiddify",
          installed: true,
          version: null,
          path: "/tmp/hiddify",
        },
        {
          id: "mihomo",
          name: "Mihomo",
          installed: true,
          version: null,
          path: "/tmp/mihomo",
        },
      ],
    });
    const first = useAppStore.getState().toggleConnection();
    await useAppStore.getState().toggleConnection();
    await first;
    expect(desktop.start).toHaveBeenCalledOnce();
    expect(useAppStore.getState().actionPending).toBe(true);
  });

  it("restores controls after a failed connection command", async () => {
    vi.mocked(desktop.start).mockRejectedValue(new Error("helper missing"));
    useAppStore.setState({
      snapshot: boot.snapshot,
      actionPending: false,
      dependencies: [
        {
          id: "hiddify",
          name: "Hiddify",
          installed: true,
          version: null,
          path: "/tmp/hiddify",
        },
        {
          id: "mihomo",
          name: "Mihomo",
          installed: true,
          version: null,
          path: "/tmp/mihomo",
        },
      ],
    });
    await useAppStore.getState().toggleConnection();
    expect(useAppStore.getState().actionPending).toBe(false);
    expect(useAppStore.getState().error).toMatch(/helper missing/);
  });

  it("starts the stack from a stopped snapshot", async () => {
    vi.mocked(desktop.start).mockResolvedValue({
      operation_id: "op",
      already_complete: false,
    });
    useAppStore.setState({
      snapshot: boot.snapshot,
      actionPending: false,
      dependencies: [
        {
          id: "hiddify",
          name: "Hiddify",
          installed: true,
          version: null,
          path: "/tmp/hiddify",
        },
        {
          id: "mihomo",
          name: "Mihomo",
          installed: true,
          version: null,
          path: "/tmp/mihomo",
        },
      ],
    });
    await useAppStore.getState().toggleConnection();
    expect(desktop.start).toHaveBeenCalledOnce();
    expect(desktop.installHelper).not.toHaveBeenCalled();
    expect(desktop.installDependency).not.toHaveBeenCalled();
    expect(useAppStore.getState().actionPending).toBe(true);
  });

  it("installs helper then apps before connecting", async () => {
    const order: string[] = [];
    vi.mocked(desktop.installHelper).mockImplementation(async () => {
      order.push("helper");
      return { installed: true };
    });
    vi.mocked(desktop.getSnapshot).mockResolvedValue({
      ...boot.snapshot,
      helper: { phase: "running", message: "ready", since: "now" },
    });
    vi.mocked(desktop.installDependency).mockImplementation(async (id) => {
      order.push(id);
      return {
        id,
        installed: true,
        path: `/tmp/${id}`,
        guide: {
          id,
          title: id,
          download_url: "https://example.invalid",
          steps: [],
        },
      };
    });
    vi.mocked(desktop.listDependencies).mockResolvedValue([
      {
        id: "hiddify",
        name: "Hiddify",
        installed: true,
        version: null,
        path: "/tmp/hiddify",
      },
      {
        id: "mihomo",
        name: "Mihomo",
        installed: true,
        version: null,
        path: "/tmp/mihomo",
      },
    ]);
    vi.mocked(desktop.start).mockResolvedValue({
      operation_id: "op",
      already_complete: false,
    });
    useAppStore.setState({
      snapshot: {
        ...boot.snapshot,
        helper: { phase: "unavailable", message: "missing", since: "now" },
      },
      dependencies: [
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
      ],
      actionPending: false,
    });
    await useAppStore.getState().toggleConnection();
    expect(order).toEqual(["helper", "hiddify", "mihomo"]);
    expect(desktop.start).toHaveBeenCalledOnce();
    expect(useAppStore.getState().installingId).toBeNull();
  });

  it("stops before start when a required install fails", async () => {
    vi.mocked(desktop.installDependency).mockResolvedValue({
      id: "hiddify",
      installed: false,
      path: null,
      guide: {
        id: "hiddify",
        title: "Install Hiddify",
        download_url: "https://example.invalid",
        steps: ["Download"],
      },
    });
    vi.mocked(desktop.listDependencies).mockResolvedValue(boot.dependencies);
    useAppStore.setState({
      snapshot: boot.snapshot,
      dependencies: boot.dependencies,
      actionPending: false,
    });
    await useAppStore.getState().toggleConnection();
    expect(desktop.start).not.toHaveBeenCalled();
    expect(useAppStore.getState().error).toMatch(/hiddify installation/);
    expect(useAppStore.getState().installGuide?.id).toBe("hiddify");
    expect(useAppStore.getState().actionPending).toBe(false);
  });

  it("pauses and resumes from a running snapshot", async () => {
    vi.mocked(desktop.pause).mockResolvedValue({
      operation_id: "pause",
      already_complete: false,
    });
    vi.mocked(desktop.resume).mockResolvedValue({
      operation_id: "resume",
      already_complete: false,
    });
    useAppStore.setState({
      snapshot: { ...boot.snapshot, phase: "running" },
      actionPending: false,
    });
    await useAppStore.getState().pauseConnection();
    expect(desktop.pause).toHaveBeenCalledOnce();
    useAppStore.setState({
      snapshot: { ...boot.snapshot, phase: "paused" },
      actionPending: false,
    });
    await useAppStore.getState().resumeConnection();
    expect(desktop.resume).toHaveBeenCalledOnce();
  });

  it("keeps a manual install guide when automatic install fails", async () => {
    vi.mocked(desktop.installDependency).mockRejectedValue(
      new Error("download blocked"),
    );
    vi.mocked(desktop.getInstallGuide).mockResolvedValue({
      id: "hiddify",
      title: "Install Hiddify on Linux",
      download_url: "https://github.com/hiddify/hiddify-app/releases/latest",
      steps: ["Download the AppImage"],
    });
    await useAppStore.getState().installDependency("hiddify");
    const state = useAppStore.getState();
    expect(state.installingId).toBeNull();
    expect(state.error).toMatch(/download blocked/);
    expect(state.installGuide?.steps[0]).toMatch(/AppImage/);
  });

  it("installs the privileged helper from the dashboard action", async () => {
    vi.mocked(desktop.installHelper).mockResolvedValue({ installed: true });
    vi.mocked(desktop.getSnapshot).mockResolvedValue(boot.snapshot);
    await useAppStore.getState().installHelper();
    expect(desktop.installHelper).toHaveBeenCalledOnce();
    expect(desktop.getSnapshot).toHaveBeenCalledOnce();
    expect(useAppStore.getState().installingId).toBeNull();
  });

  it("updates cloud rule counts after a successful sync", async () => {
    vi.mocked(desktop.syncCloudRules).mockResolvedValue({
      domain_count: 20,
      ip_count: 8,
      last_synced_at: "2026-08-13T00:00:00.000Z",
      source: "devlifeX/BiFlow",
      snapshot_revision: "767ef8bf5673",
      sets: [],
    });
    await useAppStore.getState().syncCloudRules();
    expect(useAppStore.getState().cloudRules?.source).toBe("devlifeX/BiFlow");
    expect(useAppStore.getState().actionPending).toBe(false);
  });

  it("ignores a second network refresh while one is in flight", async () => {
    useAppStore.setState({ networkRefreshing: true });
    await useAppStore.getState().refreshNetworkStatus();
    expect(desktop.getNetworkStatus).not.toHaveBeenCalled();
  });

  it("does not start a second cloud rule sync while one is pending", async () => {
    useAppStore.setState({ actionPending: true });
    await useAppStore.getState().syncCloudRules();
    expect(desktop.syncCloudRules).not.toHaveBeenCalled();
  });

  it("tracks available and failed update states", async () => {
    vi.mocked(desktop.checkUpdate).mockResolvedValue({
      available: true,
      version: "1.3.0",
      notes: "Signed release",
      app_available: true,
      rules_available: false,
      thirdparty_available: false,
    });
    await useAppStore.getState().checkForUpdate();
    expect(useAppStore.getState().update.phase).toBe("available");
    expect(useAppStore.getState().update.version).toBe("1.3.0");

    vi.mocked(desktop.checkUpdate).mockRejectedValue(new Error("bad manifest"));
    await useAppStore.getState().checkForUpdate();
    expect(useAppStore.getState().update.phase).toBe("failed");
    expect(useAppStore.getState().update.error).toMatch(/bad manifest/);
  });

  it("ignores a second update check while one is already in flight", async () => {
    let finish: (status: {
      available: boolean;
      version: string | null;
      notes: string | null;
      app_available: boolean;
      rules_available: boolean;
      thirdparty_available: boolean;
    }) => void = () => undefined;
    vi.mocked(desktop.checkUpdate).mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const first = useAppStore.getState().checkForUpdate();
    await useAppStore.getState().checkForUpdate();
    expect(desktop.checkUpdate).toHaveBeenCalledOnce();
    finish({
      available: false,
      version: null,
      notes: null,
      app_available: false,
      rules_available: false,
      thirdparty_available: false,
    });
    await first;
    expect(useAppStore.getState().update.phase).toBe("current");
  });

  it("rethrows addRule failures after recording the store error", async () => {
    useAppStore.setState({
      rules: { revision: 1, pins: [], lists: [] },
      error: null,
    });
    vi.mocked(desktop.addRule).mockRejectedValue(new Error("rules changed"));
    await expect(useAppStore.getState().addRule("example.com")).rejects.toThrow(
      /rules changed/,
    );
    expect(useAppStore.getState().error).toMatch(/rules changed/);
  });

  it("extracts the host from a pasted URL before adding a rule", async () => {
    vi.mocked(desktop.addRule).mockResolvedValue({
      revision: 2,
      pins: [],
      lists: [],
    });
    useAppStore.setState({
      rules: { revision: 1, pins: [], lists: [] },
      error: null,
    });
    await useAppStore.getState().addRule("https://console.kavenegar.com/");
    expect(desktop.addRule).toHaveBeenCalledWith("console.kavenegar.com", 1);
  });

  it("retries install after a failed update when a version is known", async () => {
    vi.mocked(desktop.installUpdate).mockResolvedValue({
      operation_id: "update-op",
      already_complete: false,
    });
    useAppStore.setState({
      update: {
        phase: "failed",
        percent: null,
        version: "1.3.0",
        error: "interrupted download",
      },
    });
    await useAppStore.getState().retryUpdate();
    expect(desktop.installUpdate).toHaveBeenCalledOnce();
    expect(useAppStore.getState().update.phase).toBe("downloading");
  });
});
