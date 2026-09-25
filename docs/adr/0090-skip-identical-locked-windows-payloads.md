# ADR 0090: Skip identical locked Windows payloads

- Status: Accepted
- Date: 2026-09-25

## Context

Windows denies overwriting a running executable with `ERROR_SHARING_VIOLATION`.
Helper reinstall copied the packaged Mihomo binary onto the active
`C:\ProgramData\iran-split\bin\mihomo.exe` even when both files had the same
SHA-256. The generic helper I/O error made this look like a configuration-file
failure.

## Decision

Before copying helper payloads, compare source and destination length and
SHA-256. Skip the copy when they match, including when the destination is a
running executable that permits reads but denies writes. Keep copying when the
bytes differ so an actual upgrade still replaces the payload after the owning
process is stopped.

Rename the generic I/O message from "helper configuration I/O failed" to
"helper I/O failed" so unrelated file operations are not misidentified.

## Consequences

Reinstall succeeds while an identical Mihomo binary is running. A real binary
upgrade still requires the process to release the old executable before it can
be replaced.
