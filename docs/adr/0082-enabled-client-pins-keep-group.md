# 0082 Enabled-client pins keep the client group

## Status

Accepted

## Context

Pinning `google.com` to Windscribe (or any enabled client) stored the pin, but
Mihomo generation rewrote the rule to `REJECT` or `DIRECT` whenever that
client was missing from the live `ready` handles. OpenVPN/Windscribe is
optional at Connect time and often still down when the operator pins a host,
so the pin never named the Windscribe group. Clash `DOMAIN-SUFFIX,google.com`
already covers the apex and every subdomain; the miss was the outbound, not
the match key.

Production also hid Google hosts from live connections (ADR 0074, repaired in
0081), which made the same scenario look like the pin never applied.

## Decision

- Domain pins of an **enabled** client emit `DOMAIN-SUFFIX,<name>,<client group>`
  even when that egress is not live yet. IP pins keep
  `RULE-SET,custom-<id>-ips,<client group>`.
- YAML always includes a proxy group for every enabled client: the live
  handle when it exists, otherwise a synthesized stub (local SOCKS for
  `LocalProxy`, a closed `127.0.0.1:1` SOCKS for `OwnedSideTunnel`) so
  Mihomo accepts the named group. The stub is connection-level fail-closed
  and does not leak to WAN.
- `MATCH` still fail-closes to `REJECT`/`DIRECT` when the **default** client
  is down (ADR 0070). Disabled-client pins stay unemitted.
- Turning fail-closed off, or `allow_direct_when_down`, still downgrades a
  down client's pins to `DIRECT`.

## Consequences

A Windscribe pin on `google.com` stays on the Windscribe group through
Connect, live apply, and side-tunnel retry. `www.google.com` and
`gemini.google.com` follow the same suffix rule. Live connections show
Google hosts (ADR 0081) once the tunnel is up and the browser reconnects.
If Windscribe itself is not running, those hosts hit the stub group until
Connect or **Try again with 30s/60s timeout** brings the tunnel up.
