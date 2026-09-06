# ADR 0075: Windscribe also offers the OpenVPN installer

## Status

Accepted

## Context

Windscribe is an `OwnedSideTunnel` that launches the system OpenVPN binary
with a generated `.ovpn` (ADR 0068). The catalog row and missing-binary
banner used Windscribe's own download URL (`windscribe.com/getconfig`),
which is the config generator, not the OpenVPN installer. Operators who
already had a profile still had no in-app path to OpenVPN.

## Decision

- Windscribe catalog and card download rows list **Download OpenVPN** first
  (same official OpenVPN page as the OpenVPN preset), then **Get Windscribe
  config**.
- The amber "OpenVPN is not installed" banner always opens the OpenVPN
  page, including on a Windscribe card.
- The URL allowlist stays derived from the preset table; Windscribe reuses
  the OpenVPN row so it cannot drift.

## Consequences

Selecting Windscribe shows both links. Connect still only needs the OpenVPN
binary plus the profile (and service credentials when the `.ovpn` asks for
them). The Windscribe GUI must stay closed.
