# ADR 0092: Windscribe OpenVPN must not take the system route

- Status: Accepted
- Date: 2026-09-25

## Context

A Windscribe `.ovpn` is a normal `client` profile: `dev tun`, `auth-user-pass`,
and a hostname on port 443. The file itself often has no `redirect-gateway`,
but `client` pulls server options. Windscribe pushes a default route, DNS,
and `block-outside-dns`. On Windows, `dev tun` also attaches the first
TAP/Wintun adapter, which may already be Mihomo. Either one takes traffic
away from BiFlow's split TUN.

`--route-noexec` already skipped route installation. It does not stop a
pushed DNS change, `block-outside-dns`, or adapter selection.

## Decision

- Before spawn, write a temporary copy of the profile that drops
  `redirect-gateway`, `dhcp-option`, `block-outside-dns`, `register-dns`,
  `dev-node`, `resolv-retry`, and `route*` lines. Inline certificates stay.
  The operator's original file is not modified.
- After `--config`, pass `--route-nopull`, a short `--resolv-retry`, and
  `--pull-filter ignore` for the same pushed options.
- On Windows, pass `--windows-driver wintun` and `--dev-node` set to the
  helper's own `tun-<id>` name so OpenVPN creates that adapter instead of
  opening Mihomo's.

## Consequences

Importing a Windscribe profile no longer changes the system default route or
DNS. The side tunnel still comes up as its own Wintun adapter, and Mihomo
sends only the pinned egress through it. A profile that is actually a TAP
device is forced to TUN, which matches the owned-side-tunnel driver.
