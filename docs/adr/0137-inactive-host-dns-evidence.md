# ADR 0137: Record classified host DNS answers while disconnected

Status: Accepted (6.2.89)

## Context

Verified native cleanup and DNS server configuration do not establish that the
host resolver returns usable addresses. An original resolver can return a
private answer for independent public destinations after the owned TUN and
proxy have been released. Conversely, a valid DNS answer or TLS handshake does
not prove that an HTTP response will arrive. Intermittent HTTP success cannot
establish the cause of an earlier failure.

## Decision

- While Stopped or Paused, environment snapshots perform a bounded, read-only
  host resolver lookup for the fixed reserved example domain `example.com`.
  No user target, direct rule, route, proxy, hosts entry, or resolver is changed.
- Record only resolution status and deduplicated address classes. Never retain
  the target, returned addresses, or raw OS resolver error in diagnostics.
- Distinguish empty replies, resolver failures, timeouts, and non-public answers
  with `stopped_public_dns_*` findings. Do not run this probe in Running/Degraded
  snapshots, where fake-IP answers are expected.
- Findings describe evidence, not ownership. They do not block native cleanup,
  replace a user's DNS/firewall/proxy policy, or claim that all Internet access
  is healthy after one successful lookup. A recovery of an external connection
  without a configuration change is not a new application fix.

## Validation

Injected resolver futures exercise private, fake-IP, loopback, link-local,
shared-address, multicast, broadcast, ordinary, empty, failed, and timed-out
responses without accessing the network. Serialization regressions ensure
addresses and raw error payloads cannot reach the snapshot. Environment tests
verify phase-dependent findings, including mixed ordinary/non-public answers.
The production probe only resolves through the existing host policy and makes
no HTTP requests. Live tunnel toggling is not required by these tests.
