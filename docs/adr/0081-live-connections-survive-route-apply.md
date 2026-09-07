# 0081 Live connections survive a route apply

## Status

Accepted

## Context

Changing a live-connection route calls `pin_route` → `apply_user_rules` →
`reload_core`. The controller can blip while providers reload, and the
Diagnostics poll used to `setRows([])` on that error. The table flashed
empty even though the pin was stored. After a process restart (Connect)
the connection table is honestly empty until the browser reconnects.

Production also hid `google.com` from that table (ADR 0074), so a
Windscribe/Happ pin on Google looked like it never took effect.

When the pinned client is not in `ready` handles, generated rules still
name that client's group (ADR 0082). A down Windscribe therefore keeps the
`google.com` pin on its group; traffic uses the live tunnel once Connect or
side-tunnel retry brings it up.

## Decision

- Keep the last live-connection rows when a poll fails, and while
  `actionPending` is true and the next poll is empty.
- Show Google hosts in live connections and Dashboard packets.
- Keep hiding the Google reachability probe in production.
- Enabled-client pins keep the client group even when that egress is still
  coming up (ADR 0082).

## Consequences

A route change no longer looks like the list vanished. Operators can
see `google.com` take the new outbound after the site reconnects. The pin
names the client group immediately; Windscribe/OpenVPN must be running for
that traffic to leave through the tunnel.
