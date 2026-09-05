import {
  Activity,
  ArrowDownUp,
  CircleDot,
  Download,
  Gauge,
  Globe2,
  LoaderCircle,
  Network,
  Pause,
  Play,
  Power,
  PowerOff,
  ShieldCheck,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ComponentStatus, StackSnapshot } from "../api/models";
import { controlsLocked, isOperating } from "../lib/lifecycle";
import { useAppStore } from "../store/app";
import { AppButton, BUTTON_ICON_PX } from "./AppButton";
import { ConnectionActionButton } from "./ConnectionActionButton";
import { ClientRegistry } from "./ClientRegistry";
import { StatusPill } from "./StatusPill";
import { presetById, type PresetId } from "../lib/presets";

export function Dashboard({ snapshot }: { snapshot: StackSnapshot }) {
  const { t } = useTranslation();
  const {
    actionPending,
    toggleConnection,
    pauseConnection,
    resumeConnection,
    cancel,
    boot,
    dependencies,
    installingId,
    installDependency,
    installHelper,
  } = useAppStore();
  const active = snapshot.phase === "running" || snapshot.phase === "degraded";
  const paused = snapshot.phase === "paused";
  const locked = controlsLocked(snapshot, actionPending);
  const operating = isOperating(snapshot);
  const needsAttention = [
    snapshot.helper,
    ...snapshot.clients.map((client) => client.status),
    snapshot.mihomo,
    snapshot.tun,
    snapshot.dns,
  ].some(({ phase }) => phase === "error" || phase === "unavailable");

  return (
    <section
      aria-labelledby="dashboard-title"
      className="flex flex-col gap-4 pb-2"
    >
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <p className="mb-1 text-sm font-medium text-brand">{t("status")}</p>
          <h1
            id="dashboard-title"
            className="text-2xl font-semibold tracking-tight"
          >
            {active
              ? t("activeTitle")
              : paused
                ? t("pausedTitle")
                : needsAttention
                  ? t("setupNeedsAttention")
                  : t("readyTitle")}
          </h1>
          <p className="mt-2 max-w-2xl text-muted">{t("routingSummary")}</p>
        </div>
        <div className="flex w-full max-w-xl flex-wrap gap-3 sm:w-auto">
          {operating && snapshot.operation_id ? (
            <AppButton
              icon={<X size={BUTTON_ICON_PX} aria-hidden />}
              onClick={() => void cancel()}
              className="rounded-2xl border border-ink/15 bg-surface px-5 py-3.5 font-semibold"
            >
              {t("cancel")}
            </AppButton>
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
      </div>

      <div
        data-testid="provider-summary"
        className="rounded-2xl border border-ink/10 bg-surface p-4 shadow-card md:hidden"
      >
        <p className="text-sm text-muted">{t("providers")}</p>
        <p className="mt-1 text-lg font-semibold">
          {snapshot.providers.ready} / {snapshot.providers.total}
        </p>
        <p className="mt-1 text-xs text-muted">
          {t("rulesLoaded")}: {snapshot.providers.rules_loaded.toLocaleString()}
        </p>
      </div>
      <div className="hidden gap-4 md:grid md:grid-cols-3">
        <Metric
          icon={<Globe2 aria-hidden />}
          label={t("exitIp")}
          value={snapshot.exit_ip ?? t("noExitIp")}
        />
        <Metric
          icon={<Network aria-hidden />}
          label={t("backend")}
          value={t("clientsTitle")}
        />
        <Metric
          icon={<Gauge aria-hidden />}
          label={t("providers")}
          value={`${snapshot.providers.ready} / ${snapshot.providers.total}`}
        />
      </div>

      <div>
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-lg font-semibold">{t("components")}</h2>
          <StatusPill phase={snapshot.phase} />
        </div>
        <div
          data-testid="connection-status-strip"
          className="grid grid-cols-5 gap-2 rounded-2xl border border-ink/10 bg-surface p-3 shadow-card md:hidden"
        >
          <StatusLight name={t("helper")} phase={snapshot.helper.phase} />
          {snapshot.clients.map((client) => (
            <StatusLight
              key={client.id}
              name={presetById(client.preset as PresetId).title}
              phase={client.status.phase}
            />
          ))}
          <StatusLight name="Mihomo" phase={snapshot.mihomo.phase} />
          <StatusLight name="TUN" phase={snapshot.tun.phase} />
          <StatusLight name="DNS" phase={snapshot.dns.phase} />
        </div>
        <div className="hidden gap-3 sm:grid-cols-2 md:grid lg:grid-cols-3 xl:grid-cols-5">
          <Component
            name={t("helper")}
            status={snapshot.helper}
            icon={<ShieldCheck />}
            installed={
              snapshot.helper.phase !== "unavailable" &&
              snapshot.helper.phase !== "error"
            }
            installing={installingId === "helper"}
            onInstall={() => void installHelper()}
          />
          {snapshot.clients.map((client) => (
            <Component
              key={client.id}
              name={presetById(client.preset as PresetId).title}
              status={client.status}
              icon={<CircleDot />}
              installed={
                client.preset === "hiddify"
                  ? dependencies.find((item) => item.id === "hiddify")
                      ?.installed
                  : true
              }
              installing={
                client.preset === "hiddify" && installingId === "hiddify"
              }
              onInstall={
                client.preset === "hiddify"
                  ? () => void installDependency("hiddify")
                  : undefined
              }
            />
          ))}
          <Component
            name="Mihomo"
            status={snapshot.mihomo}
            icon={<Activity />}
            installed={
              dependencies.find((item) => item.id === "mihomo")?.installed
            }
            installing={installingId === "mihomo"}
            onInstall={() => void installDependency("mihomo")}
          />
          <Component name="TUN" status={snapshot.tun} icon={<ArrowDownUp />} />
          <Component name="DNS" status={snapshot.dns} icon={<Network />} />
        </div>
      </div>

      <ClientRegistry />

      {active ? <TrafficFlow /> : null}

      <p className="text-xs text-muted">
        {t("lastUpdated")}: {new Date(snapshot.updated_at).toLocaleTimeString()}
        {boot?.mock_mode ? ` · ${t("mockMode")}` : ""}
      </p>
    </section>
  );
}

function StatusLight({
  name,
  phase,
}: {
  name: string;
  phase: ComponentStatus["phase"];
}) {
  const tone =
    phase === "running"
      ? "bg-success"
      : phase === "error" || phase === "unavailable"
        ? "bg-danger"
        : phase === "starting" || phase === "checking" || phase === "degraded"
          ? "bg-amber-400"
          : "bg-slate-400";
  return (
    <div className="flex min-w-0 flex-col items-center gap-1 text-center">
      <span
        className={`h-3 w-3 rounded-full ${tone}`}
        data-status-light={phase}
        aria-hidden
      />
      <span className="max-w-full truncate text-[0.65rem] font-medium">
        {name}
      </span>
      <span className="sr-only">
        {name}: {phase}
      </span>
    </div>
  );
}

function Metric({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-2xl border border-ink/10 bg-surface p-5 shadow-card">
      <div className="mb-4 flex h-10 w-10 items-center justify-center rounded-xl bg-brand/10 text-brand">
        {icon}
      </div>
      <p className="text-sm text-muted">{label}</p>
      <p className="mt-1 text-lg font-semibold leading-snug break-words">
        {value}
      </p>
    </div>
  );
}

function Component({
  name,
  status,
  icon,
  installed,
  installing,
  onInstall,
}: {
  name: string;
  status: ComponentStatus;
  icon: React.ReactNode;
  installed?: boolean;
  installing?: boolean;
  onInstall?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="rounded-2xl border border-ink/10 bg-surface p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <span className="text-muted" aria-hidden>
            {icon}
          </span>
          <span className="font-semibold">{name}</span>
        </div>
        <StatusPill phase={status.phase} />
      </div>
      <p className="mt-3 min-h-10 text-xs leading-5 text-muted">
        {status.message ?? t("statusDetailUnavailable")}
      </p>
      {installed === false && onInstall ? (
        <button
          type="button"
          disabled={installing}
          onClick={onInstall}
          className="mt-3 inline-flex items-center gap-1.5 rounded-lg bg-brand px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-50"
        >
          {installing ? (
            <LoaderCircle className="animate-spin" size={14} aria-hidden />
          ) : (
            <Download size={14} aria-hidden />
          )}
          {installing ? t("installing") : t("install")}
        </button>
      ) : null}
    </div>
  );
}

function TrafficFlow() {
  const { t } = useTranslation();
  return (
    <section className="overflow-x-hidden rounded-2xl border border-brand/15 bg-surface p-5 shadow-card">
      <h2 className="text-lg font-semibold">{t("liveRouting")}</h2>
      <p className="mt-1 text-sm text-muted">{t("liveRoutingHelp")}</p>
      <svg
        data-testid="live-routing"
        className="mt-4 h-auto w-full"
        viewBox="0 0 760 220"
        role="img"
        aria-label={t("liveRoutingAria")}
      >
        <defs>
          <marker
            id="traffic-arrow-direct"
            markerWidth="8"
            markerHeight="8"
            refX="7"
            refY="4"
            orient="auto"
          >
            <path d="M0,0 L8,4 L0,8 Z" fill="rgb(var(--success))" />
          </marker>
          <marker
            id="traffic-arrow-vpn"
            markerWidth="8"
            markerHeight="8"
            refX="7"
            refY="4"
            orient="auto"
          >
            <path d="M0,0 L8,4 L0,8 Z" fill="rgb(var(--brand))" />
          </marker>
        </defs>
        <path
          className="traffic-flow-base"
          d="M96 110 H292 C390 110 410 55 520 55 H660"
        />
        <path
          className="traffic-flow-base"
          d="M96 110 H292 C390 110 410 165 520 165 H660"
        />
        <path
          className="traffic-flow-route traffic-flow-route-direct"
          d="M96 110 H292 C390 110 410 55 520 55 H660"
          markerEnd="url(#traffic-arrow-direct)"
        />
        <path
          className="traffic-flow-route traffic-flow-route-vpn"
          d="M96 110 H292 C390 110 410 165 520 165 H660"
          markerEnd="url(#traffic-arrow-vpn)"
        />
        <circle cx="76" cy="110" r="30" fill="rgb(var(--brand) / 0.12)" />
        <circle cx="76" cy="110" r="9" fill="rgb(var(--brand))" />
        <circle cx="686" cy="55" r="27" fill="rgb(var(--success) / 0.12)" />
        <circle cx="686" cy="55" r="8" fill="rgb(var(--success))" />
        <circle cx="686" cy="165" r="27" fill="rgb(var(--brand) / 0.12)" />
        <circle cx="686" cy="165" r="8" fill="rgb(var(--brand))" />
        <text className="traffic-flow-label" x="76" y="158" textAnchor="middle">
          {t("device")}
        </text>
        <text className="traffic-flow-label" x="686" y="97" textAnchor="middle">
          {t("direct")}
        </text>
        <text
          className="traffic-flow-label"
          x="686"
          y="207"
          textAnchor="middle"
        >
          {t("vpn")}
        </text>
      </svg>
    </section>
  );
}
