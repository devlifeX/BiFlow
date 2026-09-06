import type { ClientInstance, StackSnapshot } from "../api/models";

export const SIDE_TUNNEL_CONNECT_TIMEOUTS = [15, 30, 60] as const;
export type SideTunnelConnectTimeout =
  (typeof SIDE_TUNNEL_CONNECT_TIMEOUTS)[number];

export const INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT: SideTunnelConnectTimeout = 15;

export function sideTunnelConnectTimeoutAt(
  step: number,
): SideTunnelConnectTimeout {
  const index = Math.min(
    Math.max(step, 0),
    SIDE_TUNNEL_CONNECT_TIMEOUTS.length - 1,
  );
  return (
    SIDE_TUNNEL_CONNECT_TIMEOUTS[index] ?? INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT
  );
}

export function nextSideTunnelRetryTimeout(
  lastTimeoutSeconds: SideTunnelConnectTimeout,
): SideTunnelConnectTimeout | null {
  const index = SIDE_TUNNEL_CONNECT_TIMEOUTS.indexOf(lastTimeoutSeconds);
  if (index < 0 || index >= SIDE_TUNNEL_CONNECT_TIMEOUTS.length - 1) {
    return null;
  }
  return SIDE_TUNNEL_CONNECT_TIMEOUTS[index + 1] ?? null;
}

export function isOwnedSideTunnel(client: ClientInstance): boolean {
  return client.config.kind === "owned_side_tunnel";
}

export function stackSupportsSideTunnelRetry(snapshot: StackSnapshot): boolean {
  return snapshot.phase === "running" || snapshot.phase === "degraded";
}

export function failedSideTunnelClients(
  snapshot: StackSnapshot,
  clients: ClientInstance[],
): ClientInstance[] {
  if (!stackSupportsSideTunnelRetry(snapshot)) {
    return [];
  }
  return clients.filter((client) => {
    if (!client.enabled || !isOwnedSideTunnel(client)) {
      return false;
    }
    const status = snapshot.clients.find(
      (item) => item.id === client.id,
    )?.status;
    return status?.phase === "stopped";
  });
}
