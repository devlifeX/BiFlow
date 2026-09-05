import type { LifecycleBusy, StackPhase, StackSnapshot } from "../api/models";

export const TRANSITIONAL_PHASES: StackPhase[] = [
  "starting_client",
  "preparing_runtime",
  "validating_config",
  "starting_core",
  "checking_readiness",
  "stopping",
  "recovering",
];

export const ACTION_TIMEOUT_MS = 130_000;

export function snapshotBusy(
  snapshot: StackSnapshot | null | undefined,
): LifecycleBusy | null {
  return snapshot?.busy ?? null;
}

export function isOperating(
  snapshot: StackSnapshot | null | undefined,
): boolean {
  if (!snapshot) {
    return false;
  }
  return (
    snapshotBusy(snapshot) !== null ||
    TRANSITIONAL_PHASES.includes(snapshot.phase)
  );
}

export function controlsLocked(
  snapshot: StackSnapshot | null | undefined,
  actionPending: boolean,
): boolean {
  if (actionPending) {
    return true;
  }
  if (!snapshot) {
    return false;
  }
  return (
    snapshotBusy(snapshot) !== null ||
    TRANSITIONAL_PHASES.includes(snapshot.phase)
  );
}
