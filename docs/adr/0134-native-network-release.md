# ADR 0134: Verify native network release on Windows and macOS

Status: Accepted (6.2.86)

## Context

The Windows helper used `ifconfig` to detect an owned adapter and a no-op to
remove it. A missing Unix command therefore looked like successful cleanup.
Controller shutdown also does not prove that native routing or resolver state
has disappeared. On macOS, the configured tunnel alias is replaced by a
kernel-assigned `utunN` name. Cleanup using the alias misses that identity.
Separately, startup reconciliation skipped cleanup once the core and TUN were
gone, even if a private DNS recovery record remained. An empty live resolver
was accepted as restored DNS, and some route/cache failures were only warnings.

## Decision

- Windows release uses native adapter, route, and DNS cmdlets. It selects only
  the configured name with a known Mihomo/Wintun description, rechecks its GUID,
  removes routes on that interface, resets its DNS, disables the persistent
  adapter, and reads back state. A retained device is reenabled before the next
  core start. Missing tools, inspection errors, and partial release are failures.
  Physical adapters, other VPNs, global proxy choices, and drivers are untouched.
- macOS saves the actual `utunN` emitted by the owned core, tagged with a fresh
  launch UUID so an earlier launch cannot supply its identity. The private
  record survives helper restart. Cleanup checks native interface enumeration;
  it retains recovery while that device remains instead of deleting a possibly
  reused live tunnel. Route-command failures propagate. No prefix-wide `utun`
  cleanup is permitted.
- Save macOS DNS before spawning the core. Retain recovery when the native
  primary resolver is empty or still uses a tunnel resolver, including fake-IP
  answers. Read back saved static resolvers before retiring the baseline. DNS
  restoration still runs when route verification fails. Supersede ADR 0121's
  guessed LAN-gateway fallback: a gateway is not evidence of a DNS service.
  Restore the original static settings or DHCP and retain recovery for retry.
- Startup always runs the helper's idempotent cleanup when the helper is ready.
  A live controller is not a prerequisite. The absence watchdog also tracks
  pending release and private macOS DNS/device recovery records.
- Windows/macOS cache-flush failures keep cleanup retryable. Stop/Pause/Quit
  cannot complete a proxy handoff while the helper reports incomplete cleanup.
- macOS environment snapshots now collect classified primary DNS evidence and
  report a missing resolver while stopped/paused. Raw resolver addresses are
  never written into diagnostics.

## Validation

Tests replace every Windows network cmdlet with in-memory fixtures and execute
the production script through PowerShell. They cover a surviving capture,
partial DNS failure, a no-op disable, retry, repeated release, reconnect,
unrelated routes, a physical-adapter name collision, and enumeration failure.
Portable macOS tests cover pre-spawn DNS capture, empty resolver retry, private
device persistence, launch isolation, and classified environment findings.
Engine tests cover startup without a process/TUN and blocked Stop/Quit followed
by successful DNS recovery. The existing connect/pause/resume/disconnect e2e
flow also asserts absent traffic routes while paused.

These tests do not stop the installed app or change the host network. The helper
also passes Apple Silicon target Clippy from Windows using Zig for its C
dependencies. Native end-to-end macOS behavior still requires validation on a Mac.
