import {
  CloudDownload,
  ListChecks,
  LoaderCircle,
  Plus,
  RefreshCw,
  Route,
  Search,
  Trash2,
} from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { desktop } from "../api/desktop";
import type {
  ClientInstance,
  DirectRulesDocument,
  ListCheckEntry,
  PinnedRoute,
  RouteTestResult,
  RuleListMeta,
} from "../api/models";
import { outboundKey, outboundLabel } from "../lib/outbound";
import type { SortState } from "../lib/tableSort";
import { sortRows, toggleSort } from "../lib/tableSort";
import { useAppStore } from "../store/app";
import { SortHeader } from "./SortHeader";

type PinnedRow = { rule: PinnedRoute };
type PinnedSortKey = "target" | "kind" | "outbound";

const PINNED_SORT_ACCESSORS: Record<
  PinnedSortKey,
  (row: PinnedRow) => string | number
> = {
  target: (row) => row.rule.target.value,
  kind: (row) => row.rule.target.kind,
  outbound: (row) => outboundKey(row.rule.outbound),
};

export function DirectRules({ rules }: { rules: DirectRulesDocument }) {
  const { t } = useTranslation();
  const {
    addRule,
    pinRoute,
    removeRule,
    refreshRules,
    syncCloudRules,
    cloudRules,
    actionPending,
    settings,
  } = useAppStore();
  const clients = settings?.clients ?? [];
  const enabled = clients.filter((client) => client.enabled);
  const [input, setInput] = useState("");
  const [search, setSearch] = useState("");
  const [route, setRoute] = useState<RouteTestResult | null>(null);
  const [testing, setTesting] = useState(false);
  const [sort, setSort] = useState<SortState<PinnedSortKey>>({
    key: "target",
    dir: "asc",
  });
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const rows: PinnedRow[] = rules.pins
      .filter((rule) => rule.target.value.includes(needle))
      .map((rule) => ({ rule }));
    return sortRows(rows, sort, PINNED_SORT_ACCESSORS);
  }, [rules.pins, search, sort]);
  const synced = cloudRules?.last_synced_at
    ? new Date(cloudRules.last_synced_at).toLocaleString()
    : t("neverSynced");
  const snapshotRevision =
    cloudRules?.snapshot_revision?.slice(0, 12) ??
    (cloudRules?.source === "bundled" ? t("bundledSnapshotRevision") : "—");

  async function test(target: string) {
    setTesting(true);
    try {
      setRoute(await desktop.testRoute(target));
    } finally {
      setTesting(false);
    }
  }

  return (
    <section aria-labelledby="rules-title" className="flex flex-col gap-4 pb-2">
      <header className="shrink-0">
        <h1 id="rules-title" className="text-2xl font-semibold tracking-tight">
          {t("listManagementTitle")}
        </h1>
        <p className="mt-1 text-sm text-muted">{t("listManagementHelp")}</p>
      </header>

      <RuleLists rules={rules} clients={enabled} allClients={clients} />

      <div className="rounded-2xl border border-ink/10 bg-surface p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h2 className="font-semibold">{t("cloudRules")}</h2>
            <p className="mt-1 max-w-2xl text-sm text-muted">
              {t("cloudRulesHelp")}
            </p>
          </div>
          <button
            type="button"
            disabled={actionPending}
            onClick={() => void syncCloudRules()}
            className="inline-flex items-center justify-center gap-2 rounded-xl bg-brand px-4 py-2.5 font-semibold text-white disabled:opacity-50"
          >
            <CloudDownload size={18} aria-hidden />
            {actionPending ? t("syncing") : t("updateFromCloud")}
          </button>
        </div>
        <dl className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <Stat
            label={t("domains")}
            value={(cloudRules?.domain_count ?? 0).toLocaleString()}
          />
          <Stat
            label={t("ipRanges")}
            value={(cloudRules?.ip_count ?? 0).toLocaleString()}
          />
          <Stat label={t("lastSynced")} value={synced} />
          <Stat label={t("snapshotRevision")} value={snapshotRevision} />
        </dl>
        <p className="mt-3 text-sm text-muted">
          {t("cloudRulesSource")}: devlifeX/BiFlow
        </p>
      </div>

      <form
        className="flex flex-col gap-2 rounded-2xl border border-ink/10 bg-surface p-4 sm:flex-row"
        onSubmit={(event) => {
          event.preventDefault();
          if (!input.trim()) return;
          void addRule(input)
            .then(() => setInput(""))
            .catch(() => undefined);
        }}
      >
        <input
          id="rule-input"
          aria-label={t("directRuleInput")}
          value={input}
          onChange={(event) => setInput(event.target.value)}
          required
          placeholder={t("directRulePlaceholder")}
          className="min-w-0 flex-1 rounded-xl border-ink/15 bg-canvas"
        />
        <button
          disabled={actionPending}
          className="inline-flex items-center justify-center gap-2 rounded-xl bg-brand px-4 py-2.5 font-semibold text-white disabled:opacity-50"
        >
          <Plus size={18} aria-hidden /> Add rule
        </button>
      </form>

      <h2 className="mt-2 font-semibold">{t("allPins")}</h2>
      <div className="flex flex-col gap-3 sm:flex-row">
        <label className="relative flex-1">
          <span className="sr-only">Search rules</span>
          <Search
            className="absolute left-3 top-3 text-muted"
            size={18}
            aria-hidden
          />
          <input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search direct rules"
            className="w-full rounded-xl border-ink/15 bg-surface pl-10"
          />
        </label>
        <button
          type="button"
          disabled={actionPending}
          onClick={() => void refreshRules()}
          className="inline-flex items-center justify-center gap-2 rounded-xl border border-ink/15 bg-surface px-4 py-2.5 font-semibold"
        >
          <RefreshCw size={18} aria-hidden /> Refresh resolutions
        </button>
      </div>

      <div className="rounded-2xl border border-ink/10 bg-surface">
        {filtered.length === 0 ? (
          <p className="p-8 text-center text-muted">
            No matching direct rules.
          </p>
        ) : (
          <div className="overflow-x-auto rounded-2xl">
            <table className="w-full min-w-[36rem] text-start text-sm">
              <thead>
                <tr className="bg-canvas text-muted">
                  <SortHeader
                    label={t("liveConnectionsHost")}
                    sortKey="target"
                    state={sort}
                    onToggle={(key) => setSort((prev) => toggleSort(prev, key))}
                  />
                  <SortHeader
                    label={t("ruleKind")}
                    sortKey="kind"
                    state={sort}
                    onToggle={(key) => setSort((prev) => toggleSort(prev, key))}
                  />
                  <SortHeader
                    label={t("liveConnectionsOutbound")}
                    sortKey="outbound"
                    state={sort}
                    onToggle={(key) => setSort((prev) => toggleSort(prev, key))}
                  />
                  <th className="px-3 py-2 text-start font-medium">
                    {t("liveConnectionsIp")}
                  </th>
                  <th className="px-3 py-2 text-start font-medium">
                    {t("tableActions")}
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-ink/10">
                {filtered.map(({ rule }) => {
                  const clientId =
                    rule.outbound.kind === "client"
                      ? rule.outbound.client_id
                      : null;
                  const disabledClient =
                    clientId !== null &&
                    !enabled.some((client) => client.id === clientId);
                  return (
                    <tr
                      key={`${outboundKey(rule.outbound)}:${rule.target.kind}:${rule.target.value}`}
                      className={`hover:bg-canvas/60 ${disabledClient ? "opacity-50" : ""}`}
                    >
                      <td className="px-3 py-2 font-medium break-all">
                        {rule.target.value}
                      </td>
                      <td className="px-3 py-2 font-mono text-xs text-muted">
                        {rule.target.kind.toUpperCase()}
                      </td>
                      <td className="px-3 py-2">
                        {disabledClient ? (
                          <span className="text-xs font-semibold text-muted">
                            {outboundLabel(rule.outbound, clients)} (
                            {t("disabled")})
                          </span>
                        ) : (
                          <OutboundSelect
                            value={outboundKey(rule.outbound)}
                            clients={enabled}
                            onChange={(next) =>
                              void pinRoute(rule.target.value, next).catch(
                                () => undefined,
                              )
                            }
                            disabled={actionPending}
                          />
                        )}
                      </td>
                      <td className="px-3 py-2 font-mono text-xs text-muted break-all">
                        {rule.resolved_ips.join(", ") || "—"}
                      </td>
                      <td className="px-3 py-2">
                        <div className="flex gap-1.5">
                          <button
                            type="button"
                            disabled={testing}
                            onClick={() => void test(rule.target.value)}
                            className="rounded-lg border border-ink/15 p-1.5 text-muted hover:text-brand"
                            title={`Test route for ${rule.target.value}`}
                            aria-label={`Test route for ${rule.target.value}`}
                          >
                            <Route size={16} aria-hidden />
                          </button>
                          <button
                            type="button"
                            disabled={actionPending}
                            onClick={() => void removeRule(rule.target.value)}
                            className="rounded-lg border border-ink/15 p-1.5 text-muted hover:text-danger"
                            title={`Remove ${rule.target.value}`}
                            aria-label={`Remove ${rule.target.value}`}
                          >
                            <Trash2 size={16} aria-hidden />
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {route ? <FlowResult route={route} /> : null}
    </section>
  );
}

function RuleLists({
  rules,
  clients,
  allClients,
}: {
  rules: DirectRulesDocument;
  clients: ClientInstance[];
  allClients: ClientInstance[];
}) {
  const { t } = useTranslation();
  const { createList, actionPending } = useAppStore();
  const [name, setName] = useState("");
  const [outbound, setOutbound] = useState("direct");

  return (
    <div data-testid="rule-lists" className="flex flex-col gap-3">
      <form
        className="flex flex-col gap-2 rounded-2xl border border-ink/10 bg-surface p-4 sm:flex-row sm:items-center"
        onSubmit={(event) => {
          event.preventDefault();
          if (!name.trim()) return;
          void createList(name, outbound)
            .then(() => setName(""))
            .catch(() => undefined);
        }}
      >
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          required
          placeholder={t("newListName")}
          className="min-w-0 flex-1 rounded-xl border-ink/15 bg-canvas"
        />
        <OutboundSelect
          value={outbound}
          clients={clients}
          onChange={setOutbound}
          disabled={actionPending}
        />
        <button
          disabled={actionPending}
          className="inline-flex items-center justify-center gap-2 rounded-xl bg-brand px-4 py-2.5 font-semibold text-white disabled:opacity-50"
        >
          <Plus size={18} aria-hidden />
          {t("newList")}
        </button>
      </form>

      <div className="grid gap-3 lg:grid-cols-2">
        {rules.lists.map((list) => (
          <RuleListCard
            key={list.id}
            list={list}
            pins={rules.pins.filter((pin) => pin.list_id === list.id)}
            clients={clients}
            allClients={allClients}
          />
        ))}
      </div>
    </div>
  );
}

function RuleListCard({
  list,
  pins,
  clients,
  allClients,
}: {
  list: RuleListMeta;
  pins: PinnedRoute[];
  clients: ClientInstance[];
  allClients: ClientInstance[];
}) {
  const { t } = useTranslation();
  const {
    renameList,
    deleteList,
    setListOutbound,
    pinToList,
    removeRule,
    actionPending,
    snapshot,
  } = useAppStore();
  const [entry, setEntry] = useState("");
  const [editedName, setEditedName] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [checking, setChecking] = useState(false);
  const [checkResults, setCheckResults] = useState<ListCheckEntry[] | null>(
    null,
  );
  const clientId =
    list.outbound.kind === "client" ? list.outbound.client_id : null;
  const boundClient = allClients.find((client) => client.id === clientId);
  const disabledClient = clientId !== null && !boundClient?.enabled;
  const running = snapshot?.phase === "running";
  const hasDomains = pins.some((pin) => pin.target.kind === "domain");
  const checkable =
    hasDomains &&
    (running ||
      (boundClient !== undefined &&
        boundClient.enabled &&
        boundClient.config.kind === "local_proxy"));

  async function check() {
    setChecking(true);
    setCheckResults(null);
    try {
      setCheckResults(await desktop.checkRuleList(list.id));
    } catch {
      setCheckResults([]);
    } finally {
      setChecking(false);
    }
  }

  return (
    <article
      data-testid={`rule-list-${list.id}`}
      className="rounded-2xl border border-ink/10 bg-surface p-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <form
          className="flex min-w-0 items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (editedName !== null && editedName.trim() !== list.name) {
              void renameList(list.id, editedName).catch(() => undefined);
            }
            setEditedName(null);
          }}
        >
          <input
            value={editedName ?? list.name}
            aria-label={t("renameList")}
            onChange={(event) => setEditedName(event.target.value)}
            onBlur={(event) => {
              if (
                editedName !== null &&
                editedName.trim() &&
                editedName.trim() !== list.name
              ) {
                void renameList(list.id, event.target.value).catch(
                  () => undefined,
                );
              }
              setEditedName(null);
            }}
            className="min-w-0 rounded-lg border-transparent bg-transparent font-semibold hover:border-ink/15 focus:border-ink/15 focus:bg-canvas"
          />
        </form>
        <span className="text-xs text-muted">
          {t("listEntries", { count: pins.length })}
        </span>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <span className="text-xs font-medium text-muted">
          {t("listOutbound")}
        </span>
        {disabledClient ? (
          <span className="text-xs font-semibold text-muted">
            {outboundLabel(list.outbound, allClients)} ({t("disabled")})
          </span>
        ) : (
          <OutboundSelect
            value={outboundKey(list.outbound)}
            clients={clients}
            onChange={(next) =>
              void setListOutbound(list.id, next).catch(() => undefined)
            }
            disabled={actionPending}
          />
        )}
        <button
          type="button"
          disabled={!checkable || checking}
          onClick={() => void check()}
          title={
            hasDomains ? t("checkListNeedsStack") : t("checkListNeedsDomains")
          }
          className="ms-auto inline-flex items-center gap-1.5 rounded-lg border border-ink/15 px-2.5 py-1.5 text-xs font-semibold disabled:opacity-40"
        >
          {checking ? (
            <LoaderCircle className="animate-spin" size={14} aria-hidden />
          ) : (
            <ListChecks size={14} aria-hidden />
          )}
          {checking ? t("checkingList") : t("checkList")}
        </button>
      </div>

      {pins.length === 0 ? (
        <p className="mt-3 text-xs text-muted">{t("listEmptyHint")}</p>
      ) : (
        <ul className="mt-3 space-y-1">
          {pins.map((pin) => (
            <li
              key={`${pin.target.kind}:${pin.target.value}`}
              className="flex items-center justify-between gap-2 text-sm"
            >
              <span className="break-all">{pin.target.value}</span>
              <button
                type="button"
                disabled={actionPending}
                onClick={() =>
                  void removeRule(pin.target.value).catch(() => undefined)
                }
                className="text-xs font-semibold text-muted hover:text-danger"
              >
                {t("remove")}
              </button>
            </li>
          ))}
        </ul>
      )}

      {checkResults ? (
        <ul className="mt-3 space-y-1 rounded-xl bg-canvas p-3">
          {checkResults.length === 0 ? (
            <li className="text-xs text-muted">{t("checkListNeedsStack")}</li>
          ) : (
            checkResults.map((result) => (
              <li
                key={result.target}
                className="flex items-center justify-between gap-2 text-xs"
              >
                <span className="break-all">{result.target}</span>
                <span
                  className={`font-semibold ${
                    result.status === "ok"
                      ? "text-success"
                      : result.status === "fail"
                        ? "text-danger"
                        : "text-muted"
                  }`}
                >
                  {result.status}
                  {result.latency_ms !== null
                    ? ` · ${result.latency_ms}ms`
                    : ""}
                </span>
              </li>
            ))
          )}
        </ul>
      ) : null}

      <form
        className="mt-3 flex gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (!entry.trim()) return;
          void pinToList(entry, list.id)
            .then(() => setEntry(""))
            .catch(() => undefined);
        }}
      >
        <input
          value={entry}
          onChange={(event) => setEntry(event.target.value)}
          placeholder={t("entryPlaceholder")}
          className="min-w-0 flex-1 rounded-xl border-ink/15 bg-canvas text-sm"
        />
        <button
          disabled={actionPending}
          className="rounded-xl border border-ink/15 px-3 py-2 text-xs font-semibold"
        >
          {t("addEntry")}
        </button>
      </form>

      <div className="mt-3 border-t border-ink/10 pt-3">
        {deleting ? (
          <div className="space-y-2 text-sm" role="dialog">
            <p>
              {t("deleteListConfirm", {
                name: list.name,
                count: pins.length,
              })}
            </p>
            <div className="flex gap-2">
              <button
                type="button"
                onClick={() =>
                  void deleteList(list.id)
                    .then(() => setDeleting(false))
                    .catch(() => setDeleting(false))
                }
                className="rounded-lg bg-danger px-3 py-1.5 text-xs font-semibold text-white"
              >
                {t("deleteList")}
              </button>
              <button
                type="button"
                onClick={() => setDeleting(false)}
                className="rounded-lg border border-ink/15 px-3 py-1.5 text-xs font-semibold"
              >
                {t("close")}
              </button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => setDeleting(true)}
            className="inline-flex items-center gap-1 text-xs font-semibold text-muted hover:text-danger"
          >
            <Trash2 size={14} aria-hidden />
            {t("deleteList")}
          </button>
        )}
      </div>
    </article>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl bg-canvas p-4">
      <dt className="text-sm text-muted">{label}</dt>
      <dd className="mt-1 text-xl font-semibold">{value}</dd>
    </div>
  );
}

