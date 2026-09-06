# ADR 0077: Multi-client primaries and the Hiddify install requirement

## Status

Accepted

## Context

The client registry allows removing Hiddify and promoting another client
(e.g. Happ) to the default route, but two seams still assumed Hiddify:

- `missingConnectRequirements` demanded a Hiddify install whenever the
  dependency scan did not find one, even with no enabled Hiddify client —
  a Happ-primary operator would be forced through the Hiddify installer on
  Connect.
- The multi-client case matrix (non-Hiddify primary, dead secondary,
  three concurrent clients, required-primary aborts) had no covering
  tests, so regressions in `match_group`/`ready_handles` or the
  required-client semantics would land silently.

## Decision

- Hiddify is a connect requirement only while the snapshot lists an
  enabled Hiddify client. Before the first snapshot the legacy
  requirement stands. Helper and Mihomo requirements are unchanged.
- The case matrix is pinned by tests where each behavior lives:
  `iran-split-mihomo` (MATCH follows a non-Hiddify primary, a dead
  secondary fails closed without moving MATCH, three ready clients route
  their own pins, duplicate local ports rejected), `iran-split-config`
  (Happ-primary config without Hiddify validates; endpoint helpers fall
  back to catalog defaults), `iran-split-platform-linux` (the
  default-route client is required and aborts connect when unstartable; a
  dead optional secondary does not), `reachability` (VPN probes fall back
  to the remaining SOCKS endpoint), and the desktop lib
  (`isDefaultRouteValid`/`sanitizeDefaultRoute`/`canAddPreset`,
  connect-requirement gating).

## Consequences

Removing Hiddify and running Happ (or any local proxy) as primary is a
first-class, tested configuration: Connect no longer detours through the
Hiddify installer, and primary swaps, added second/third clients, and
dead-secondary routing are locked by unit tests on every platform seam.
