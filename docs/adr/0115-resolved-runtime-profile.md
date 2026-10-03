# 0115: One resolved runtime profile owns every profile-dependent resource

## Status

Accepted in 6.2.60.

## Context

`BIFLOW_DEV_PROFILE` was read independently in seven places, and the site
that actually _changed_ behavior was a reusable library.

- `iran-split-config::MihomoConfig::isolate_from_installed_app` read the
  environment from inside `ConfigStore::load`. That is the one place where
  isolation is enforced — it remaps the controller, mixed, and DNS ports and
  the TUN name — yet it was untestable, could not be controlled by an
  embedder, and made the workspace test suite depend on the ambient
  environment of whatever machine ran it.
- `AppPaths::discover` rebuilt config/data/cache, and
  `diagnostics::default_log_path` rebuilt `debug.log`, from the same variable
  with no link between them. `debug.log` and the config file agreed by
  coincidence.
- The Linux and macOS helper-path resolvers were near-duplicate functions that
  each re-read the variable.
- `environment::apps::install_info` and the `install_helper` production-refusal
  guard each re-derived profile identity on their own.

Two isolation defects followed from the drift:

1. A development run had no single answer to "which helper do I talk to", so a
   missing `dev.sh` override silently substituted an unreachable placeholder
   with nothing logged.
2. Windows had no development override _at all_. A development run isolated
   its config, ports, and TUN, but then drove the installed SYSTEM helper and
   staged generations into the machine-wide, privileged
   `C:\ProgramData\iran-split\staging`. `dev.sh` provisions a per-user
   transient helper on Linux only, so Windows could not supply a real
   alternative — but it also did not need to inherit the production one.

3. The `install_helper` Tauri command refused a development run, but
   `helper_install::install_helper` — the function itself — had no such guard,
   and `prepare_stack_start` calls it directly for a missing helper and for a
   version-mismatch reinstall. With Windows pointing at a deliberately
   unreachable pipe, a development Connect would classify the helper as
   unavailable and enter the production SYSTEM installer. The guard sat one
   level above the only place that mattered.
4. The Windows installer hardcoded `WINDOWS_HELPER_STAGING` instead of using the
   resolved staging policy, so the installer wrote to the privileged directory
   even where the backend did not.
5. A missing Mihomo override fell back to the installed binary, so a
   development run could execute `C:\ProgramData\iran-split\bin\mihomo.exe`
   against development configuration.
6. An unavailable host directory silently became `"."`, so a production run
   would have written its configuration document and permanent `debug.log`
   relative to the working directory.
7. The `generation_staging_for` test asserted against the Windows constant
   unconditionally. That constant is `cfg(any(windows, test))`, so the test
   _compiled_ on Linux and macOS and failed on the value — a contradiction no
   Windows host run could reveal.
8. The Mihomo policy ran _after_ dependency discovery. `mihomo_candidates`
   includes `PATH`, so an installed `mihomo.exe` was selected before the
   profile rule was consulted, and testing the policy function in isolation
   proved nothing because the call order was the defect.

## Decision

- `iran-split-config` gains `profile.rs` as the single authoritative
  representation. `ProfileEnv` is the only place the workspace reads the
  profile environment; `ResolvedProfile::resolve` turns it, plus
  caller-supplied base directories, into one resource policy.
- The policy distinguishes three classes of resource: unprivileged user state
  (`UserResources`), privileged development-only overrides
  (`PrivilegedOverrides`), and packaged read-only assets, which stay with the
  Tauri resource directory. Privileged overrides are never relocated into a
  mutable workspace, and ownership, permissions, peer authentication, and
  executable validation are unchanged.
- `RuntimeProfile::resolve` keeps the documented semantics exactly: unset and
  empty mean production, nonempty means a development run rooted there. A
  development run that is missing a privileged override reports the missing
  variables instead of inheriting production, and resolves to a per-platform
  unreachable placeholder.
- `ConfigStore::new` takes an explicit `RuntimeProfile`. The library no longer
  reads the environment, so the workspace tests no longer depend on ambient
  `BIFLOW_DEV_PROFILE`.
