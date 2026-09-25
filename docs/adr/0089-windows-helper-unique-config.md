# ADR 0089: Use unique files for locked Windows helper artifacts

- Status: Accepted
- Date: 2026-09-25

## Context

Some Windows installs keep the previous machine-wide `helper.toml` or
`helper-task.xml` open long enough that bounded retries still end with
`ERROR_SHARING_VIOLATION`. Reusing either path makes an otherwise valid helper
reinstall fail.

## Decision

Each Windows helper install writes timestamped configuration and Task XML files
under `C:\ProgramData\iran-split\runtime` and registers the scheduled task
with those files. The legacy `helper.toml` and `helper-task.xml` are left
untouched when another process has them open.

## Consequences

Reinstall can complete even when the previous configuration is persistently
locked. Old configuration files remain on disk and can be removed later by a
dedicated cleanup flow once no task references them.
