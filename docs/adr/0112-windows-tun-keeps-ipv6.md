# 0112: Windows TUN keeps IPv6 so strict-route does not block `::1`

## Status

Accepted. Corrects ADR 0108 and the Windows `ipv6: false` choice in ADR 0038.

## Context

On a Windows 11 host, `localhost:<port>` failed while connected. That
covered Docker Desktop ports, dev servers, and Hiddify's own port. The
host resolves `localhost` to `::1` first. The environment snapshot's
loopback self-test (ADR 0110) showed the change caused by Connect:

| probe                 | stopped | running                    |
| --------------------- | ------- | -------------------------- |
| ephemeral `127.0.0.1` | ok      | ok                         |
| ephemeral `[::1]`     | ok      | refused immediately (0 ms) |
| Hiddify `[::1]:12334` | ok      | refused                    |

ADR 0108's `route-exclude-address: ::1/128` could not help, because the
connections are not being routed: they are refused. With top-level
`ipv6: false`, Mihomo's `parseIPV6` clears `tun.inet6-address`. When
`strict-route` is on and there is no inet6 address, sing-tun adds a WFP
filter named "block ipv6" on `FWPM_LAYER_ALE_AUTH_CONNECT_V6`. The filter
has no conditions. The only permit filter above it is for Mihomo's own
application ID, so every other process loses all IPv6, loopback included.

## Decision

- Generate top-level `ipv6: true` on Windows, as Linux already does.
- Emit `tun.inet6-address: fdfe:dcba:9876::1/126` explicitly so a future
  Mihomo build that applies it will keep the inet6 address on the TUN.
- Keep `dns.ipv6: false` on Windows. Fake-ip answers stay IPv4-only, so
  domain routing is unchanged.
- **Set `strict-route: false` on Windows.** Mihomo v1.19.29 does not apply
  `inet6-address` to the Wintun adapter — `GET /configs` reports
  `inet4-address` with no `inet6-address`, and the adapter only gets a
  link-local `fe80::` address. With `strict-route: true` and no inet6
  address, sing-tun installs the unconditional WFP "block ipv6" connect
  filter that only exempts Mihomo, so `localhost` -> `::1` is refused while
  connected. Until a Mihomo build that actually sets the inet6 address is
  available, Windows must run with `strict-route: false` so loopback IPv6
  works. `dns.ipv6: false` already keeps AAAA out of fake-ip, so the IPv6
  leak surface is minimal.
- Keep the loopback route exclusions (`127.0.0.0/8`, `::1/128`) for when
  `strict-route` can be re-enabled.
- IPv6 literal traffic (for example a browser's own DoH AAAA answers) now
  enters TUN and follows the rules instead of being dropped.

- Kubeconfig clusters in the snapshot also record their address family,
  a 3-second TCP connect result from the desktop, and the file names of
  `exec` credential plugins. `kube_cluster_unreachable` then tells "BiFlow
  breaks the cluster" (fails only while running) apart from "the cluster
  is unreachable from this network" (fails while stopped too).

## Consequences

`localhost` works while connected on hosts that prefer `::1`. A future
regression shows up as `loopback_ipv6_blocked` in the snapshot. Findings
no longer flag BiFlow's own split-default routes, one IPv4 plus one IPv6
default route, or a SYSTEM helper task that a standard user cannot list
while the helper is reachable.

### Orphaned Mihomo reclamation (6.2.54)

A second root cause kept the fix from taking effect on an upgraded
machine. `spawn_mihomo` uses `kill_on_drop(false)` so connectivity
survives a helper crash, but when the helper itself restarts (machine
reboot, helper reinstall, or a new desktop session), `self.child` is
empty and the previous Mihomo is an orphan still holding the controller
port and the TUN adapter. The new Mihomo then fails to bind
`127.0.0.1:19090`, the desktop silently talks to the stale process, and
the new `inet6-address` config is never applied — `GET /configs` keeps
reporting the old `inet4`-only TUN.

Before every spawn the helper reclaims only processes whose command uses
its exact executable and `-d` / `-f` generation paths below its own runtime.
On Windows it also verifies `Win32_Process.ExecutablePath` before stopping
the matching PID. Unix `pkill -f` uses an anchored, escaped command pattern;
exit 1 means no owned process exists. Other failures stop Connect and emit
an audit event. Name-only `taskkill /IM` and `pkill -x` are forbidden because
they also kill the other dev/production profile and unrelated VPN clients
(ADR 0114).

### Sniffer override-destination and kubectl EOF (6.2.56)

A third root cause kept `kubectl` from reaching a cluster API while
connected. The cluster server `78.109.203.123:443` is in
`iran-networks` (78.109.192.0/20) and `kubectl.exe` is in the process
bypass list (`PROCESS-NAME,kubectl.exe,DIRECT`), so the connection
should have gone DIRECT. But the sniffer had `override-destination:
true`: when kubectl connected to the IP and sent a TLS SNI
(`tls-server-name` from kubeconfig), the sniffer resolved the SNI
hostname through fake-ip DNS and overrode the destination to the
resulting 198.18.x.x fake-ip. The DIRECT outbound then connected to
the fake-ip, which is not a real server, and the TLS handshake failed
with EOF in 3 ms. The connection never appeared in the live
connections list because the override happened before rule evaluation.

Setting `override-destination: false` keeps the original destination
IP while still allowing domain-based rules to use the sniffed SNI. The
PROCESS-NAME,DIRECT rule then connects to the real cluster IP.
