# BiFlow agent rules

Follow these rules in every change. If a rule is missing or a new failure mode appears, update this file in the same change.

## Done gate (hard rule)

A change is **not done** until the parts you touched build and test with **zero failures and zero warnings from project code**. Do not report the task complete, skip the gate, or leave a warning or red command.

**Commits are gated.** `.githooks/pre-commit` runs the CI-mirror checks
(`cargo fmt --all --check`, workspace Clippy with `-D warnings`, workspace
tests, `cargo deny check`, `pnpm check`) on the areas a commit touches.
Activate it once per clone with `git config core.hooksPath .githooks`.
Never commit with `--no-verify` to skip a failing check; fix the failure.
A red GitHub Actions run on a plain push is a process failure, not bad luck.

After every change, run only what that change affects:

1. Frontend (TypeScript, UI, scripts, or package manifests): `pnpm check` and `pnpm build`
2. Rust (`.rs`, crate `Cargo.toml`, or `Cargo.lock`): `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets -- -D warnings` for each **changed workspace crate**. Cargo already rebuilds only dirty units. Do **not** `cargo clean`. Do **not** run `cargo test --workspace` and `cargo build --workspace` after every task.
3. Rust formatting: after any `.rs` change, run `cargo fmt --all --check` **before** reporting the change done. Clippy and tests do not prove rustfmt. If the check fails, apply `cargo fmt --all` and re-check. CI runs this first on Linux and Windows.
4. User-visible UI (layout, chrome, Dashboard, Diagnostics, Settings, or any screen in `docs/screenshots/`): refresh the README shots in the same change with `BIFLOW_CAPTURE_README=1 pnpm exec playwright test e2e/readme-screenshots.spec.ts`. Update README alt text if a shot's subject changed. A UI change is not done while the landing-page images still show the previous layout.

If many crates or workspace dependencies changed, use one incremental `cargo test --workspace` plus `cargo clippy --workspace --all-targets -- -D warnings`. Do not follow them with a second full `cargo build --workspace`.

If Node, pnpm, or Cargo is missing, install it first (see `./build.sh`) and re-run. A missing toolchain is not a pass.

If a required command fails or emits a warning from project code, fix it in the same change and re-run that command. Do not hide warnings with a broad `allow`; a narrow suppression is acceptable only when the condition is intentional and documented at the suppression site. Hundreds of `Compiling` lines mean a cold `target/` cache, not a required full rebuild.

## Testing

- Add or update an **e2e test** for every primary user flow (install missing apps, connect/disconnect, cloud rule sync, custom direct rules, diagnostics DIRECT vs VPN).
- Add or update **unit tests** for core logic (Rust crates, especially `iran-split-core`, rules, and installers) and UI logic (store, mock transport, React screens).
- `pnpm test` runs UI unit tests. `cargo test --workspace` runs core unit tests. `pnpm test:e2e` runs Playwright against the mock UI.
- Do not merge a behavior change that has no covering unit test, and do not add a primary flow without an e2e assertion.

## Rust diagnostics

- Read the current per-user `biflow/debug.log` first when diagnosing a runtime report. It is permanent newline-delimited JSON across application sessions and is also included by **Diagnostics → Export**. Do not assume it exists before the app has launched, and do not commit a generated `debug.log`.
- Every new or changed desktop user action, background task, platform/helper boundary, warning, ignored failure, and error path in Rust must emit structured `tracing` events that reach `debug.log`. Reuse the command trace helpers in `src-tauri/src/diagnostics.rs` at Tauri boundaries. The separately privileged helper must emit the same safe fields to its service journal; the desktop records each helper request and result in `debug.log` without making the root service write into a user's data directory.
- Action events must identify `event`, `section`, `initiator`, `cause`, and `trace_route`; use a stable operation/request UUID as `trace_id` when available. Never log passwords, controller secrets, tokens, credentials, subscription links, raw URLs, full settings, direct-rule values, diagnostic targets, or unredacted user content.
- Do not ignore fallible Rust operations with `let _ =` unless failure is provably irrelevant. Log the warning/error and cause when execution continues, and preserve a regression test for diagnostic lifecycle, redaction, and file-size behavior.
- `debug.log` stays in append mode across process launches and is flushed after every event. Do not truncate, rotate, cap, or delete it automatically on startup or shutdown. Only the explicit Diagnostics delete action clears its contents; it must keep the active file handle usable and resume logging immediately.

## Errors and lessons

