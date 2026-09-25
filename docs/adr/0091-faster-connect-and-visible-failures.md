# ADR 0091: Faster Connect and visible failure reasons

- Status: Accepted
- Date: 2026-09-25

## Context

Connect, pause, and resume stayed slow on a machine where the helper and
Hiddify were already running. The engine polled full runtime health every
250ms, and each poll opened the helper pipe and called the Mihomo controller
while `StartMihomo` and rule-provider loading were in progress. Pause then
asked that same controller whether TUN was up after Mihomo had already
exited, so the HTTP client waited out its timeout. Resume repeated the
egress `generate_204` probe and a full `mihomo -t` pass even when the config
hash and the client port had not changed.

When those steps failed, the stack phase became `error` and the status pill
showed that word. Platform failures were translated as an internal error, and
the specific cause in `technical_details` never reached the dashboard.

## Decision

- Progress polls use a TCP-only health snapshot and run once a second. They
  do not open the helper pipe or call the controller.
- Windows TUN status treats a closed controller port as inactive and does not
  issue an HTTP request.
- A successful egress probe is reused while that client port stays open.
  Disconnect clears the cache. Pause does not.
- `mihomo -t` runs once per config hash in a process. Resume skips it when
  the hash is unchanged.
- The dashboard shows the translated failure plus `technical_details`.
  Platform errors use `errors.platform` so they are not labeled as a generic
  internal fault.

## Consequences

A resumed stack trusts the previous probe until the port closes or the user
disconnects. The primary-egress watchdog still marks the stack degraded if
that proxy later stops forwarding. The first Connect in a process still
validates config and probes egress.
