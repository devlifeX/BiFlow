# 0079 Side-tunnel helper IPC timeout

## Status

Accepted

## Context

Windscribe and other `OwnedSideTunnel` clients start OpenVPN through the
privileged helper (`StartSideTunnel`). The helper may legitimately work for
tens of seconds while OpenVPN brings up a TUN device, retries through a SOCKS
proxy, and installs scoped routes. The command carries its own
`timeout_seconds` (default 45, max 300).

The desktop platform backends wait for one framed helper reply after sending
the command. On Windows the read deadline was a flat five seconds, so Connect
logged `helper.request_failed` with `"helper request timed out"` while OpenVPN
was still starting. Linux had a fixed sixty-second side-tunnel budget, which
did not track custom `start_timeout_seconds` values above sixty.

## Decision

- Centralize helper reply deadlines in `iran-split-ipc::helper_ipc_reply_timeout`.
- Keep five seconds for routine helper commands (status, Mihomo control, cleanup).
- For `StartSideTunnel`, wait `timeout_seconds + 15` seconds so the engine
  outlives the helper's OpenVPN budget plus route installation and reply
  framing.
- Platform backends still use the five-second frame budget for connect, hello,
  and write operations; only the reply read uses the command-specific deadline.

## Consequences

- Windscribe/OpenVPN side tunnels can finish Connect on Windows without a
  false IPC timeout.
- Operators who raise `start_timeout_seconds` get a matching IPC wait without
  widening every helper call.
- Regression tests in `iran-split-ipc` lock the short vs long budgets; platform
  source-contract tests ensure both backends call the shared helper.
