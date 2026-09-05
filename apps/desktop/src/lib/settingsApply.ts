import type { AppConfig, StackPhase } from "../api/models";

export interface SettingsApplyNotice {
  previous: AppConfig;
}

/** Live Mihomo can consume a new generation without stopping clients. */
export function stackNeedsSettingsApply(
  phase: StackPhase | undefined,
): boolean {
  return phase === "running" || phase === "degraded";
}

/** Fields that only take effect after Mihomo reloads its generation. */
export function settingsAffectLiveMihomo(
  before: AppConfig,
  after: AppConfig,
): boolean {
  return (
    JSON.stringify(liveSettingsSlice(before)) !==
    JSON.stringify(liveSettingsSlice(after))
  );
}

function liveSettingsSlice(config: AppConfig) {
  return {
    clients: config.clients,
    default_route: config.default_route,
    mihomo: {
      controller_host: config.mihomo.controller_host,
      controller_port: config.mihomo.controller_port,
      mixed_port: config.mihomo.mixed_port,
      dns_port: config.mihomo.dns_port,
      tun_name: config.mihomo.tun_name,
      log_level: config.mihomo.log_level,
      direct_dns_preset: config.mihomo.direct_dns_preset,
      direct_dns_servers: config.mihomo.direct_dns_servers,
    },
    fail_closed: config.behavior.fail_closed,
    rules: config.rules,
  };
}
