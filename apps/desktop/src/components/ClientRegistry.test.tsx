import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { desktop } from "../api/desktop";
import type { BootstrapResult } from "../api/models";
import { createClientInstance } from "../lib/clients";
import { baseSettings, baseSnapshot } from "../test/fixtures";
import { useAppStore } from "../store/app";
import { ClientRegistry } from "./ClientRegistry";

vi.mock("../api/desktop", () => ({
  desktop: {
    pickProfileFile: vi.fn(),
    openUrl: vi.fn().mockResolvedValue(undefined),
  },
}));

const pickProfileFile = vi.mocked(desktop.pickProfileFile);

beforeEach(() => {
  pickProfileFile.mockReset();
  pickProfileFile.mockResolvedValue("/tmp/office.ovpn");
  const openvpn = createClientInstance(
    "openvpn",
    "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
  );
  const windscribe = createClientInstance(
    "windscribe",
    "22222222-2222-2222-2222-222222222222",
  );
  useAppStore.setState({
    settings: {
      ...baseSettings(),
      clients: [...baseSettings().clients, openvpn, windscribe],
    },
    rules: { revision: 0, pins: [], lists: [] },
    snapshot: baseSnapshot(),
    actionPending: false,
    boot: { platform: "linux" } as BootstrapResult,
    updateClient: vi.fn(),
    addClient: vi.fn(),
    deleteClient: vi.fn(),
    setClientEnabled: vi.fn(),
    setClientAllowDirectWhenDown: vi.fn(),
    setDefaultRoute: vi.fn(),
    pinRoute: vi.fn(),
    removeRule: vi.fn(),
    routeFallbackNotice: null,
    clearRouteFallbackNotice: vi.fn(),
  });
});

describe("ClientRegistry profile picker", () => {
  it("replaces the profile text box with a file picker on OpenVPN and Windscribe", async () => {
    const updateClient = vi.fn();
    useAppStore.setState({ updateClient });
    render(<ClientRegistry />);

    expect(screen.queryByPlaceholderText("profile.ovpn")).toBeNull();

    for (const preset of ["openvpn", "windscribe"] as const) {
      const card = screen.getByTestId(`client-card-${preset}`);
      expect(within(card).getByText("No file chosen")).toBeVisible();
      await userEvent.click(
        within(card).getByRole("button", { name: "Choose file" }),
      );
    }

    expect(pickProfileFile).toHaveBeenCalledTimes(2);
    expect(updateClient).toHaveBeenCalledTimes(2);
    expect(updateClient).toHaveBeenNthCalledWith(
      1,
      "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      {
        kind: "owned_side_tunnel",
        profile_path: "/tmp/office.ovpn",
        executable: "auto",
        username: null,
        password: null,
        start_timeout_seconds: 45,
      },
    );
    expect(updateClient).toHaveBeenNthCalledWith(
      2,
      "22222222-2222-2222-2222-222222222222",
      {
        kind: "owned_side_tunnel",
        profile_path: "/tmp/office.ovpn",
        executable: "auto",
        username: null,
        password: null,
        start_timeout_seconds: 45,
      },
    );
  });

  it("shows the chosen file name and can clear it", async () => {
    const updateClient = vi.fn();
    const openvpn = createClientInstance(
      "openvpn",
      "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    );
    if (openvpn.config.kind === "owned_side_tunnel") {
      openvpn.config.profile_path = "C:\\Users\\a\\windscribe.ovpn";
    }
    useAppStore.setState({
      updateClient,
      settings: {
        ...baseSettings(),
        clients: [openvpn],
      },
    });
    render(<ClientRegistry />);

    const card = screen.getByTestId("client-card-openvpn");
    expect(within(card).getByTestId("profile-file-name")).toHaveTextContent(
      "windscribe.ovpn",
    );
    await userEvent.click(
      within(card).getByRole("button", { name: "Clear file" }),
    );
    expect(updateClient).toHaveBeenCalledWith(
      "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      {
        kind: "owned_side_tunnel",
        profile_path: null,
        executable: "auto",
        username: null,
        password: null,
        start_timeout_seconds: 45,
      },
    );
  });

  it("leaves the path unchanged when the picker is cancelled", async () => {
    const updateClient = vi.fn();
    pickProfileFile.mockResolvedValueOnce(null);
    useAppStore.setState({ updateClient });
    render(<ClientRegistry />);

    await userEvent.click(
      within(screen.getByTestId("client-card-openvpn")).getByRole("button", {
        name: "Choose file",
      }),
    );
    expect(updateClient).not.toHaveBeenCalled();
  });
});
