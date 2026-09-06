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
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { desktop } from "../api/desktop";
import type {
  ClientInstance,
  ComponentStatus,
  StackSnapshot,
} from "../api/models";
import { controlsLocked, isOperating } from "../lib/lifecycle";
import { useAppStore } from "../store/app";
import { BUTTON_ICON_PX } from "./AppButton";
import { ConnectionActionButton } from "./ConnectionActionButton";
import { ClientRegistry } from "./ClientRegistry";
import { LifecycleCancelButton } from "./LifecycleCancelButton";
import { StatusPill } from "./StatusPill";
import { isHiddenProductionHost } from "../lib/hiddenHosts";
import { clientColor } from "../lib/outbound";
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
      className="flex flex-col gap-3 pb-2"
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
        <div className="flex w-full max-w-xl flex-wrap gap-3 sm:w-auto sm:flex-nowrap">
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
      </div>

      <div
        data-testid="provider-summary"
        className="rounded-2xl border border-ink/10 bg-surface p-3.5 shadow-card md:hidden"
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
    <div className="rounded-2xl border border-ink/10 bg-surface p-3.5 shadow-card">
      <div className="flex items-center gap-2">
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-brand/10 text-brand [&>svg]:h-4 [&>svg]:w-4">
          {icon}
        </span>
        <p className="text-xs text-muted">{label}</p>
      </div>
      <p className="mt-1.5 text-base font-semibold leading-snug break-words">
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
    <div className="rounded-2xl border border-ink/10 bg-surface p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <span
            className="shrink-0 text-muted [&>svg]:h-4 [&>svg]:w-4"
            aria-hidden
          >
            {icon}
          </span>
          <span className="min-w-0 truncate text-sm font-semibold">{name}</span>
        </div>
        <span className="shrink-0">
          <StatusPill phase={status.phase} />
        </span>
      </div>
      <p className="mt-2 line-clamp-2 min-h-8 text-xs leading-4 text-muted">
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

const FLOW_WIDTH = 760;
const FLOW_PACKET_LIFE_MS = 4_200;
const FLOW_PACKET_STAGGER_MS = 900;
const FLOW_MAX_PACKETS = 6;

type FlowBranch = {
  key: string;
  label: string;
  /** Concrete accent color; DIRECT is green, every client gets its own. */
  color: string;
  y: number;
  d: string;
  isDefault: boolean;
  client?: ClientInstance;
};

type FlowPacket = {
  id: string;
  label: string;
  branchKey: string;
  born: number;
  /** Alternates labels above/below the dot so they never overlap. */
  lane: 1 | -1;
};

