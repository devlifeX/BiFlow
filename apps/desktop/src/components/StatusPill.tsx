import { PowerOff } from "lucide-react";
import type { ComponentPhase, StackPhase } from "../api/models";
import { statusChipLabel } from "../lib/statusChip";

const colors: Record<ComponentPhase | StackPhase, string> = {
  uninitialized: "bg-slate-400/10 text-slate-500",
  unknown: "bg-slate-400/10 text-slate-500",
  checking: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  stopped: "bg-slate-400/10 text-slate-500",
  starting: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  starting_client: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  preparing_runtime: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  validating_config: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  starting_core: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  checking_readiness: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  running: "bg-success/15 text-success",
  paused: "bg-sky-400/15 text-sky-700 dark:text-sky-300",
  degraded: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  unavailable: "bg-danger/15 text-danger",
  stopping: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  recovering: "bg-amber-400/15 text-amber-700 dark:text-amber-300",
  error: "bg-danger/15 text-danger",
};

export function StatusPill({
  phase,
  disabled = false,
}: {
  phase: ComponentPhase | StackPhase;
  disabled?: boolean;
}) {
  const label = statusChipLabel(phase, disabled);
  const tone = disabled
    ? "bg-slate-400/10 text-slate-500 opacity-60"
    : colors[phase];

  return (
    <span
      data-status-phase={phase}
      className={`inline-flex h-[18px] w-16 shrink-0 items-center justify-center gap-0.5 rounded-[5px] text-[10px] font-semibold leading-none ${tone}`}
    >
      {disabled ? (
        <PowerOff size={10} aria-hidden className="shrink-0" />
      ) : null}
      <span className="truncate">{label}</span>
    </span>
  );
}
