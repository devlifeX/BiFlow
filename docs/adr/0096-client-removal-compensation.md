# ADR 0096: Compensate client removal when route updates fail

- Status: Accepted
- Date: 2026-09-25

## Context

Client settings and direct-rule pins are persisted by separate backend
operations. The client registry previously attempted the pin move/discard first,
then removed the client even when the settings write had failed. That could
leave the UI and persistent settings disagreeing, or remove routing intent
before confirming that the client itself could be removed.

## Decision

- Make settings persistence return an explicit success value to registry
  workflows; a failed settings write must stop dependent mutations.
- On client removal, persist the settings change before reassigning or
  discarding pins. If the dependent rule operation fails, restore the client
  using the latest settings revision and keep the delete dialog open.
- Report whether restoration also failed so the operator knows to reload
  settings before retrying.
- Treat the compensation sequence as an interim consistency measure, not as a
  transaction across settings and rule documents. A process crash between the
  writes can still leave the documents inconsistent; R2 remains open until a
  backend transaction or crash-recovery journal is implemented and tested.

## Consequences

Routine storage and rule-operation failures no longer silently advance the
client registry workflow, and recoverable route failures restore the client.
The crash window is documented and remains an explicit reliability gap; this
ADR does not authorize marking transactional client updates complete.
