# ADR 0069: Named rule lists

## Status

Accepted

## Context

Route pins were one flat set split by outbound. Users wanted to group
domains and IPs into named bundles ("Office", "Streaming"), assign a bundle
to a client, and check that a bundle actually answers through that client.
The engine contract (flat pins consumed by generation) is stable and must
not fork per feature.

## Decision

- `RoutePinsDocument` gains `lists: Vec<RuleListMeta { id, name, outbound }>`
  and every pin carries `list_id`. Lists are the user-facing model; the
  engine and generation still read the flat pins by outbound. Provider
  files, the helper allowlist, Mihomo YAML, and the drivers are untouched:
  all lists with the same outbound are already merged because their pins
  share that outbound.
- Invariant: `pin.outbound == its list's outbound`. Re-binding a list
  rewrites its member pins (validating each IP against the target pin
  policy). Loading canonicalizes membership: orphan pins are attached to
  (or get) a default-named list per outbound.
- Adding a client from the catalog auto-creates an empty list named after
  the preset, bound to it (best-effort; the client works without one).
  Deleting a client deletes its lists with its pins; moving pins re-binds
  the lists.
- An empty list contributes nothing at generation and is labelled "not in
  use" — it never blocks Connect (the MATCH default needs no pins).
- List names are trimmed, 1-60 chars, UI-only. They never appear in YAML,
  file names, or helper input.
- `check_rule_list` probes the first 3 domain entries of a list. With the
  stack running it fetches through Mihomo's mixed port so the real rules
  route the probe; stopped, a LocalProxy-bound list is probed through the
  client's SOCKS endpoint directly; otherwise the check asks the user to
  connect first. Any HTTP status counts as reachable; only transport
  errors fail. Over 1.5 s is "slow".
- The Direct Rules page is renamed List Management. The flat search table
  stays below the list cards as the global view.

## Consequences

- `pin_route` keeps its signature: it pins into (auto-creating) the default
  list of the target outbound, so Diagnostics one-tap flows are unchanged.
- Legacy documents load unchanged; `list_id` defaults to none and
  canonicalization assigns membership on first load.
