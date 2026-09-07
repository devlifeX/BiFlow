import type { ReactNode } from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { StackSnapshot } from "../api/models";
import { useAppStore } from "../store/app";
import { ClientRegistry } from "./ClientRegistry";
import { ComponentStatusList } from "./ComponentStatusList";
import { StatStrip } from "./StatStrip";

type DashboardTab = "status" | "components" | "clients" | "routes";

const TAB_ORDER: DashboardTab[] = ["status", "components", "clients", "routes"];

export function DashboardWorkbenchTabs({
  snapshot,
  routesPanel,
}: {
  snapshot: StackSnapshot;
  routesPanel: ReactNode;
}) {
  const { t } = useTranslation();
  const { boot, dependencies, installingId, installDependency, installHelper } =
    useAppStore();
  const active = snapshot.phase === "running" || snapshot.phase === "degraded";
  const [tab, setTab] = useState<DashboardTab>("status");

  const labels: Record<DashboardTab, string> = {
    status: t("dashboardTabStatus"),
    components: t("dashboardTabComponents"),
    clients: t("dashboardTabClients"),
    routes: t("dashboardTabRoutes"),
  };

  return (
    <div className="flex min-h-0 flex-col">
      <div
        role="tablist"
        aria-label={t("dashboard")}
        className="flex shrink-0 gap-0 border-b border-[rgb(var(--border-default))] bg-[rgb(var(--toolbar))]"
      >
        {TAB_ORDER.map((key) => {
          const disabled = key === "routes" && !active;
          return (
            <button
              key={key}
              type="button"
              role="tab"
              id={`dashboard-tab-${key}`}
              aria-selected={tab === key}
              aria-controls={`dashboard-panel-${key}`}
              disabled={disabled}
              onClick={() => setTab(key)}
              className={`h-8 shrink-0 border-b-2 px-3 text-[11px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-45 ${
                tab === key
                  ? "border-accent text-ink"
                  : "border-transparent text-muted hover:text-ink"
              }`}
            >
              {labels[key]}
            </button>
          );
        })}
      </div>

      <div className="min-h-0 flex-1 pt-3">
        {tab === "status" ? (
          <div
            role="tabpanel"
            id="dashboard-panel-status"
            aria-labelledby="dashboard-tab-status"
            className="flex flex-col gap-3"
          >
            <div>
              <p className="text-[11px] font-medium text-accent">
                {t("status")}
              </p>
              <h1
                id="dashboard-title"
                className="text-lg font-semibold leading-snug tracking-tight"
              >
                {snapshot.phase === "running" || snapshot.phase === "degraded"
                  ? t("activeTitle")
                  : snapshot.phase === "paused"
                    ? t("pausedTitle")
                    : t("readyTitle")}
              </h1>
              <p className="mt-1 max-w-2xl text-[12px] leading-snug text-muted">
                {t("routingSummary")}
              </p>
            </div>
            <StatStrip snapshot={snapshot} />
            <p className="text-[11px] text-muted">
              {t("lastUpdated")}:{" "}
              <span className="tabular-nums">
                {new Date(snapshot.updated_at).toLocaleTimeString()}
              </span>
              {boot?.mock_mode ? ` · ${t("mockMode")}` : ""}
            </p>
          </div>
        ) : null}

        {tab === "components" ? (
          <div
            role="tabpanel"
            id="dashboard-panel-components"
            aria-labelledby="dashboard-tab-components"
          >
            <ComponentStatusList
              snapshot={snapshot}
              dependencies={dependencies}
              installingId={installingId}
              onInstallHelper={() => void installHelper()}
              onInstallDependency={(id) => void installDependency(id)}
            />
          </div>
        ) : null}

        {tab === "clients" ? (
          <div
            role="tabpanel"
            id="dashboard-panel-clients"
            aria-labelledby="dashboard-tab-clients"
          >
            <ClientRegistry />
          </div>
        ) : null}

        {tab === "routes" && active ? (
          <div
            role="tabpanel"
            id="dashboard-panel-routes"
            aria-labelledby="dashboard-tab-routes"
          >
            {routesPanel}
          </div>
        ) : null}
      </div>
    </div>
  );
}
