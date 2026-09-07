import { useTranslation } from "react-i18next";
import type { StackSnapshot } from "../api/models";
import { LifecycleControls } from "./LifecycleControls";

export function LifecycleActionBar({ snapshot }: { snapshot: StackSnapshot }) {
  const { t } = useTranslation();
  const phaseLabel =
    snapshot.phase === "running" || snapshot.phase === "degraded"
      ? t("activeTitle")
      : snapshot.phase === "paused"
        ? t("pausedTitle")
        : t("readyTitle");

  return (
    <div
      data-testid="lifecycle-action-bar"
      className="app-action-bar shrink-0 border-t border-[rgb(var(--border-default))] bg-[rgb(var(--toolbar))] px-4 py-2"
    >
      <div className="mx-auto flex w-full max-w-6xl flex-wrap items-center justify-between gap-2">
        <p className="min-w-0 truncate text-[11px] text-muted">
          {t("status")}:{" "}
          <span className="font-semibold text-ink">{phaseLabel}</span>
        </p>
        <LifecycleControls snapshot={snapshot} />
      </div>
    </div>
  );
}
