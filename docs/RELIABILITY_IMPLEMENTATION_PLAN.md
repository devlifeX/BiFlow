# BiFlow reliability implementation plan

Status: in progress. This plan is an execution checklist, not a claim that a
platform has been validated. A task is complete only after its acceptance
criteria and the applicable `AGENTS.md` gates pass.

## Scope and safety

Authorized work: change application, helper, installer, packaging, tests,
documentation, and version metadata needed to implement the items below; run
builds and non-destructive tests; test installation in disposable Windows and
Linux environments. Preserve existing user changes, profiles, credentials,
rules, logs, and external VPN installations. Do not perform a privileged
install, uninstall, upgrade, or network-route change on the user's host as a
substitute for a disposable E2E environment. Do not publish, push, or create a
release without a separate request.

## Required invariants

1. A client add/remove/enable/configuration change either commits all related
   settings and pins or leaves the old state intact. Unrelated routes remain
   connected.
2. A failed live apply restores the active Mihomo generation and persisted
   document to the same known-good revision. No implicit DIRECT fallback.
3. Component health separates local listener, actual client egress, Internet
   reachability, DNS, Helper, and Mihomo. Failed probes never masquerade as
   proof that the Internet itself is down.
4. Fresh install and upgrade cannot report success until the package, Helper,
   protocol, permissions, and runtime paths are verified. An unverifiable
   installation has an actionable repair state.
5. Updates authenticate the exact target-version artifact before disrupting
   the active stack. A failed update preserves user data and offers recovery.

## Work packages

### R0 — Baseline and reproducibility

- [ ] Record the dirty-worktree baseline and distinguish pre-existing edits.
- [ ] Confirm each suspected defect against current source and add minimal
      failing regression tests before changing behavior.
- [ ] Define a native test matrix for clean install, upgrade, interrupted
      install, rollback, and third-party client churn on both OSes. Mark mock
      Playwright tests separately from native package tests.

### R1 — Installer and Helper lifecycle (first implementation priority)

- [ ] Windows NSIS propagates Helper installation failure and verifies a
      compatible, authorized service rather than only launching the process.
- [ ] Windows in-app update wait script propagates UAC/installer failure,
      verifies the installed version and Helper, and does not relaunch the app
      under a false success state. Handle locked payloads without deleting a
      running binary.
- [ ] Linux maintainer scripts distinguish upgrade from removal; do not
      disable a healthy Helper on upgrade. Never infer an authorized user from
      absent or untrusted `SUDO_*` variables. Preserve or explicitly repair
      existing authorization and staging configuration.
- [ ] Cover fresh `.deb`, upgrade, AppImage repair, fresh NSIS, reinstall, and
      denied elevation with contract tests and disposable native E2E tests.

### R2 — Transactional client and route changes

- [ ] Propagate failed settings writes to callers. Client, list, and pin
      mutations are atomic or have tested compensation, including conflicts.
- [ ] Stage and validate live runtime changes before commit; roll back both
      active generation and persisted revision on overlay/reload/readiness
      failure or timeout. Preserve unrelated live connections.
- [ ] Persist an explicit pending-apply state across UI dismissal/restart so
      effective and desired routing cannot silently diverge.

### R3 — Client health and self-recovery

- [ ] Track configured endpoint and process identity with egress handles;
      invalidate stale handles and cached exit IPs on client restart, update,
      endpoint change, or failed egress probe.
- [ ] Probe actual egress of the default and optional pinned clients with
      bounded timeouts, hysteresis, and backoff. Distinguish listener-up from
      upstream-up; preserve fail-closed behavior.
- [ ] Recover a client independently, hot-apply only the affected route, and
      avoid stealing controller/Helper capacity during Connect.

### R4 — Truthful connectivity diagnosis

- [ ] Introduce typed, independently sampled states for underlying network,
      direct path, VPN path, DNS, and probe-service availability.
- [ ] Treat failures of public-IP services and desktop IPC as unknown/probe
      failures, not as definitive Internet outages. Account for captive portal,
      blocked domains, stale samples, and transient flapping.
- [ ] Show localized, actionable user messages that identify the failing
      layer without exposing credentials or diagnostic targets in logs.

### R5 — Trusted application updates

- [ ] Require exact platform, architecture, and semantic-version asset match;
      reject mismatches instead of selecting a loose fallback.
- [ ] Authenticate every downloaded package before pausing the stack (prefer
      the existing signed updater, or a verified manifest/signature protocol).
- [ ] Add post-install version, Helper, protocol, binary, migration, and rule
      checks. Provide rollback or a recoverable failed-update state.

### R6 — Verification and release gate

- [ ] Unit/integration tests cover every failure transition listed above.
- [ ] Windows and Linux disposable native E2E cover clean install, upgrade
      while connected, denied elevation, helper failure, offline network,
      dead-upstream/open-port client, port change, and client restart.
- [ ] Run the exact `AGENTS.md` gates for touched areas, update ADRs and
      lessons, bump the root `version`, synchronize manifests, and record any
      unavailable native test as **unverified**, never as a pass.

## Execution order

R0 → R1 → R2 → R3 → R4 → R5 → R6. Independently testable corrections may be
landed in smaller increments, but no work package is marked complete from a
mock-only or source-only check.
