# 0121: Restore macOS DNS without writing the tunnel resolver back

## Status

Accepted

The LAN-gateway fallback below is superseded by
[ADR 0134](./0134-native-network-release.md). Restoration now preserves the
original static resolvers or DHCP and retains recovery until verification.

## Context

Disconnect on macOS reported a successful DNS restore and then every site
failed. Mihomo had exited, so nothing answered `127.0.0.1:53`, but the
system resolver was still that address. A direct TCP connection to an IP
could succeed while `google.com` could not.

The helper stores each network service's DNS in `system-dns.toml` before
pointing those services at `127.0.0.1`. Restore writes that file back and
deletes it. If a previous session captured `127.0.0.1` (the resolver was
already ours, or `networksetup` reported success while `scutil --dns` stayed
on the tunnel), the next disconnect installs `127.0.0.1` again and the
following connect saves it once more. Split routes via `198.18.0.1` on the
TUN can also outlive the process when the child handle was lost or Mihomo
did not remove them.

## Decision

- A loopback address is never a saved resolver. Capture and restore drop
  `127.0.0.0/8` and `::1`. A service left with no servers is restored with
  `networksetup` `Empty` (DHCP).
- After that write, read `scutil --dns` and each service. If the live
  resolver is still loopback, set every restored service to the private IPv4
  default gateway. Do not use `198.18.0.1` or a public gateway. If no private
  gateway exists, keep the recovery file and report the failure.
- Delete the recovery file only after the live resolver is not loopback.
- On stop, kill a scoped Mihomo orphan (the same command-line match as
  start) and delete `netstat` routes whose interface is this helper's TUN,
  including a default route that still points at that interface. Leave the
  physical interface's default route in place.

## Consequences

- Disconnect returns name lookup to DHCP, or to the LAN gateway when DHCP
  does not clear a stuck `127.0.0.1`.
- A custom resolver that was only `127.0.0.1` before Connect is restored as
  DHCP. A saved non-loopback resolver is kept.
- The gateway address is not written to `helper.log`.
