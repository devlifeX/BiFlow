# 0080 Dev-profile Mihomo isolation

## Status

Accepted

## Context

`./dev.sh` writes `debug.log` under `biflow-dev-profile`, but both the
development profile and the installed app defaulted to
`127.0.0.1:19090` / mixed port `17890` / TUN `clash-iran` with
independent controller secrets. Connect from the dev profile then talked
to the already-running packaged Mihomo, got HTTP 401, and waited out the
20s readiness budget as if the controller were down.

The debug Linux helper path also fell back to
`/run/iran-split/helper.sock` when `BIFLOW_DEV_HELPER_SOCKET` was unset,
so a leftover Tauri window after `dev.sh` exited could drive the
production helper.

## Decision

- Treat HTTP 401 from the Mihomo controller as
  `MihomoError::Unauthorized` immediately. Do not retry it as
  "controller unavailable".
- Surface that as `CoreError::ControllerUnauthorized` with copy that
  tells the operator to quit the other BiFlow window.
- When `BIFLOW_DEV_PROFILE` is set, remap the production defaults to
  controller `19091`, mixed `17891`, DNS `2053`, and TUN `biflow-dev`,
  and persist them.
- `./dev.sh` reads TUN name from the dev-profile config (default
  `biflow-dev`), not from `~/.config/biflow/config.toml`.
- A debug build with `BIFLOW_DEV_PROFILE` set and no helper-socket
  override must not use the production helper socket.

## Consequences

- `./dev.sh` and the installed app can stay connected at the same time.
- A leftover dev window without the transient helper fails closed
  instead of configuring production Mihomo with the wrong secret.
