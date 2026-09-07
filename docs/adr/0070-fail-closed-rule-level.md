# ADR 0070: Rule-level fail-closed

## Status

Accepted

## Context

When a client's egress is down at Connect time, its pinned hosts and (for the
MATCH default) all unmatched traffic used to fall back to DIRECT — the user's
real IP leaked exactly when their tunnel was broken. Users asked for a
kill-switch: block locally instead of leaking, globally, with per-client
exclusions.

## Decision

- `behavior.fail_closed: bool`, default **true**. Serde default keeps schema 3.
- `ClientInstance.allow_direct_when_down: bool`, default false — the
  per-client exclusion: that client's traffic may fall back to DIRECT.
- Generation emits rule-sets for **every enabled client** (provider files are
  always staged for enabled clients): a ready client keeps its proxy group;
  a down client's **user pins** still name that group (ADR 0082, stub proxy)
  instead of rewriting to `REJECT`. `MATCH` still fail-closes to `REJECT`
  (or `DIRECT` when excluded or the global switch is off) when the default
  client is down. `REJECT` is not a proxy group, so DoH is left unpinned in
  that case.
- While running, a select group whose only proxy is a dead loopback port or a
  removed side-tunnel interface already fails closed at the connection level;
  no extra rules are needed for mid-session death.

## Consequences

- This is **rule-level** fail-closed: it holds as long as Mihomo owns the
  TUN. If Mihomo itself dies or the stack is stopped, the TUN and routes are
  cleaned up and the system routes normally — a firewall-level kill switch
  (helper-owned nftables / WFP rules) is future work and out of scope here.
- Users who prefer the old behaviour turn the toggle off in Settings →
  Behavior, or exclude individual clients on their cards.
