import { Download } from "lucide-react";
import { useTranslation } from "react-i18next";
import logo from "../assets/logo.png";
import type { StackSnapshot } from "../api/models";
import { useAppStore } from "../store/app";
import { AppButton, BUTTON_ICON_PX } from "./AppButton";
import { ComponentStatusList } from "./ComponentStatusList";
import { LifecycleControls } from "./LifecycleControls";
import { SideTunnelRetryBanner } from "./SideTunnelRetryBanner";
import { StatStrip } from "./StatStrip";

export function BasicDashboard({ snapshot }: { snapshot: StackSnapshot }) {
  const { t } = useTranslation();
  const {
    error,
    installDependency,
    settings,
    dependencies,
    installingId,
    installHelper,
  } = useAppStore();
  const active = snapshot.phase === "running" || snapshot.phase === "degraded";
  const paused = snapshot.phase === "paused";
  const missing = snapshot.last_error?.remediation === "install_dependency";
  const missingId =
    snapshot.last_error?.code === "MIHOMO_NOT_FOUND" ? "mihomo" : "hiddify";
  const showError =
    error ?? (snapshot.last_error ? t(snapshot.last_error.message_key) : null);
  const clients = settings?.clients ?? [];

  return (
    <section
      aria-labelledby="basic-dashboard-title"
      className="flex flex-col gap-3 px-4 py-3 pb-2"
    >
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-[rgb(var(--border-default))] pb-3">
        <div className="flex min-w-0 items-center gap-2">
          <img
            src={logo}
            alt=""
            className="h-8 w-8 shrink-0 rounded-[5px] object-contain"
          />
          <div className="min-w-0 text-start">
            <p className="text-[11px] font-semibold text-brand">
              {t("appName")}
            </p>
            <h1
              id="basic-dashboard-title"
              className="truncate text-[13px] font-semibold leading-snug"
            >
              {active
                ? t("activeTitle")
                : paused
                  ? t("pausedTitle")
                  : t("readyTitle")}
            </h1>
          </div>
        </div>
        <LifecycleControls snapshot={snapshot} />
      </div>

      {showError ? (
        <div
          className="rounded-md border border-danger/20 bg-danger/5 px-3 py-2 text-[12px] text-danger"
          role="alert"
        >
          <p>{showError}</p>
          {missing ? (
            <AppButton
              icon={<Download size={BUTTON_ICON_PX} aria-hidden />}
              className="mt-2 h-[30px] rounded-[5px] bg-brand px-2 text-[11px] font-semibold text-white"
              onClick={() => void installDependency(missingId)}
            >
              {t("install")} {missingId === "mihomo" ? "Mihomo" : "Hiddify"}
            </AppButton>
          ) : null}
        </div>
      ) : null}

      <StatStrip snapshot={snapshot} />

      <div>
        <h2 className="mb-2 text-[13px] font-semibold">{t("components")}</h2>
        <ComponentStatusList
          snapshot={snapshot}
          dependencies={dependencies}
          installingId={installingId}
          onInstallHelper={() => void installHelper()}
          onInstallDependency={(id) => void installDependency(id)}
        />
      </div>

      {settings ? (
        <SideTunnelRetryBanner snapshot={snapshot} clients={clients} />
      ) : null}

      <p className="text-[11px] leading-snug text-muted">
        {t("basicModeHelp")}
      </p>
    </section>
  );
}
