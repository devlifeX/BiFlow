import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { desktop } from "../api/desktop";
import { MOCK_HIDDIFY_ID } from "../lib/outbound";
import { useAppStore } from "../store/app";
import { baseSettings, baseSnapshot } from "../test/fixtures";
import { Diagnostics } from "./Diagnostics";

vi.mock("../api/desktop", () => ({
  desktop: {
    queryLogs: vi.fn().mockResolvedValue([]),
    debugLogStatus: vi.fn().mockResolvedValue({
      path: "/home/user/.local/share/biflow/debug.log",
      size_bytes: 48_512,
    }),
    revealDebugLog: vi.fn().mockResolvedValue({
      path: "/home/user/.local/share/biflow/debug.log",
      size_bytes: 48_512,
    }),
    deleteDebugLog: vi.fn().mockResolvedValue({
      path: "/home/user/.local/share/biflow/debug.log",
      size_bytes: 512,
    }),
    testRoute: vi.fn().mockResolvedValue({
      target: "openai.com",
      outbound: {
        kind: "client",
        client_id: "11111111-1111-1111-1111-111111111111",
      },
      reason: "default_proxy",
      matched_rule: "MATCH",
      reachable: true,
      tested_at: new Date().toISOString(),
    }),
    exportBundle: vi.fn(),
    listActiveConnections: vi.fn().mockResolvedValue([]),
    checkReachability: vi.fn().mockResolvedValue([
      {
        id: "google",
        domain: "google.com",
        path: "vpn",
        via_proxy: true,
        status: "unreachable",
        latency_ms: null,
        detail: "tls closed",
      },
      {
        id: "facebook",
        domain: "facebook.com",
        path: "vpn",
        via_proxy: true,
        status: "slow",
        latency_ms: 3200,
        detail: null,
      },
      {
        id: "iran",
        domain: "iran.ir",
        path: "direct",
        via_proxy: false,
        status: "ok",
        latency_ms: 95,
        detail: null,
      },
    ]),
    freshHiddifyStart: vi.fn().mockResolvedValue({
      data_dir: "/home/user/.local/share/hiddify",
      backup_dir: "/home/user/.local/share/biflow/backups/hiddify-20260815",
      cleared: ["configs", "data", "app.log"],
      preserved: ["db.sqlite", "shared_preferences.json"],
      stopped: true,
      started: true,
    }),
  },
}));

