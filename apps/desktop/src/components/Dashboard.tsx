import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { desktop } from "../api/desktop";
import type { ClientInstance, StackSnapshot } from "../api/models";
import { isHiddenProductionHost } from "../lib/hiddenHosts";
import { clientColor } from "../lib/outbound";
import { presetById, type PresetId } from "../lib/presets";
import { useAppStore } from "../store/app";
import { DashboardWorkbenchTabs } from "./DashboardWorkbenchTabs";
import { StatusPill } from "./StatusPill";

export function Dashboard({ snapshot }: { snapshot: StackSnapshot }) {
  const active = snapshot.phase === "running" || snapshot.phase === "degraded";

  return (
    <section aria-labelledby="dashboard-title" className="flex flex-col pb-2">
      <DashboardWorkbenchTabs
        snapshot={snapshot}
        routesPanel={active ? <TrafficFlow /> : null}
      />
    </section>
  );
}

const FLOW_WIDTH = 760;
const FLOW_PACKET_LIFE_MS = 4_200;
const FLOW_PACKET_STAGGER_MS = 900;
const FLOW_MAX_PACKETS = 6;

type FlowBranch = {
  key: string;
  label: string;
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
        // Stack may be tearing down between polls.
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
    <section className="overflow-x-hidden rounded-[6px] border border-[rgb(var(--border-default))] bg-surface px-3 py-3">
      <h2 className="text-[13px] font-semibold">{t("liveRouting")}</h2>
      <p className="mt-1 text-[12px] leading-snug text-muted">
        {t("liveRoutingHelp")}
      </p>
      <div className="relative">
        <svg
          data-testid="live-routing"
          className="mt-3 h-auto w-full"
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
            className="absolute end-1 z-10 max-w-56 rounded-md border border-[rgb(var(--border-default))] bg-canvas p-2 text-[11px]"
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
                  <p className="mt-1 tabular-nums">
                    {t("exitIp")}: {selectedClientStatus.exit_ip}
                  </p>
                ) : null}
              </>
            ) : (
              <>
                <p className="mt-1 text-muted">{t("directTooltip")}</p>
                {directIp ? (
                  <p className="mt-1 tabular-nums">
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
