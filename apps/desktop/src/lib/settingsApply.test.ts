import { describe, expect, it } from "vitest";
import { baseSettings } from "../test/fixtures";
import {
  settingsAffectLiveMihomo,
  stackNeedsSettingsApply,
} from "./settingsApply";

describe("settingsAffectLiveMihomo", () => {
  it("ignores tray and launch-at-login toggles", () => {
    const before = baseSettings();
    const after = {
      ...before,
      behavior: { ...before.behavior, close_to_tray: false },
    };
    expect(settingsAffectLiveMihomo(before, after)).toBe(false);
  });

  it("flags client, DNS, and default-route edits", () => {
    const before = baseSettings();
    expect(
      settingsAffectLiveMihomo(before, {
        ...before,
        default_route: { kind: "direct" },
      }),
    ).toBe(true);
    expect(
      settingsAffectLiveMihomo(before, {
        ...before,
        mihomo: { ...before.mihomo, direct_dns_preset: "shecan" },
      }),
    ).toBe(true);
    expect(
      settingsAffectLiveMihomo(before, {
        ...before,
        behavior: { ...before.behavior, fail_closed: false },
      }),
    ).toBe(true);
  });
});

describe("stackNeedsSettingsApply", () => {
  it("is true only while Mihomo is live", () => {
    expect(stackNeedsSettingsApply("running")).toBe(true);
    expect(stackNeedsSettingsApply("degraded")).toBe(true);
    expect(stackNeedsSettingsApply("paused")).toBe(false);
    expect(stackNeedsSettingsApply("stopped")).toBe(false);
  });
});
