# ADR 0067: OwnedSideTunnel invariants

## Status

Accepted

## Context

A side tunnel (OpenVPN today, WireGuard later) must carry selected prefixes
without installing a second system default route. A GUI that takes `0.0.0.0/0`
and a kill switch fights Mihomo’s TUN and breaks DIRECT Iran/LAN traffic.

## Decision

- The helper starts the tunnel binary. The desktop never runs it as root from
  a mutable workspace path.
- OpenVPN is spawned with `--route-noexec` and `--script-security 0` after
  `--config`. Profile audit rejects `up`, `down`, `plugin`, and extra
  `script-security`. A profile without a remote is an error.
- Helper-installed routes are scoped. `0.0.0.0/0` is rejected.
- Mihomo binds that instance’s `direct` outbound to the helper-owned device
  (Linux fwmark + policy table; Windows `interface-name`).
- The tunnel server `/32` is emitted `DIRECT` first so transport cannot
  recurse into the capture TUN.
- Side-tunnel rule-sets sit **above** `private-networks` so RFC1918 behind
  the tunnel is reachable. Loopback still wins and cannot be pinned there.
- A non-MATCH side tunnel may fail alone: Connect continues, that instance is
  degraded. If it is `default_route`, Connect fails.
- Windows spawn after `env_clear` restores `SYSTEMROOT` and uses
  `CREATE_NO_WINDOW`.

## Consequences

Products reuse this driver. They do not add `Outbound::OpenVpn`,
`openvpn_rules`, or a `starting_openvpn` stage.