export function FlowResult({
  route,
  onMove,
  moving = false,
}: {
  route: RouteTestResult;
  onMove?: (target: string, to: string) => void;
  moving?: boolean;
}) {
  const { t } = useTranslation();
  const clients = useAppStore((state) => state.settings?.clients ?? []);
  const enabled = clients.filter((client) => client.enabled);
  const current = outboundKey(route.outbound);
  const vpn = current !== "direct";
  const actionable = route.reason !== "private_or_local";
  return (
    <div
      className={`rounded-2xl border p-4 ${vpn ? "border-brand/20 bg-brand/5" : "border-success/20 bg-success/5"}`}
      role="status"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="min-w-0 break-all font-semibold">
          {route.target} → {outboundLabel(route.outbound, clients)}
        </p>
        {onMove && actionable ? (
          <div className="flex items-center gap-2">
            {moving ? (
              <LoaderCircle className="animate-spin" size={16} aria-hidden />
            ) : null}
            <OutboundSelect
              value={current}
              clients={enabled}
              onChange={(next) => onMove(route.target, next)}
              disabled={moving}
            />
          </div>
        ) : null}
      </div>
      <p className="mt-1 text-sm text-muted">
        {route.reason.replaceAll("_", " ")} · matched{" "}
        {route.matched_rule ?? "none"}
      </p>
      {onMove && !actionable ? (
        <p className="mt-2 text-sm text-muted">{t("moveLocalUnavailable")}</p>
      ) : null}
    </div>
  );
}

export function OutboundSelect({
  value,
  clients,
  onChange,
  disabled,
}: {
  value: string;
  clients: ClientInstance[];
  onChange: (next: string) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <select
      value={value}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value)}
      className="max-w-[390px] rounded-lg border border-ink/15 bg-canvas px-2 py-1 text-xs font-semibold"
    >
      <option value="direct">{t("direct")}</option>
      {clients.map((client) => (
        <option key={client.id} value={client.id}>
          {outboundLabel(client.id, clients)}
        </option>
      ))}
    </select>
  );
}
