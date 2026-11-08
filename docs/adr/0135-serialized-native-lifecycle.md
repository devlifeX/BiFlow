# ADR 0135: Serialize native release and budget lifecycle replies

Status: Accepted (6.2.87)

## Context

The first native Windows release used textual `AdminStatus` fixtures. The actual
MSFT adapter property is numeric: 1 means Up and 2 means Down. Comparing it with
`Down` missed reenabling a retained adapter and repeated an unsupported disable.
The desktop also abandoned lifecycle replies after the five-second frame budget,
while bounded native work continued in the helper. A subsequent Connect could
then overlap that release and have its new tunnel disabled by the previous Stop.

## Decision

- Use numeric adapter administration state in production and executable fixtures.
  Reenable Down before start and skip an already completed disable. Keep GUID
  checks and native readback; accept only the known driver description and its
  Windows-added numeric suffix. Skip DNS reset when the interface's DNS state
  is already absent; a down Wintun device can retain an adapter without a DNS row.
- Give lifecycle replies a separate 90-second budget, and an atomic Restart twice
  that budget. Connection, frame IO, and routine status retain their short
  deadlines. Native commands remain independently bounded.
- A helper lifecycle mutex spans Start, Stop, Cleanup, Restart, and generation
  overlay, including the native release after the child lock is dropped. Cleanup
  calls the internal stop under that same lock. A successful verified Stop need
  not repeat native release inside Cleanup. Status stays independent of the
  lifecycle lock. The absence watchdog skips busy work and rechecks desktop
  contact while holding the lock before it can clean up.
- Native failures return only a whitelisted stage, never PowerShell stderr or
  network settings. Environment findings expose a disabled owned native adapter
  even when the controller still claims Running.

## Validation

The production PowerShell script runs against numeric in-memory adapter fixtures,
including reconnect, repeated release, and an already-disabled failure trap.
Another test verifies the production safe error framing. Public lifecycle methods
must wait behind a held native-release lock without executing network commands.
A duplex IPC test delivers an actual framed reply after the old five-second
deadline. Environment tests reject controller-only health for a disabled adapter.
No test disconnects the developer's network.
