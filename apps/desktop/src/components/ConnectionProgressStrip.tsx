import type { StackSnapshot } from "../api/models";
import {
  busyAction,
  connectionButtonProgress,
} from "../lib/connectionProgress";

export function ConnectionProgressStrip({
  snapshot,
  installingId,
  actionPending,
}: {
  snapshot: StackSnapshot | null;
  installingId?: string | null;
  actionPending?: boolean;
}) {
  if (!snapshot?.busy) return null;
  const action = busyAction(snapshot.busy);
  if (!action) return null;
  const progress = connectionButtonProgress(
    snapshot,
    action,
    installingId,
    actionPending,
  );
  if (!progress.processing) return null;

  return (
    <div
      data-testid="connection-progress-strip"
      className="h-1 w-full shrink-0 bg-ink/10"
      aria-hidden
    >
      <div
        className="connection-progress-fill h-full bg-accent transition-[width] duration-300 motion-reduce:transition-none"
        style={{ width: `${progress.percent}%` }}
      />
    </div>
  );
}
