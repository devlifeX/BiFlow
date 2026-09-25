import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  planSteps,
  readWorkflow,
  SETUP_ONLY,
  workflowRunCommands,
} from "./ci-local.mjs";

/**
 * @param {string} workflow
 * @param {string} job
 */
function jobBlock(workflow, job) {
  const normalized = workflow.replaceAll("\r\n", "\n");
  const normalizedStart = normalized.indexOf(`\n  ${job}:\n`);
  assert.notEqual(normalizedStart, -1, `job ${job} is missing`);
  const rest = normalized.slice(normalizedStart + 1);
  const next = rest.slice(1).search(/\n {2}[a-z][a-z-]*:\n/);
  return next === -1 ? rest : rest.slice(0, next + 1);
}

describe("ci-local", () => {
  it("mirrors every verification command in ci.yml", () => {
    const mirrored = new Set(
      planSteps("linux").steps.map((step) => step.ciCommand),
    );
    const unmirrored = workflowRunCommands(readWorkflow("ci.yml")).filter(
      (command) => !mirrored.has(command) && !SETUP_ONLY.includes(command),
    );

    assert.deepEqual(
      unmirrored,
      [],
      "add these to scripts/ci-local.mjs or to SETUP_ONLY",
    );
  });

  it("mirrors the cargo and pnpm gates in the release verify job", () => {
    const mirrored = new Set(
      planSteps("linux").steps.map((step) => step.ciCommand),
    );
    const verify = jobBlock(readWorkflow("release.yml"), "verify");
    const gates = workflowRunCommands(verify).filter((command) =>
      /^(cargo|pnpm) /.test(command),
    );

    assert.ok(gates.length > 0, "release verify runs no cargo or pnpm gate");
    assert.deepEqual(
      gates.filter(
        (command) => !mirrored.has(command) && !SETUP_ONLY.includes(command),
      ),
      [],
    );
  });

  it("cross-compiles the Windows job from a Linux host", () => {
    const { steps, gaps } = planSteps("linux");
    const windows = steps.find((step) => step.id === "clippy-windows");

    assert.ok(windows, "the windows-2025 Clippy job is not mirrored");
    assert.equal(windows.runner, "windows-2025");
    assert.deepEqual(windows.command, [
      "cargo",
      "xwin",
      "clippy",
      "--workspace",
      "--all-targets",
      "--target",
      "x86_64-pc-windows-msvc",
      "--",
      "-D",
      "warnings",
    ]);
    assert.equal(windows.requires, "cargo-xwin");
    assert.match(gaps.join("\n"), /windows-2025/);
  });

  it("executes the Windows test binaries, not just type-checks them", () => {
    const tests = planSteps("linux").steps.find(
      (step) => step.id === "test-windows",
    );

    assert.ok(tests, "windows-2025 `cargo test` is not mirrored");
    assert.equal(tests.ciCommand, "cargo test --workspace");
    assert.equal(tests.runner, "windows-2025");
    assert.equal(tests.requires, "wine");
    assert.ok(tests.command.includes("x86_64-pc-windows-msvc"));
    assert.ok(tests.command.includes("test"));
  });

  it("reports the Linux job as uncovered when run from Windows", () => {
    const { steps, gaps } = planSteps("win32");
    assert.equal(
      steps.find((step) => step.id === "clippy-windows"),
      undefined,
    );
    assert.equal(
      steps.find((step) => step.id === "clippy-host").runner,
      "windows-2025",
    );
    assert.match(gaps.join("\n"), /ubuntu-24\.04/);
  });

  it("provisions Playwright browsers before the e2e step", () => {
    const ids = planSteps("linux").steps.map((step) => step.id);
    assert.ok(
      ids.indexOf("browsers") >= 0 &&
        ids.indexOf("browsers") < ids.indexOf("e2e"),
      "e2e fails on a stale browser cache unless install runs first",
    );
  });

  it("orders the Windows Clippy step after the host one", () => {
    const ids = planSteps("linux").steps.map((step) => step.id);
    assert.equal(ids.indexOf("clippy-windows"), ids.indexOf("clippy-host") + 1);
  });

  it("declares an install command for every required executable", () => {
    for (const step of planSteps("linux").steps) {
      if (!step.requires) continue;
      assert.ok(step.install, `${step.id} requires ${step.requires} silently`);
    }
  });

  it("reads inline and block run commands", () => {
    const workflow = [
      "jobs:",
      "  demo:",
      "    steps:",
      "      - run: pnpm check",
      "      - name: Multi",
      "        shell: bash",
      "        run: |",
      "          set -euo pipefail",
      "          echo hi",
      "      - run: cargo fmt --all --check",
      "",
    ].join("\n");

    assert.deepEqual(workflowRunCommands(workflow), [
      "pnpm check",
      "set -euo pipefail\necho hi",
      "cargo fmt --all --check",
    ]);
  });
});
