import type { AppConfig, ClientInstance, DefaultRoute } from "../api/models";
import { PRESETS, presetById, type PresetId } from "./presets";

export function createClientInstance(
  preset: PresetId,
  id = crypto.randomUUID(),
): ClientInstance {
  const spec = presetById(preset);
  if (spec.kind === "local_proxy") {
    return {
      id,
      preset,
      enabled: true,
      allow_direct_when_down: false,
      config: {
        kind: "local_proxy",
        host: "127.0.0.1",
        port: spec.defaultPort ?? 1080,
        executable: "auto",
        start_timeout_seconds: 45,
        stop_with_stack: preset === "hiddify",
      },
    };
  }
  if (spec.kind === "owned_side_tunnel") {
    return {
      id,
      preset,
      enabled: true,
      allow_direct_when_down: false,
      config: {
        kind: "owned_side_tunnel",
        profile_path: null,
        executable: "auto",
        username: null,
        password: null,
        start_timeout_seconds: 45,
      },
    };
  }
  return {
    id,
    preset,
    enabled: true,
    allow_direct_when_down: false,
    config: { kind: "unsupported" },
  };
}

export function workingPresets(): PresetId[] {
  return PRESETS.filter((preset) => preset.status === "working").map(
    (preset) => preset.id,
  );
}

/** Last path segment of a stored profile, for the choose-file label. */
export function profileFileName(
  path: string | null | undefined,
): string | null {
  if (!path) return null;
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts.at(-1) ?? path;
}

export function canAddPreset(
  preset: PresetId,
  clients: ClientInstance[],
): boolean {
  const spec = presetById(preset);
  if (spec.status !== "working") return false;
  return !clients.some((client) => client.preset === preset);
}

export function enabledClients(clients: ClientInstance[]): ClientInstance[] {
  return clients.filter((client) => client.enabled);
}

export function isDefaultRouteValid(
  route: DefaultRoute,
  clients: ClientInstance[],
): boolean {
  if (route.kind === "direct") return true;
  return clients.some(
    (client) => client.id === route.client_id && client.enabled,
  );
}

export function sanitizeDefaultRoute(config: AppConfig): AppConfig {
  if (isDefaultRouteValid(config.default_route, config.clients)) {
    return config;
  }
  return { ...config, default_route: { kind: "direct" } };
}
