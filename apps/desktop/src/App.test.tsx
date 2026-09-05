import { render, screen } from "@testing-library/react";
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
  });
});

describe("App", () => {
  it("boots BiFlow and walks the primary screens", async () => {
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();
    expect(screen.getByText("BiFlow")).toBeVisible();
    expect(screen.getAllByRole("button", { name: /^Install$/ })).toHaveLength(
      2,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "List Management" }),
    );
    expect(
      screen.getByRole("heading", { name: "List Management" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: /update from cloud/i }),
    ).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Diagnostics" }));
    expect(screen.getByRole("heading", { name: "Diagnostics" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Test flow" })).toBeDisabled();
    await userEvent.type(
      screen.getByLabelText("Test IP or domain"),
      "example.ir",
    );
    expect(screen.getByRole("button", { name: "Test flow" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("heading", { name: "Settings" })).toBeVisible();

    await userEvent.click(screen.getByRole("button", { name: "About" }));
    expect(screen.getByRole("heading", { name: "About" })).toBeVisible();
    expect(screen.getByText(APP_VERSION)).toBeVisible();
    expect(screen.getByText("Dariush Vesal")).toBeVisible();
  });

  it("opens Basic mode on a first launch with no stored preference", async () => {
    localStorage.removeItem(UI_MODE_STORAGE_KEY);
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "List Management" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeVisible();
  });

  it("hides advanced chrome in Basic mode", async () => {
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();

    await userEvent.click(screen.getByRole("radio", { name: "Basic" }));
    expect(
      screen.queryByRole("button", { name: "List Management" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toBeVisible();
    expect(screen.getByRole("button", { name: "Connect" })).toBeVisible();
  });

  it("leaves About for the Basic dashboard when Basic is selected", async () => {
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "About" }));
    expect(screen.getByRole("heading", { name: "About" })).toBeVisible();
    await userEvent.click(screen.getByRole("radio", { name: "Basic" }));
    expect(
      screen.queryByRole("heading", { name: "About" }),
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

  it("exposes the version file through bootstrap", async () => {
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();
    expect(APP_VERSION).toMatch(/^\d+\.\d+\.\d+$/);
  });
});
