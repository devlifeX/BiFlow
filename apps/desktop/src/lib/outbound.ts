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
  if (key === "direct" || key === "DIRECT") return "DIRECT";
  // Live connections report Mihomo group/proxy names (`client-<uuid>`,
  // `proxy-<uuid>`); resolve them to the preset title so the UI never
  // shows a raw uuid.
  const id = key.replace(/^(client|proxy)-/u, "");
  const client = clients.find((item) => item.id === id);
  if (!client) return id === key ? "Client" : key;
  return presetById(client.preset as PresetId).title;
}

/** Humanizes rule names that embed a client uuid (`custom-<uuid>-domains`). */
export function ruleLabel(rule: string, clients: ClientInstance[]): string {
  const match = /^custom-([0-9a-f-]{36})-(domains|ips)$/u.exec(rule);
  if (!match) return rule;
  const client = clients.find((item) => item.id === match[1]);
  if (!client) return rule;
  return `${presetById(client.preset as PresetId).title} ${match[2]}`;
}

export function localProxyConfig(client: ClientInstance): {
  host: string;
  port: number;
  start_timeout_seconds: number;
  stop_with_stack: boolean;
} | null {
  return client.config.kind === "local_proxy" ? client.config : null;
}
