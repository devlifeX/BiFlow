import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { baseSnapshot } from "../test/fixtures";
import { BasicDashboard } from "./BasicDashboard";
import { LifecycleActionBar } from "./LifecycleActionBar";

const now = new Date().toISOString();
const stopped = baseSnapshot({ updated_at: now });

describe("BasicDashboard", () => {
  it("puts Connect progress on the button instead of a status card", () => {
    render(
      <>
        <BasicDashboard
          snapshot={{
            ...stopped,
            phase: "starting_client",
            busy: "connecting",
            operation_stage: "starting_client",
            operation_id: "op-1",
          }}
        />
        <LifecycleActionBar
          snapshot={{
            ...stopped,
            phase: "starting_client",
            busy: "connecting",
            operation_stage: "starting_client",
            operation_id: "op-1",
          }}
        />
      </>,
    );
    const connect = screen.getByRole("button", { name: /^Start client/ });
    expect(connect).toBeDisabled();
    expect(connect).toHaveAttribute("data-progress", "25");
    expect(connect).toHaveAttribute("data-connect-glow", "off");
    expect(screen.queryByText("%")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
  });
});
