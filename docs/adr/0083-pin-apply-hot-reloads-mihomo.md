# 0083 Pin apply hot-reloads Mihomo without a process restart

## Status

Accepted (narrows the restart described in [0081](./0081-live-connections-survive-route-apply.md))

## Context

Pinning `google.com` to Windscribe (or moving any host between lists) called
`apply_user_rules` → `start_core` → helper `StartMihomo`. A new generation
id always killed the running process, so TUN dropped for the whole apply
(validate + spawn + readiness, often many seconds).

During that gap the browser resolved Google over system DNS and loaded it
off-Windscribe (WAN or MATCH/Hiddify after TUN returned, because the
connection used real IPs instead of Mihomo fake-ip). Live connections stayed
empty until sites reconnected, so Windscribe looked like it never took the
pin. Unrelated tabs stalled too.

Mihomo Meta 1.19+ resolves rule-provider paths against the process `-d`
workdir, so the desktop cannot `PUT /configs` at a sibling generation
directory.

## Decision

- Live pin/list applies **overlay** the new generation files into the
  running workdir, then `PUT /configs?force=true` with empty `path` and
  `payload` so Meta reloads the process default file. Rule-provider
  paths inside YAML stay relative (ADR 0016). A relative PUT `path` of
  `config.yaml` is rejected immediately (`path is not a absolute path`);
  an absolute sibling generation directory is also rejected. The Mihomo
  process, TUN, side tunnels, and fake-ip cache stay up.
- After a successful reload, close only connections whose host or
  destination matches the moved pin so that name reconnects on the new
  outbound. Other sites keep their sockets.
- Connect still uses `StartMihomo`. Overlay is a no-op spawn fallback when
  the process is already gone.
- Settings **Restart Mihomo** follows the same live path (hot reload). A
  true process restart remains Disconnect/Connect.

## Consequences

A Windscribe pin on `google.com` keeps TUN up, so the name cannot leak to
system DNS during apply. The live table updates as soon as the browser
reconnects that host. Unrelated connections are not dropped. ADR 0081's
"keep last rows while apply is in flight" still covers the brief reload.