function TrafficFlow() {
  const { t } = useTranslation();
  const settings = useAppStore((state) => state.settings);
  const snapshot = useAppStore((state) => state.snapshot);
  const clients = settings?.clients;
  const defaultClientId =
    settings?.default_route.kind === "client"
      ? settings.default_route.client_id
      : null;
  const defaultIsDirect = settings?.default_route.kind === "direct";
  const [selected, setSelected] = useState<string | null>(null);
  const [packets, setPackets] = useState<FlowPacket[]>([]);
  const recentPackets = useRef<Map<string, number>>(new Map());
  const laneFlip = useRef<1 | -1>(1);
  const pathRefs = useRef<Map<string, SVGPathElement>>(new Map());
  const packetRefs = useRef<Map<string, SVGGElement>>(new Map());
  const packetsRef = useRef<FlowPacket[]>([]);
  packetsRef.current = packets;

  // One branch for DIRECT plus one per enabled client; geometry grows with
  // the branch count so the diagram stays readable as clients are added.
  const { branches, height, deviceY } = useMemo(() => {
    const enabled = (clients ?? []).filter((client) => client.enabled);
    const rows: Array<Omit<FlowBranch, "y" | "d">> = [
      {
        key: "direct",
        label: t("direct"),
        color: "rgb(34 197 94)",
        isDefault: defaultIsDirect,
      },
      ...enabled.map((client) => ({
        key: client.id,
        label: presetById(client.preset as PresetId).title,
        color: clientColor(client.id, clients ?? []),
        isDefault: client.id === defaultClientId,
        client,
      })),
    ];
    const gap = 96;
    const top = 58;
    const svgHeight = Math.max(top + (rows.length - 1) * gap + 78, 210);
    const centerY = svgHeight / 2 - 6;
    const placed = rows.map((row, index) => {
      const y = top + index * gap;
      return {
        ...row,
        y,
        d: `M96 ${centerY} H292 C390 ${centerY} 410 ${y} 520 ${y} H660`,
      };
    });
    return { branches: placed, height: svgHeight, deviceY: centerY };
  }, [clients, defaultClientId, defaultIsDirect, t]);

  // Live packets: poll the active connections and float each new host along
  // its real route so the user sees which domain uses which client.
  // A stable signature keeps the polling effect from restarting on every
  // render (branch objects are rebuilt whenever settings re-memoize).
  const branchSignature = branches.map((branch) => branch.key).join("|");
  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    let stopped = false;
    const timers: number[] = [];
    const branchKeys = new Set(branchSignature.split("|"));
    const branchForOutbound = (outbound: string): string | null => {
      const value = outbound.toLowerCase();
      if (value === "direct") return "direct";
      const id = value.replace(/^(client|proxy)-/u, "");
      return branchKeys.has(id) ? id : null;
    };
    const tick = async () => {
      try {
        const rows = await desktop.listActiveConnections();
        if (stopped || media.matches) return;
        setPackets((previous) => {
          const next = [...previous];
          let added = false;
          let stagger = 0;
          for (const row of rows) {
            if (next.length >= FLOW_MAX_PACKETS) break;
            const branchKey = branchForOutbound(row.outbound);
            if (!branchKey) continue;
            const label = row.host || row.destination_ip;
            if (!label || isHiddenProductionHost(label)) continue;
            const dedupe = `${label}|${branchKey}`;
            const lastSeen = recentPackets.current.get(dedupe) ?? 0;
            if (Date.now() - lastSeen < FLOW_PACKET_LIFE_MS * 2) continue;
            recentPackets.current.set(dedupe, Date.now());
            added = true;
            laneFlip.current = laneFlip.current === 1 ? -1 : 1;
            const id = `${dedupe}|${Date.now()}`;
            next.push({
              id,
              label: label.length > 22 ? `${label.slice(0, 21)}…` : label,
              branchKey,
              // Staggered births keep simultaneous packets apart on the path.
              born: Date.now() + stagger,
              lane: laneFlip.current,
            });
            timers.push(
              window.setTimeout(() => {
                setPackets((current) =>
                  current.filter((packet) => packet.id !== id),
                );
              }, FLOW_PACKET_LIFE_MS + stagger),
            );
            stagger += FLOW_PACKET_STAGGER_MS;
          }
          return added ? next : previous;
        });
      } catch {
        // The stack may be tearing down between polls; skip this round.
      }
    };
    void tick();
    const interval = window.setInterval(() => void tick(), 2_500);
    return () => {
      stopped = true;
      window.clearInterval(interval);
      for (const timer of timers) window.clearTimeout(timer);
    };
  }, [branchSignature]);

  // SMIL animateMotion does not start reliably for dynamically inserted
  // nodes in the embedded webview, so packets are moved by hand along the
  // measured path every animation frame.
  const hasPackets = packets.length > 0;
  useEffect(() => {
    if (!hasPackets) return;
    let frame = 0;
    const step = () => {
      const now = Date.now();
      for (const packet of packetsRef.current) {
        const node = packetRefs.current.get(packet.id);
        const path = pathRefs.current.get(packet.branchKey);
        if (!node || !path) continue;
        const progress = Math.min(
          (now - packet.born) / (FLOW_PACKET_LIFE_MS - 200),
          1,
        );
        if (progress < 0) {
          node.setAttribute("opacity", "0");
          continue;
        }
        const point = path.getPointAtLength(progress * path.getTotalLength());
        node.setAttribute("transform", `translate(${point.x} ${point.y})`);
        // Soft fade at both ends instead of popping in and out.
        const fade =
          progress < 0.15
            ? progress / 0.15
            : progress > 0.82
              ? Math.max((1 - progress) / 0.18, 0)
              : 1;
        node.setAttribute("opacity", fade.toFixed(3));
      }
      frame = window.requestAnimationFrame(step);
    };
    frame = window.requestAnimationFrame(step);
    return () => window.cancelAnimationFrame(frame);
  }, [hasPackets]);

  const selectedBranch = branches.find((branch) => branch.key === selected);
  const selectedClientStatus = selectedBranch?.client
    ? snapshot?.clients.find((item) => item.id === selectedBranch.key)
    : null;
  const selectedStatus = selectedClientStatus?.status ?? null;
  const directIp = useAppStore((state) => state.networkStatus?.public_ip);

  return (
    <section className="overflow-x-hidden rounded-2xl border border-brand/15 bg-surface p-3.5 shadow-card">
      <h2 className="text-lg font-semibold">{t("liveRouting")}</h2>
      <p className="mt-1 text-sm text-muted">{t("liveRoutingHelp")}</p>
      <div className="relative">
        <svg
          data-testid="live-routing"
          className="mt-4 h-auto w-full"
          viewBox={`0 0 ${FLOW_WIDTH} ${height}`}
          role="img"
          aria-label={t("liveRoutingAria")}
        >
          {branches.map((branch) => (
            <g key={`route-${branch.key}`}>
              <path
                className="traffic-flow-base"
                d={branch.d}
                ref={(node) => {
                  if (node) pathRefs.current.set(branch.key, node);
                  else pathRefs.current.delete(branch.key);
                }}
              />
              <path
                className="traffic-flow-route"
                style={{ stroke: branch.color }}
                d={branch.d}
              />
            </g>
          ))}

          <circle cx="76" cy={deviceY} r="30" fill="rgb(var(--brand) / 0.12)" />
          <circle cx="76" cy={deviceY} r="9" fill="rgb(var(--brand))" />
          <text
            className="traffic-flow-label"
            x="76"
            y={deviceY + 48}
            textAnchor="middle"
          >
            {t("device")}
          </text>

          {branches.map((branch) => (
            <g
              key={`node-${branch.key}`}
              role="button"
              tabIndex={0}
              aria-label={`${branch.label} status`}
              className="cursor-pointer focus:outline-none"
              onClick={() =>
                setSelected((current) =>
                  current === branch.key ? null : branch.key,
                )
              }
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  setSelected((current) =>
                    current === branch.key ? null : branch.key,
                  );
                }
              }}
            >
              <circle
                cx="686"
                cy={branch.y}
                r="27"
                fill={branch.color}
                opacity="0.14"
              />
              <circle cx="686" cy={branch.y} r="8" fill={branch.color} />
              <text
                className="traffic-flow-label"
                x="686"
                y={branch.y + 42}
                textAnchor="middle"
              >
                {branch.label}
              </text>
              {branch.isDefault ? (
                <text
                  className="traffic-flow-default"
                  x="686"
                  y={branch.y - 36}
                  textAnchor="middle"
                >
                  {t("matchDefault")}
                </text>
              ) : null}
            </g>
          ))}

          {packets.map((packet) => {
            const branch = branches.find(
              (item) => item.key === packet.branchKey,
            );
            if (!branch) return null;
            return (
              <g
                key={packet.id}
                className="traffic-flow-packet"
                ref={(node) => {
                  if (node) packetRefs.current.set(packet.id, node);
                  else packetRefs.current.delete(packet.id);
                }}
              >
                <circle
                  r="6"
                  fill={branch.color}
                  stroke="rgb(var(--surface))"
                  strokeWidth="1.5"
                />
                <text
                  className="traffic-flow-packet-label"
                  y={packet.lane === 1 ? -12 : 23}
                  textAnchor="middle"
                >
                  {packet.label}
                </text>
              </g>
            );
          })}
        </svg>

        {selectedBranch ? (
          <div
            role="status"
            className="absolute end-1 z-10 max-w-56 rounded-xl border border-ink/15 bg-canvas p-3 text-xs shadow-card"
            style={{ top: `${(selectedBranch.y / height) * 100}%` }}
          >
            <p className="font-semibold">{selectedBranch.label}</p>
            {selectedBranch.client ? (
              <>
                <div className="mt-1">
                  <StatusPill phase={selectedStatus?.phase ?? "stopped"} />
                </div>
                {selectedStatus?.message ? (
                  <p className="mt-1 text-muted">{selectedStatus.message}</p>
                ) : null}
                {selectedClientStatus?.exit_ip ? (
                  <p className="mt-1 font-mono">
                    {t("exitIp")}: {selectedClientStatus.exit_ip}
                  </p>
                ) : null}
              </>
            ) : (
              <>
                <p className="mt-1 text-muted">{t("directTooltip")}</p>
                {directIp ? (
                  <p className="mt-1 font-mono">
                    {t("exitIp")}: {directIp}
                  </p>
                ) : null}
              </>
            )}
          </div>
        ) : null}
      </div>
    </section>
  );
}
