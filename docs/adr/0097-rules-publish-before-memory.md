# ADR 0097: Publish rule documents before replacing in-memory state

- Status: Accepted
- Date: 2026-09-25

## Context

`RuleManager` kept a mutex-protected copy of `direct-rules.json`. Its mutators
updated that copy and then atomically published the file. If publication failed
(for example, a locked or invalid destination), the command returned an error
but subsequent commands observed the unpublished revision and pins. A restart
then silently restored the older disk state.

## Decision

All rule mutators construct a candidate from the current document while holding
the mutex. They publish the candidate first and replace the in-memory document
only after publication succeeds. No-op operations do not publish or advance the
revision. `restore` follows the same ordering. A regression test blocks the
destination path and asserts that failed add, list, and restore operations leave
the observed document unchanged on Windows and Linux.

## Consequences

Memory and disk stay aligned for a failed single-document publication. This
does not make settings and rules a cross-document transaction: the client
removal crash window in ADR 0096 remains open.
