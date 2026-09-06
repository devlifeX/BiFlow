import type { ComponentPhase, StackPhase } from "../api/models";

export function statusChipLabel(
  phase: ComponentPhase | StackPhase,
  disabled = false,
): string {
  if (disabled) return "Off";
  switch (phase) {
    case "running":
      return "Ready";
    case "starting":
    case "checking":
    case "starting_client":
    case "preparing_runtime":
    case "validating_config":
    case "starting_core":
    case "checking_readiness":
    case "stopping":
    case "recovering":
      return "Starting";
    case "paused":
      return "Paused";
    case "degraded":
      return "Degraded";
    case "error":
    case "unavailable":
      return "Failed";
    case "stopped":
    case "unknown":
    case "uninitialized":
    default:
      return "Idle";
  }
}
