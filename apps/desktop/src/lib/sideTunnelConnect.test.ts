import { describe, expect, it } from "vitest";
import { baseSettings, baseSnapshot } from "../test/fixtures";
import {
  failedSideTunnelClients,
  INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT,
  nextSideTunnelRetryTimeout,
  sideTunnelConnectTimeoutAt,
  SIDE_TUNNEL_CONNECT_TIMEOUTS,
} from "./sideTunnelConnect";
import { createClientInstance } from "./clients";

describe("sideTunnelConnect", () => {
  it("escalates connect timeouts 15 → 30 → 60", () => {
    expect(SIDE_TUNNEL_CONNECT_TIMEOUTS).toEqual([15, 30, 60]);
    expect(INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT).toBe(15);
    expect(sideTunnelConnectTimeoutAt(0)).toBe(15);
    expect(sideTunnelConnectTimeoutAt(1)).toBe(30);
    expect(sideTunnelConnectTimeoutAt(2)).toBe(60);
    expect(nextSideTunnelRetryTimeout(15)).toBe(30);
    expect(nextSideTunnelRetryTimeout(30)).toBe(60);
    expect(nextSideTunnelRetryTimeout(60)).toBeNull();
  });

  it("lists stopped enabled side tunnels while the stack is live", () => {
    const windscribe = createClientInstance("windscribe");
    const snapshot = baseSnapshot({
      phase: "running",
      clients: [
        {
          id: windscribe.id,
          preset: "windscribe",
          enabled: true,
          exit_ip: null,
          status: {
            phase: "stopped",
            message: "helper request timed out",
            since: new Date().toISOString(),
          },
        },
      ],
    });
    const failed = failedSideTunnelClients(snapshot, [
      ...baseSettings().clients,
      windscribe,
    ]);
    expect(failed).toHaveLength(1);
    expect(failed[0]?.preset).toBe("windscribe");
  });
});
