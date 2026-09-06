import { describe, expect, it } from "vitest";
import { statusChipLabel } from "./statusChip";

describe("statusChipLabel", () => {
  it("maps runtime phases to fixed-width chip labels", () => {
    expect(statusChipLabel("stopped")).toBe("Idle");
    expect(statusChipLabel("starting")).toBe("Starting");
    expect(statusChipLabel("running")).toBe("Ready");
    expect(statusChipLabel("error")).toBe("Failed");
    expect(statusChipLabel("stopped", true)).toBe("Off");
  });
});
