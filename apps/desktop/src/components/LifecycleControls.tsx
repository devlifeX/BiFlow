import { Pause, Play, Power, PowerOff, X } from "lucide-react";
import type { StackSnapshot } from "../api/models";
import { controlsLocked, isOperating } from "../lib/lifecycle";
import { useAppStore } from "../store/app";
import {
  CONNECTION_BUTTON_ICON_PX,
  ConnectionActionButton,
} from "./ConnectionActionButton";
import { LifecycleCancelButton } from "./LifecycleCancelButton";

export function LifecycleControls({ snapshot }: { snapshot: StackSnapshot }) {
  const {
    actionPending,
    toggleConnection,
    pauseConnection,
    resumeConnection,
    cancel,
    installingId,
  } = useAppStore();
  const active = snapshot.phase === "running" || snapshot.phase === "degraded";
  const paused = snapshot.phase === "paused";
  const locked = controlsLocked(snapshot, actionPending);
  const operating = isOperating(snapshot);

  return (
    <div className="flex shrink-0 flex-wrap items-center gap-2 sm:flex-nowrap">
      {operating && snapshot.operation_id ? (
        <LifecycleCancelButton
          icon={<X size={CONNECTION_BUTTON_ICON_PX} aria-hidden />}
          onClick={() => void cancel()}
        />
      ) : null}
      {active ? (
        <ConnectionActionButton
          action="pause"
          snapshot={snapshot}
          installingId={installingId}
          actionPending={actionPending}
          disabled={locked}
          onClick={() => void pauseConnection()}
          icon={<Pause size={CONNECTION_BUTTON_ICON_PX} aria-hidden />}
          variant="secondary"
        />
      ) : null}
      {paused ? (
        <ConnectionActionButton
          action="resume"
          snapshot={snapshot}
          installingId={installingId}
          actionPending={actionPending}
          disabled={locked}
          onClick={() => void resumeConnection()}
          icon={<Play size={CONNECTION_BUTTON_ICON_PX} aria-hidden />}
          variant="primary"
        />
      ) : null}
      <ConnectionActionButton
        action={active || paused ? "disconnect" : "connect"}
        snapshot={snapshot}
        installingId={installingId}
        actionPending={actionPending}
        disabled={locked}
        onClick={() => void toggleConnection()}
        icon={
          active || paused ? (
            <PowerOff size={CONNECTION_BUTTON_ICON_PX} aria-hidden />
          ) : (
            <Power size={CONNECTION_BUTTON_ICON_PX} aria-hidden />
          )
        }
        variant={paused ? "secondary" : "primary"}
      />
    </div>
  );
}
