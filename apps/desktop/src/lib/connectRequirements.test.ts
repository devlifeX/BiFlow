import { describe, expect, it } from "vitest";
import type { DependencyStatus, StackSnapshot } from "../api/models";
import { missingConnectRequirements } from "./connectRequirements";

const snapshot = (
  phase: StackSnapshot["helper"]["phase"],
  clients?: { preset: string; enabled: boolean }[],
): StackSnapshot =>
  ({
    helper: { phase, message: null, since: "now" },
    ...(clients
      ? {
          clients: clients.map((client, index) => ({
            id: `client-${index}`,
            preset: client.preset,
            enabled: client.enabled,
            status: { phase: "stopped", message: null, since: "now" },
            exit_ip: null,
          })),
        }
      : {}),
  }) as StackSnapshot;

const deps = (hiddify: boolean, mihomo: boolean): DependencyStatus[] => [
  {
    id: "hiddify",
    name: "Hiddify",
    installed: hiddify,
    version: null,
    path: null,
  },
  {
    id: "mihomo",
    name: "Mihomo",
    installed: mihomo,
    version: null,
    path: null,
  },
];

describe("missingConnectRequirements", () => {
  it("installs helper, Hiddify, then Mihomo in that order", () => {
    expect(
      missingConnectRequirements(snapshot("unavailable"), deps(false, false)),
    ).toEqual(["helper", "hiddify", "mihomo"]);
  });

  it("skips services that are already present", () => {
    expect(
      missingConnectRequirements(snapshot("running"), deps(true, true)),
    ).toEqual([]);
  });

  it("does not invent missing apps when the dependency list is empty", () => {
    expect(missingConnectRequirements(snapshot("running"), [])).toEqual([]);
  });

  it("does not demand Hiddify when the operator replaced it with Happ", () => {
    const happOnly = snapshot("running", [{ preset: "happ", enabled: true }]);
    expect(missingConnectRequirements(happOnly, deps(false, true))).toEqual([]);
  });

  it("still demands Hiddify while an enabled Hiddify client exists", () => {
    const withHiddify = snapshot("running", [
      { preset: "hiddify", enabled: true },
      { preset: "happ", enabled: true },
    ]);
    expect(missingConnectRequirements(withHiddify, deps(false, true))).toEqual([
      "hiddify",
    ]);
  });

  it("treats a disabled Hiddify client as removed", () => {
    const disabled = snapshot("running", [
      { preset: "hiddify", enabled: false },
      { preset: "happ", enabled: true },
    ]);
    expect(missingConnectRequirements(disabled, deps(false, true))).toEqual([]);
  });
});
