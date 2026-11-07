# ADR 0130: Recover after unlock and serialize network release on Quit

Status: Accepted (6.2.81)

## Context

This change starts at upstream main `f35ece9` (6.2.80). Its existing macOS
download-and-drag install flow (ADR 0129), DNS recovery, helper absence watchdog,
and native speed displays remain in place. A higher version on an older feature
branch does not establish that it contains current upstream fixes.

The native health loop refreshes labels but does not rebuild a tunnel after
sleep, Windows unlock, or persistent local runtime failure. The helper may also
have released the capture while the desktop was suspended. Quit starts cleanup,
but a second exit request can pass through while cleanup is pending. Cleanup
failure is logged and the old quit path still exits.

## Decision

- Detect native timer gaps over 30 seconds, Windows lock-to-unlock transitions,
  and two consecutive unhealthy Mihomo/TUN/DNS samples. The session query checks
  only LogonUI in this desktop session, runs without a console, and has a
  three-second timeout. Query failures are logged and leave the other checks active.
- Serialize recovery with lifecycle actions and preserve paused/stopped intent.
  A 60-second cooldown limits repeated repairs. Drain owned state, invalidate
  cached client egress, rebuild, and check readiness. Failed recovery attempts
  detach the capture and retain the error for an explicit retry.
- Quit cancels and drains pending engine operations and client maintenance,
  rejects new work, and awaits owned network cleanup. Every repeated Quit waits
  until cleanup has completed. Failure keeps the app available for retry.
  Installer exits retain the existing paused-stack behavior from ADR 0120.
- Cleanup always stops the core, including from Error. A proxy failure must not
  skip tunnel/DNS cleanup, and must still be reported after that cleanup rather
  than producing a false successful disconnect.
- Environment findings identify a running/degraded stack whose Mihomo, tunnel,
  or DNS component is unhealthy. Recovery and exit paths emit structured events.
- Windows installer staging uses NSIS's `COMMONPROGRAMDATA` variable. The
  retired `PROGRAMDATA` token expands to an invalid path and emits a packaging
  warning; positive and negative contracts pin the valid spelling. Compile the
  generated installer separately with warnings treated as errors before installing.

## Validation

Engine tests cover Quit during Connect, maintenance drain, cleanup from Error
despite a proxy failure, a lost TUN, paused intent, and failed recovery. Desktop
tests cover repeated Quit, retry after failed cleanup, installer exit policy,
unlock, timer gaps, cooldown, and environment findings. The connection e2e flow
disconnects, reconnects, and disconnects again. Native lock/sleep behavior still
requires a real host smoke test; mock browser tests cannot exercise OS adapters.
