import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const devSh = readFileSync(join(root, "dev.sh"), "utf8");
const profileRs = readFileSync(
  join(root, "crates", "iran-split-config", "src", "profile.rs"),
  "utf8",
);

/**
 * `dev.sh` provisions the transient helper and exports the development
 * profile. It cannot share the Rust implementation, so the variable names and
 * the subdirectory layout it assumes are a contract. `dev.sh` deriving a
 * different path than the app produced `INVALID_GENERATION` in the field
 * (ADR 0115), and silently ignored overrides made the installed helper a
 * fallback for a development run.
 */

const PROFILE_VARIABLES = [
  "BIFLOW_DEV_PROFILE",
  "BIFLOW_DEV_HELPER_SOCKET",
  "BIFLOW_DEV_SYSTEM_RUNTIME",
  "BIFLOW_DEV_MIHOMO_BINARY",
];

/** Strip comments so a documented variable does not satisfy the contract. */
function executableLines(source) {
  return source
    .split("\n")
    .filter((line) => !line.trimStart().startsWith("#"))
    .join("\n");
}

/** The only sources allowed to name a profile environment variable. */
const PROFILE_BOUNDARY = [
  "crates/iran-split-config/src/profile.rs",
  "src-tauri/src/profile.rs",
];

function rustSources(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (
      ["target", "node_modules", ".git", "vendor", "artifacts"].includes(
        entry.name,
      )
    ) {
      continue;
    }
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...rustSources(path));
    } else if (entry.name.endsWith(".rs")) {
      files.push(path);
    }
  }
  return files;
}

const devShExecutable = executableLines(devSh);

/**
 * Strip Rust line and doc comments, and everything from `mod tests {` onward.
 *
 * Test modules legitimately assert the variable names, and a test that reads
 * them is not a runtime env read. Scanning only the production half is the
 * same rule the `include_str!("lib.rs")` source contracts already follow:
 * otherwise the assertion matches itself.
 */
