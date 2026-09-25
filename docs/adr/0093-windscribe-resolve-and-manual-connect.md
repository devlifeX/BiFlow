# ADR 0093: Resolve Windscribe through Hiddify, then connect it by hand

- Status: Accepted
- Date: 2026-09-25

## Context

Windscribe's server name is filtered. After Hiddify is up, the app used to
look up the real address and bypass it. That lookup became best-effort: a
failed `DoH` query fell through to the hostname, OpenVPN retried forever,
and the helper reported only that the tunnel did not come up. Once Mihomo's
TUN was already running, `openvpn.exe` was not a process bypass, so the
dial entered the tunnel. On Windows, `--dev tun-<id>` is not an adapter
name, so a session that did connect was invisible to the readiness check.

Operators also had no way to start one dropped tunnel without disconnecting
the whole stack.

## Decision

- Resolve the profile hostname through a ready local proxy. If that proxy
  exists and `DoH` fails, refuse to start OpenVPN and show the resolve error.
- Record the resolved host as a `/32` exclude and emit `openvpn.exe` /
  `openvpn` as a DIRECT process bypass before the helper spawns the process.
- On Windows use `--dev tun` with `--dev-node tun-<id>`, and
  `--connect-retry-max 2`, so a blocked address fails into the SOCKS fallback
  instead of hanging for the whole budget. Include the last OpenVPN log line
  in the timeout error.
- Each client card has Connect and Disconnect. Connect starts that side
  tunnel while the stack is up. Disconnect stops that tunnel only. A local
  proxy still starts and stops with the stack, so its Disconnect stays
  disabled.

## Consequences

A dropped Windscribe tunnel can be started again from its card. A filtered
name fails immediately with a resolve error instead of a 60 second timeout
that hides the OpenVPN log.
