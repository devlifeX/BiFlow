# 0108: Keep the GUI process as the phase-zero runtime owner

## Status

Accepted

## Context

The Engine currently lives in Tauri application state and drives one platform
backend and one TUN stack. Starting an Engine in the CLI would allow two
processes to mutate Mihomo, routes, and helper-owned state. Extracting a
per-user daemon requires a larger lifecycle change and should build on a
verified local protocol.

## Decision

- For phase zero, the GUI process remains the sole Engine and stack owner.
- The CLI sends bounded, versioned, request-correlated commands over local IPC.
  Linux uses a Unix socket below `XDG_RUNTIME_DIR` with a private directory
  and socket mode `0700` / `0600`. Windows uses a per-user named-pipe endpoint;
  dev and production profiles use different endpoints.
- Phase zero exposes only live status and Connect. Connect is accepted by the
  existing Engine, which retains ownership after the CLI connection closes.
- Closing the main window hides the GUI to tray, so the runtime remains
  available. After application Quit, CLI reports that the runtime is
  unavailable.
- The CLI never opens the privileged helper endpoint and never creates an
  Engine. Phase one may extract a per-user daemon while preserving this IPC
  contract and single-owner behavior.

## Consequences

The prototype is small enough to validate on Windows and Linux while retaining
the current Tauri lifecycle. CLI operations require BiFlow to be running in
the user's session. The Windows named-pipe access control and cross-session
TUN lease must be verified before daemonization and multi-user support.