function executableRust(source) {
  const withoutTests = source.split(/^mod tests \{/m)[0];
  return withoutTests
    .split("\n")
    .filter((line) => {
      const trimmed = line.trimStart();
      return !trimmed.startsWith("//") && !trimmed.startsWith("///");
    })
    .join("\n");
}

/** Only explicit Windows-only cfgs exempt an item; `test` never does. */
function windowsOnlyCfg(expression) {
  const windows = /^(?:windows|target_os\s*=\s*"windows")$/;
  if (windows.test(expression.trim())) return true;
  const all = expression.match(/^all\((.*)\)$/);
  if (!all) return false;
  const terms = all[1].split(",").map((term) => term.trim());
  return (
    terms.some((term) => windows.test(term)) &&
    terms.every((term) => windows.test(term) || term === "test")
  );
}

/** Scan the conventional rustfmt test module, consuming cfgs per item. */
function platformIndependentWindowsReferences(source) {
  const offenders = [];
  let inTests = false;
  let moduleWindowsOnly = false;
  let pendingWindowsOnly = false;
  let currentItemWindowsOnly = false;
  let insideItem = false;
  source.split("\n").forEach((line, index) => {
    const trimmed = line.trim();
    const cfg = trimmed.match(/^#\[cfg\((.+)\)\]$/);
    if (cfg) {
      pendingWindowsOnly ||= windowsOnlyCfg(cfg[1]);
      return;
    }
    if (!trimmed || trimmed.startsWith("//") || trimmed.startsWith("#[")) {
      return;
    }
    if (/^mod tests \{/.test(trimmed)) {
      inTests = true;
      moduleWindowsOnly = pendingWindowsOnly;
      pendingWindowsOnly = false;
      return;
    }
    if (!inTests) {
      pendingWindowsOnly = false;
      return;
    }
    if (/^(?:pub )?(?:const|fn) [A-Za-z0-9_]+/.test(trimmed)) {
      insideItem = true;
      currentItemWindowsOnly = moduleWindowsOnly || pendingWindowsOnly;
      pendingWindowsOnly = false;
    }
    if (!insideItem || currentItemWindowsOnly) return;
    const names = trimmed.match(/\bWINDOWS_[A-Z0-9_]+\b/g);
    if (names) {
      offenders.push(
        `platform_paths.rs:${index + 1} references ${[...new Set(names)].join(", ")} in an unconditional test`,
      );
    }
  });
  return offenders;
}

describe("dev.sh development profile contract", () => {
  it("exports every variable the Rust profile reads", () => {
    for (const variable of PROFILE_VARIABLES) {
      assert.ok(
        profileRs.includes(`"${variable}"`),
        `profile.rs no longer reads ${variable}`,
      );
      assert.ok(
        new RegExp(`export ${variable}=`).test(devShExecutable),
        `dev.sh no longer exports ${variable}`,
      );
    }
  });

  it("exports the profile root before it starts the helper and the app", () => {
    const exportIndex = devShExecutable.indexOf("export BIFLOW_DEV_PROFILE=");
    assert.ok(exportIndex >= 0, "BIFLOW_DEV_PROFILE is never exported");
    // Match the call sites, not the function definitions above them.
    for (const [marker, pattern] of [
      ["prepare_dev_helper", /^\s*prepare_dev_helper\s*$/m],
      ["pnpm tauri dev", /^\s*(command\s+)?pnpm tauri dev\s*$/m],
    ]) {
      const match = devShExecutable.match(pattern);
      assert.ok(match, `dev.sh no longer runs ${marker}`);
      assert.ok(
        exportIndex < match.index,
        `BIFLOW_DEV_PROFILE must be exported before ${marker}, or the helper and the app resolve different profiles`,
      );
    }
  });

  it("creates the config, data, and cache subdirectories the app expects", () => {
    // UserResources derives these three from the profile root, so a missing
    // directory is a startup failure rather than a silent default.
    for (const subdirectory of ["config", "data", "cache"]) {
      assert.ok(
        devShExecutable.includes(`/${subdirectory}"`),
        `dev.sh no longer creates the ${subdirectory} subdirectory of the profile root`,
      );
    }
  });

  it("keeps the development helper off noexec and mutable-workspace paths", () => {
    // /run is typically noexec and the workspace is mutable; the root helper
    // must not execute either.
    assert.ok(
      /DEV_HELPER_LIB_ROOT="\/var\/lib\/biflow-dev-/.test(devShExecutable),
      "development helper executables must live under /var/lib/biflow-dev-<uid>",
    );
  });
});

describe("profile resolution contract", () => {
  it("reads the profile environment in exactly one place per boundary", () => {
    // The defect this ADR fixes was seven independent reconstructions. A new
    // `std::env::var_os("BIFLOW_DEV_PROFILE")` anywhere else silently
    // reintroduces the drift, and host Clippy cannot see it because most of
    // the call sites are `cfg`-gated per platform.
    const offenders = [];
    for (const file of rustSources(root)) {
      const relative = file
        .slice(root.length + 1)
        .split("\\")
        .join("/");
      if (PROFILE_BOUNDARY.includes(relative)) {
        continue;
      }
      const source = executableRust(readFileSync(file, "utf8"));
      for (const variable of PROFILE_VARIABLES) {
        if (source.includes(`"${variable}"`)) {
          offenders.push(`${relative} names ${variable}`);
        }
      }
    }
    assert.deepEqual(
      offenders,
      [],
      `only ${PROFILE_BOUNDARY.join(" and ")} may read the profile environment`,
    );
  });

  it("reaches every privileged installer only through the gated entry point", () => {
    // `helper_install::install_helper` is the single privileged provisioning
    // entry and it carries the development-profile refusal. A call to one of
    // the per-platform elevation routines from anywhere else would bypass that
    // gate — which is exactly how a development Connect reached the SYSTEM
    // installer before the guard moved into the installer (ADR 0115).
    const installers = ["install_linux", "install_windows", "install_macos"];
    const owner = "src-tauri/src/helper_install.rs";
    const offenders = [];
    for (const file of rustSources(root)) {
      const relative = file
        .slice(root.length + 1)
        .split("\\")
        .join("/");
      if (relative === owner) {
        continue;
      }
      const source = executableRust(readFileSync(file, "utf8"));
      for (const installer of installers) {
        // Match a call, not a doc mention.
        if (new RegExp(`\\b${installer}\\s*\\(`, "m").test(source)) {
          offenders.push(`${relative} calls ${installer} directly`);
        }
      }
    }
    assert.deepEqual(
      offenders,
      [],
      "privileged installers are reachable without the gate",
    );
  });

  it("gates provisioning inside the installer, not only at its call sites", () => {
    // A guard on the Tauri command alone left `prepare_stack_start` free to run
    // the production installer. Pin the gate to the installer entry itself.
    const installer = readFileSync(
      join(root, "src-tauri", "src", "helper_install.rs"),
      "utf8",
    );
    const body =
      executableRust(installer).split("pub async fn install_helper")[1] ?? "";
    const firstStatement =
      body.split(";").find((part) => part.trim().length > 0) ?? "";
    assert.ok(
      /ensure_provisioning_allowed/.test(firstStatement),
      "install_helper must check ensure_provisioning_allowed before it touches anything privileged",
    );
  });

  it("keeps extracted helper-path names out of every desktop cfg branch", () => {
    const source = readFileSync(
      join(root, "src-tauri", "src", "lib.rs"),
      "utf8",
    );
    const retired =
      /\b(?:PRODUCTION_HELPER_SOCKET|PRODUCTION_SYSTEM_RUNTIME|linux_helper_paths_with_overrides)\b/g;
    assert.deepEqual(source.match(retired) ?? [], []);
    // Reproduce the CI failure in memory, including its Linux-only cfg. Scan
    // the entire source, rather than dropping tests the host cannot compile.
    const regressed = `${source}
#[cfg(target_os = "linux")]
fn stale_helper_test() {
    assert_eq!(PRODUCTION_HELPER_SOCKET, PRODUCTION_SYSTEM_RUNTIME);
    linux_helper_paths_with_overrides(None, None, None);
}`;
    assert.equal((regressed.match(retired) ?? []).length, 3);
    assert.equal((source.match(/^\s*use super::\*;/gm) ?? []).length, 1);
  });

  it("keeps platform-named constants out of platform-independent tests", () => {
    // The staging test once asserted against `WINDOWS_HELPER_STAGING` while
    // comparing `generation_staging_for(false, ..)`. That constant is defined
    // `cfg(any(windows, test))`, so it *compiles* on Linux and macOS and
    // simply yields the wrong value there: the test fails on every host but
    // Windows. No compiler catches it, so ban the pattern outright — a
    // platform-independent test must compare against the platform's own
    // production constant, never a `WINDOWS_*` literal.
    const source = readFileSync(
      join(root, "src-tauri", "src", "platform_paths.rs"),
      "utf8",
    );
    assert.deepEqual(
      platformIndependentWindowsReferences(source),
      [],
      "a test depends on a platform-specific constant",
    );
  });

  it("detects the original Windows assertion regression in the first test", () => {
    const source = readFileSync(
      join(root, "src-tauri", "src", "platform_paths.rs"),
      "utf8",
    );
    const original =
      /(generation_staging_for\(false, &data\),\s*PathBuf::from\()PRODUCTION_GENERATION_STAGING(\))/;
    assert.match(source, original, "the original assertion was not located");
    const regressed = source.replace(
      original,
      "$1crate::helper_install::WINDOWS_HELPER_STAGING$2",
    );
    const offenders = platformIndependentWindowsReferences(regressed);
    assert.equal(offenders.length, 1);
    assert.match(offenders[0], /references WINDOWS_HELPER_STAGING/);
  });

  it("consumes module cfgs and exempts only Windows-only items", () => {
    const source = `#[cfg(test)]
mod tests {
    #[test]
    fn first() { assert_eq!(value, WINDOWS_FIRST); }
    #[cfg(windows)]
    #[test]
    fn windows_only() { assert_eq!(value, WINDOWS_ALLOWED); }
    #[cfg(any(windows, test))]
    #[test]
    fn every_test_host() { assert_eq!(value, WINDOWS_ANY); }
    #[cfg(test)]
    #[test]
    fn test_only() { assert_eq!(value, WINDOWS_TEST); }
    #[cfg(all(test, target_os = "windows"))]
    #[test]
    fn windows_and_test() { assert_eq!(value, WINDOWS_ALSO_ALLOWED); }
    #[test]
    fn after_windows() { assert_eq!(value, WINDOWS_AFTER); }
}`;
    const offenders = platformIndependentWindowsReferences(source);
    assert.equal(offenders.length, 4);
    for (const name of [
      "WINDOWS_FIRST",
      "WINDOWS_ANY",
      "WINDOWS_TEST",
      "WINDOWS_AFTER",
    ]) {
      assert.ok(
        offenders.some((offender) => offender.includes(name)),
        name,
      );
    }
  });

  it("routes the Mihomo binary through the profile policy, not the search result", () => {
    // The candidate list includes `PATH`, so a discovery hit is a production
    // binary. The choice must go through `mihomo_binary`, which discards it in
    // a development profile.
    const lib = readFileSync(join(root, "src-tauri", "src", "lib.rs"), "utf8");
    const body = executableRust(lib).split("fn create_services")[1] ?? "";
    const direct =
      body.match(/deps::first_existing\(&deps::mihomo_candidates/g) ?? [];
    assert.deepEqual(
      direct,
      [],
      "create_services resolves Mihomo before the profile policy sees it",
    );
    assert.ok(
      body.includes("platform_paths::mihomo_binary("),
      "create_services must resolve Mihomo through the profile policy",
    );
  });

  it("keeps unset and empty BIFLOW_DEV_PROFILE meaning production", () => {
    // The Rust side filters empty values; dev.sh must never export an empty
    // profile root, which would silently turn a development run into the
    // installed application.
    const exportLine = devShExecutable
      .split("\n")
      .find((line) => line.includes("export BIFLOW_DEV_PROFILE="));
    assert.ok(exportLine, "BIFLOW_DEV_PROFILE export not found");
    assert.ok(
      !exportLine.includes('BIFLOW_DEV_PROFILE=""'),
      "dev.sh must not export an empty development profile",
    );
  });

  it("never lets the app fall back to the installed helper", () => {
    // The Rust rule is: a development run resolves to an unreachable
    // placeholder when an override is missing. dev.sh therefore has to export
    // all three overrides before the app starts, which the ordering test
    // above covers.
    const desktopProfile = readFileSync(
      join(root, "src-tauri", "src", "profile.rs"),
      "utf8",
    );
    assert.ok(
      desktopProfile.includes("missing_endpoint"),
      "the desktop no longer has a missing-override placeholder for a development run",
    );
    assert.ok(
      desktopProfile.includes("development_overrides_missing"),
      "the desktop no longer warns about missing development overrides",
    );
  });
});
