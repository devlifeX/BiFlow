# ADR 0100: Reset live connections when the default route reloads

- Status: Accepted
- Date: 2026-09-25

## Context

Changing the unmatched default from Hiddify to Windscribe, then restarting
Mihomo, reloads the `MATCH` group. Existing sockets stay on the previous
outbound. The live diagram polls those sockets, so it keeps drawing them on
Hiddify. The "Default for unmatched" caption also sat above the selected
node and read as a label on the client above it.

## Decision

- A settings reload with no single-host rebind closes every Mihomo connection
  after the hot reload. New sockets follow the reloaded default. A pin move
  still closes only the moved host.
- The reload `PUT` sends the overlaid `config.yaml` bytes as `payload`.
  An empty `path` returns success and leaves the previous `MATCH` in memory.
  On Windows, a nested absolute path under the runtime home is still rejected
  as outside `SAFE_PATHS`, so the same generation files are also copied to the
  runtime root where that home resolves relative provider names.
- The diagram drops a host's packet as soon as a newer poll places that host
  on another branch, and it clears its animation when the default client
  changes. The default caption is drawn under that node's own name.

## Consequences

After Restart Mihomo, unmatched traffic and the diagram both move to the
selected default without waiting for the old sockets to idle out.
