import type { AppConfig, StackSnapshot } from "../api/models";
import { MOCK_HIDDIFY_ID } from "../lib/outbound";

const now = "now";

export function baseSettings(overrides: Partial<AppConfig> = {}): AppConfig {
  return {
    schema_version: 3,
    revision: 0,
    clients: [
      {
        id: MOCK_HIDDIFY_ID,
        preset: "hiddify",
        enabled: true,
        allow_direct_when_down: false,
        config: {
          kind: "local_proxy",
          host: "127.0.0.1",
          port: 12334,
          executable: "auto",
          start_timeout_seconds: 45,
          stop_with_stack: true,
        },
      },
    ],
    default_route: { kind: "client", client_id: MOCK_HIDDIFY_ID },
    mihomo: {
      controller_host: "127.0.0.1",
      controller_port: 19090,
      controller_secret: "redacted",
      mixed_port: 17890,
      dns_port: 1053,
      tun_name: "clash-iran",
      log_level: "info",
      direct_dns_preset: "fake_ip",
      direct_dns_servers: [],
    },
    rules: { refresh_interval_minutes: 15, upstream_refresh_hours: 24 },
    behavior: {
      launch_at_login: false,
      connect_at_launch: false,
      close_to_tray: true,
      fail_closed: true,
    },
    ...overrides,
  };
}

export function baseSnapshot(
  overrides: Partial<StackSnapshot> = {},
): StackSnapshot {
  return {
    revision: 1,
    phase: "stopped",
    operation_id: null,
    helper: { phase: "running", message: "Helper is ready", since: now },
    clients: [
      {
        id: MOCK_HIDDIFY_ID,
        preset: "hiddify",
        enabled: true,
        status: { phase: "stopped", message: null, since: now },
      },
    ],
    mihomo: { phase: "stopped", message: null, since: now },
    tun: { phase: "stopped", message: null, since: now },
    dns: { phase: "stopped", message: null, since: now },
    providers: { ready: 0, total: 0, rules_loaded: 0, last_refresh: null },
    exit_ip: null,
    backend: "external_hiddify",
    last_error: null,
    updated_at: now,
    ...overrides,
  };
}
