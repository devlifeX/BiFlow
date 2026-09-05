import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { baseSettings } from "../test/fixtures";
import { useAppStore } from "../store/app";
import { SettingsApplyBanner } from "./SettingsApplyBanner";

describe("SettingsApplyBanner", () => {
  beforeEach(() => {
    useAppStore.setState({
      actionPending: false,
      settingsApplyNotice: { previous: baseSettings() },
      applyPendingSettings: vi.fn(),
      revertPendingSettings: vi.fn(),
      dismissSettingsApplyNotice: vi.fn(),
    });
  });

  it("offers restart, revert, and dismiss", async () => {
    const applyPendingSettings = vi.fn();
    const revertPendingSettings = vi.fn();
    const dismissSettingsApplyNotice = vi.fn();
    useAppStore.setState({
      applyPendingSettings,
      revertPendingSettings,
      dismissSettingsApplyNotice,
    });
    render(<SettingsApplyBanner />);
    expect(
      screen.getByText(/Restart Mihomo to apply these settings/),
    ).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Restart Mihomo" }),
    );
    expect(applyPendingSettings).toHaveBeenCalledOnce();
    await userEvent.click(
      screen.getByRole("button", { name: "Revert changes" }),
    );
    expect(revertPendingSettings).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(dismissSettingsApplyNotice).toHaveBeenCalledOnce();
  });

  it("renders nothing when there is no pending apply", () => {
    useAppStore.setState({ settingsApplyNotice: null });
    const { container } = render(<SettingsApplyBanner />);
    expect(container).toBeEmptyDOMElement();
  });
});
