# ADR 0078: Fast Connect and the primary-egress watchdog

## Status

Accepted

## Context

Production `debug.log` measurements (46 operations) showed Connect at a
median of 4.1s but 21–23s whenever an enabled Happ was not serving: the
optional client's launch waited for its port on the critical path, and an
optional Hiddify could hold Connect for its full 45s retry window. The
reachability button was pinned at p95 = 5s by the connect-timeout clamp.

Separately, every component check is TCP-listening based. A proxy whose
upstream died keeps its port open, so the whole Diagnostics page stays
green while MATCH traffic blackholes — the operator has no clue and
restarts the app.

## Decision

- **Optional clients never block Connect.** A non-required local proxy
  that is not listening is spawned and skipped (its egress joins routing
  through ADR 0076 recovery once it serves). An optional Hiddify gets one
  3-second egress probe instead of the 45s retry loop. Required
  (default-route) clients keep the strict launch-wait and egress
  verification from ADR 0018.
- **Primary-egress watchdog.** Each health tick runs one end-to-end
  egress probe of the default-route local proxy
  (`PlatformBackend::probe_primary_egress`). Three consecutive failures
  (~30s) flip the stack to Degraded with the retryable
  `hiddifyEgressUnavailable` error ("open but not connected…"); one
  successful probe restores Running and clears the error. The watchdog
  never touches routing, so a false positive cannot cut a working
  connection.
- **Faster reachability.** Probe timeouts drop from 5s/8s to 3s/6s;
  `SLOW_THRESHOLD` still classifies slow-but-alive links.

## Consequences

Connect stays in the low single-digit seconds regardless of how many
optional clients are configured or how broken they are. "All green but
nothing works" now surfaces within ~30s as a Degraded phase with an
actionable message and recovers by itself when the operator fixes the
client — no app restart or reconnect required. Ping worst case halves.
