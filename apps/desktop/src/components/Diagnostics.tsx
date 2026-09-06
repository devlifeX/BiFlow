import {
  CheckCircle2,
  CircleAlert,
  Download,
  FileText,
  FolderOpen,
  LoaderCircle,
  Play,
  RefreshCw,
  Route,
  Sparkles,
  Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { desktop } from "../api/desktop";
import type {
  ActiveConnection,
  DiagnosticsReport,
  DebugLogStatus,
  ExportResult,
  FreshStartReport,
  LogEntry,
  ReachabilityResult,
  RouteTestResult,
} from "../api/models";
import type { ConnectionGroup } from "../lib/connectionGroups";
import { formatGroupIps, groupConnections } from "../lib/connectionGroups";
import { isHiddenProductionHost } from "../lib/hiddenHosts";
import { extractHost } from "../lib/host";
import type { SortState } from "../lib/tableSort";
import { sortRows, toggleSort } from "../lib/tableSort";
import { useAppStore } from "../store/app";
import { clientColor, outboundLabel, ruleLabel } from "../lib/outbound";
import { FlowResult, OutboundSelect } from "./DirectRules";
import { SortHeader } from "./SortHeader";

export function Diagnostics({ report }: { report: DiagnosticsReport | null }) {
  const { t } = useTranslation();
  const { runDiagnostics, actionPending, pinRoute } = useAppStore();
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [level, setLevel] = useState("all");
  const [exported, setExported] = useState<ExportResult | null>(null);
  const [target, setTarget] = useState("");
  const [route, setRoute] = useState<RouteTestResult | null>(null);
  const [testing, setTesting] = useState(false);
  const [debugLog, setDebugLog] = useState<DebugLogStatus | null>(null);
  const [logAction, setLogAction] = useState<
    "refresh" | "reveal" | "delete" | null
  >(null);
  const [logMessage, setLogMessage] = useState<string | null>(null);
  const [freshStarting, setFreshStarting] = useState(false);
  const [freshStart, setFreshStart] = useState<FreshStartReport | null>(null);
  const [freshStartError, setFreshStartError] = useState<string | null>(null);
  const [moving, setMoving] = useState(false);
  const [moveError, setMoveError] = useState<string | null>(null);

  const refreshDebugLog = () => {
    setLogAction("refresh");
    setLogMessage(null);
    void desktop
      .debugLogStatus()
      .then(setDebugLog)
      .catch((error: unknown) =>
        setLogMessage(
          error instanceof Error
            ? error.message
            : "Could not inspect debug.log",
        ),
      )
      .finally(() => setLogAction(null));
  };

  useEffect(() => {
    void desktop
      .queryLogs()
      .then(setLogs)
      .catch(() => setLogs([]));
  }, [report]);

  useEffect(() => {
    refreshDebugLog();
  }, []);

  const visibleLogs =
    level === "all" ? logs : logs.filter((entry) => entry.level === level);

  return (
    <section
      aria-labelledby="diagnostics-title"
      className="flex flex-col gap-3 pb-2"
    >
      <div className="flex shrink-0 flex-wrap items-start justify-between gap-4">
        <header>
          <h1
            id="diagnostics-title"
            className="text-2xl font-semibold tracking-tight"
          >
            Diagnostics
          </h1>
          <p className="mt-1 text-sm text-muted">
            Check the helper, upstream, providers, tunnel cleanup, and routing
            in one bounded run.
          </p>
        </header>
        <button
          type="button"
          disabled={actionPending}
          onClick={() => void runDiagnostics()}
          className="inline-flex items-center gap-2 rounded-xl bg-brand px-4 py-3 font-semibold text-white disabled:opacity-50"
        >
          {actionPending ? (
            <LoaderCircle className="animate-spin" size={18} aria-hidden />
          ) : (
            <Play size={18} aria-hidden />
          )}
          Run full diagnostics
        </button>
      </div>

      <LiveConnectionsCard />

      <form
        className="rounded-2xl border border-ink/10 bg-surface p-3.5"
        onSubmit={(event) => {
          event.preventDefault();
          // Accept whatever the user pasted — a full URL, a host:port, an IP —
          // and test the host a routing rule would actually match.
          const host = extractHost(target);
          if (!host) return;
          setTesting(true);
          setMoveError(null);
          void desktop
            .testRoute(host)
            .then(setRoute)
            .finally(() => setTesting(false));
        }}
      >
        <h2 className="font-semibold">{t("testFlow")}</h2>
        <p className="mt-1 text-sm text-muted">{t("testFlowHelp")}</p>
        <div className="mt-3 flex flex-col gap-2 sm:flex-row">
          <label className="sr-only" htmlFor="flow-target">
            {t("testFlow")}
          </label>
          <input
            id="flow-target"
            value={target}
            onChange={(event) => setTarget(event.target.value)}
            placeholder={t("testFlowPlaceholder")}
            className="min-w-0 flex-1 rounded-xl border-ink/15 bg-canvas"
          />
          <button
            type="submit"
            disabled={testing || !extractHost(target)}
            className="inline-flex items-center justify-center gap-2 rounded-xl bg-brand px-4 py-2.5 font-semibold text-white disabled:opacity-50"
          >
            <Route size={18} aria-hidden />
            {t("testFlowButton")}
          </button>
        </div>
        {route ? (
          <div className="mt-4">
            <FlowResult
              route={route}
              moving={moving}
              onMove={(host, destination) => {
                setMoving(true);
                setMoveError(null);
                // pinRoute moves the host between the two user lists; the
                // re-test then shows the outbound it actually resolves to.
                void pinRoute(host, destination)
                  .then(() => desktop.testRoute(host))
                  .then(setRoute)
                  .catch((error: unknown) =>
                    setMoveError(
                      error instanceof Error ? error.message : String(error),
                    ),
                  )
                  .finally(() => setMoving(false));
              }}
            />
            {moveError ? (
              <p className="mt-2 text-sm text-danger" role="alert">
                {moveError}
              </p>
            ) : null}
          </div>
        ) : null}
      </form>

      <div className="grid gap-3 xl:grid-cols-2">
        <ReachabilityCard />

        <div className="rounded-2xl border border-ink/10 bg-surface p-3.5">
          <h2 className="mb-3 font-semibold">Test timeline</h2>
          {!report ? (
            <p className="text-sm text-muted">
              No diagnostic run in this session.
            </p>
          ) : (
            <ol className="space-y-3">
              {report.steps.map((step) => (
                <li key={step.id} className="flex items-start gap-3">
                  {step.status === "passed" ? (
                    <CheckCircle2
                      className="mt-0.5 text-success"
                      size={19}
                      aria-hidden
                    />
                  ) : step.status === "warning" || step.status === "failed" ? (
                    <CircleAlert
                      className="mt-0.5 text-amber-500"
                      size={19}
                      aria-hidden
                    />
                  ) : (
                    <LoaderCircle
                      className="mt-0.5 text-muted"
                      size={19}
                      aria-hidden
                    />
                  )}
                  <div>
                    <p className="font-medium">{step.label}</p>
                    {step.detail ? (
                      <p className="text-sm text-muted">{step.detail}</p>
                    ) : null}
                  </div>
                </li>
              ))}
            </ol>
          )}
        </div>
      </div>

      <div className="grid gap-3 xl:grid-cols-3 [&>div]:flex [&>div]:h-full [&>div]:flex-col">
        <div className="rounded-2xl border border-ink/10 bg-surface p-3.5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <h2 className="font-semibold">{t("freshHiddify")}</h2>
              <p className="mt-1 max-w-prose text-sm text-muted">
                {t("freshHiddifyHelp")}
              </p>
            </div>
          </div>
          {freshStart ? (
            <div
              className="mt-4 space-y-1 rounded-xl bg-canvas p-3 text-sm"
              role="status"
            >
              <p className="font-medium">
                {freshStart.started
                  ? t("freshHiddifyDone")
                  : t("freshHiddifyNotStarted")}
              </p>
              <p className="text-muted">
                {t("freshHiddifyCleared")}:{" "}
                {freshStart.cleared.length > 0
                  ? freshStart.cleared.join(", ")
                  : t("freshHiddifyNothing")}
              </p>
              <p className="text-muted">
                {t("freshHiddifyKept")}: {freshStart.preserved.join(", ")}
              </p>
              {freshStart.cleared.length > 0 ? (
                <p className="min-w-0 break-all font-mono text-xs text-muted">
                  {t("freshHiddifyBackup")}: {freshStart.backup_dir}
                </p>
              ) : null}
            </div>
          ) : null}
          {freshStartError ? (
            <p
              className="mt-4 rounded-xl bg-canvas p-3 text-sm text-danger"
              role="alert"
            >
              {freshStartError}
            </p>
          ) : null}
          <div className="mt-auto flex flex-wrap items-center gap-2 border-t border-ink/10 pt-3">
            <button
              type="button"
              disabled={freshStarting}
              onClick={() => {
                if (!window.confirm(t("freshHiddifyConfirm"))) return;
                setFreshStarting(true);
                setFreshStart(null);
                setFreshStartError(null);
                void desktop
                  .freshHiddifyStart()
                  .then(setFreshStart)
                  .catch((error: unknown) =>
                    setFreshStartError(
                      error instanceof Error ? error.message : String(error),
                    ),
                  )
                  .finally(() => setFreshStarting(false));
              }}
              className="inline-flex items-center gap-1.5 rounded-lg border border-ink/15 px-3 py-1.5 text-xs font-semibold disabled:opacity-50"
            >
              {freshStarting ? (
                <LoaderCircle className="animate-spin" size={14} aria-hidden />
              ) : (
                <Sparkles size={14} aria-hidden />
              )}
              {t("freshHiddifyButton")}
            </button>
          </div>
        </div>
        <div className="rounded-2xl border border-ink/10 bg-surface p-3.5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <h2 className="font-semibold">Permanent debug.log</h2>
              <p className="mt-1 text-sm text-muted">
                Preserved across app restarts until you delete it. Send this
                file to the support team when reporting a problem.
              </p>
              {debugLog ? (
                <dl className="mt-3 space-y-1 text-sm">
                  <div className="flex gap-2">
                    <dt className="font-medium">Size:</dt>
                    <dd data-testid="debug-log-size">
                      {formatBytes(debugLog.size_bytes)}
                    </dd>
                  </div>
                  <div className="flex min-w-0 gap-2">
                    <dt className="shrink-0 font-medium">Location:</dt>
                    <dd className="min-w-0 break-all font-mono text-xs text-muted">
                      {debugLog.path}
                    </dd>
                  </div>
                </dl>
              ) : (
                <p className="mt-3 text-sm text-muted">Reading file status…</p>
              )}
            </div>
          </div>
          {logMessage ? (
            <p className="mt-3 rounded-xl bg-canvas p-3 text-sm" role="status">
              {logMessage}
            </p>
          ) : null}
          <div className="mt-auto flex flex-wrap items-center gap-2 border-t border-ink/10 pt-3">
            <button
              type="button"
              disabled={logAction !== null}
              onClick={refreshDebugLog}
              className="inline-flex items-center gap-2 rounded-lg border border-ink/15 px-3 py-1.5 text-xs font-semibold disabled:opacity-50"
            >
              <RefreshCw size={14} aria-hidden /> Refresh
            </button>
            <button
              type="button"
              disabled={logAction !== null || debugLog === null}
              onClick={() => {
                setLogAction("reveal");
                setLogMessage(null);
                void desktop
                  .revealDebugLog()
                  .then((status) => {
                    setDebugLog(status);
                    setLogMessage("Opened the folder containing debug.log.");
                  })
                  .catch((error: unknown) =>
                    setLogMessage(
                      error instanceof Error
                        ? error.message
                        : "Could not locate debug.log",
                    ),
                  )
                  .finally(() => setLogAction(null));
              }}
              className="inline-flex items-center gap-2 rounded-lg border border-ink/15 px-3 py-1.5 text-xs font-semibold disabled:opacity-50"
            >
              <FolderOpen size={14} aria-hidden /> Show file
            </button>
            <button
              type="button"
              disabled={logAction !== null || debugLog === null}
              onClick={() => {
                if (
                  !window.confirm(
                    "Delete all existing debug.log content? New app events will continue logging immediately.",
                  )
                ) {
                  return;
                }
                setLogAction("delete");
                setLogMessage(null);
                void desktop
                  .deleteDebugLog()
                  .then((status) => {
                    setDebugLog(status);
                    setLogMessage(
                      "Previous log content was deleted. New events are being recorded.",
                    );
                  })
                  .catch((error: unknown) =>
                    setLogMessage(
                      error instanceof Error
                        ? error.message
                        : "Could not delete debug.log",
                    ),
                  )
                  .finally(() => setLogAction(null));
              }}
              className="inline-flex items-center gap-2 rounded-lg border border-danger/30 px-3 py-1.5 text-xs font-semibold text-danger disabled:opacity-50"
            >
              <Trash2 size={14} aria-hidden /> Delete log
            </button>
          </div>
        </div>
        <div className="rounded-2xl border border-ink/10 bg-surface p-3.5">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="font-semibold">Support bundle</h2>
              <p className="mt-1 text-sm text-muted">
                Versions, redacted config, state, and the permanent debug.log.
              </p>
            </div>
          </div>
          {exported ? (
            <div
              className="mt-4 rounded-xl bg-canvas p-3 text-sm"
              role="status"
            >
              <p className="flex items-center gap-2 font-medium">
                <FileText size={17} aria-hidden /> {exported.path}
              </p>
              <p className="mt-1 text-muted">
                Included: {exported.files.join(", ")}
              </p>
            </div>
          ) : null}
          <div className="mt-auto flex flex-wrap items-center gap-2 border-t border-ink/10 pt-3">
            <button
              type="button"
              onClick={() => void desktop.exportBundle().then(setExported)}
              className="inline-flex items-center gap-1.5 rounded-lg border border-ink/15 px-3 py-1.5 text-xs font-semibold"
            >
              <Download size={14} aria-hidden /> Export
            </button>
          </div>
        </div>
      </div>

      <div className="rounded-2xl border border-ink/10 bg-surface p-3.5">
        <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
          <h2 className="font-semibold">Recent redacted logs</h2>
          <select
            aria-label="Log level"
            value={level}
            onChange={(event) => setLevel(event.target.value)}
            className="rounded-lg border-ink/15 bg-canvas py-1.5 text-sm"
          >
            <option value="all">All levels</option>
            <option value="info">Info</option>
            <option value="warn">Warning</option>
            <option value="error">Error</option>
          </select>
        </div>
        <div className="diagnostics-selectable max-h-48 overflow-auto rounded-xl bg-canvas p-3 font-mono text-xs">
          {visibleLogs.length === 0 ? (
            <p className="text-muted">No logs in this filter.</p>
          ) : (
            visibleLogs.map((entry, index) => (
              <p
                key={`${entry.timestamp}-${index}`}
                className="mb-1 whitespace-pre-wrap break-all"
              >
                <span className="text-muted">{entry.timestamp}</span>{" "}
                {entry.level.toUpperCase()} {entry.event}{" "}
                {JSON.stringify(entry.fields)}
              </p>
            ))
          )}
        </div>
      </div>
    </section>
  );
}

const REACHABILITY_DOT: Record<ReachabilityResult["status"], string> = {
  ok: "bg-success",
  slow: "bg-amber-500",
  unreachable: "bg-danger",
};

function reachabilityCauses(
  row: ReachabilityResult,
  t: (key: string) => string,
): { title: string; items: string[] } {
  if (row.status === "slow") {
    return {
      title: t("reachability.causeSlowTitle"),
      items: [t("reachability.causeSlow1")],
    };
  }
  if (row.path === "vpn" && !row.via_proxy) {
    return {
      title: t("reachability.causeVpnOfflineTitle"),
      items: [t("reachability.causeVpnOffline1")],
    };
  }
  if (row.path === "vpn") {
    return {
      title: t("reachability.causeVpnBlockedTitle"),
      items: [
        t("reachability.causeVpnBlocked1"),
        t("reachability.causeVpnBlocked2"),
        t("reachability.causeVpnBlocked3"),
      ],
    };
  }
  return {
    title: t("reachability.causeDirectTitle"),
    items: [
      t("reachability.causeDirect1"),
      t("reachability.causeDirect2"),
      t("reachability.causeDirect3"),
    ],
  };
}

function ReachabilityCard() {
  const { t } = useTranslation();
  const snapshot = useAppStore((state) => state.snapshot);
  const connected =
    snapshot?.phase === "running" || snapshot?.phase === "degraded";
  const [rows, setRows] = useState<ReachabilityResult[] | null>(null);
  const [checking, setChecking] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const runCheck = () => {
    setChecking(true);
    return desktop
      .checkReachability()
      .then((next) => {
        setRows(next);
        return next;
      })
      .catch(() => {
        setRows([]);
        return [] as ReachabilityResult[];
      })
      .finally(() => setChecking(false));
  };

  // Re-probe when the stack connects or disconnects: both the path taken and
  // the expected outcome change with it.
  useEffect(() => {
    void runCheck();
  }, [connected]);

  const visibleRows = (rows ?? []).filter(
    (row) => !isHiddenProductionHost(row.domain),
  );
  const selected = visibleRows.find((row) => row.id === selectedId) ?? null;
  const statusLabel = (status: ReachabilityResult["status"]) =>
    status === "ok"
      ? t("reachability.statusOk")
      : status === "slow"
        ? t("reachability.statusSlow")
        : t("reachability.statusUnreachable");

  return (
    <div
      data-testid="reachability"
      className="rounded-2xl border border-ink/10 bg-surface p-3.5"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="font-semibold">{t("reachability.title")}</h2>
          <p className="mt-1 text-sm text-muted">{t("reachability.help")}</p>
        </div>
        <button
          type="button"
          disabled={checking}
          onClick={() => void runCheck()}
          className="inline-flex items-center gap-2 rounded-xl border border-ink/15 px-3 py-2 text-sm font-semibold disabled:opacity-50"
        >
          {checking ? (
            <LoaderCircle className="animate-spin" size={16} aria-hidden />
          ) : (
            <RefreshCw size={16} aria-hidden />
          )}
          {t("reachability.refresh")}
        </button>
      </div>
      {rows === null ? (
        <p className="mt-3 text-sm text-muted">{t("reachability.checking")}</p>
      ) : (
        <ul className="mt-3 divide-y divide-ink/10 rounded-xl border border-ink/10">
          {visibleRows.map((row) => {
            const inner = (
              <>
                <span
                  className={`h-2.5 w-2.5 shrink-0 rounded-full ${REACHABILITY_DOT[row.status]}`}
                  aria-hidden
                />
                <span className="font-medium">{row.domain}</span>
                <span className="rounded-md bg-canvas px-2 py-0.5 text-xs font-semibold text-muted">
                  {row.path === "vpn"
                    ? t("reachability.pathVpn")
                    : t("reachability.pathDirect")}
                </span>
                <span className="ms-auto flex items-center gap-3 text-sm text-muted">
                  {row.latency_ms !== null ? (
                    <span className="font-mono text-xs">
                      {row.latency_ms} ms
                    </span>
                  ) : null}
                  <span>{statusLabel(row.status)}</span>
                </span>
              </>
            );
            return (
              <li key={row.id}>
                {row.status === "ok" ? (
                  <div className="flex items-center gap-3 px-3 py-2.5">
                    {inner}
                  </div>
                ) : (
                  <button
                    type="button"
                    onClick={() => setSelectedId(row.id)}
                    title={t("reachability.rowHint")}
                    className="flex w-full items-center gap-3 px-3 py-2.5 text-start hover:bg-canvas/60"
                  >
                    {inner}
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {selected ? (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-ink/40 p-4"
          onClick={() => setSelectedId(null)}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-label={t("reachability.modalTitle", {
              domain: selected.domain,
              status: statusLabel(selected.status),
            })}
            onClick={(event) => event.stopPropagation()}
            className="w-full max-w-md rounded-2xl border border-ink/10 bg-surface p-3.5 shadow-xl"
          >
            <h3 className="flex items-center gap-2 font-semibold">
              <span
                className={`h-2.5 w-2.5 rounded-full ${REACHABILITY_DOT[selected.status]}`}
                aria-hidden
              />
              {t("reachability.modalTitle", {
                domain: selected.domain,
                status: statusLabel(selected.status),
              })}
            </h3>
            {(() => {
              const causes = reachabilityCauses(selected, t);
              return (
                <div className="mt-3">
                  <p className="text-sm font-medium">{causes.title}</p>
                  <ul className="mt-2 list-disc space-y-2 ps-5 text-sm text-muted">
                    {causes.items.map((item) => (
                      <li key={item}>{item}</li>
                    ))}
                  </ul>
                </div>
              );
            })()}
            {selected.detail ? (
              <p className="mt-3 break-all rounded-xl bg-canvas p-2 font-mono text-xs text-muted">
                {selected.detail}
              </p>
            ) : null}
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setSelectedId(null)}
                className="rounded-xl border border-ink/15 px-3 py-2 text-sm font-semibold"
              >
                {t("reachability.close")}
              </button>
              <button
                type="button"
                disabled={checking}
                onClick={() =>
                  void runCheck().then((next) => {
                    const fresh = next.find((row) => row.id === selectedId);
                    if (fresh?.status === "ok") setSelectedId(null);
                  })
                }
                className="inline-flex items-center gap-2 rounded-xl bg-brand px-3 py-2 text-sm font-semibold text-white disabled:opacity-50"
              >
                {checking ? (
                  <LoaderCircle
                    className="animate-spin"
                    size={16}
                    aria-hidden
                  />
                ) : (
                  <RefreshCw size={16} aria-hidden />
                )}
                {t("reachability.tryAgain")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

type ConnectionSortKey = "host" | "count" | "ip" | "outbound" | "rule";

const CONNECTION_SORT_ACCESSORS: Record<
  ConnectionSortKey,
  (group: ConnectionGroup) => string | number
> = {
  host: (group) => group.host || group.ips[0] || "",
  count: (group) => group.count,
  ip: (group) => group.ips[0] ?? "",
  outbound: (group) => group.outbound,
  rule: (group) => group.rule,
};

function LiveConnectionsCard() {
  const { t } = useTranslation();
  const snapshot = useAppStore((state) => state.snapshot);
  const pinRoute = useAppStore((state) => state.pinRoute);
  const actionPending = useAppStore((state) => state.actionPending);
  const [rows, setRows] = useState<ActiveConnection[]>([]);
  const [sort, setSort] = useState<SortState<ConnectionSortKey>>({
    key: "count",
    dir: "desc",
  });
  const [query, setQuery] = useState("");
  const settings = useAppStore((state) => state.settings);
  const clients = settings?.clients ?? [];
  const [outboundFilter, setOutboundFilter] = useState("all");
  const [ruleFilter, setRuleFilter] = useState("all");
  const live = snapshot?.phase === "running" || snapshot?.phase === "degraded";

  useEffect(() => {
    if (!live) {
      setRows([]);
      return;
    }
    let cancelled = false;
    const load = () => {
      void desktop
        .listActiveConnections()
        .then((next) => {
          if (!cancelled) setRows(next);
        })
        .catch(() => {
          if (!cancelled) setRows([]);
        });
    };
    load();
    const timer = window.setInterval(load, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [live]);

  const groups = groupConnections(
    rows.filter((row) => !isHiddenProductionHost(row.host)),
  );
  const ruleOptions = [
    ...new Set(groups.map((group) => group.rule).filter(Boolean)),
  ].sort();
  // Keep a vanished rule visible in the select instead of blanking it; the
  // user can still switch back to "all rules".
  if (ruleFilter !== "all" && !ruleOptions.includes(ruleFilter)) {
    ruleOptions.push(ruleFilter);
  }
  const needle = query.trim().toLowerCase();
  const filtered = groups.filter(
    (group) =>
      (outboundFilter === "all" || group.outbound === outboundFilter) &&
      (ruleFilter === "all" || group.rule === ruleFilter) &&
      (needle === "" ||
        group.host.toLowerCase().includes(needle) ||
        group.ips.some((ip) => ip.toLowerCase().includes(needle))),
  );

  return (
    <div
      data-testid="live-connections"
      className="rounded-2xl border border-ink/10 bg-surface p-3.5"
    >
      <h2 className="font-semibold">{t("liveConnections")}</h2>
      <p className="mt-1 text-sm text-muted">{t("liveConnectionsHelp")}</p>
      {rows.length === 0 ? (
        <p className="mt-3 text-sm text-muted">{t("liveConnectionsEmpty")}</p>
      ) : (
        <>
          <div className="mt-3 flex flex-col gap-2 sm:flex-row">
            <input
              aria-label={t("liveConnectionsSearch")}
              placeholder={t("liveConnectionsSearch")}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              className="min-w-0 flex-1 rounded-xl border-ink/15 bg-canvas text-sm"
            />
            <select
              aria-label={t("liveConnectionsOutbound")}
              value={outboundFilter}
              onChange={(event) => setOutboundFilter(event.target.value)}
              className="rounded-xl border-ink/15 bg-canvas text-sm"
            >
              <option value="all">{t("liveConnectionsAllRoutes")}</option>
              <option value="direct">{t("direct")}</option>
              {clients
                .filter((client) => client.enabled)
                .map((client) => (
                  <option key={client.id} value={client.id}>
                    {outboundLabel(client.id, clients)}
                  </option>
                ))}
            </select>
            <select
              aria-label={t("liveConnectionsRule")}
              value={ruleFilter}
              onChange={(event) => setRuleFilter(event.target.value)}
              className="rounded-xl border-ink/15 bg-canvas text-sm"
            >
              <option value="all">{t("liveConnectionsAllRules")}</option>
              {ruleOptions.map((rule) => (
                <option key={rule} value={rule}>
                  {ruleLabel(rule, clients)}
                </option>
              ))}
            </select>
          </div>
          {filtered.length === 0 ? (
            <p className="mt-3 text-sm text-muted">
              {t("liveConnectionsNoMatch")}
            </p>
          ) : (
            <div className="mt-3 max-h-80 overflow-auto rounded-xl border border-ink/10">
              <table className="w-full min-w-[36rem] text-start text-sm">
                <thead className="sticky top-0 z-10">
                  <tr className="bg-canvas text-muted">
                    <SortHeader
                      label={t("liveConnectionsHost")}
                      sortKey="host"
                      state={sort}
                      onToggle={(key) =>
                        setSort((prev) => toggleSort(prev, key))
                      }
                    />
                    <SortHeader
                      label={t("liveConnectionsCount")}
                      sortKey="count"
                      state={sort}
                      onToggle={(key) =>
                        setSort((prev) => toggleSort(prev, key, "desc"))
                      }
                    />
                    <SortHeader
                      label={t("liveConnectionsIp")}
                      sortKey="ip"
                      state={sort}
                      onToggle={(key) =>
                        setSort((prev) => toggleSort(prev, key))
                      }
                    />
                    <SortHeader
                      label={t("liveConnectionsOutbound")}
                      sortKey="outbound"
                      state={sort}
                      onToggle={(key) =>
                        setSort((prev) => toggleSort(prev, key))
                      }
                    />
                    <SortHeader
                      label={t("liveConnectionsRule")}
                      sortKey="rule"
                      state={sort}
                      onToggle={(key) =>
                        setSort((prev) => toggleSort(prev, key))
                      }
                    />
                    <th className="px-3 py-2 text-start font-medium">
                      {t("tableActions")}
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-ink/10">
                  {sortRows(filtered, sort, CONNECTION_SORT_ACCESSORS).map(
                    (group) => {
                      const target = group.host || group.ips[0] || "";
                      return (
                        <tr key={group.key} className="hover:bg-canvas/60">
                          <td className="px-3 py-2 font-medium break-all">
                            {group.host || "—"}
                          </td>
                          <td className="px-3 py-2 font-mono text-xs text-muted">
                            {group.count}
                          </td>
                          <td className="px-3 py-2 font-mono text-xs text-muted">
                            {formatGroupIps(group.ips) || "—"}
                          </td>
                          <td className="px-3 py-2">
                            <span
                              className={`rounded-md px-2 py-0.5 text-xs font-semibold ${
                                group.outbound === "direct"
                                  ? "bg-success/10 text-success"
                                  : ""
                              }`}
                              style={
                                group.outbound === "direct"
                                  ? undefined
                                  : {
                                      color: clientColor(
                                        group.outbound,
                                        clients,
                                      ),
                                      backgroundColor: `${clientColor(group.outbound, clients)}1f`,
                                    }
                              }
                            >
                              {outboundLabel(group.outbound, clients)}
                            </span>
                          </td>
                          <td className="px-3 py-2 font-mono text-xs text-muted">
                            {group.rule ? ruleLabel(group.rule, clients) : "—"}
                          </td>
                          <td className="px-3 py-2">
                            {target ? (
                              <OutboundSelect
                                value={group.outbound}
                                clients={clients.filter((item) => item.enabled)}
                                disabled={actionPending}
                                onChange={(next) =>
                                  void pinRoute(target, next).catch(
                                    () => undefined,
                                  )
                                }
                              />
                            ) : null}
                          </td>
                        </tr>
                      );
                    },
                  )}
                </tbody>
              </table>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}
