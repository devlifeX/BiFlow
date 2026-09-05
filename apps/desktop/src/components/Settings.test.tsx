import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig } from "../api/models";
import { useAppStore } from "../store/app";
import { baseSettings } from "../test/fixtures";
import { Settings } from "./Settings";

vi.mock("../api/desktop", () => ({
  desktop: {
    validateSettings: vi.fn().mockResolvedValue([]),
  },
}));

const settings: AppConfig = baseSettings();

describe("Settings", () => {
  beforeEach(() => {
    useAppStore.setState({ actionPending: false });
  });

  it("lets the operator pick a DIRECT DNS preset including Mokhaberat", async () => {
    const saveSettings = vi
      .fn<(draft: AppConfig) => Promise<void>>()
      .mockResolvedValue(undefined);
    useAppStore.setState({ saveSettings });
    render(<Settings settings={settings} />);
    await userEvent.click(screen.getByRole("tab", { name: "Mihomo" }));
    const dns = screen.getByLabelText("DIRECT DNS");
    expect(dns).toHaveValue("fake_ip");
    expect(screen.getByRole("option", { name: "Fake-ip" })).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /Mokhaberat \(5\.200\.200\.200\)/ }),
    ).toBeInTheDocument();
    await userEvent.selectOptions(dns, "custom");
    expect(screen.getByLabelText("Custom resolvers")).toBeVisible();
    await userEvent.selectOptions(dns, "mokhaberat");
    expect(screen.queryByLabelText("Custom resolvers")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Save settings" }),
    );
    expect(saveSettings.mock.calls[0]?.[0].mihomo.direct_dns_preset).toBe(
      "mokhaberat",
    );
  });
});
