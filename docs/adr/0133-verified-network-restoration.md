# ADR 0133: Preserve incomplete proxy restoration and verify network release

Status: Accepted (6.2.85)

## Context

The Windows diagnostic log recorded `saved_client_unavailable` during a
Disconnect. All native proxy backends then deleted the original proxy baseline
and logged the handoff as completed. A temporary client outage therefore lost
the record needed to restore the user's ordinary proxy on a later retry.

Live checks also reproduced an untrusted-root TLS error for a direct request to
`rade.ir`, while the same request through Hiddify passed certificate validation.
Chrome's Proxy Switcher was on No Proxy and bypassed the restored system proxy.
With System Proxy selected, the site opened while BiFlow was disconnected.
BiFlow does not manage that browser extension or change certificate validation.

## Decision

- Preserve ADR 0132's ownership boundary: return the original managed proxy
  only after releasing capture, while the client is reachable and settings
  still match the state BiFlow left. Respect intervening user or corporate
  settings and leave clients that were already running alone.
- An unavailable original client now returns an error and retains the private
  baseline on Windows, macOS, and Linux. It is never enabled while unreachable.
  A retry of Disconnect or Quit can complete restoration. Quit keeps the app
  open when that restoration is incomplete. Deliberate outside settings changes
  and unmanaged baselines remain safe reasons to retire a record.
- Independently check the owned core process and TUN after cleanup, before
  proxy restoration on Stop, Pause, rollback, and startup reconciliation. A
  successful helper report alone does not prove the running process is gone.
- Phase waiters accept a terminal phase/error only after lifecycle work is idle.
  A queued retry must not immediately return the previous Error before it runs.
- Log an incomplete handoff as `system_proxy.handoff_deferred`, without proxy
  settings. Include `saved_user_proxy_handoff_pending` in Error diagnostics as
  well as stopped/paused diagnostics.
- "Normal" means the state before Connect: an existing healthy Hiddify proxy
  stays available; an originally disabled proxy stays disabled. It does not
  mean forcing ISP access or changing client configuration, browser proxy
  extensions, DNS providers, security products, or certificate trust.

## Validation

Filesystem tests assert that a failed handoff retains the exact record and a
verified retry retires it. Shared engine tests exercise Disconnect and Quit with
a failed proxy restore and with a surviving core despite a clean helper report;
they assert capture cleanup, baseline retention, blocked completion, and retry.
The primary-flow e2e checks continued internet status and absent traffic routes
after a second Connect/Disconnect cycle. Environment tests cover pending
restoration in Error. Native CI validates the shared policy on every platform.

These checks do not guarantee that an independently stopped client or an ISP's
invalid certificate can be repaired by BiFlow. They ensure those conditions do
not silently discard recovery state or produce a successful cleanup result.
