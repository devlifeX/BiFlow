import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "./App";
import { resetMockState } from "./api/mock";
import { UI_MODE_STORAGE_KEY } from "./lib/uiMode";
import { useAppStore } from "./store/app";
import { APP_VERSION } from "./version";

beforeEach(() => {
  resetMockState();
  localStorage.setItem(UI_MODE_STORAGE_KEY, "advanced");
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
    diagnostics: null,
    error: null,
    installGuide: null,
    settingsApplyNotice: null,
    pageTabs: {},
    toast: null,
  });
});

async function homeHeading() {
  return screen.findByRole("heading", { level: 1 });
}

describe("App", () => {
  it("boots BiFlow and walks the five sections and their tabs", async () => {
    render(<App />);
    expect(await homeHeading()).toHaveTextContent("Not connected");
    expect(screen.getByText("BiFlow")).toBeVisible();
    expect(screen.getByTestId("add-site")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: /Health/ }));
    expect(screen.getAllByRole("button", { name: /^Install$/ })).toHaveLength(
      2,
    );

    await userEvent.click(screen.getByRole("button", { name: "Routing" }));
    expect(screen.getByRole("heading", { name: "Routing" })).toBeVisible();
    await userEvent.click(screen.getByRole("tab", { name: "Iran rules" }));
    expect(
      screen.getByRole("button", { name: /update from cloud/i }),
    ).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Troubleshoot" }));
    expect(screen.getByRole("heading", { name: "Troubleshoot" })).toBeVisible();
    await userEvent.click(screen.getByRole("tab", { name: "Test" }));
    expect(screen.getByRole("button", { name: "Test flow" })).toBeDisabled();
    await userEvent.type(
      screen.getByLabelText("Test IP or domain"),
      "example.ir",
    );
    expect(screen.getByRole("button", { name: "Test flow" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("heading", { name: "Settings" })).toBeVisible();
    await userEvent.click(screen.getByRole("tab", { name: "About" }));
    expect(screen.getByText(APP_VERSION)).toBeVisible();
    expect(
      screen.getByText("Dariush Vesal · Omis Asgari · Reza Mahdavi"),
    ).toBeVisible();

    // Coming back to Routing keeps the tab the user left it on.
    await userEvent.click(screen.getByRole("button", { name: "Routing" }));
    expect(screen.getByRole("tab", { name: "Iran rules" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("opens the legacy About page as the Settings About tab", async () => {
    render(<App />);
    await homeHeading();
    useAppStore.getState().setPage("about");
    expect(
      await screen.findByRole("heading", { name: "Settings" }),
    ).toBeVisible();
    expect(screen.getByRole("tab", { name: "About" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("opens Basic mode on a first launch with no stored preference", async () => {
    localStorage.removeItem(UI_MODE_STORAGE_KEY);
    render(<App />);
    expect(await homeHeading()).toHaveTextContent("Not connected");
    expect(
      screen.queryByRole("button", { name: "Routing" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeVisible();
    expect(screen.getByTestId("add-site")).toBeVisible();
  });

  it("hides advanced chrome in Basic mode", async () => {
    render(<App />);
    await homeHeading();

    await userEvent.click(screen.getByRole("radio", { name: "Basic" }));
    expect(
      screen.queryByRole("button", { name: "Routing" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toBeVisible();
    expect(screen.getByRole("button", { name: "Connect" })).toBeVisible();
  });

  it("returns to the Basic home when Basic is picked from Settings", async () => {
    render(<App />);
    await homeHeading();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("heading", { name: "Settings" })).toBeVisible();
    await userEvent.click(screen.getByRole("radio", { name: "Basic" }));
    expect(
      screen.queryByRole("heading", { name: "Settings" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeVisible();
  });

  it("blocks the document context menu", () => {
    render(<App />);
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it("picks a profile file for OpenVPN instead of typing a path", async () => {
    render(<App />);
    await homeHeading();
    await userEvent.click(screen.getByRole("button", { name: "Clients" }));
    await userEvent.click(screen.getByRole("button", { name: "Add client" }));
    await userEvent.click(screen.getByRole("button", { name: /^OpenVPN/ }));
    const card = await screen.findByTestId("client-card-openvpn");
    await userEvent.click(within(card).getByText("Settings & pinned hosts"));
    expect(card.querySelector("input[placeholder='profile.ovpn']")).toBeNull();
    expect(screen.getByText("No file chosen")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Choose file" }));
    expect(screen.getByText("biflow-mock-profile.ovpn")).toBeVisible();
  });

  it("exposes the version file through bootstrap", async () => {
    render(<App />);
    await homeHeading();
    expect(APP_VERSION).toMatch(/^\d+\.\d+\.\d+$/);
  });
});
