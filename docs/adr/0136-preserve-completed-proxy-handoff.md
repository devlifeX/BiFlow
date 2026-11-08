# ADR 0136: Preserve a completed proxy handoff on repeated Stop and Quit

Status: Accepted (6.2.88)

## Context

A successful Stop restores the original local proxy and retires its private
baseline. The Stopped shutdown path nevertheless called `stop_user_proxy` again.
A backend still holding a child handle could then see no saved baseline, stop
the returned client, and leave the restored system proxy pointing at a dead
socket. A repeated Stop had the same hazard, including another unnecessary
clear/restore cycle. This is a global ownership bug, independent of destinations.

## Decision

- Quit from an already detached Stopped/Uninitialized stack only retries pending
  restoration; it does not reclaim the returned client.
- Repeated Stop verifies that capture is absent and retries pending restoration
  without stopping clients or clearing their already restored system proxy.
- Keep full cleanup and error retry for active and Error states. Never reenable
  a proxy whose client is unavailable. An uninitialized app does not own a
  preexisting user client merely because Quit was requested.
- Diagnose an enabled loopback system proxy whose configured client refuses both
  address families as `system_proxy_points_to_unavailable_client`. A process name
  or a successful old handoff event is not evidence of current socket readiness.
- Do not add per-domain routes, hosts entries, DNS exceptions, or site fixtures.
  Native capture cleanup, system proxy state, DNS policy, and external network
  reachability are distinct system-wide checks.

## Validation

An engine regression starts with an original live proxy, performs Stop, retires
the baseline through the production handoff flow, then exercises both repeated
Stop and Quit. The backend fixture models a retained process handle that would
kill the returned client after baseline retirement. This test fails before the
fix and passes after it. Environment fixtures cover the global dead-proxy finding
and its absence for a disabled proxy. No real tunnel is disconnected by tests.
