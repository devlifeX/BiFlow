# ADR 0072: Native profile picker for side-tunnel clients

## Status

Accepted

## Context

OpenVPN and Windscribe (and later WireGuard) need a local `.ovpn` / `.conf`
file. The client card stored that path in a free-text box, which is easy to
mistype and does not match how operators actually pick a downloaded profile.

An HTML `<input type="file">` does not expose a filesystem path in the Tauri
webview, and the helper must read the original file so relative `ca` / `cert`
references inside the profile still resolve.

## Decision

- Every `OwnedSideTunnel` card uses a native choose-file control. The path is
  never typed.
- The desktop command `pick_client_profile` opens `tauri-plugin-dialog` with
  `.ovpn` and `.conf` filters. Cancel returns `null`. The chosen path is not
  written to `debug.log`.
- Vite / Playwright mock the same command and return
  `/tmp/biflow-mock-profile.ovpn` (or `window.__BIFLOW_NEXT_PROFILE_PATH__`).
- The card shows the file name only. Username and password stay optional text
  fields because those are not files.

## Consequences

Windscribe keeps riding the OpenVPN driver. WireGuard inherits the same picker
when its driver ships. Operators can still move the original profile; BiFlow
stores the path, not a copy.
