# ADR 0076: Live recovery of local-proxy clients

## Status

Accepted

## Context

Connect probes each optional local-proxy client (Happ, v2rayN, …) exactly
once with a 3-second egress check. Happ's SOCKS port only answers after Happ
itself connects to a server, so the probe almost always fails on a fresh
launch. The failed client then loses its egress handle. Generation used to
drop that Mihomo group and rewrite pins to REJECT (or DIRECT). ADR 0082 keeps
the named group via a stub; recovery still matters so the stub is replaced by
a live SOCKS bind. Nothing re-ran the ensure step, so the only cure was a
full disconnect/reconnect or an app restart — the "unstable, must reopen
repeatedly" report from production `debug.log` (three
`happ egress probe failed on 127.0.0.1:10808` entries in one morning).

## Decision

- `PlatformBackend::recover_clients` (default `Ok(false)`) re-checks every
  enabled `LocalProxy` client that has no egress handle: a cheap TCP gate
  first, then the standard egress probe. On success the synthesized handle
  and exit IP join the backend state. Side tunnels are excluded — they are
  owned processes with their own lifecycle.
- `Engine::recover_clients` runs on the existing 10-second health tick,
  only while the stack is Running/Degraded and no operation is busy. When a
  client recovers it reuses `apply_user_rules` — the proven pin-edit path —
  to re-stage, validate, and hot-apply the Mihomo config so the recovered
  group rejoins routing without restarting the stack.
- A `route_refresh_pending` flag survives a failed apply and retries on the
  next tick; it is cleared whenever the stack leaves Running/Degraded so a
  stale refresh never fires after a later connect.

## Consequences

Connecting Happ minutes after BiFlow is already up brings its pinned
domains back within one health tick (≤ ~15 s), with `client.recovered` /
`client.recovery_applied` events in `debug.log`. Connect-time behavior is
unchanged; recovery never touches clients that already hold a handle, so it
cannot flap a healthy egress.
