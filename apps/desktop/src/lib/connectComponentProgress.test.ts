import { describe, expect, it } from "vitest";
import type { ComponentPhase, StackSnapshot } from "../api/models";

function sawRunningBeforeStack(
  snapshots: StackSnapshot[],
  pick: (snapshot: StackSnapshot) => ComponentPhase,
): boolean {
  return snapshots.some(
    (snapshot) => snapshot.phase !== "running" && pick(snapshot) === "running",
  );
}

describe("connect component progress contract", () => {
  it("expects helper, client, mihomo, and tun to reach running before stack running", () => {
    const snapshots: StackSnapshot[] = [
      {
        phase: "starting_client",
        helper: { phase: "running", message: null, since: "" },
        clients: [
          {
            id: "11111111-1111-1111-1111-111111111111",
            preset: "hiddify",
            enabled: true,
            status: { phase: "starting", message: null, since: "" },
            exit_ip: null,
          },
        ],
        mihomo: { phase: "stopped", message: null, since: "" },
        tun: { phase: "stopped", message: null, since: "" },
        dns: { phase: "stopped", message: null, since: "" },
      } as StackSnapshot,
      {
        phase: "starting_client",
        helper: { phase: "running", message: null, since: "" },
        clients: [
          {
            id: "11111111-1111-1111-1111-111111111111",
            preset: "hiddify",
            enabled: true,
            status: { phase: "running", message: null, since: "" },
            exit_ip: null,
          },
        ],
        mihomo: { phase: "stopped", message: null, since: "" },
        tun: { phase: "stopped", message: null, since: "" },
        dns: { phase: "stopped", message: null, since: "" },
      } as StackSnapshot,
      {
        phase: "checking_readiness",
        helper: { phase: "running", message: null, since: "" },
        clients: [
          {
            id: "11111111-1111-1111-1111-111111111111",
            preset: "hiddify",
            enabled: true,
            status: { phase: "running", message: null, since: "" },
            exit_ip: null,
          },
        ],
        mihomo: { phase: "running", message: null, since: "" },
        tun: { phase: "running", message: null, since: "" },
        dns: { phase: "running", message: null, since: "" },
      } as StackSnapshot,
      {
        phase: "running",
        helper: { phase: "running", message: null, since: "" },
        clients: [
          {
            id: "11111111-1111-1111-1111-111111111111",
            preset: "hiddify",
            enabled: true,
            status: { phase: "running", message: null, since: "" },
            exit_ip: null,
          },
        ],
        mihomo: { phase: "running", message: null, since: "" },
        tun: { phase: "running", message: null, since: "" },
        dns: { phase: "running", message: null, since: "" },
      } as StackSnapshot,
    ];

    expect(
      sawRunningBeforeStack(snapshots, (snapshot) => snapshot.helper.phase),
    ).toBe(true);
    expect(
      sawRunningBeforeStack(
        snapshots,
        (snapshot) => snapshot.clients[0]?.status.phase ?? "unknown",
      ),
    ).toBe(true);
    expect(
      sawRunningBeforeStack(snapshots, (snapshot) => snapshot.mihomo.phase),
    ).toBe(true);
    expect(
      sawRunningBeforeStack(snapshots, (snapshot) => snapshot.tun.phase),
    ).toBe(true);
  });
});
