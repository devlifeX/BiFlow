# ADR 0087: Native PowerShell Windows builder

- Status: Accepted
- Date: 2026-09-25

## Context

`build.sh` is the shared release builder, but a native Windows checkout does
not require Bash and cannot directly execute the Linux-oriented helper staging
script. Windows developers need a one-command local build for the portable
executable and NSIS installer.

## Decision

Add `build.ps1` as the native Windows packaging entry point. It reads the same
root `version` and `scripts/build-plan.mjs` metadata as `build.sh`, builds the
Windows helper directly with Cargo, invokes the workspace Tauri CLI, supports
the `compile`, `nsis`, and `collect` stages, and writes the same artifact names
under `artifacts/windows`.

The script intentionally supports only Windows artifacts. Linux packaging and
Linux cross-compilation remain the responsibility of `build.sh` and CI.

## Consequences

Windows developers can run `./build.ps1` from PowerShell without Git Bash.
The script still requires the pinned Node, pnpm, Rust, and NSIS toolchains and
uses the same version-stability and resumable-stage checks as the shell
builder.
