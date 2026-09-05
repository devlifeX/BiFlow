import type { ComponentPhase, StackPhase } from "../api/models";

const colors: Record<ComponentPhase | StackPhase, string> = {
  uninitialized: "bg-slate-400/15 text-slate-500",
  unknown: "bg-slate-400/15 text-slate-500",
  checking: "bg-sky-400/15 text-sky-600 dark:text-sky-300",
  stopped: "bg-slate-400/15 text-slate-500",
  starting: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  starting_client: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  preparing_runtime: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  validating_config: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  starting_core: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  checking_readiness: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  running: "bg-success/15 text-success",
  paused: "bg-sky-400/15 text-sky-700 dark:text-sky-300",
  degraded: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  unavailable: "bg-danger/15 text-danger",
  stopping: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  recovering: "bg-amber-400/15 text-amber-600 dark:text-amber-300",
  error: "bg-danger/15 text-danger",
};

export function StatusPill({ phase }: { phase: ComponentPhase | StackPhase }) {
  return (
    <span
      className={`inline-flex rounded-full px-2.5 py-1 text-xs font-semibold capitalize ${colors[phase]}`}
    >
      {phase.replaceAll("_", " ")}
    </span>
  );
}
