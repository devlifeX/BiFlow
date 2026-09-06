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

## Progressive connect UX (6.2.5)

Connect passes an explicit side-tunnel start budget on each attempt:

1. First Connect uses **15 seconds**.
2. If an enabled side tunnel is still stopped afterward, the client registry
   shows **Try again with 30s timeout**.
3. After another failure, the button offers **60 seconds**; further retries
   stop with an exhausted message.

`retry_side_tunnels` re-starts failed side tunnels on a live stack without a
full disconnect, and the IPC reply wait stays `timeout_seconds + 15` via
`helper_ipc_reply_timeout`.

## Per-component connect progress (6.2.6)

Connect no longer waits until the end to paint every component green. The
engine publishes partial `StackSnapshot` updates as real readiness arrives:

- Helper moves to `checking`, then `running`, as soon as the helper probe
  succeeds.
- Each enabled client flips to `running` when its egress handle is registered;
  platform backends commit `egress_handles` after every client instead of only
  at the end of `ensure_clients`.
- While `ensure_clients`, `check_readiness`, and `confirm_core_and_tun` run,
  the engine polls `runtime_health` and merges component status without
  downgrading in-flight `starting` states.
- Mihomo, TUN, and DNS advance independently during core start and readiness.

The mock transport mirrors the same staged snapshot updates for Playwright and
unit tests.
