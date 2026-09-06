# BiFlow architecture & stability review (ChatGPT × Claude × code)

**Date:** 2026-09-06 (Asia/Tehran)  
**Repo:** [devlifeX/BiFlow](https://github.com/devlifeX/BiFlow) @ `v5.8.0`  
**Method:** Multi-round ChatGPT discussion → each claim verified against the on-disk codebase → Claude as **final reviewer** before this document.  
**Scope:** Stability / reliability of the privileged split-routing stack. Not a full security audit, UX review, or broad refactor plan.

---

## 0. Product snapshot (verified)

BiFlow is a Linux/Windows desktop app (Tauri 2 + React 19 + Rust) that keeps Iranian / private / LAN traffic **DIRECT** and sends the rest through a local VPN client (primarily Hiddify) via bundled **Mihomo**. It does not replace the VPN; a privileged helper owns TUN/routes.

Tagline: *Right traffic. Right route.*

### Verified code facts (ground truth)

| Area | Fact |
| --- | --- |
| IPC | `PROTOCOL_VERSION = 1`; `Envelope { protocol_version, request_id: Uuid, payload }` in `crates/iran-split-ipc` |
| Helper API | High-level allowlist: `RegisterRuntimeGeneration`, `Start/Stop/RestartMihomo`, `CleanupOwnedNetworkState`, `PrepareForUpdate`, `Start/StopSideTunnel` |
| Lifecycle | `StackPhase`: `Uninitialized`, `Stopped`, `StartingClient`, `PreparingRuntime`, `ValidatingConfig`, `StartingCore`, `CheckingReadiness`, `Running`, `Paused`, `Degraded`, `Stopping`, `Recovering`, `Error` + rollback on failed start/resume |
| Startup recovery | `run_reconcile`: if helper ready and (`mihomo` process running **OR** `tun.active`) → `cleanup_owned_state` |
| Cleanup report | `CleanupReport { process_stopped, tun_removed, routes_removed, dns_restored, warnings }` |
| Watchdog | Egress watchdog (ADR 0078) → `Degraded` / recover |
| Packaging | Linux `iran-split-helper.service` with hardening (`NoNewPrivileges`, `ProtectSystem=strict`, `DeviceAllow=/dev/net/tun`, …) |
| Proxy | System proxy snapshot/restore persisted (`iran-split-platform-linux`) |
| Rules | `resources/rules/manifest.json` — **sha256 only** (no Ed25519); fetch from BiFlow GitHub raw |
| Side tunnel | `StartSideTunnel` takes `PathBuf` for `profile` / `executable?` / `auth_file?` |
| Profile audit | `audit_openvpn_profile` **already rejects symlink profiles** and dangerous directives (`iran-split-clients/src/profile_audit.rs`) |
| Binary resolve | `resolve_binary(Some(path))` in `crates/iran-split-helper/src/openvpn.rs` returns path **as-is** (no symlink check). `mihomo_binary` **does** reject symlinks. `None` → fixed system candidate paths |
| Ops guard | `CoreError::OperationInProgress` / `LifecycleBusy` when another connection operation is in progress |
| Size / naming | `src-tauri/src/lib.rs` ~3793 LOC; `iran-split-core` ~2490 LOC; crates/units still `iran-split-*` |

---

## 1. Round 0 — ChatGPT initial architecture opinion (summary)

ChatGPT praised privilege separation (UI → engine → helper → Mihomo) and scored roughly:

| Area | Score |
| --- | --- |
| Privilege separation | 9/10 |
| Security thinking | 8.5/10 |
| Lifecycle | 7/10 |
| Extensibility | 7/10 |
| Maintainability | 6.5/10 |
| Production readiness | 7.5/10 |

**Early recommendations (many already existed in code):** replay protection (sequence/nonce), full owned-state ledger, `RouterEngine` abstraction, Ed25519/TUF rules, formal connection state machine, crash-recovery daemon, event timeline observability, macOS Network Extension / TUN-only strategy, rename `iran-split`.

**Human code review pushback (before Round 1):** IPC already has `request_id`; `StackPhase` already exists; systemd helper + egress watchdog already exist; structured tracing already exists; helper API is already high-level; real gaps are rules authenticity, `StartSideTunnel` path surface, monoliths, naming, mid-apply ledger precision.

---

## 2. Round 1 — ChatGPT after code facts

ChatGPT admitted overlap with existing code and proposed three high-ROI stability items:

1. **P0 — Full owned-state ledger on disk** (generation, phase, owned tun/routes/processes) + fault injection  
2. **P0 — Remove `PathBuf` from `StartSideTunnel`** → `tunnel_id` + generation; helper resolves known artifacts  
3. **P1 — Property-based / failure-matrix testing** per `StackPhase`  

Deprioritized: Ed25519 (P2), monolith split (P2), rename (P3).

### Code verification of Round 1

| Claim | Verdict |
| --- | --- |
| Need full owned-state ledger | **Partial.** No unified disk ledger, but `run_reconcile` + `cleanup_owned_state` + `CleanupReport` + `tun_name`-based cleanup already recover coarse leftover state. Real gap is mid-apply **transaction breadcrumb**, not total inability to cleanup. |
| Remove all PathBuf from side tunnel | **Overstated.** Profile path is audited (symlink rejected). Real hole is `resolve_binary(Some(path))` asymmetry vs Mihomo. |
| Failure matrix missing | **Fair.** Unit tests cover pause/rollback/watchdog; no systematic per-phase fault matrix. |

---

## 3. Round 2 — ChatGPT after verification nuances

ChatGPT revised:

- **Full ledger is not P0** — reconcile already provides recovery; journal should be a **minimal engine-written transaction breadcrumb**, not a second owned-route inventory (dual source of truth risk).
- Suggested journal fields: `schema_version`, `generation_id`, `phase`, `started_at`, `intent{needs_tun,needs_routes,needs_dns,side_tunnel}`, `last_committed_step`, `completed`; write temp → fsync → rename; delete on success.
- **Do not fully delete OpenVPN paths** (file-oriented ecosystem). Prefer harden: no-symlink, root-owned staging copy + hash, catalog id for executable.
- Explicitly left **section C** (dangerous transitions) unanswered in that turn.

### Extra code note applied after Round 2

`audit_openvpn_profile` already rejects symlink **profiles**. ChatGPT’s “OpenVPN accepts symlinks” was too broad — the unchecked path is primarily **custom executable** (`Some(path)`), plus TOCTOU / lack of staging+hash.

---

## 4. Round 3 — ChatGPT fault transitions + 2-week plan

### ChatGPT’s three “dangerous transitions” (as stated)

1. `StartingCore → ApplyingNetwork` — kill helper mid-apply  
2. `StartingVPN → StartingCore` — upstream/SOCKS not ready + watchdog  
3. `Connected` concurrent with Pause/Stop during apply/rollback  

**Plus a 2-week plan:** W1 fault matrix + journal; W2 OpenVPN harden + regression + internal RC.

### Code verification of Round 3

| Issue | Verdict |
| --- | --- |
| Phase names | **Fabricated.** Real enum has no `ApplyingNetwork`, `StartingVPN`, or `Connected`. Closest map: StartingVPN≈`StartingClient`; ApplyingNetwork≈`StartingCore`/`CheckingReadiness`; Connected≈`Running`. |
| Category error | Side tunnel is `StartSideTunnel` / `StopSideTunnel`, **not** primary `StackPhase` of the Mihomo stack. |
| Concurrent ops | Code already has `OperationInProgress` / `LifecycleBusy` — needs **verification of atomicity** at risky windows, not invention of a brand-new mutex concept. |

---

## 5. Claude — final reviewer (authoritative for this doc)

Claude graded the ChatGPT thread against verified facts and produced the backlog below.

### What Claude said ChatGPT got right

- Dropping Ed25519 / monolith / rename as near-term priorities after Round 1.  
- Round 2’s **ledger downgrade** (best move in the conversation).  
- Directional call: **harden** OpenVPN paths rather than delete the feature.

### What Claude said ChatGPT got wrong / overfit

- Round 3 invented non-existent phases — regression to a generic VPN FSM template.  
- Round 1 “remove PathBuf entirely” was an overcorrection; the real gap is `resolve_binary(Some)` vs `mihomo_binary` asymmetry.  
- Minimal journal is the right **shape** but **P1, not P0** — reconcile already covers total-loss coarseness.

### Claude correction note (editor)

Claude briefly located `resolve_binary` under `iran-split-core`; **on disk it lives in** `crates/iran-split-helper/src/openvpn.rs` (Mihomo symlink checks are in helper `lib.rs`). Touchpoints below use the verified paths.

---

## 6. Final prioritized backlog (Claude + code-corrected)

### P0 — OpenVPN executable path TOCTOU / symlink asymmetry

- **Failure mode:** Privileged helper may spawn `executable: Option<PathBuf>` via `resolve_binary(Some(path))` with no symlink/regular-file rejection (unlike Mihomo). Path can be swapped to a symlink before spawn → arbitrary binary under helper privilege. Profile audit does **not** cover this field.  
- **Touchpoints:** `crates/iran-split-helper/src/openvpn.rs` (`resolve_binary`), `StartSideTunnel` IPC + handler; mirror Mihomo checks in helper `lib.rs`.  
- **Acceptance:** Unit test rejecting symlinked executable; IPC integration test: `StartSideTunnel` with symlink executable → explicit error, **no process spawned**.

### P0/P1 — Concurrent-operation races at real phase boundaries

- **Failure mode:** Despite `OperationInProgress`, interleaving `Stop`/`Pause`/`RestartMihomo` during `Recovering` / `Stopping` / `CheckingReadiness` could leave mixed ownership that coarse reconcile doesn’t fully unwind.  
- **Touchpoints:** `iran-split-core` busy/queue guards; helper command dispatch; `RegisterRuntimeGeneration`.  
- **Acceptance:** Fault-inject concurrent ops across the **real 13** `StackPhase` values; exactly one winner per generation; post-reconcile OS state matches `CleanupReport`.

### P1 — Minimal write-ahead transaction journal

- **Failure mode:** Crash mid-transition loses `last_committed_step`; reconcile only sees binary process/TUN signals — safe but undiagnosable and can race a new session.  
- **Touchpoints:** Engine writes journal (temp+fsync+rename); startup reads breadcrumb then existing reconcile; **do not** duplicate route inventory.  
- **Acceptance:** Kill helper at synthetic Start/Stop/Rollback checkpoints; restart converges to clean `Stopped` verified against OS routes/DNS/TUN.

### P1 — `StackPhase` fault matrix (real enum only)

- **Failure mode:** Untested `Degraded` / `Recovering` / `Error` paths skip invariants (e.g. resume to `Running` without re-validation).  
- **Touchpoints:** `iran-split-core` transitions; egress watchdog (ADR 0078).  
- **Acceptance:** Table-driven tests over the real enum; cover `StartingCore→CheckingReadiness` under Mihomo hang; `Degraded→Recovering→Running` under repeated watchdog trips.

### P2 — Rules manifest hash provenance

- **Failure mode:** sha256 integrity is weak if hash and body come from the **same** GitHub-raw channel (compromised push updates both).  
- **Touchpoints:** `iran-split-rules` cloud/manifest fetch.  
- **Acceptance:** Document provenance; if same-channel, capture “modified body + recomputed hash accepted” then add out-of-band pin / release-signed hash list.

---

## 7. Explicit non-goals (next ~2 weeks)

- Ed25519 / fully signed manifests (pending P2 provenance decision)  
- Splitting `lib.rs` / `iran-split-core` monoliths  
- Renaming `iran-split-*`  
- General `RouterEngine` abstraction  
- New observability platform  
- Changing egress watchdog unless a concrete bug appears  
- Full owned-state ledger duplicating route inventory  

---

## 8. Suggested 2-week execution order (merged)

| Days | Work |
| --- | --- |
| 1–2 | P0: harden `resolve_binary(Some)` like Mihomo + tests |
| 2–4 | P0/P1: concurrent-op fault harness on real `StackPhase` |
| 4–6 | P1: minimal transaction journal + startup breadcrumb |
| 6–8 | P1: expand lifecycle fault matrix (`Degraded`/`Recovering`/…) |
| 9–10 | Optional OpenVPN profile staging+hash (beyond executable harden) |
| 11–14 | Stabilize races found; log phase transitions; internal RC |

---

## 9. Final verdict (Claude)

BiFlow’s reliability posture is **structurally sound but not yet production-adequate** for a privileged network agent. The FSM, rollback, coarse reconcile, watchdog, and IPC privilege split are real and already working — most of Round 0’s “foundation” proposals solved problems that don’t exist here.

**Single highest-leverage gap:** the `resolve_binary(Some(path))` symlink/TOCTOU hole on the OpenVPN executable path — cheap to fix by copying the existing Mihomo pattern, sitting directly on a privileged spawn path.

---

## 10. Appendix — discussion artifacts

- ChatGPT chat (architecture): `https://chatgpt.com/c/6a9d4899-0fc4-83eb-8010-e98d51917fd1` (title: بررسی معماری BiFlow)  
- Claude: new-chat final review (2026-09-06)

---

*This document consolidates the full multi-party review for the BiFlow maintainers. Prefer §6–§9 for execution; earlier sections preserve the audit trail of how conclusions were reached and corrected against code.*
