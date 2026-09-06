import { render, screen } from "@testing-library/react";
import { Power } from "lucide-react";
import { describe, expect, it } from "vitest";
import { baseSnapshot } from "../test/fixtures";
import {
  CONNECTION_BUTTON_HEIGHT_CLASS,
  CONNECTION_BUTTON_ICON_PX,
  CONNECTION_BUTTON_WIDTH_CLASS,
  ConnectionActionButton,
} from "./ConnectionActionButton";

const now = new Date().toISOString();
const stopped = baseSnapshot({ updated_at: now });

describe("ConnectionActionButton", () => {
  it("renders a fixed 128x30 connect button with nowrap label", () => {
    render(
      <ConnectionActionButton
        action="connect"
        snapshot={stopped}
        disabled={false}
        onClick={() => undefined}
        icon={<Power size={CONNECTION_BUTTON_ICON_PX} aria-hidden />}
        variant="primary"
      />,
    );
    const button = screen.getByRole("button", { name: "Connect" });
    expect(button).toHaveAttribute("data-progress", "0");
    expect(button).toHaveAttribute("data-processing", "false");
    expect(button).toHaveAttribute("data-connect-glow", "available");
    expect(button.className).toContain(CONNECTION_BUTTON_WIDTH_CLASS);
    expect(button.className).toContain(CONNECTION_BUTTON_HEIGHT_CLASS);
    const label = button.querySelector(".connection-action-label");
    expect(label?.className).toMatch(/truncate/);
    expect(label?.className).toMatch(/whitespace-nowrap/);
    expect(
      button.querySelector(".connection-action-countdown"),
    ).toHaveTextContent("60s");
  });

  it("shows the current stage and fill while processing", () => {
    render(
      <ConnectionActionButton
        action="connect"
        snapshot={{
          ...stopped,
          phase: "starting_core",
          busy: "connecting",
          operation_stage: "starting_core",
          operation_id: "op-1",
        }}
        disabled
        onClick={() => undefined}
        icon={<Power size={CONNECTION_BUTTON_ICON_PX} aria-hidden />}
        variant="primary"
      />,
    );
    const button = screen.getByRole("button", { name: /^Start Mihomo/ });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("data-progress", "70");
    expect(button).toHaveAttribute("aria-busy", "true");
    expect(button).toHaveAttribute("data-connect-glow", "off");
    expect(button.className).toMatch(/connection-action-processing/);
    expect(button.className).not.toMatch(/connect-button-glow/);
    const fill = button.querySelector(".connection-action-fill");
    expect(fill).toHaveStyle({ width: "70%" });
  });
});
