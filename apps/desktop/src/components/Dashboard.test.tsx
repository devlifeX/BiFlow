import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { baseSnapshot } from "../test/fixtures";
import { useAppStore } from "../store/app";
import { Dashboard } from "./Dashboard";

vi.mock("../api/desktop", () => ({
  desktop: {
    start: vi
      .fn()
      .mockResolvedValue({ operation_id: "op", already_complete: false }),
    stop: vi
      .fn()
      .mockResolvedValue({ operation_id: "op", already_complete: false }),
    pause: vi
      .fn()
      .mockResolvedValue({ operation_id: "op", already_complete: false }),
    resume: vi
      .fn()
      .mockResolvedValue({ operation_id: "op", already_complete: false }),
    cancel: vi.fn().mockResolvedValue(true),
    installHelper: vi.fn().mockResolvedValue({ installed: true }),
    installDependency: vi.fn(),
    listDependencies: vi.fn(),
    getSnapshot: vi.fn(),
  },
}));

const now = new Date().toISOString();
const stopped = baseSnapshot({ updated_at: now });

describe("Dashboard", () => {
  it("shows real component state and starts without blocking the UI", async () => {
    useAppStore.setState({ snapshot: stopped, actionPending: false });
    render(<Dashboard snapshot={stopped} />);
    expect(screen.getAllByText("stopped")).toHaveLength(5);
    const connect = screen.getByRole("button", { name: "Connect" });
    expect(connect.querySelector("svg")).not.toBeNull();
    expect(connect).toHaveAttribute("data-connect-glow", "available");
    await userEvent.click(connect);
    expect(useAppStore.getState().actionPending).toBe(true);
  });

  it("disables every lifecycle control during a transition", () => {
    const { rerender } = render(
      <Dashboard
        snapshot={{
          ...stopped,
          phase: "starting_client",
          busy: "connecting",
          operation_stage: "starting_client",
          operation_id: "operation-1",
        }}
      />,
    );
    const connecting = screen.getByRole("button", { name: "Start client" });
    expect(connecting).toBeDisabled();
    expect(connecting).toHaveAttribute("data-progress", "25");
    expect(connecting).toHaveAttribute("data-connect-glow", "off");
    expect(
      screen.getByRole("button", { name: "Cancel operation" }),
    ).toBeEnabled();
    expect(screen.queryByText("%")).toBeNull();

    const running = {
      phase: "running" as const,
      message: "Ready",
      since: now,
    };
    rerender(
      <Dashboard
        snapshot={{
          ...stopped,
          phase: "running",
          busy: "pausing",
          helper: running,
          clients: stopped.clients.map((client) => ({
            ...client,
            status: running,
          })),
          mihomo: running,
          tun: running,
          dns: running,
        }}
      />,
    );
    expect(screen.getByRole("button", { name: "Stop Mihomo" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeDisabled();
  });

  it("exposes cancellation and in-button progress during an operation", () => {
    render(
      <Dashboard
        snapshot={{
          ...stopped,
          phase: "starting_core",
          busy: "connecting",
          operation_stage: "starting_core",
          operation_id: "operation-1",
        }}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Cancel operation" }),
    ).toBeEnabled();
    const connect = screen.getByRole("button", { name: "Start Mihomo" });
    expect(connect).toBeDisabled();
    expect(connect).toHaveAttribute("data-progress", "70");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows install actions when Hiddify and Mihomo are missing", async () => {
    const install = vi.fn().mockResolvedValue(undefined);
    useAppStore.setState({
      snapshot: stopped,
      actionPending: false,
      installingId: null,
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
      installDependency: install,
    });
    render(<Dashboard snapshot={stopped} />);
    const buttons = screen.getAllByRole("button", { name: /^Install$/ });
    expect(buttons).toHaveLength(2);
    await userEvent.click(buttons[0]!);
    expect(install).toHaveBeenCalledWith("hiddify");
  });

  it("shows a helper install action when the helper is unavailable", async () => {
    const installHelper = vi.fn().mockResolvedValue(undefined);
    const unavailable = {
      ...stopped,
      helper: {
        phase: "unavailable" as const,
        message: "Helper service is not installed or running",
        since: now,
      },
    };
    useAppStore.setState({
      snapshot: unavailable,
      actionPending: false,
      installingId: null,
      dependencies: [
        {
          id: "hiddify",
          name: "Hiddify",
          installed: true,
          version: "1",
          path: "/tmp/hiddify",
        },
        {
          id: "mihomo",
          name: "Mihomo",
          installed: true,
          version: "1",
          path: "/tmp/mihomo",
        },
      ],
      installHelper,
    });
    render(<Dashboard snapshot={unavailable} />);
    await userEvent.click(screen.getByRole("button", { name: /^Install$/ }));
    expect(installHelper).toHaveBeenCalledOnce();
  });

  it("hides install actions when Hiddify and Mihomo are already installed", () => {
    useAppStore.setState({
      snapshot: stopped,
      actionPending: false,
      installingId: null,
      dependencies: [
        {
          id: "hiddify",
          name: "Hiddify",
          installed: true,
          version: "4.1.1",
          path: "/usr/bin/hiddify",
        },
        {
          id: "mihomo",
          name: "Mihomo",
          installed: true,
          version: "1.19.29",
          path: "/usr/bin/mihomo",
        },
      ],
    });
    render(<Dashboard snapshot={stopped} />);
    expect(screen.queryByRole("button", { name: /^Install$/ })).toBeNull();
    expect(screen.getAllByText("stopped")).toHaveLength(5);
  });

  it("shows animated direct and VPN routes only while connected", () => {
    const running = {
      phase: "running" as const,
      message: "Ready",
      since: now,
    };
    const { rerender } = render(
      <Dashboard
        snapshot={{
          ...stopped,
          phase: "running",
          helper: running,
          clients: stopped.clients.map((client) => ({
            ...client,
            status: running,
          })),
          mihomo: running,
          tun: running,
          dns: running,
        }}
      />,
    );

    expect(
      screen.getByRole("img", {
        name: /traffic leaving this device and splitting/i,
      }),
    ).toBeVisible();

    rerender(<Dashboard snapshot={stopped} />);
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("shows Pause while running and Resume while paused", async () => {
    const running = {
      phase: "running" as const,
      message: "Ready",
      since: now,
    };
    useAppStore.setState({ snapshot: stopped, actionPending: false });
    const { rerender } = render(
      <Dashboard
        snapshot={{
          ...stopped,
          phase: "running",
          helper: running,
          clients: stopped.clients.map((client) => ({
            ...client,
            status: running,
          })),
          mihomo: running,
          tun: running,
          dns: running,
        }}
      />,
    );
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeEnabled();
    rerender(
      <Dashboard
        snapshot={{
          ...stopped,
          phase: "paused",
          clients: stopped.clients.map((client) => ({
            ...client,
            status: running,
          })),
        }}
      />,
    );
    expect(screen.getByRole("button", { name: "Resume" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(useAppStore.getState().actionPending).toBe(true);
  });

  it("lets metric values wrap instead of clipping on narrow columns", () => {
    render(<Dashboard snapshot={stopped} />);
    const exitIp = screen.getByText("Available after connection");
    expect(exitIp.className).toMatch(/break-words/);
    expect(exitIp.className).not.toMatch(/truncate/);
  });

  it("lets the shell scroll overflowing dashboard content", () => {
    const { container } = render(<Dashboard snapshot={stopped} />);
    expect(container.querySelector("section")?.className).not.toMatch(
      /overflow-y-auto/,
    );
    expect(container.querySelector("section")?.className).toMatch(/pb-2/);
  });

  it("renders compact mobile status and provider summaries", () => {
    render(<Dashboard snapshot={stopped} />);
    expect(screen.getByTestId("connection-status-strip")).toBeInTheDocument();
    expect(screen.getByTestId("provider-summary")).toBeInTheDocument();
    expect(
      screen
        .getByTestId("connection-status-strip")
        .querySelectorAll("[data-status-light]"),
    ).toHaveLength(5);
  });
});
