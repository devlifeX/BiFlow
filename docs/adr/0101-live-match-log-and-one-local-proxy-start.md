# 0101: Log the live MATCH and start every local proxy the same way

## Status

Accepted

## Context

A default-route change to Windscribe reloaded Mihomo, then the live diagram
went empty and new sites did not open. The controller error that explained an
earlier failed reload was clipped at 200 characters, and a successful reload
logged only HTTP 204. The running `MATCH` group and generation id were not in
`debug.log`.

Hiddify still had its own start function even though it is a local proxy like
Happ. On Windows the side-tunnel proxy also carried a Linux `routing-mark`,
and the OpenVPN adapter had no route that Mihomo could bind to, so unmatched
traffic and the DNS that follows `MATCH` had nowhere to go.

## Decision

- Keep the full Mihomo controller message (up to 4096 characters). After every
  hot reload, log the generation id and the live `MATCH` proxy from `GET /rules`.
- Start every `LocalProxy`, including Hiddify, through `start_local_proxy_client`.
- Do not emit `routing-mark` on Windows. Install a high-metric `0.0.0.0/0` on
  the side-tunnel adapter, using the OpenVPN `route-gateway` when the log
  reports one, so sockets bound to that adapter can leave without replacing
  the system default route. An existing route is success.
- When the default client is a side tunnel, measure the exit address through
  Mihomo's mixed port and store it as that client's exit IP.

## Consequences

- A reload that leaves `MATCH` on the previous client is visible on one log line.
- Hiddify is launched and probed like any other local proxy. Its executable
  search and stop-with-stack behavior stay.
- Unmatched traffic for a connected Windscribe tunnel can leave the adapter,
  so the live diagram and the exit address refer to that tunnel.
