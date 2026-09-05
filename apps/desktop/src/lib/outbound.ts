import type { ClientInstance, DefaultRoute, Outbound } from "../api/models";
import { presetById, type PresetId } from "./presets";

export const MOCK_HIDDIFY_ID = "11111111-1111-1111-1111-111111111111";

export function outboundKey(outbound: Outbound | DefaultRoute): string {
  return outbound.kind === "direct" ? "direct" : outbound.client_id;
}

export function outboundFromKey(value: string): Outbound {
  return value === "direct"
    ? { kind: "direct" }
    : { kind: "client", client_id: value };
}

export function defaultRouteFromKey(value: string): DefaultRoute {
  return value === "direct"
    ? { kind: "direct" }
    : { kind: "client", client_id: value };
}

export function outboundLabel(
  outbound: Outbound | DefaultRoute | string,
  clients: ClientInstance[],
): string {
  const key = typeof outbound === "string" ? outbound : outboundKey(outbound);
  if (key === "direct") return "DIRECT";
  const client = clients.find((item) => item.id === key);
  if (!client) return key.startsWith("client-") ? key : "Client";
  return presetById(client.preset as PresetId).title;
}

export function localProxyConfig(client: ClientInstance): {
  host: string;
  port: number;
  start_timeout_seconds: number;
  stop_with_stack: boolean;
} | null {
  return client.config.kind === "local_proxy" ? client.config : null;
}
