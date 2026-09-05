# ADR 0068: Client registry and egress kinds

## Status

Accepted

## Context

Hiddify was a hardcoded SOCKS egress (`HiddifyConfig`, `ensure_hiddify`,
`MATCH,VPN`). Adding OpenVPN as a third `Outbound` variant would repeat that
fork in config, rules, Mihomo YAML, UI unions, and e2e. The stack still has
exactly two ways traffic can leave: a loopback proxy, or a helper-owned
interface that must not take the default route.

## Decision

- Two stable egress kinds: `LocalProxy` and `OwnedSideTunnel`. Catalog
  presets (Hiddify, OpenVPN, Happ, v2rayN, Nekoray, Shadowsocks, WireGuard,
  Windscribe) never add `Outbound` variants.
- Windscribe ships as an `OwnedSideTunnel` riding the OpenVPN driver: the
  user generates a standard `.ovpn` with service credentials at
  build.windscribe.com. The Windscribe GUI itself (default route + kill
  switch) and its Proxy Gateway (only alive while that GUI owns the route)
  stay unsupported. Every catalog row carries official per-platform
  download links; the app opens the vendor page and never downloads or
  executes installers itself.
- `Outbound` is `Direct | Client { client_id }`. `DefaultRoute` is
  `Direct | Client { client_id }`. Direct is a valid MATCH.
- Schema 3 stores `clients` plus `default_route`. A schema 2 `[hiddify]`
  blob becomes one enabled Hiddify instance with `default_route = Client(id)`.
- One pin list: `RoutePinsDocument { pins }`. Delete confirms with pin count
  and can move pins to another enabled client. Disable keeps pins in the
  document but does not emit them into Mihomo YAML.
- Deleting or disabling the MATCH client falls back to `Direct` with a UI
  notice.
- Helper `GENERATION_FILES` allowlist: fixed names plus
  `custom-<id>-domains.txt` / `custom-<id>-ips.txt` where `<id>` matches
  `[0-9a-f-]{36}`. No caller paths, no `..`. YAML group names come from that
  sanitized id, never the UI label.
- Connect announces a generic `starting_client` stage with optional
  `operation_client { preset, client_id }`.
- Settings no longer has a Hiddify tab. Client fields live on Dashboard
  cards. Basic mode keeps five nav items and the migrated Hiddify instance.

ADR 0018 (probe LocalProxy before TUN) and ADR 0025 (Pause keeps the
upstream up) stay. They no longer assume Hiddify is the only upstream.

## Consequences

v1 allows one instance per working preset. WireGuard is catalog-only.
Windscribe stays unsupported until it exposes a local proxy.