- When you hit an error and fix it, record the cause and the fix under [Lessons](#lessons) so the next agent does not repeat it.
- Prefer a regression test alongside the lesson.

## Architecture decisions

- Record non-obvious choices as ADRs in `docs/adr/`.
- After each behavioral or architectural change, add a new ADR or update the existing one. Keep `docs/adr/README.md` current.

## UI design system (hard rule)

- `docs/DESIGN.md` is the binding UI contract: compact card density, the
  footer-button pattern for action cards, equal-height tiles, usage-ordered
  sections, horizontal rule-list rows, collapsible client cards, the shared
  per-client color palette, sticky apply banner, themed scrollbars, and the
  per-page boot skeleton.
- Read it before touching any component; do not regress a rule silently. If
  a rule must change, change `docs/DESIGN.md` in the same commit.

## Version

- The only version source is the `version` file in the repository root (semver `X.Y.Z`).
- Initial version is `1.0.0`. Increase it after every user-facing or process change (`1.0.1`, `1.1.0`, …).
- Do not edit version strings in `package.json`, `Cargo.toml`, or `tauri.conf.json` by hand. Run `pnpm version:sync` (also runs at the start of `pnpm check` and `pnpm build`) so those files follow `version`.
- Runtime UI and Tauri bootstrap read `version` directly (`__APP_VERSION__` / `include_str!`).

## Lessons

- `cfg(windows)` code and tests never compile on the Linux host, and a
  full `--target x86_64-pc-windows-msvc` clippy from Linux dies in `ring`'s
  build script. When changing anything the Windows crates assert on
  (generated YAML strings, group names, spawn helpers), grep the
  `iran-split-platform-win` and `iran-split-helper` test modules for the
  old strings before pushing — CI's Windows job is the first compiler
  those files ever see. Put generated-config assertions in
  `iran-split-mihomo` (cross-platform, takes `Platform::Windows`) instead
  of duplicating them under `cfg(windows)`.
- tokio's `process::Command` exposes `creation_flags` inherently on
  Windows; importing `std::os::windows::process::CommandExt` for it trips
  `-D unused-imports` (and `items-after-statements` if placed mid-body).
  The std trait import is only needed for `std::process::Command`.
- Dev and release builds must never share a profile: config schema
  migration is one-way and the rules document format moves, so one dev run
  bricked the installed app twice (config.toml `UnsupportedSchema`, then
  `direct-rules.json` missing legacy `rules` field). `dev.sh` exports
  `BIFLOW_DEV_PROFILE` and every path — config, data, cache, debug.log,
  and the helper `staging_dir` — must derive from that one variable; the
  helper staging path silently diverging produced `INVALID_GENERATION`.
- A document another build wrote must never keep this build from
  starting: `RuleManager::load` quarantines undecodable JSON to
  `.corrupt` and starts empty instead of failing the Tauri setup hook.
- A dev-profile app must never run the production helper installer: when
  the transient dev helper is gone (dev.sh exited but the window
  survived), the Install button ran the packaged 4.7.2 pkexec installer
  with the dev staging dir and reconfigured the SYSTEM helper against the
  dev profile. `install_helper` refuses under `BIFLOW_DEV_PROFILE` and
  points at ./dev.sh; recovery is one Install click from the installed
  app (it rewrites helper.toml with production paths).
- `open_external_url` is allowlist-gated. New UI links fail silently as
  "URL is not allowlisted" unless the allowlist grows with them; derive it
  from the preset catalog so it cannot drift from the buttons.
- An HTML file input does not expose a filesystem path in the Tauri
  webview. Side-tunnel profiles (OpenVPN, Windscribe, later WireGuard)
  must be chosen through `pick_client_profile` so the helper reads the
  original file and relative `ca` / `cert` references still resolve. Do
  not log the chosen path.
- `blocking_pick_file` from a sync Tauri command deadlocks the GTK/WebKit
  main thread. GNOME then offers Force Quit while the window looks hung.
  Open the dialog with the callback picker from an async command and await
  a oneshot. Never block the invoke thread on the file dialog.
- An inner timeout must exceed the operations it wraps: the 5s live-apply
  budget raced `validate_with_binary`'s own 10s, so every pin move timed
  out and silently restored the previous document ("moves don't apply").
- `StartSideTunnel` can legitimately run for the command's
  `timeout_seconds` while OpenVPN starts. The desktop helper IPC client must
  wait `timeout_seconds + margin` for that reply only; a flat 5s read on
  Windows turned every slow Windscribe start into `helper request timed out`.
  Use `iran-split-ipc::helper_ipc_reply_timeout` on both platform backends.
- `sr-only` labels are absolutely positioned; without a positioned
  ancestor they anchor to the page and extend
  `documentElement.scrollHeight` once their form scrolls below the fold,
  failing the no-document-overflow e2e. Prefer `aria-label` on the input.
- A link rendered inside a `disabled` button never receives clicks;
  catalog rows must keep the download link outside the add-client button.
- Preset defaults are guesses until verified on a real install: Happ runs
  an Xray core listening on 10808 (not 3067), and its process bypass must
  include `xray`/`v2ray` or the core's own egress loops back into the TUN.
  Debian installs `/usr/bin/happ` → `/opt/happ/bin/Happ`; a case-sensitive
  PATH lookup for `Happ` reports "executable was not found". Search
  well-known paths and ignore filename case. `google.com` is a debug-only
  reachability/live-host label — release builds still route it, they just
  omit the hostname from the UI.
- Windscribe rides the OpenVPN driver. The missing-binary banner and
  catalog row must offer the OpenVPN installer page, not only the
  Windscribe config generator. Do not tell operators to run the Windscribe GUI.
- Backticks inside a double-quoted shell search pattern are command
  substitutions. Quote `rg` patterns with single quotes when they contain
  Markdown code spans so validation does not accidentally execute the text.
- Large `apply_patch` edits against actively changing or freshly formatted files can miss shifted context. Split lifecycle, command, and e2e instrumentation into narrow patches against freshly inspected line ranges; a failed patch applies no partial changes.
- A Rust raw byte string containing `\n` stores a backslash and `n`, not a newline. Diagnostic JSONL tests must use a real newline byte so they exercise redaction instead of the malformed-event fallback.
- Playwright `getByText` can match both a heading and descriptive text containing the same phrase. Select diagnostics cards with an exact heading role.
- Playwright `getByText("Reachable")` is a substring match, so it also hits every "Unreachable" label. Pass `{ exact: true }` when one status label is a suffix of another.
- An isolated `CARGO_TARGET_DIR` can consume enough disk to make a later workspace link fail with `No space left on device`. Remove only the known disposable isolated target; never use `cargo clean` on the shared incremental cache.
- In the managed sandbox, `pnpm version:sync` can fail with `spawn EPERM` when pnpm launches the configured Node binary. Re-run the same synchronization command with approved execution; do not bypass the root `version` source by hand-editing generated manifest versions.
- Structured audit calls can push an existing Rust handler over Clippy's `too_many_lines` limit. Extract request execution plus its start/result audit events into a focused helper instead of suppressing the warning.
- Diagnostics contains several independent live regions, so Playwright `getByRole("status")` is ambiguous after multiple actions. Assert the unique result text or scope the locator to the relevant card.
- Cross-target Clippy sees only the active `cfg` branch; a helper that can fail only on Linux may look unnecessarily wrapped on Windows. Prefer a total cross-platform helper when a safe fallback exists, and validate both host and `cargo xwin clippy` targets.
- A native Tauri dev launch is not operational when its privileged helper is absent. `dev.sh` must prepare and verify the helper boundary before starting the UI, and must keep the root helper's executable/config/runtime outside the mutable workspace.
- Shell EXIT and signal traps must not invoke privileged cleanup twice. Convert INT, TERM, and HUP to exit statuses and keep one EXIT cleanup handler; put per-user dev locks below the private user runtime directory, not shared `/tmp`.

- Older Pillow has no `Image.Resampling`; generate icons with `Image.LANCZOS` / `Image.BICUBIC`.
- Inner `#![allow(...)]` attributes must be the first item in a Rust module, before `use`.
- Vitest coverage config requires `provider: "v8"` (or `istanbul`) in this Vite version.
- `let unsubscribe = () => undefined` is typed `() => undefined` and cannot store a `() => void` unlisten function; annotate `let unsubscribe: () => void`.
- Playwright e2e against Vite mock state must call `window.__BIFLOW_RESET_MOCK()` and reload, because the mock module keeps process-wide state.
- `__APP_VERSION__` must be declared inside `declare global` in `vite-env.d.ts`. That file is a module (`export {}`), so a top-level `declare const` is not visible to `tsc`.
- Vitest does not enable Testing Library auto-cleanup when `globals` are off. Call `cleanup()` in `src/test/setup.ts` `afterEach`. Do not import `store/app` from setup: that loads `desktop` before `vi.mock` and breaks store unit tests.
- Typed ESLint (`recommendedTypeChecked`) must be scoped to `*.{ts,tsx}` and those files must be in a tsconfig `include`. Ignore `eslint.config.js` / `postcss.config.js`. Mock async methods and Vitest spies need `require-await` / `unbound-method` off in those files.
- The diagnostics **Test flow** button stays disabled until the target field is non-empty.
- The Zustand store is a process singleton. App tests that change `page` must reset store state in `beforeEach`, or the next test stays on Settings and never sees the dashboard heading.
- `getByRole(..., { name: "Install" })` substring-matches **Installing…**. Use `{ name: /^Install$/ }` in Vitest and `{ exact: true }` in Playwright.
- Playwright `getByRole("button", { name: "Connect" })` also matches the status-bar **Internet connected** control. Use `{ name: "Connect", exact: true }`. After click the accessible name becomes the current stage, so keep asserting the same control with `[data-connection-action='connect']`. Stage labels last only a few hundred milliseconds, so record them with a `MutationObserver` instead of sequential `getByRole` name waits. Basic mode has no sidebar **BiFlow** wordmark, so wait for the mode switch instead.
- `scripts/sync-version.mjs` must only sync manifests when it is the process entry point. Importing `readAppVersion` from tests or `build-plan.mjs` must not rewrite `package.json`.
- After installing rustup, the same shell must prepend `$HOME/.cargo/bin` (or `source "$HOME/.cargo/env"`) or `cargo` is still missing. Both `./build.sh` and `./dev.sh` do this before every toolchain check, including clean/non-interactive shells.
- Hiddify/Mihomo Install buttons must use PATH and `~/.local/bin`, not only `~/.local/share/biflow`. Mock UI reads the same locations at Vite startup; Playwright still forces missing deps via `sessionStorage` so e2e can test Install.
- `zip` 2.6.1 is yanked on crates.io; pin `3.0.0` (2.4.2 also exists) or `cargo build` cannot resolve the crate.
- Edition 2021 + rustc 1.88 does not allow `if cond && let Some(...)` let-chains. Split into nested `if`.
- `u32::from([100, 64, 0, 0])` does not compile; use `u32::from_be_bytes([100, 64, 0, 0])` for CGNAT `100.64.0.0/10`.
- Workspace Clippy is `pedantic`, and warnings are errors. The Rust gate includes incremental `cargo clippy -p <changed crate> --all-targets -- -D warnings`; fix every diagnostic before completion.
- `iran-split-cli` uses `toml::from_str` in `main`; add `toml.workspace = true` or `cargo test --workspace` fails compiling the CLI binary tests.
- Tauri `bundle.resources` globs must match at least one file. Keep `resources/licenses/NOTICE.txt` so `../resources/licenses/*` does not fail the desktop build script.
- After `execute(..., request.payload)`, do not call `request.reply(...)`; `payload` was moved. Copy `request_id` / `protocol_version` first, then build a new `Envelope`.
- `iran-split-mihomo` tests use `chrono::Utc::now()`; add `chrono` under `[dev-dependencies]` or `cargo test --workspace` fails.
- Latest `cargo-xwin` (0.20+) needs rustc 1.89. Pin `0.19.2` in `build.sh` so Windows cross-compile install works on the repo toolchain 1.88.
- Run the Tauri CLI from the workspace root, which owns `src-tauri`. Do not delegate the root `tauri` script through `pnpm --filter @iran-split/desktop`; pnpm changes into `apps/desktop`, and Tauri then cannot discover `src-tauri/tauri.conf.json`. Tauri shell hooks also run from the workspace root, so `beforeDevCommand` / `beforeBuildCommand` use `apps/desktop`; `frontendDist` remains config-relative as `../apps/desktop/dist`.
- `./dev.sh` and `./dev.sh dev` must compile and launch the native Tauri application so the UI uses Rust commands and events. Keep browser/mock development behind the explicit `./dev.sh web` command; `desktop` remains a native alias.
- Native Linux `./dev.sh` must provision a per-UID transient privileged helper before Tauri starts, verify its root-owned helper/Mihomo copies and private socket, pass only debug-build path overrides, and stop/remove the transient unit on every exit path. Never run the UI as root or execute a mutable workspace binary from the root helper.
- Build requirement checks must not run `apt-get update` or request `sudo` when all packages are already installed. Let `apt_install_missing` update package indexes only after it finds a missing package.
- Do not `cargo clean` or re-run `cargo test --workspace` plus `cargo build --workspace` after every task. That recompiles hundreds of dependency crates. Use `cargo test -p <changed crate>` so Cargo rebuilds only dirty units.
- A Linux Tauri CLI only accepts Linux values for `--bundles`. For a Windows cross-build, pass `--runner cargo-xwin --target x86_64-pc-windows-msvc` without `--bundles nsis`; Tauri selects NSIS from the target and uses host `makensis`.
- Snapshot the root version at build start, select exact versioned source artifacts, and reject a mid-build version change. Never copy the first wildcard match or label a package with a version different from its embedded metadata.
- Tauri's synchronous `setup` callback is not entered into a Tokio reactor. Do not call `tokio::spawn` implicitly from constructors used there; pass `tauri::async_runtime::handle().inner()` into the Rust engine and spawn through that explicit handle. Keep a regression test that constructs the engine outside an entered runtime.
- Tauri array resources containing `../` are placed below `_up_`; runtime code that reads `$RESOURCE/rules` or `$RESOURCE/dependencies` must use object resource mappings with explicit target paths. Validate the pinned rule manifest and target-specific Mihomo checksum in both dev and build hooks.
- Startup health inspection must probe helper, Hiddify, Mihomo, TUN, and DNS independently. A missing helper must not short-circuit the other probes or leave user-visible component state as `unknown`; show a precise stopped, running, degraded, unavailable, or error state with a reason.
- Ship the verified target-specific Mihomo executable as a Tauri resource and install from it before attempting the network. Network downloads remain a fallback, verify both archive and executable SHA-256, and retry once without environment proxies when the proxy-aware request fails.
- Public-IP connectivity checks must be bounded and performed by Rust, with proxy-aware and direct clients. The React status bar displays only the typed result and must not call third-party location services directly.
- Playwright forks inherit both `NO_COLOR` and `FORCE_COLOR` in this execution environment, which makes every Node child print a warning. Delete `process.env.NO_COLOR` in `playwright.config.ts` before Playwright starts its web server and workers.
- Version synchronization must replace only the existing JSON `version` value. Re-serializing the entire manifest changes Prettier's compact array layout, so `precheck` makes `format:check` fail immediately afterward.
- React Fast Refresh warns when a component module also exports a plain helper. Put shared helpers such as country-flag conversion in a separate module so `eslint --max-warnings 0` stays clean.
- `cargo deny check` uses an explicit license allow list. Add `MPL-2.0` (cssparser via Tauri/wry), `Apache-2.0 WITH LLVM-exception`, `CDLA-Permissive-2.0`, and `BSL-1.0` (`clipboard-win` via `tauri-plugin-clipboard-manager`) when the lockfile needs them; do not blanket-allow. Workspace `path` crates look like `*` wildcards unless `allow-wildcard-paths = true` and those crates are unpublished (`publish = false`); cargo-deny 0.20 does not apply the path exception to public crates. Transitive unmaintained crates (GTK3 via Tauri, unic via urlpattern, proc-macro-error via glib) fail unless `unmaintained = "workspace"`.
- Windows Clippy `unnecessary_wraps` fires on `#[cfg(not(unix))]` permission stubs that always `Ok(())`. Keep `Result` on the Unix chmod path and a `()` no-op off Unix, with `cfg` at the call site. Helper `main` must `cfg` Unix-only imports such as `tracing_subscriber::EnvFilter`; Windows Clippy treats them as unused.
- `github.ref_protected` is false for typical `v*` tags, so a release job with that `if` never runs. Trigger `.github/workflows/release.yml` on `push: tags: ["v*"]` only, verify before building, upload workflow artifacts, and publish only after every platform succeeds.
- Ubuntu 24.04 renamed AppImage's FUSE 2 runtime package to `libfuse2t64`. Use that exact package in the Ubuntu 24.04 workflow and let `build.sh` select `libfuse2t64` or `libfuse2` from the local apt catalog.
- A palette-only UI request changes the existing RGB tokens in `apps/desktop/src/index.css`. Do not add a second theme runtime, shared component CSS system, or TSX behavior/layout changes unless the request explicitly includes them.
- Vitest's jsdom transform can expose a non-`file:` `import.meta.url`. Tests that inspect frontend source files should resolve them from the package test working directory (for example, `path.join(process.cwd(), "src/index.css")`).
- Windows `cargo clippy --workspace --all-targets` still enumerates the Linux platform workspace crate. Keep `#![cfg(target_os = "linux")]` as its first item so Tokio's `UnixStream` and Linux-only code are not compiled for Windows.
- Tokio Windows named-pipe `ClientOptions::open` returns a synchronous `io::Result`, not a future. Open it inside a timeout-wrapped async retry loop; retry only `ERROR_PIPE_BUSY` and map other errors or timeout to helper unavailable.
- Windows-only modules are invisible to host Clippy. Use explicit imports there; `use super::*` fails the Windows `clippy::wildcard_imports` gate. Windows Clippy `borrow_as_ptr` rejects `&mut value` at a Win32 `*mut T` out-parameter; write `&raw mut value`. Keep a source contract test, and prove the crate with `cargo xwin clippy -p iran-split-helper-winacl --all-targets --target x86_64-pc-windows-msvc -- -D warnings`.
- Windows Clippy rejects case-sensitive extension checks and `Default::default()` for unit structs. Compare `Path::extension()` with `eq_ignore_ascii_case`, and construct `WindowsBackend` directly.
- Linux `/run` is commonly mounted `noexec`. `./dev.sh` must install development helper and Mihomo executables under `/var/lib/biflow-dev-<uid>/bin`, not under `/run/biflow-dev-<uid>`.
- Mihomo Meta 1.19+ restricts rule-provider paths to the process workdir. Generate relative filenames (`private.txt`, …), validate with `mihomo -t -d <generation>`, and capture stdout as well as stderr when reporting rejections.
- Empty custom direct-rule providers stay at `ruleCount == 0`. Count them ready when they have no error; require bundled Iran/private providers to load rules. Include the last controller/provider status in readiness timeouts.
- Probe Hiddify egress before starting TUN. After TUN, a desktop SOCKS probe can fail in milliseconds because AppImage comm `Hiddify-Linux-x` was not DIRECT. Use clash's `PROCESS-NAME-WILDCARD,*Hiddify*,DIRECT`, generate_204 probes, and do not kill Hiddify on Mihomo rollback. A listening Hiddify port is not egress: try `socks5h` then `http` on the mixed port, Cloudflare then gstatic `generate_204`, and `.no_proxy()`. Do not reuse `MihomoError::Http` for that probe — the log then blames the controller. Pin Linux DoH to `#VPN` as well as Windows, or Google/MATCH hosts stay on fake-ip when WAN DoH is blocked.
- Bundled rule snapshots are SHA-256 verified byte-for-byte. Mark `resources/rules/*` as `-text` in `.gitattributes` and disable `core.autocrlf` on Windows CI checkout or `pnpm rules:check` fails on Windows.
- Directory `sync_all` via `OpenOptions::read` on a folder path is Unix-only. On Windows it returns `PermissionDenied`; gate `sync_directory` with `#[cfg(unix)]` and no-op off Unix. Call `fs::OpenOptions` inside the Unix function. A crate-level `use std::fs::OpenOptions` is unused on the Windows cfg branch and fails `cargo clippy --workspace -D warnings` on `windows-2025`.
- `#[cfg(debug_assertions)] { return expr; }` fails `clippy::needless_return` under CI `-D warnings` because Clippy compiles only the active cfg branch. Use a tail expression without `return` or a trailing semicolon. The same applies to `#[cfg(windows)]` / `#[cfg(target_os = "linux")]` last-statement returns.
- GitHub `windows-2025` (and current `windows-latest`) does not include NSIS. Install `makensis` on that runner before a Tauri NSIS bundle. After Chocolatey install, add the NSIS directory to `$GITHUB_PATH` and the current `$env:Path`; `$GITHUB_PATH` only applies to later steps, so `Get-Command makensis` in the same step otherwise fails. Set `core.autocrlf false` globally **before** checkout. Cache Cargo at the workspace root (`./target`), not `src-tauri/target`. Keep the rust OS matrix `fail-fast: false` so a Linux Clippy failure cannot cancel Windows.
- A shell-script contract test looking for the literal `mihomo -t` fails when the maintainer wrapper invokes `"${MIHOMO}" -t -d`. Match the quoted binary expansion.
- GitHub Actions job-level `if` cannot use the `matrix` context (`available contexts are github, inputs, needs, vars`). Filter a dry-run OS matrix in a prior `select` job and expand with `fromJSON(needs.select.outputs.include)`.
- `pnpm install --frozen-lockfile` fails in CI when `apps/desktop/package.json` specifiers drift from `pnpm-lock.yaml`. Keep `@types/node` on 24.x with the Node 24 pin and regenerate the lockfile in the same change.
- `cargo fmt --all --check` is the first Rust CI step and part of the done gate. Clippy and tests can pass while rustfmt still wants a one-line `.and_then` or a wrapped `assert!`. Run the check before calling the change done; apply `cargo fmt --all` in the same change if it fails.
- Tauri 2.5 AppImage bundling downloads `AppRun-x86_64` with ureq/rustls. GitHub can close TLS without `close_notify`, which rustls reports as `peer closed connection without sending TLS close_notify`. Prefetch the tools into `~/.cache/tauri` with `curl --retry` (`scripts/prefetch-appimage-tools.sh`) before `tauri build`; Tauri skips the download when the files already exist.
- `bundle.createUpdaterArtifacts` plus a committed updater public key makes local `tauri build` fail with `A public key has been found, but no private key` after the packages already exist. When `TAURI_SIGNING_PRIVATE_KEY` is unset, `build.sh` merges `createUpdaterArtifacts: false` so unsigned local packages still finish. GitHub Release signing that fails with `incorrect updater private key password: Missing comment in secret key` is a secret-format or password mismatch, not a bundler bug: the NSIS/deb/AppImage already exist. Use repository secrets (not environment secrets); store the Base64 private key from `pnpm tauri signer generate`; set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` only when the key was generated with a password. Do not export an empty password env var at workflow scope. Validate with `scripts/prepare-tauri-signing.mjs --require --verify-sign` before `tauri-action`. Invoke the Tauri CLI as `node node_modules/@tauri-apps/cli/tauri.js`; `spawn("pnpm")` is `ENOENT` on Windows runners. After rotating keys, commit the new `plugins.updater.pubkey` in the same change.
- `pnpm tauri build --bundles deb,appimage` always rebuilds the frontend, the release binary, and both packages. Split `build.sh` into compile/deb/appimage/collect (and compile/nsis/collect on Windows). Skip a stage when this version's artifact already exists; `--from STAGE` starts at the failed stage; `--force` rebuilds all. Skip `beforeBuildCommand` when `apps/desktop/dist/index.html` is present, and prefetch AppImage tools only in the AppImage stage.
- `set -e` treats a trailing `[[ -f missing ]] && log` as a failed function. `print_summary` must use `if` so a Linux-only package run does not exit 1 after writing artifacts.
- WebKitGTK 2.44+ DMA-BUF rendering paints a blank Tauri window on VMware SVGA and NVIDIA while JavaScript and IPC still run. Set `WEBKIT_DISABLE_DMABUF_RENDERER=1` before GTK starts; also set `WEBKIT_DISABLE_COMPOSITING_MODE=1` on virtual/NVIDIA GPUs and `LIBGL_ALWAYS_SOFTWARE=1` on virtual machines. Workspace `unsafe_code = "forbid"` blocks `env::set_var`, so `exec()` the process with those variables instead of `Command::status()`-waiting on a child (that leftover parent keeps a terminal open). Do not wait until after `tauri::Builder::build`. Closing the window leaves the process in the tray; `tauri-plugin-single-instance` then shows that old window when a new package is launched. Include the full `X.Y.Z` version in the Linux D-Bus id (`app.biflow.desktop.v1_2_6`) so a newly built package is not swallowed by an older tray instance. The plugin `semver` feature only suffixes the major version and does not separate 1.2.2 from 1.2.5.
- A Windows GUI exe without `#![cfg_attr(windows, windows_subsystem = "windows")]` is a console binary, so Explorer and NSIS open a cmd window behind the UI. Put that attribute on `src-tauri/src/main.rs` and `iran-split-helper`; logs already go to `debug.log`. Pin Linux `.desktop` `Terminal=false` via `bundle.linux.deb.desktopTemplate`.
- cargo-xwin 0.19.2 downloads CRT vsix and SDK/UCRT payloads with ureq. Microsoft CDNs often close that body early (`io: unexpected end of file` / `Failed to setup MSVC CRT`). Prefetch the VS 16 channel manifest, vsman, CRT Desktop/Store vsix, SDK MSI, and UCRT cabs with `curl --retry` into `~/.cache/cargo-xwin/xwin/dl` using xwin's cache filenames (`scripts/list-xwin-payloads.py`), export `XWIN_ARCH=x86_64` and `RAYON_NUM_THREADS=1`, and retry the Windows compile. Do not `cargo install xwin --version 0.6.6`; that crate version is yanked. A `--max-time 180` curl limit is too short for the 44 MiB CRT Desktop vsix and 139 MiB UCRT cab.
- Windows cannot `File::set_len(0)` on a handle opened with `.append(true)` (`PermissionDenied` / `SetEndOfFile` on `FILE_APPEND_DATA`). Close the handle, truncate, then reopen append so Diagnostics delete keeps logging. Dependency status tests must write `mihomo_install_path(data)` (`bin` + `mihomo.exe` on Windows). Do not hardcode Unix `/tmp/...` strings in Windows `reveal_command` assertions; use `path.to_string_lossy()`.
- `vendor/mihomo/linux-x86_64/mihomo` is a real file on Windows checkouts. A test that runs that ELF fails with "not a valid Win32 application"; use `vendor/mihomo/windows-x86_64/mihomo.exe` and `Platform::Windows` on Windows. Do not spawn `cmd /C start` or `xdg-open` from unit tests.
- Windows `resource_dir()` is the directory that contains `BiFlow.exe`. Collecting only that exe omits `rules/`, so bootstrap `CloudRuleStore::status` fails with `os error 3` (`ERROR_PATH_NOT_FOUND`). Embed the bundled snapshot, materialize it into the user data directory when packaged files are missing, and copy `resources/rules` next to the portable exe.
- Packaged Linux/Windows apps never started a helper. Ship `iran-split-helper` plus an elevated installer (systemd/`pkexec` on Linux, scheduled task and a Medium-integrity named-pipe SDDL on Windows). Do not run the helper from an AppImage or portable path as root. Stage `packaging/staged/` before `tauri build` and GitHub Release `tauri-action`. Linux `./build.sh windows` must `cargo xwin build` the helper for `x86_64-pc-windows-msvc`; `cargo build --target` looks for `link.exe` and fails. Linux-only items in `helper_install.rs` (`LINUX_HELPER_ROOT`, candidate helpers, `/proc` uid parsing, and test `PathBuf` imports used only by those tests) must be `#[cfg(target_os = "linux")]`; otherwise Windows `dead_code` / `unused_imports` fails the desktop lib. Prove with `cargo xwin clippy -p iran-split-desktop --all-targets --target x86_64-pc-windows-msvc -- -D warnings`.
- Windows Clippy also lints **doc comments** in `#[cfg(windows)]` modules that host Clippy never compiles. `clippy::doc_markdown` rejects bare camel-case identifiers such as `ProgramData`; backtick them. All-caps words (`SYSTEM`, `TUN`) are not flagged. Grep windows-gated `///` lines for camel-case words before tagging a release.
- `actions/download-artifact` preserves each upload's own directory layout, and `merge-multiple: true` does not flatten it. The Linux artifact arrives as `appimage/*.AppImage` and the Windows one as `target/release/bundle/nsis/*-setup.exe`, so a non-recursive `readdirSync` in `scripts/generate-latest-json.mjs` found 0 signed updater artifacts and failed publish after both builds succeeded. Walk the tree, match on `basename`, and reject duplicate bundles per platform.
- Running the packaged app with `sudo` makes `install_helper` pass `--authorized-uid 0`, and `install-helper.sh` refuses root (exit 2). Reject uid 0 in `install_linux` before spawning `pkexec` and tell the operator to relaunch without sudo. Capture the script with `Command::output()` and append its last stderr line; `.status()` discards the only line that says which check failed, leaving the dialog on a bare "privileged helper installation failed".
- An `AppImage` FUSE-mounts itself as the calling user, and without `allow_other` that mount denies every other uid **including root**. `pkexec <path inside /tmp/.mount_*>` therefore dies with `Error accessing …: Permission denied` (exit 127) before the script runs. Copy the script, helper, Mihomo, and unit into a 0700 directory under the app data dir and elevate those paths; a `.deb` install already sits in root-owned `/usr/lib/biflow` and must stay in place so the polkit action's `exec.path` annotation still matches. Hash the packaged originals and let the elevated script re-verify the copies. Do not stage under `/run` (noexec, ADR 0015) — pkexec has to exec the script.
- pkexec exit 126 is a dismissed polkit dialog; 127 means it could not execute the program at all. Treating both as "cancelled" hides real failures behind a message that blames the operator.
- Windows `Start-Process -Verb RunAs -Wait` reports PowerShell's exit code, not the elevated child's, so a failed helper install and a refused UAC prompt both looked like success and only surfaced later as an unreachable-helper timeout. Add `-PassThru`, `$ErrorActionPreference = 'Stop'`, `exit $process.ExitCode`, and map a refused prompt to `ERROR_CANCELLED` (1223).
- `pnpm github:action-test` (`scripts/ci-local.mjs`) runs every `ci.yml` gate before a push, including the `rust (windows-2025)` Clippy job via `cargo xwin clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`. That job is the one that keeps breaking, because host Clippy only compiles the host cfg. A missing `cargo-xwin` or `cargo-deny` is reported as a failure, not a skip. `scripts/ci-local.test.mjs` fails when a workflow gains a `run:` command that no local step mirrors, so the two cannot drift. Windows `cargo test --workspace` still cannot run on a Linux host; the mirror prints that gap instead of implying full coverage.
- Hiddify sometimes opens on a blank window because a generated profile config or its runtime state is corrupt. Diagnostics → **Fresh Hiddify start** (`fresh_hiddify_start`, `src-tauri/src/hiddify_reset.rs`) moves `configs/`, `data/`, and `*.log` into a timestamped folder under `<data>/backups/` and relaunches Hiddify. Never touch `db.sqlite` (subscriptions) or `shared_preferences.json` (settings), and never act on a directory that holds none of the marker files — the resolved path gets emptied. Terminate the running instance by matching `/proc/<pid>/exe` (Linux) or the image name (Windows) against the discovered executable; a command-line substring match also hits terminals and editors.
- Windows Clippy only **type-checks** `#[test]` bodies. An assertion that is true on Linux and false on Windows — `Path::new("/run/…").is_absolute()` is `false` on Windows, because a leading `/` is root-relative there, not absolute — compiles clean and fails only when the test binary runs on `windows-2025`. Gate Linux-path tests with `#[cfg(unix)]`. `pnpm github:action-test` mirrors that job with `cargo xwin test --workspace --target x86_64-pc-windows-msvc`, which needs `wine64`; without it the step fails with the install command rather than silently skipping. The step exports `WINEARCH=win64` and a project-local `WINEPREFIX=target/.wine` — the default wrapper builds a WOW64 prefix and stops on `wine32 is missing`, and the target is 64-bit anyway.
- `include_str!("lib.rs")` plus `!contains(forbidden)` matches the assertion itself when the needle is a contiguous literal in the same file. `#![cfg(windows)]` crates never run that test on Linux, so 4.3.0 shipped green locally and failed `cargo test --workspace` on `windows-2025`. Scan only the production half before `mod tests {`, split the needle, and keep the same check in `scripts/tauri-contract.test.mjs`.
- `iran-split-platform-win` implements the full `PlatformBackend`, not a stub (ADR 0033). Keep `#![cfg(windows)]` as its first item, mirror any change to `iran-split-platform-linux` on both sides, and call `generate_config` with `Platform::Windows` — that flag is what emits `strict-route`. Read TUN state from Mihomo's `/configs`, never by enumerating adapters: `GetAdaptersAddresses` needs `unsafe`, which the workspace forbids outside `iran-split-helper-winacl`. `scripts/tauri-contract.test.mjs` fails when a trait method is left unimplemented or the two backends stage different generation files.
- About → **Check for updates** uses the public GitHub Releases API
  (`/repos/devlifeX/BiFlow/releases/latest`), not signed `latest.json`. There is
  no startup poll. Two attempts with a 60s timeout cover transient DNS/TLS.
  `UpdateCoordinator` stays idempotent and must never return `"an update is
already in progress"`. Cache the last `UpdateInfo` (never log asset URLs).
  Debian is `pkexec apt-get install -y` then phase `installed` (quit and reopen).
  AppImage and NSIS spawn a wait-for-PID helper and `app.exit(0)`. Do not copy
  `BiFlow.exe` over a per-machine NSIS install. Clippy `doc_markdown` flags
  `DBack` and `AppImage` in rustdoc; backtick them. A `map_err` that only
  emits then returns the same error is `inspect_err`. A source contract must
  allow rustfmt to split `.try_begin(` onto the next line.
- A `.deb` install puts the helper at `/usr/lib/biflow/iran-split-helper`, which is also `install-helper.sh`'s destination, so `install(1)` aborts with "are the same file" and the in-app Install fails. Compare `readlink -f` of source and destination and re-apply ownership and mode in place instead of copying.
- Dashboard and every other page grow inside the shell's `overflow-y-auto`. A page root with `h-full overflow-y-auto` locks height to the viewport and traps the live-routing SVG. Use `flex flex-col gap-4 pb-2` like Direct Rules.
- Diagnostics accepts any pasted form (`https://www.rade.ir/`, `host:port`, IPv6) and reduces it with `extractHost` before calling `test_route`; the rule parser rejects URLs outright. The flow result offers the opposite move — Add to direct for a VPN host, Add to VPN for a direct one — and re-tests afterwards so the card shows the new routing.
- User route pins are one list (ADR 0034/0068): `RoutePinsDocument.pins` with `Outbound::Direct | Client { client_id }`. `pinRoute` takes `"direct"` or a ClientId uuid. Disabled-client pins stay in the document and are grey in Direct Rules but are not emitted to Mihomo. Deleting a client confirms with pin count and can move pins; MATCH `default_route` falls back to Direct. Pins store the PSL registrable root (private suffixes included); `api.shop.example.com` is `example.com`, `user.github.io` is not all of `github.io`. `pin` must not wait on DoH. Writers emit `+.example.com` and never copy `resolved_ips` into IP providers. Precedence in `RuleSet::decide` must match generated Mihomo rules — private/LAN first, then enabled client pins, then DIRECT pins, then bundled Iran domains, then curated `iran-business-domains`, then `MATCH` to `default_route`. Never let a private, loopback, or CGNAT address onto a LocalProxy. Helper `GENERATION_FILES` accept the fixed names plus `custom-<uuid>-domains.txt` / `custom-<uuid>-ips.txt` where `<id>` is `[0-9a-f-]{36}`. Live apply when the stack is running or degraded; persist-only when stopped.
- `resources/rules/manifest.json` `commit` is the **upstream Chocolate4U** revision the rules came from, not a BiFlow commit. Building a `raw.githubusercontent.com/devlifeX/BiFlow/<commit>/…` URL from it always 404s and surfaces as "cloud rule download failed". Fetch snapshot files from the same branch the manifest itself is fetched from; the per-file SHA-256 in the manifest is what guarantees integrity, not the ref.
- Windows `.join("biflow/debug.log")` keeps the `/`, so Explorer `/select,C:\…\biflow/debug.log` is ignored and opens This PC. Join `biflow` and `debug.log` as separate components, normalize leftover `/` to `\`, and pass `/select,` with `raw_arg`. The same class of bug is `mihomo_file_name()` returning `"bin/mihomo.exe"` — join `bin` and the file name separately. Helper Install must not elevate `C:\ProgramData\iran-split\bin\iran-split-helper.exe`: a leftover copy `fs::copy`s onto itself and a GUI-subsystem helper leaves stderr empty. Elevate the packaged helper, skip same-file copy, write `ProgramData\iran-split\install.log` on failure, and use NSIS `$PROGRAMDATA` (not `$COMMONPROGRAMDATA`) for helper/Mihomo install destinations. Packaged Windows helper staging is `$PROGRAMDATA\iran-split\staging` (ADR 0064). NSIS `perMachine` `$LOCALAPPDATA` expands to `C:\ProgramData`, so `$LOCALAPPDATA\biflow\runtime\generations` recorded `C:\ProgramData\biflow\runtime\generations` while the desktop wrote under the user's LocalAppData. The elevated installer ignores a mismatched `--staging-dir`, grants Builtin\Users modify with `icacls`, and the desktop stages into `WindowsPaths.generation_staging_dir`.
- `Start-Process -ArgumentList @('--mihomo', 'C:\Program Files\…')` concatenates array entries without quoting, so clap sees `C:\Program` and `Files\…`, exits 2 inside `Arguments::parse()`, and never reaches `persist_install_error`. Pass one Windows-quoted command line. Use `try_parse()` and write a redacted clap kind (`unexpected argument`) to `install.log` before `error.exit()`.
- `schtasks /TR "\"exe\" --config \"file\""` stores one broken action: `/Create` and `/Run` return 0, the GUI-subsystem helper never starts, and the desktop times out with “installed but is not reachable yet” while every pipe open is `os error 2`. Register the task from UTF-16 XML with separate `Command` and `Arguments`, wait for `\\.\pipe\iran-split-helper-v1` before returning success, and persist `run_named_pipe` failures to `install.log`.
- Never probe a named pipe with `Path::exists()`. It calls `fs::metadata`, an NPFS object has no file attributes to return, and the check reports a healthy helper as missing — turning a working install into a 15s timeout. Open the pipe the way the desktop does and treat only `ERROR_FILE_NOT_FOUND` (2) as absent; a busy instance or a denied ACL still proves it exists. Related Task Scheduler footguns: `<AllowHardTerminate>false</AllowHardTerminate>` makes `schtasks /End` a no-op, so a reinstall then dies on `ERROR_SHARING_VIOLATION` copying over the running helper's own image; `schtasks` writes UTF-16 to a pipe (sometimes with no BOM), so `from_utf8_lossy` alone silently yields text nothing can be found in; and `install.log` is read back one line at a time, so every message written to it must be collapsed to one line first.
- Windows `register_runtime_generation` failing with `INVALID_GENERATION` / `cannot find the file specified (os error 2)` at `C:\ProgramData\biflow\runtime\generations` is the 4.2 NSIS `$LOCALAPPDATA` all-users expansion (ADR 0064), not a missing generation UUID. Do not point helper.toml at a user profile: an elevated Install records the administrator's `%LOCALAPPDATA%` and every later run as the normal user misses it — the Windows twin of the Linux `sudo` uid-0 bug. Reinstalling the 4.3 helper rewrites staging to `C:\ProgramData\iran-split\staging` and the ACL. Never let `canonicalize` errors reach the client bare; name the staging root and the generation directory so one log line identifies the mismatch.
- `RuleManager::pin` must not wait on DoH. Resolve remains metadata-only on `refresh()`. A live apply rebuilds the generation and starts Mihomo on it; success is persist plus apply when the stack is running or degraded.
- The connection glow rings the shell from `.connection-glow::after` (a fixed, `pointer-events: none` overlay), never the shell's own border — a real border shifts the fixed 1120x760 layout. `running` is green, `paused` amber, every other phase unringed; `data-connection-glow` carries the state so e2e can assert it without reading colours. The pulse is disabled under `prefers-reduced-motion`.
- `install.log` is the only channel out of the elevated Windows helper (`Start-Process -Verb RunAs` cannot redirect stdio and the helper is a `windows` subsystem binary), so the desktop must delete it **before** elevating. The helper overwrites it only when it reaches its own error path; a process that dies earlier leaves the previous attempt's line behind, and reporting a stale reason is worse than reporting none.
- Assigning `field.value` in a custom paste menu does not update a controlled React input. Use the native value setter from `Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")` and dispatch a bubbling `InputEvent`. Capture the target before the clipboard read; unmount or disable afterwards is a no-op. `addRule`/`pinRoute` must rethrow so the form clears only on success. Native WebView2/WebKitGTK `navigator.clipboard.readText()` is often `NotAllowedError`; read and write through `tauri-plugin-clipboard-manager` in Tauri and keep `navigator.clipboard` for Vite/Playwright. Surface a short paste/copy/cut error instead of swallowing it. Direct Rules must `extractHost` before `addRule` so a pasted URL is accepted.
- Session traffic is an in-memory delta accumulator on the desktop process. Do not load `traffic-totals.json`. A repeated poll of the same Mihomo snapshot must add zero.
- A Windows Connect that dies at Mihomo readiness with `error sending request for url` is not an internal server error. `CoreError::Platform` maps to `errors.internal` in the UI; map a readiness timeout to `ControllerTimeout`. The controller client must use `no_proxy()` or Hiddify's HTTP proxy intercepts `127.0.0.1:19090`. The helper must not `env_clear()` Windows Mihomo down to PATH-only — restore `SYSTEMROOT` (and spawn with `CREATE_NO_WINDOW`), wait briefly for an immediate exit, and ship `wintun.dll` next to `mihomo.exe`.
- A later Windows field log reached `ready: 7` / `rules_loaded: 65734` and then rolled Mihomo back with "process or TUN disappeared". `GET /configs` is still the TUN authority (no adapter enumeration), but `tun.device` on Windows is often `Meta` or empty — treat truthy `tun.enable` as active, retry the post-readiness process/TUN check for 5s, and split the error. Generate Windows YAML like clash-master: `find-process-mode: always`, `ipv6: false`, `auto-redirect: false`, DoH `#VPN`.
- An interrupted `cargo test`/`clippy` can leave `corrupt metadata` in `target/debug/deps/*.rmeta`. Delete only the file rustc names (and its sibling `.rlib`) and rebuild that crate. Do not `cargo clean`.
- A `tokio::sync::watch` subscriber misses intermediate `operation_stage`
  values when several `update()` calls run in one worker poll. Yield after
  each announced milestone, and collect stages with `tokio::join!` against
  `receiver.changed()` so the waiter is armed before the operation is queued.
- `pause_stack` used to treat `Stopped` as already complete without checking the lifecycle lock, so Pause succeeded while Connect was still reserved. Reject any other `busy` kind before the idempotent phase shortcuts.
- Workspace Clippy `map_unwrap_or` rejects `option.map(f).unwrap_or(default)`. Use `map_or(default, f)` at the tray setup site and similar lookups.
- `pnpm format:check` includes `docs/considering/*.md`. Compact table separators (`|---|---|`) fail Prettier and the frontend GitHub Action. Run `pnpm exec prettier --write` on new markdown before merge.
- Bundled rule bytes are pinned with `resources/rules/*.json -text`, which covers both `manifest.json` and `iran-business-domains.sources.json`. Contract tests must match that glob; requiring a literal `manifest.json -text` line fails after the curated catalog lands.
- `RuleSet::from_sources` takes the curated catalog as a fourth iterator. Merging those names into `iran_domains` hides the Mihomo order (Iran list, then business catalog, then `MATCH`) from `test_route` and the CLI.
- Playwright `listitem` text for a subdomain pin is the registrable root (`rade.ir`, not `www.rade.ir`). Responsive screenshots prefer `/opt/cursor/artifacts/screenshots` and fall back to `test-results/responsive-screenshots` when that path is not writable.
- Pause never calls `stop_user_proxy` (Hiddify stays up) but must `clear_hiddify_system_proxy` when the OS proxy still points at Hiddify. Connect must not set the OS proxy; Resume restores the user-data snapshot without logging the endpoint. CLI `DemoBackend` must implement those required trait methods as no-ops; Windows `cargo test --workspace` compiles `iran-split-cli` and fails first if they are missing.
- Basic mobile always shows `data-testid="bottom-nav"`. Tapping Rules, Diagnostics, or Settings writes Advanced and opens that page; Dashboard and About stay in Basic.
- `list_active_connections` may log a row count, never hosts or URLs. Outbound is the last Mihomo `chains` entry (`DIRECT` → `"direct"`, otherwise the client uuid).
- Playwright `getByRole("cell", { name: "DIRECT" })` on live connections matches both the route badge and the outbound `<select>` that also lists DIRECT. Assert the badge `span` or the select value.
- `test_route` is `RuleSet::decide` only. A DIRECT pin can still resolve to Mihomo fake-ip (`198.18.0.0/16`) while Cloudflare DoH answers the lookup. Default DIRECT DNS is Mihomo fake-ip; do not emit `nameserver-policy` or DIRECT `fake-ip-filter` entries unless the operator picks Shecan/Electro/Radar/Mokhaberat/Custom. Pause/Connect after a DNS or pin change. Log the preset name, never custom resolver IPs. Radar `10.x` is a valid DIRECT DNS address; do not reject all RFC1918 resolvers. Schema 2 rewrites the 3.9 implicit Shecan default back to fake-ip. Schema 3 moves `[hiddify]` into `clients` and sets `default_route` to that Hiddify id.
- Do not add `Outbound::OpenVpn`, `openvpn_rules`, `AppConfig.openvpn`, `StackSnapshot.openvpn`, or `starting_openvpn`. New products are catalog presets on `LocalProxy` or `OwnedSideTunnel`. YAML group names come from the ClientId, never the UI label.
- `docs/adr/README.md` is Prettier-formatted like every markdown table: a hand-added index row with different column widths fails `pnpm check`. Run `pnpm exec prettier --write` on any touched ADR/markdown before the gate.
- `createClientInstance`'s `id` parameter inherits `crypto.randomUUID()`'s template-literal type, so a plain literal like `"happ-id"` fails `tsc`. Use UUID-shaped constants (`"11111111-1111-1111-1111-111111111111"`) in tests.
- An import used only by `mod tests` in a platform crate (e.g. `DefaultRoute`) must live inside the test module; at the top of the file the lib build fails `-D unused-imports`.
- Happ and v2rayN share catalog default port 10808 and `generate_config` rejects duplicate local ports (`configured ports must be unique`). Multi-client tests (and operators) must move one client off the shared default.
- Backend tests that call ensure/recover paths must `config.clients.clear()` before pushing fixtures: the default Hiddify instance points at 127.0.0.1:12334, and on a dev host a real Hiddify may be listening — the test would probe a live proxy.
