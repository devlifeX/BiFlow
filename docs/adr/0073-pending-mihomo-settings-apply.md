# ADR 0073: Pending Mihomo apply after live settings edits

## Status

Accepted

## Context

`save_settings` persists the document and updates the backend copy. It does
not regenerate the running Mihomo generation, so a profile, DNS, port, or
default-route change while the stack is `running` / `degraded` stays inert
until the next Connect. Pins already live-apply; settings did not. Operators
also must not be forced to stop Hiddify or OpenVPN just to reload routing.

## Decision

- After a save that affects live Mihomo (clients, default route, Mihomo
  ports/DNS/TUN, fail-closed, rule refresh) while the stack is running or
  degraded, show a banner: restart Mihomo, revert, or dismiss.
- **Restart Mihomo** calls `apply_user_rules` (same path as pin live-apply).
  Clients stay up. The banner clears on success.
- **Revert** writes the pre-edit document back and clears the banner. The
  first pending snapshot is kept across further edits so revert undoes the
  whole batch.
- **Dismiss** hides the banner and keeps the saved document. The next
  Connect or a later restart still consumes it.
- Stopping the stack clears the banner; the next start uses the saved
  revision. Launch-at-login / close-to-tray toggles do not raise the banner.

## Consequences

Side-tunnel profile bytes still need that process to reread them. The banner
is honest about Mihomo. A later Connect remains the full apply for clients
that were not restarted.
