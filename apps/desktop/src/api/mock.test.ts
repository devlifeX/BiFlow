import { beforeEach, describe, expect, it, vi } from "vitest";
import { MOCK_HIDDIFY_ID } from "../lib/outbound";
import { APP_VERSION } from "../version";
import type { StackSnapshot } from "./models";
import { mockApi, resetMockState } from "./mock";

describe("mock transport", () => {
  beforeEach(() => {
    sessionStorage.removeItem("biflow-mock-force-missing-helper");
    resetMockState();
  });

  it("bootstraps the current application version", async () => {
    const boot = await mockApi.bootstrap();
    expect(boot.app_version).toBe(APP_VERSION);
    expect(boot.mock_mode).toBe(true);
    expect(boot.cloud_rules.domain_count).toBeGreaterThan(0);
    expect(boot.dependencies).toHaveLength(2);
    expect(boot.dependencies.every((item) => item.installed === false)).toBe(
      true,
    );
  });

  it("installs missing third-party apps into the user data path", async () => {
    const result = await mockApi.installDependency("hiddify");
    expect(result.installed).toBe(true);
    const [hiddify] = await mockApi.listDependencies();
    expect(hiddify?.installed).toBe(true);
    expect(hiddify?.path).toContain("biflow");
    expect(localStorage.getItem("biflow-mock-installed-deps")).toMatch(
      /"installed":true/,
    );
  });

  it("installs a missing mock helper", async () => {
    sessionStorage.setItem("biflow-mock-force-missing-helper", "1");
    resetMockState();
    const boot = await mockApi.bootstrap();
    expect(boot.snapshot.helper.phase).toBe("unavailable");
    await mockApi.installHelper();
    const snapshot = await mockApi.getSnapshot();
    expect(snapshot.helper.phase).toBe("running");
  });

  it("routes .ir hosts direct and other hosts through the vpn", async () => {
    await expect(mockApi.testRoute("digikala.ir")).resolves.toMatchObject({
      outbound: { kind: "direct" },
    });
    await expect(mockApi.testRoute("openai.com")).resolves.toMatchObject({
      outbound: { kind: "client", client_id: MOCK_HIDDIFY_ID },
      matched_rule: "MATCH",
    });
  });

  it("keeps subdomain pins exact and the longest match wins", async () => {
    // Root pin stays DIRECT and covers every subdomain.
    const first = await mockApi.addRule("google.com", 1);
    expect(first.pins.map((item) => item.target.value)).toEqual(
      expect.arrayContaining(["example.ir", "google.com"]),
    );
    // A more specific pin routes its own subtree to the client.
    const moved = await mockApi.pinRoute(
      "developer.google.com",
      MOCK_HIDDIFY_ID,
      first.revision,
    );
    expect(
      moved.pins.find((item) => item.outbound.kind === "client")?.target.value,
    ).toBe("developer.google.com");
    await expect(mockApi.testRoute("gemini.google.com")).resolves.toMatchObject(
      {
        outbound: { kind: "direct" },
        matched_rule: "google.com",
      },
    );
    await expect(
      mockApi.testRoute("developer.google.com"),
    ).resolves.toMatchObject({
      outbound: { kind: "client", client_id: MOCK_HIDDIFY_ID },
      matched_rule: "developer.google.com",
    });
    await expect(
      mockApi.testRoute("api.developer.google.com"),
    ).resolves.toMatchObject({
      outbound: { kind: "client", client_id: MOCK_HIDDIFY_ID },
      matched_rule: "developer.google.com",
    });
    await expect(mockApi.testRoute("notgoogle.com")).resolves.toMatchObject({
      matched_rule: "MATCH",
    });
  });

  it("keeps github.io tenants separate and routes curated businesses direct", async () => {
    const pinned = await mockApi.addRule("user.github.io", 1);
    expect(
      pinned.pins.some((item) => item.target.value === "user.github.io"),
    ).toBe(true);
    await expect(mockApi.addRule("github.io", pinned.revision)).rejects.toThrow(
      /public suffixes/i,
    );
    await expect(
      mockApi.testRoute("www.technolife.com"),
    ).resolves.toMatchObject({
      outbound: { kind: "direct" },
      matched_rule: "technolife.com",
    });
    await expect(
      mockApi.testRoute("selleracademy.technolife.com"),
    ).resolves.toMatchObject({ outbound: { kind: "direct" } });
    await expect(
      mockApi.testRoute("console.kavenegar.com"),
    ).resolves.toMatchObject({
      outbound: { kind: "direct" },
      matched_rule: "kavenegar.com",
    });
  });

  it("resyncs cloud rule counts from the BiFlow snapshot", async () => {
    const synced = await mockApi.syncCloudRules();
    expect(synced.source).toBe("devlifeX/BiFlow");
    expect(synced.snapshot_revision).toBeTruthy();
    expect(synced.last_synced_at).toBeTruthy();
    expect(synced.domain_count).toBeGreaterThan(62_829);
  });

  it("reports an available mock update when session storage requests it", async () => {
    sessionStorage.setItem("biflow-mock-update-available", "1");
    await expect(mockApi.checkUpdate()).resolves.toMatchObject({
      available: true,
      version: "9.9.9",
    });
  });

  it("emits download progress during mock install", async () => {
    sessionStorage.setItem("biflow-mock-update-available", "1");
    const phases: string[] = [];
    const unsubscribe = mockApi.subscribeUpdateProgress((progress) => {
      phases.push(progress.phase);
    });
    await mockApi.installUpdate();
    unsubscribe();
    expect(phases).toContain("downloading");
    expect(phases.at(-1)).toBe("restarting");
  });

  it("publishes real start stages on the snapshot", async () => {
    const stages: Array<string | null | undefined> = [];
    const componentSnapshots: StackSnapshot[] = [];
    const unsubscribe = mockApi.subscribe((snapshot) => {
      stages.push(snapshot.operation_stage);
      componentSnapshots.push(structuredClone(snapshot));
    });
    await mockApi.start();
    await vi.waitFor(
      async () => {
        const snapshot = await mockApi.getSnapshot();
        expect(snapshot.phase).toBe("running");
      },
      { timeout: 5_000 },
    );
    unsubscribe();
    expect(stages).toContain("preparing");
    expect(stages).toContain("starting_client");
    expect(stages).toContain("starting_core");
    expect(stages).toContain("checking_readiness");
    expect(
      componentSnapshots.some(
        (snapshot) =>
          snapshot.phase !== "running" && snapshot.helper.phase === "running",
      ),
    ).toBe(true);
    expect(
      componentSnapshots.some(
        (snapshot) =>
          snapshot.phase !== "running" &&
          snapshot.clients.some((client) => client.status.phase === "running"),
      ),
    ).toBe(true);
    expect(
      componentSnapshots.some(
        (snapshot) =>
          snapshot.phase !== "running" && snapshot.mihomo.phase === "running",
      ),
    ).toBe(true);
    expect(
      componentSnapshots.some(
        (snapshot) =>
          snapshot.phase !== "running" && snapshot.tun.phase === "running",
      ),
    ).toBe(true);
  });

  it("lists mock DIRECT and VPN connections only while connected", async () => {
    await expect(mockApi.listActiveConnections()).resolves.toEqual([]);
    await mockApi.start();
    await vi.waitFor(async () => {
      const snapshot = await mockApi.getSnapshot();
      expect(snapshot.phase).toBe("running");
    });
    await expect(mockApi.listActiveConnections()).resolves.toEqual([
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
    ]);
  });

  it("rejects a second connection operation while one is running", async () => {
    const first = mockApi.start();
    await expect(mockApi.stop()).rejects.toThrow(/already in progress/);
    await expect(mockApi.pause()).rejects.toThrow(/already in progress/);
    await first;
    await vi.waitFor(async () => {
      const snapshot = await mockApi.getSnapshot();
      expect(snapshot.phase).toBe("running");
      expect(snapshot.busy).toBeNull();
    });
  });

  it("fails mock install when signature verification is forced to fail", async () => {
    sessionStorage.setItem("biflow-mock-update-available", "1");
    sessionStorage.setItem("biflow-mock-update-fail", "1");
    await expect(mockApi.installUpdate()).rejects.toThrow(
      /signature verification failed/i,
    );
  });

  it("rejects an empty custom DIRECT DNS list", async () => {
    const settings = await mockApi.getSettings();
    const issues = await mockApi.validateSettings({
      ...settings,
      mihomo: {
        ...settings.mihomo,
        direct_dns_preset: "custom",
        direct_dns_servers: [],
      },
    });
    expect(issues.some((issue) => issue.code === "DIRECT_DNS_REQUIRED")).toBe(
      true,
    );
  });

  it("follows MATCH Direct and keeps disabled-client pins out of decide", async () => {
    const settings = await mockApi.getSettings();
    await mockApi.saveSettings(
      { ...settings, default_route: { kind: "direct" } },
      settings.revision,
    );
    await expect(mockApi.testRoute("openai.com")).resolves.toMatchObject({
      outbound: { kind: "direct" },
      matched_rule: "MATCH",
    });
    const pinned = await mockApi.pinRoute("openai.com", MOCK_HIDDIFY_ID, 1);
    const disabled = await mockApi.getSettings();
    await mockApi.saveSettings(
      {
        ...disabled,
        clients: disabled.clients.map((client) => ({
          ...client,
          enabled: false,
        })),
      },
      disabled.revision,
    );
    await expect(mockApi.testRoute("openai.com")).resolves.toMatchObject({
      outbound: { kind: "direct" },
      matched_rule: "MATCH",
    });
    expect(
      pinned.pins.some(
        (pin) =>
          pin.target.value === "openai.com" && pin.outbound.kind === "client",
      ),
    ).toBe(true);
  });

  it("returns a mock side-tunnel profile path", async () => {
    await expect(mockApi.pickProfileFile()).resolves.toBe(
      "/tmp/biflow-mock-profile.ovpn",
    );
    window.__BIFLOW_NEXT_PROFILE_PATH__ = null;
    await expect(mockApi.pickProfileFile()).resolves.toBeNull();
    window.__BIFLOW_NEXT_PROFILE_PATH__ = "/home/user/windscribe.ovpn";
    await expect(mockApi.pickProfileFile()).resolves.toBe(
      "/home/user/windscribe.ovpn",
    );
    resetMockState();
    await expect(mockApi.pickProfileFile()).resolves.toBe(
      "/tmp/biflow-mock-profile.ovpn",
    );
  });
});
