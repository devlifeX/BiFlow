# ADR 0088: Retry locked Windows helper configuration writes

- Status: Accepted
- Date: 2026-09-25

## Context

The elevated Windows helper rewrites `C:\ProgramData\iran-split\helper.toml`
while reinstalling the scheduled task. Windows can briefly report
`ERROR_SHARING_VIOLATION` or `ERROR_ACCESS_DENIED` when Defender or the
previous helper still has the machine-wide file open. A single write turns
that transient condition into a failed Helper install.

## Decision

Write the helper configuration with a bounded retry loop. Retry only Windows
sharing and access errors, wait briefly between attempts, emit a structured
retry event, and return the original I/O error when the bounded budget is
exhausted.

## Consequences

Reinstall tolerates short-lived file locks without hiding a persistent
permission or filesystem failure. A failed install remains diagnosable from
the original error and the structured retry events.