- `src-tauri/src/profile.rs` is the process boundary: it resolves once into a
  `OnceLock` before Tauri state exists and hands the policy to consumers.
- `src-tauri/src/platform_paths.rs` owns the per-OS locations. All three
  platforms route through one `profile::helper_paths` call, so "a development
  run never inherits a production location" has one implementation.
- The development-profile refusal lives **inside**
  `helper_install::install_helper`, the single privileged provisioning entry,
  rather than at its callers. `profile::ensure_provisioning_allowed_for` is a
  pure decision so the refusal is testable, and it names the refused route in
  both the log event and the operator message. Two source contracts in
  `scripts/dev-profile-contract.test.mjs` fail the build if a per-platform
  elevation routine is called from anywhere else, or if the gate stops being
  the installer's first statement.
- The Mihomo binary choice is a pure `mihomo_binary_for` that receives the
  **discovered** candidate as an input. A development run uses the `dev.sh`
  binary, and with no override returns a path inside the user's data root so
  the dependency reads as missing — never a discovered production binary, not
  even one found on `PATH`. The override is honoured only in a development
  run, matching `PrivilegedOverrides`.
- A staging test must compare against the platform's own production constant.
  `PRODUCTION_GENERATION_STAGING` is defined for `test` on every host, and
  `generation_staging_for` is compiled under `cfg(any(windows, test))` so the
  development rule is exercised everywhere while staying out of a non-Windows
  `-D warnings` build.
- An unavailable host directory is a hard error, not `"."`. `try_resolved`
  reports it and `resolved` panics with the cause; a development run still
  resolves because it discards the base directories entirely.
- Windows generation staging follows the same rule: production uses the
  installer-recorded root in `helper.toml`, a development run stages under its
  own unprivileged profile, and the **installer** now resolves that staging
  through the same policy rather than hardcoding it.
- Every policy that can be tested directly has a pure `*_for` core taking
  explicit inputs, so no assertion depends on process-global environment. This
  is not stylistic: the suite must pass both bare and with
  `BIFLOW_DEV_PROFILE` set, which is how this project is normally developed.
- `dev.sh` cannot share the Rust implementation, so its variable names, export
  ordering, and subdirectory layout are pinned by
  `scripts/dev-profile-contract.test.mjs`, as is the rule that only the two
  profile modules may read the profile environment.
- `create_services` verifies that the open `debug.log` is the one the resolved
  profile owns, and records a `profile.diagnostics_path_mismatch` error if a
  future change reintroduces an independent path decision.

- The platform-constant source contract consumes cfg attributes at the module
  and item boundaries. `cfg(test)` and `cfg(any(windows, test))` do not exempt
  cross-platform tests. A negative test restores the original incorrect
  assertion in the first staging test in memory and requires its detection;
  additional fixtures protect attribute scope and explicit Windows-only tests.

## Linux CI follow-up (6.2.61)

The extraction left two Linux-only tests calling removed root-module symbols
and a duplicate glob import. Production-path assertions now live beside the
Linux constants in `platform_paths`; explicit development overrides exercise
`profile::helper_paths_for` on every host. A source contract scans the complete
desktop source, including inactive cfg branches, for the retired identifiers.
Linux validation uses native Ubuntu 22.04 Rust inside WSL in addition to the
Windows commit gate.

Path-policy fixtures that construct `AppPaths` create directories. They now
use separate roots inside disposable temporary directories rather than fake
absolute `/host` paths, so Linux tests run as an ordinary user and leave no
shared profile state behind.

## Consequences

- Development and production isolation is now testable without touching
  process-global environment, and `debug.log` agreement with the application
  data root holds by construction rather than by convention.
- The Windows development path stays non-functional by design. It reports an
  isolated helper as unavailable instead of reaching into the installed
  app's privileged state. Provisioning a real per-user Windows development
  helper remains future work; `dev.sh` remains the supported development
  entrypoint.
- `PlatformBackend` and `Engine` are unchanged. The profile policy sits below
  them, in the crate that already owned the profile constants, so no competing
  abstraction was introduced.
- The reusable `iran-split-config` crate gained no dependency on Tauri, on a
  directory-locating library, or on a new workspace crate.
