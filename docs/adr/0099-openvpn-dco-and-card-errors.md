# ADR 0099: OpenVPN 2.7 uses ovpn-dco, and client errors stay on the card

- Status: Accepted
- Date: 2026-09-25

## Context

OpenVPN 2.7 removed Wintun. A Windscribe profile that lists `AES-256-CBC` in
`ncp-ciphers` disables ovpn-dco, and the process then dies trying to open a
TAP adapter that is already in use. After authentication the server also
pushes `tcp-nodelay`, which 2.7 rejects as a fatal options error. The helper
reported only `exit code: 1` because that text is on stdout.

`--windows-driver wintun` and `--dev-node tun-<id>` (ADR 0092) no longer
create an adapter `netsh` can see. The adapter OpenVPN 2.7 opens is an
existing ovpn-dco name such as `OpenVPN Connect DCO Adapter`.

A failed per-client Connect wrote the global store error, so every click
opened the application dialog.

## Decision

- On Windows pass `--data-ciphers AES-256-GCM:AES-128-GCM` and do not pass
  `--windows-driver` or `--dev-node`. Let ovpn-dco open its adapter.
- Ignore a pushed `tcp-nodelay` the same way other route and DNS pushes are
  ignored.
- Pin `--script-security 2`. Level 0 blocks the `netsh` command OpenVPN 2.7
  uses to assign the tunnel address. The sanitized profile still has no
  `up` or `down` script.
- Treat `Initialization Sequence Completed` as ready, and use the adapter
  name from the `device […] opened` log line for scoped routes.
- An early exit includes the last stdout line.
- Per-client Connect and Disconnect failures stay on that card
  (`clientActionError`). They do not set the global error dialog.

## Consequences

Windscribe can finish startup on OpenVPN 2.7 without taking the system
default route. A failed card Connect shows the OpenVPN line on the card, and
repeating it does not stack dialogs.