describe("Diagnostics", () => {
  // The module mock is shared by every test, so call history has to be dropped
  // or a "was not called" assertion sees the previous test's click.
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(desktop.listActiveConnections).mockResolvedValue([]);
    useAppStore.setState({
      snapshot: null,
      settings: baseSettings(),
      actionPending: false,
    });
  });

  it("tests whether a host is direct or vpn", async () => {
    render(<Diagnostics report={null} />);
    await userEvent.type(
      screen.getByLabelText("Test IP or domain"),
      "openai.com",
    );
    await userEvent.click(screen.getByRole("button", { name: "Test flow" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "openai.com → Hiddify",
    );
  });

  it("shows the permanent debug log location and size", async () => {
    render(<Diagnostics report={null} />);
    expect(await screen.findByTestId("debug-log-size")).toHaveTextContent(
      "47 KiB",
    );
    expect(
      screen.getByText("/home/user/.local/share/biflow/debug.log"),
    ).toBeVisible();
  });

  it("accepts a full URL and tests only its host", async () => {
    render(<Diagnostics report={null} />);
    await userEvent.type(
      screen.getByLabelText("Test IP or domain"),
      "https://www.rade.ir/some/path?a=1",
    );
    await userEvent.click(screen.getByRole("button", { name: "Test flow" }));

    const { desktop } = await import("../api/desktop");
    expect(desktop.testRoute).toHaveBeenCalledWith("www.rade.ir");
  });

  it("offers to move a VPN host to direct and re-tests it", async () => {
    const pinRoute = vi.fn().mockResolvedValue(undefined);
    const previous = useAppStore.getState().pinRoute;
    useAppStore.setState({ pinRoute });
    try {
      render(<Diagnostics report={null} />);
      await userEvent.type(
        screen.getByLabelText("Test IP or domain"),
        "openai.com",
      );
      await userEvent.click(screen.getByRole("button", { name: "Test flow" }));

      const status = await screen.findByRole("status");
      const picker = status.querySelector("select");
      expect(picker).not.toBeNull();
      await userEvent.selectOptions(picker!, "direct");

      expect(pinRoute).toHaveBeenCalledWith("openai.com", "direct");
      const { desktop } = await import("../api/desktop");
      // The result is re-tested so the card reflects the new routing.
      expect(desktop.testRoute).toHaveBeenCalledTimes(2);
    } finally {
      useAppStore.setState({ pinRoute: previous });
    }
  });

  it("offers Add to VPN for a host the bundled Iran list keeps direct", async () => {
    const pinRoute = vi.fn().mockResolvedValue(undefined);
    const previous = useAppStore.getState().pinRoute;
    useAppStore.setState({ pinRoute });
    const { desktop } = await import("../api/desktop");
    vi.mocked(desktop.testRoute).mockResolvedValueOnce({
      target: "iran.ir",
      outbound: { kind: "direct" },
      reason: "iran_domain",
      matched_rule: "ir",
      reachable: true,
      tested_at: new Date().toISOString(),
    });
    try {
      render(<Diagnostics report={null} />);
      await userEvent.type(
        screen.getByLabelText("Test IP or domain"),
        "iran.ir",
      );
      await userEvent.click(screen.getByRole("button", { name: "Test flow" }));

      const status = await screen.findByRole("status");
      const picker = status.querySelector("select");
      expect(picker).not.toBeNull();
      await userEvent.selectOptions(picker!, MOCK_HIDDIFY_ID);
      expect(pinRoute).toHaveBeenCalledWith("iran.ir", MOCK_HIDDIFY_ID);
    } finally {
      useAppStore.setState({ pinRoute: previous });
    }
  });

  it("never offers to move a private or local address onto the VPN", async () => {
    const { desktop } = await import("../api/desktop");
    vi.mocked(desktop.testRoute).mockResolvedValueOnce({
      target: "192.168.1.1",
      outbound: { kind: "direct" },
      reason: "private_or_local",
      matched_rule: "192.168.1.1",
      reachable: true,
      tested_at: new Date().toISOString(),
    });
    render(<Diagnostics report={null} />);
    await userEvent.type(
      screen.getByLabelText("Test IP or domain"),
      "192.168.1.1",
    );
    await userEvent.click(screen.getByRole("button", { name: "Test flow" }));
    await screen.findByText(/192\.168\.1\.1 → DIRECT/);

    expect(screen.queryByRole("button", { name: /to VPN/ })).toBeNull();
    expect(screen.getByText(/always stay direct/)).toBeVisible();
  });

  it("offers Add to VPN when a custom rule is what made it direct", async () => {
    const { desktop } = await import("../api/desktop");
    vi.mocked(desktop.testRoute).mockResolvedValueOnce({
      target: "example.ir",
      outbound: { kind: "direct" },
      reason: "custom_rule",
      matched_rule: "example.ir",
      reachable: true,
      tested_at: new Date().toISOString(),
    });
    render(<Diagnostics report={null} />);
    await userEvent.type(
      screen.getByLabelText("Test IP or domain"),
      "example.ir",
    );
    await userEvent.click(screen.getByRole("button", { name: "Test flow" }));

    const status = await screen.findByRole("status");
    const picker = status.querySelector("select");
    expect(picker).not.toBeNull();
    expect(picker).toHaveValue("direct");
    expect(picker?.querySelector("option[value='direct']")).not.toBeNull();
  });

  it("restarts Hiddify on clean state and reports the backup", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<Diagnostics report={null} />);
    await userEvent.click(
      screen.getByRole("button", { name: /Fresh Hiddify start/ }),
    );

    expect(confirm).toHaveBeenCalledOnce();
    const status = await screen.findByRole("status");
    expect(status).toHaveTextContent(
      "Hiddify restarted on clean runtime state",
    );
    expect(status).toHaveTextContent("configs, data, app.log");
    expect(status).toHaveTextContent("db.sqlite, shared_preferences.json");
    expect(status).toHaveTextContent(
      "/home/user/.local/share/biflow/backups/hiddify-20260815",
    );
  });

  it("does not touch Hiddify when the confirmation is declined", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<Diagnostics report={null} />);
    await userEvent.click(
      screen.getByRole("button", { name: /Fresh Hiddify start/ }),
    );

    expect(confirm).toHaveBeenCalledOnce();
    const { desktop } = await import("../api/desktop");
    expect(desktop.freshHiddifyStart).not.toHaveBeenCalled();
  });

  it("requires confirmation before deleting the log", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<Diagnostics report={null} />);
    await screen.findByTestId("debug-log-size");
    await userEvent.click(screen.getByRole("button", { name: "Delete log" }));
    expect(confirm).toHaveBeenCalledOnce();
    const { desktop } = await import("../api/desktop");
    expect(desktop.deleteDebugLog).not.toHaveBeenCalled();
  });

  it("shows live DIRECT and VPN connections while the stack is running", async () => {
    vi.mocked(desktop.listActiveConnections).mockResolvedValue([
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
    useAppStore.setState({
      settings: baseSettings(),
      snapshot: baseSnapshot({
        phase: "running",
        helper: { phase: "running", message: null, since: "now" },
        clients: [
          {
            id: MOCK_HIDDIFY_ID,
            preset: "hiddify",
            enabled: true,
            status: { phase: "running", message: null, since: "now" },
            exit_ip: null,
          },
        ],
        mihomo: { phase: "running", message: null, since: "now" },
        tun: { phase: "running", message: null, since: "now" },
        dns: { phase: "running", message: null, since: "now" },
        providers: {
          ready: 6,
          total: 6,
          rules_loaded: 12,
          last_refresh: null,
        },
        exit_ip: "203.0.113.42",
      }),
    });
    render(<Diagnostics report={null} />);
    expect(
      await screen.findByRole("heading", { name: "Live connections" }),
    ).toBeVisible();
    expect(await screen.findByText("digikala.ir")).toBeVisible();
    expect(screen.getByText("openai.com")).toBeVisible();
    // Route badges; the actions column also renders DIRECT/VPN as the
    // switch-route button label, so scope to the badge spans.
    const direct = screen
      .getAllByRole("cell", { name: "DIRECT" })
      .filter((cell) => cell.querySelector("span"));
    const vpn = screen
      .getAllByRole("cell", { name: "Hiddify" })
      .filter((cell) => cell.querySelector("span"));
    expect(direct).toHaveLength(1);
    expect(vpn).toHaveLength(1);
    expect(
      screen.getByTestId("live-connections").querySelectorAll("select").length,
    ).toBeGreaterThanOrEqual(3);
  });

  it("filters live connections by route and rule and searches host or IP", async () => {
    vi.mocked(desktop.listActiveConnections).mockResolvedValue([
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
    useAppStore.setState({
      settings: baseSettings(),
      snapshot: baseSnapshot({
        phase: "running",
        helper: { phase: "running", message: null, since: "now" },
        clients: [
          {
            id: MOCK_HIDDIFY_ID,
            preset: "hiddify",
            enabled: true,
            status: { phase: "running", message: null, since: "now" },
            exit_ip: null,
          },
        ],
        mihomo: { phase: "running", message: null, since: "now" },
        tun: { phase: "running", message: null, since: "now" },
        dns: { phase: "running", message: null, since: "now" },
        providers: {
          ready: 6,
          total: 6,
          rules_loaded: 12,
          last_refresh: null,
        },
        exit_ip: "203.0.113.42",
      }),
    });
    render(<Diagnostics report={null} />);
    await screen.findByText("digikala.ir");

    // Route filter narrows to VPN rows only.
    await userEvent.selectOptions(
      screen.getByLabelText("Route"),
      MOCK_HIDDIFY_ID,
    );
    expect(screen.queryByText("digikala.ir")).toBeNull();
    expect(screen.getByText("openai.com")).toBeVisible();
    await userEvent.selectOptions(screen.getByLabelText("Route"), "all");

    // Rule filter offers the observed rules and narrows to one of them.
    await userEvent.selectOptions(
      screen.getByLabelText("Matched rule"),
      "iran-domains",
    );
    expect(screen.getByText("digikala.ir")).toBeVisible();
    expect(screen.queryByText("openai.com")).toBeNull();
    await userEvent.selectOptions(screen.getByLabelText("Matched rule"), "all");

    // Search matches destination IPs as well as hosts.
    const search = screen.getByLabelText("Search host or IP");
    await userEvent.type(search, "104.18");
    expect(screen.getByText("openai.com")).toBeVisible();
    expect(screen.queryByText("digikala.ir")).toBeNull();

    await userEvent.clear(search);
    await userEvent.type(search, "no-such-host");
    expect(screen.getByText("No connections match this filter.")).toBeVisible();
  });

  it("shows a reachability row per fixed probe domain", async () => {
    render(<Diagnostics report={null} />);
    expect(
      await screen.findByRole("heading", { name: "Reachability" }),
    ).toBeVisible();
    expect(await screen.findByText("google.com")).toBeVisible();
    expect(screen.getByText("facebook.com")).toBeVisible();
    expect(screen.getByText("iran.ir")).toBeVisible();
    expect(screen.getByText("Unreachable")).toBeVisible();
    expect(screen.getByText("Slow")).toBeVisible();
    expect(screen.getByText("Reachable")).toBeVisible();
    // Only degraded rows invite a click for causes; the green row stays plain.
    expect(screen.getAllByTitle("Click for likely causes")).toHaveLength(2);
  });

  it("opens likely causes for an unreachable domain and retries", async () => {
    render(<Diagnostics report={null} />);
    await screen.findByText("google.com");
    const googleRow = screen.getAllByTitle("Click for likely causes")[0];
    if (!googleRow) throw new Error("expected a clickable reachability row");
    await userEvent.click(googleRow);

    const dialog = await screen.findByRole("dialog");
    // google is VPN-path and was probed through the proxy, so the causes
    // point at the Hiddify node rather than at being disconnected.
    expect(dialog).toHaveTextContent("Switch to a different node in Hiddify");
    expect(dialog).toHaveTextContent("tls closed");

    await userEvent.click(screen.getByRole("button", { name: /Try again/ }));
    expect(desktop.checkReachability).toHaveBeenCalledTimes(2);
  });

  it("explains an unreachable VPN domain as expected while disconnected", async () => {
    vi.mocked(desktop.checkReachability).mockResolvedValueOnce([
      {
        id: "google",
        domain: "google.com",
        path: "vpn",
        via_proxy: false,
        status: "unreachable",
        latency_ms: null,
        detail: null,
      },
    ]);
    render(<Diagnostics report={null} />);
    await userEvent.click(await screen.findByTitle("Click for likely causes"));
    expect(await screen.findByRole("dialog")).toHaveTextContent(
      "Press Connect first",
    );
  });
});
