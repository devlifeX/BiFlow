# 0114: Authorize Unix helper peers and preserve macOS DNS recovery

## Status

Accepted in 6.2.58, while integrating macOS support and the Windows
localhost/kubectl fixes from PR #10.

## Context

The initial macOS helper trusted group access to its Unix socket and
recorded the configured UID as the caller. Multiple desktop users can
share a group. DNS takeover also kept its original values only in memory,
ignored failed commands, and tried to restore DHCP DNS without the required
`Empty` argument. A helper restart or failed cleanup could leave the system
resolver pointing at a stopped loopback listener.

The new orphan cleanup killed every process named Mihomo, including a
different BiFlow profile. Linux and Windows health inspection also read a
helper result after consuming it, so those backends did not compile.

## Decision

- Share the Unix peer authorization code between Linux and macOS. Tokio's
  safe `UnixStream::peer_cred()` supplies the actual UID. Accept only the
  configured UID or root, independently of the socket's group permissions.
- Before DNS mutation, atomically publish and sync a mode-0600 TOML snapshot
  under the helper's runtime. Keep original values across repeated starts.
  Never include this file or its resolver/service values in diagnostics.
- Restore DHCP with `networksetup -setdnsservers <service> Empty`. Check
  command status, attempt every restore, and remove the recovery file only
  after all restores succeed. Roll back partial apply; failed rollback keeps
  recovery pending and causes Connect to fail.
- Attempt persisted DNS recovery when the macOS helper starts. Report a
  failure and retain the snapshot so later Connect/cleanup can retry.
- Scope orphan termination to the exact helper binary and its generation
  paths. On Windows verify the executable path before stopping a PID. Use
  a bounded command, audit start/result without process command lines, and
  fail Connect when cleanup fails. Never kill by process name alone.
- Copy helper version from the successful health result before consuming
  the result. Keep Linux/Windows behavior consistent.
- Quote both shell arguments and the enclosing AppleScript when elevating
  a macOS install; atomically publish the root-owned mode-0600 config.
- The DMG installer accepts only `BiFlow.app`, quotes its source path,
  and finishes a staged copy before removing the installed bundle. Always
  attempt image detach after a mounted install, including failure, and
  report cleanup errors without logging paths. Remove only the empty mount
  directory after a successful detach.
- Windows delete-pending log files can remain visible while the append
  handle is open. Close the flushed handle before each append's path check,
  then reopen in append mode. Keep the existing recreation flag so the
  environment collector emits a fresh snapshot; never truncate the log.

## Validation

Host-independent tests cover DHCP restoration, failed restore/retry,
original DNS preservation across helper restarts, partial-apply rollback,
invalid resolver output, cross-profile process matching, UID rejection,
and shell/AppleScript quoting. Update-script tests also verify that copying
precedes installed-app removal. Unix tests read real socket credentials and
verify an unrelated process using the same executable survives cleanup.
The existing debug-log deletion, append, clear, and redaction regressions
also run under Windows/Wine.
Native macOS command behavior still requires a macOS host; Linux tests do
not claim to execute launchd, networksetup, or AppleScript.
