import { Download, Pause, Play, Power, PowerOff, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { StackSnapshot } from "../api/models";
import { controlsLocked, isOperating } from "../lib/lifecycle";
import { useAppStore } from "../store/app";
import { AppButton, BUTTON_ICON_PX } from "./AppButton";
import { ConnectionActionButton } from "./ConnectionActionButton";
import { LifecycleCancelButton } from "./LifecycleCancelButton";

export function BasicDashboard({ snapshot }: { snapshot: StackSnapshot }) {
  const { t } = useTranslation();
  const {
    actionPending,
    toggleConnection,
    pauseConnection,
    resumeConnection,
    cancel,
    error,
    installDependency,
    installingId,
  } = useAppStore();
  const active = snapshot.phase === "running" || snapshot.phase === "degraded";
  const paused = snapshot.phase === "paused";
  const locked = controlsLocked(snapshot, actionPending);
  const operating = isOperating(snapshot);
  const missing = snapshot.last_error?.remediation === "install_dependency";
  const missingId =
    snapshot.last_error?.code === "MIHOMO_NOT_FOUND" ? "mihomo" : "hiddify";
  const showError =
    error ?? (snapshot.last_error ? t(snapshot.last_error.message_key) : null);

  return (
    <section
      aria-labelledby="basic-dashboard-title"
      className="flex h-full flex-col items-center justify-center gap-6 px-4 text-center"
    >
      <div className="max-w-md space-y-2">
        <h1 id="basic-dashboard-title" className="text-2xl font-semibold">
          {active
            ? t("activeTitle")
            : paused
              ? t("pausedTitle")
              : t("readyTitle")}
        </h1>
        <p className="text-sm text-muted">{t("basicModeHelp")}</p>
      </div>

      {showError ? (
        <div
          className="w-full max-w-md rounded-2xl border border-danger/20 bg-danger/5 p-4 text-sm text-danger"
          role="alert"
        >
          <p>{showError}</p>
          {missing ? (
            <AppButton
              icon={<Download size={BUTTON_ICON_PX} aria-hidden />}
              className="mt-3 rounded-xl bg-brand px-4 py-2 font-semibold text-white"
              onClick={() => void installDependency(missingId)}
            >
              {t("install")} {missingId === "mihomo" ? "Mihomo" : "Hiddify"}
            </AppButton>
          ) : null}
        </div>
      ) : null}

      <div className="flex w-full max-w-xl flex-wrap items-center justify-center gap-3">
        {operating && snapshot.operation_id ? (
          <LifecycleCancelButton
            icon={<X size={BUTTON_ICON_PX} aria-hidden />}
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
            icon={<Pause size={BUTTON_ICON_PX} aria-hidden />}
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
            icon={<Play size={BUTTON_ICON_PX} aria-hidden />}
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
              <PowerOff size={BUTTON_ICON_PX} aria-hidden />
            ) : (
              <Power size={BUTTON_ICON_PX} aria-hidden />
            )
          }
          variant={paused ? "secondary" : "primary"}
        />
      </div>
    </section>
  );
}
