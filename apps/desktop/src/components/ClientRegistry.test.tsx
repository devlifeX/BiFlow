import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { desktop } from "../api/desktop";
import type { BootstrapResult } from "../api/models";
import { createClientInstance } from "../lib/clients";
import { baseSettings, baseSnapshot } from "../test/fixtures";
import { useAppStore } from "../store/app";
import { ClientRegistry } from "./ClientRegistry";

async function openDetails(card: HTMLElement) {
  await userEvent.click(within(card).getByText("Settings & pinned hosts"));
}

vi.mock("../api/desktop", () => ({
  desktop: {
    pickProfileFile: vi.fn(),
    openUrl: vi.fn().mockResolvedValue(undefined),
    clientBinaryInstalled: vi.fn().mockResolvedValue(true),
  },
}));

const pickProfileFile = vi.mocked(desktop.pickProfileFile);

beforeEach(() => {
  pickProfileFile.mockReset();
  pickProfileFile.mockResolvedValue("/tmp/office.ovpn");
  vi.mocked(desktop.clientBinaryInstalled).mockReset();
  vi.mocked(desktop.clientBinaryInstalled).mockResolvedValue(true);
  vi.mocked(desktop.openUrl).mockClear();
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
      await openDetails(card);
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
    await openDetails(card);
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

  it("lets the user type a Windscribe password and commits it on blur", async () => {
    const updateClient = vi.fn();
    useAppStore.setState({ updateClient });
    render(<ClientRegistry />);

    const card = screen.getByTestId("client-card-windscribe");
    await openDetails(card);
    const password = within(card).getByLabelText("Password (optional)");
    await userEvent.type(password, "hunter2secret");
    // Typing must stay local: a per-keystroke save echoes the redacted
    // config back and erases the field.
    expect(password).toHaveValue("hunter2secret");
    expect(updateClient).not.toHaveBeenCalled();
    await userEvent.tab();
    expect(updateClient).toHaveBeenCalledTimes(1);
    expect(updateClient).toHaveBeenCalledWith(
      "22222222-2222-2222-2222-222222222222",
      {
        kind: "owned_side_tunnel",
        profile_path: null,
        executable: "auto",
        username: null,
        password: "hunter2secret",
        start_timeout_seconds: 45,
      },
    );
  });

  it("keeps typing usable when the saved password is redacted", async () => {
    const updateClient = vi.fn();
    const windscribe = createClientInstance(
      "windscribe",
      "22222222-2222-2222-2222-222222222222",
    );
    if (windscribe.config.kind === "owned_side_tunnel") {
      windscribe.config.password = "[REDACTED]";
    }
    useAppStore.setState({
      updateClient,
      settings: { ...baseSettings(), clients: [windscribe] },
    });
    render(<ClientRegistry />);

    const card = screen.getByTestId("client-card-windscribe");
    await openDetails(card);
    const password = within(card).getByLabelText("Password (optional)");
    expect(password).toHaveAttribute("placeholder", "••••••••");
    await userEvent.type(password, "new-secret");
    expect(password).toHaveValue("new-secret");
    await userEvent.tab();
    expect(updateClient).toHaveBeenCalledTimes(1);
    expect(updateClient).toHaveBeenLastCalledWith(
      expect.anything(),
      expect.objectContaining({ password: "new-secret" }),
    );
  });

  it("commits username and port edits on blur, rejecting invalid ports", async () => {
    const updateClient = vi.fn();
    useAppStore.setState({ updateClient });
    render(<ClientRegistry />);

    const card = screen.getByTestId("client-card-windscribe");
    await openDetails(card);
    const username = within(card).getByLabelText("Username (optional)");
    await userEvent.type(username, "user@example.com");
    expect(updateClient).not.toHaveBeenCalled();
    await userEvent.tab();
    expect(updateClient).toHaveBeenCalledTimes(1);
    expect(updateClient).toHaveBeenLastCalledWith(
      expect.anything(),
      expect.objectContaining({ username: "user@example.com" }),
    );

    const hiddify = screen.getByTestId("client-card-hiddify");
    await openDetails(hiddify);
    const port = within(hiddify).getByLabelText("Local port");
    await userEvent.clear(port);
    await userEvent.type(port, "99999");
    await userEvent.tab();
    // 99999 is out of range: the draft is discarded, nothing is saved.
    expect(updateClient).toHaveBeenCalledTimes(1);
    await userEvent.clear(port);
    await userEvent.type(port, "2080");
    await userEvent.tab();
    expect(updateClient).toHaveBeenCalledTimes(2);
    expect(updateClient).toHaveBeenLastCalledWith(
      expect.anything(),
      expect.objectContaining({ port: 2080 }),
    );
  });

  it("offers OpenVPN download on the Windscribe catalog and missing-binary banner", async () => {
    vi.mocked(desktop.clientBinaryInstalled).mockResolvedValue(false);
    render(<ClientRegistry />);

    await userEvent.click(screen.getByRole("button", { name: "Add client" }));
    const row = screen.getByTestId("client-catalog-windscribe");
    expect(
      within(row).getByRole("button", { name: "Download OpenVPN" }),
    ).toBeVisible();
    expect(
      within(row).getByRole("button", { name: "Get Windscribe config" }),
    ).toBeVisible();
    await userEvent.click(
      within(row).getByRole("button", { name: "Download OpenVPN" }),
    );
    expect(desktop.openUrl).toHaveBeenCalledWith(
      "https://openvpn.net/community-downloads/",
    );

    const card = screen.getByTestId("client-card-windscribe");
    expect(
      await within(card).findByText("OpenVPN is not installed on this system."),
    ).toBeVisible();
    const missingBanner = within(card)
      .getByText("OpenVPN is not installed on this system.")
      .closest("p");
    if (!missingBanner) throw new Error("expected the OpenVPN missing banner");
    await userEvent.click(
      within(missingBanner).getByRole("button", { name: "Download OpenVPN" }),
    );
    expect(desktop.openUrl).toHaveBeenLastCalledWith(
      "https://openvpn.net/community-downloads/",
    );
  });

  it("leaves the path unchanged when the picker is cancelled", async () => {
    const updateClient = vi.fn();
    pickProfileFile.mockResolvedValueOnce(null);
    useAppStore.setState({ updateClient });
    render(<ClientRegistry />);

    const card = screen.getByTestId("client-card-openvpn");
    await openDetails(card);
    await userEvent.click(
      within(card).getByRole("button", { name: "Choose file" }),
    );
    expect(updateClient).not.toHaveBeenCalled();
  });
});
