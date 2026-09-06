import { describe, expect, it } from "vitest";
import type { StackSnapshot } from "../api/models";
import { baseSnapshot } from "../test/fixtures";
import {
  CONNECTION_ACTION_LABEL_KEYS,
  connectionButtonProgress,
  longestConnectionActionLabel,
  resolveOperationStage,
} from "./connectionProgress";

const now = new Date().toISOString();

const base = (overrides: Partial<StackSnapshot> = {}): StackSnapshot =>
  baseSnapshot({ updated_at: now, ...overrides });

describe("connectionButtonProgress", () => {
  it("keeps idle labels until a matching operation starts", () => {
    const snapshot = base();
    expect(connectionButtonProgress(snapshot, "connect")).toEqual({
      labelKey: "connect",
      percent: 0,
      processing: false,
    });
    expect(connectionButtonProgress(snapshot, "disconnect").processing).toBe(
      false,
    );
  });

  it("follows Connect stages from backend milestones", () => {
    const start = base({
      busy: "connecting",
      operation_stage: "starting_client",
      phase: "starting_client",
      operation_id: "op-1",
    });
    expect(connectionButtonProgress(start, "connect")).toEqual({
      labelKey: "stages.startClient",
      percent: 25,
      processing: true,
    });
    expect(connectionButtonProgress(start, "disconnect").processing).toBe(
      false,
    );

    const mihomo = {
      ...start,
      phase: "starting_core" as const,
      operation_stage: "starting_core" as const,
    };
    expect(connectionButtonProgress(mihomo, "connect")).toEqual({
      labelKey: "stages.startMihomo",
      percent: 70,
      processing: true,
    });
  });

  it("uses install milestones before the stack start stages", () => {
    expect(
      resolveOperationStage(base({ busy: "connecting" }), "hiddify"),
    ).toEqual({
      percent: 16,
      labelKey: "stages.installHiddify",
    });
  });

  it("maps Disconnect and Pause to their stop stages", () => {
    const disconnecting = base({
      phase: "stopping",
      busy: "disconnecting",
      operation_stage: "stopping_proxy",
      clients: base().clients.map((client) => ({
        ...client,
        status: { phase: "running", message: null, since: now },
      })),
    });
    expect(connectionButtonProgress(disconnecting, "disconnect")).toEqual({
      labelKey: "stages.stopHiddify",
      percent: 65,
      processing: true,
    });

    const pausing = base({
      phase: "stopping",
      busy: "pausing",
      operation_stage: "stopping_core",
      mihomo: { phase: "running", message: null, since: now },
    });
    expect(connectionButtonProgress(pausing, "pause")).toEqual({
      labelKey: "stages.stopMihomo",
      percent: 35,
      processing: true,
    });
  });

  it("fills to 100% on the last published stage before idle", () => {
    const ready = base({
      phase: "checking_readiness",
      busy: "resuming",
      operation_stage: "checking_readiness",
    });
    expect(connectionButtonProgress(ready, "resume").percent).toBe(85);
  });

  it("shows an optimistic preparing fill before the first snapshot", () => {
    expect(
      connectionButtonProgress(base(), "connect", null, true),
    ).toMatchObject({
      labelKey: "stages.preparing",
      processing: true,
    });
  });
});

describe("longestConnectionActionLabel", () => {
  it("uses stage labels that are longer than idle connect", () => {
    const labels = Object.fromEntries(
      CONNECTION_ACTION_LABEL_KEYS.map((key) => [
        key,
        key === "stages.checkReadiness" ? "Check readiness" : "Connect",
      ]),
    ) as Record<string, string>;
    const translate = (key: string) => labels[key] ?? key;
    expect(longestConnectionActionLabel(translate)).toBe("Check readiness");
  });
});
