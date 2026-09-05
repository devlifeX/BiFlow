import { Download, FolderOpen, Plus, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { desktop } from "../api/desktop";
import type { ClientInstance, PinnedRoute, RuleListMeta } from "../api/models";
import { canAddPreset, enabledClients, profileFileName } from "../lib/clients";
import { defaultRouteFromKey, outboundKey } from "../lib/outbound";
import {
  downloadUrlFor,
  PRESETS,
  presetById,
  type PresetId,
} from "../lib/presets";
import { useAppStore } from "../store/app";
import { StatusPill } from "./StatusPill";

export function ClientRegistry() {
  const { t } = useTranslation();
  const {
    settings,
    rules,
    snapshot,
    addClient,
    deleteClient,
    setClientEnabled,
    setClientAllowDirectWhenDown,
    updateClient,
    setDefaultRoute,
    pinRoute,
    removeRule,
    routeFallbackNotice,
    clearRouteFallbackNotice,
    actionPending,
    boot,
  } = useAppStore();
  const platform = boot?.platform ?? "linux";
  const [catalogOpen, setCatalogOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [moveTo, setMoveTo] = useState("direct");
  const clients = settings?.clients ?? [];
  const pins = rules?.pins ?? [];
  const lists = rules?.lists ?? [];

  if (!settings) return null;

  return (
    <section data-testid="client-registry" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">{t("clientsTitle")}</h2>
          <p className="mt-1 text-sm text-muted">{t("clientsHelp")}</p>
        </div>
        <button
          type="button"
          onClick={() => setCatalogOpen((open) => !open)}
          className="inline-flex items-center gap-2 rounded-xl bg-brand px-4 py-2.5 font-semibold text-white"
        >
          <Plus size={18} aria-hidden />
          {t("addClient")}
        </button>
      </div>

      {routeFallbackNotice ? (
        <p
          className="rounded-xl border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-sm"
          role="status"
        >
          {t(routeFallbackNotice)}
          <button
            type="button"
            className="ms-2 font-semibold underline"
            onClick={() => clearRouteFallbackNotice()}
          >
            {t("close")}
          </button>
        </p>
      ) : null}

      <label className="flex max-w-xl flex-col gap-1 text-sm font-medium">
        <span>{t("defaultRouteLabel")}</span>
        <select
          data-testid="default-route"
          value={outboundKey(settings.default_route)}
          disabled={actionPending}
          onChange={(event) =>
            void setDefaultRoute(defaultRouteFromKey(event.target.value))
          }
          className="rounded-xl border-ink/15 bg-surface"
        >
          <option value="direct">{t("direct")}</option>
          {enabledClients(clients).map((client) => (
            <option key={client.id} value={client.id}>
              {presetById(client.preset as PresetId).title}
            </option>
          ))}
        </select>
      </label>

      {catalogOpen ? (
        <div
          data-testid="client-catalog"
          className="grid gap-2 rounded-2xl border border-ink/10 bg-surface p-4 sm:grid-cols-2"
        >
          {PRESETS.map((preset) => {
            const added = !canAddPreset(preset.id, clients);
            const disabled = preset.status !== "working" || added;
            return (
              <div
                key={preset.id}
                className={`flex flex-col rounded-xl border border-ink/10 p-3 text-start ${
                  disabled ? "opacity-60" : ""
                }`}
              >
                <button
                  type="button"
                  disabled={disabled || actionPending}
                  onClick={() => {
                    void addClient(preset.id).then(() => setCatalogOpen(false));
                  }}
                  className="text-start"
                >
                  <p className="font-semibold">{preset.title}</p>
                  <p className="mt-1 text-xs text-muted">
                    {preset.installHint}
                  </p>
                  {added ? (
                    <p className="mt-2 text-xs font-semibold">
                      {t("alreadyAdded")}
                    </p>
                  ) : null}
                  {preset.status !== "working" ? (
                    <p className="mt-2 text-xs font-semibold">
                      {t("catalogOnly")}
                    </p>
                  ) : null}
                </button>
                {preset.status === "working" ? (
                  <button
                    type="button"
                    onClick={() =>
                      void desktop.openUrl(downloadUrlFor(preset, platform))
                    }
                    className="mt-2 inline-flex items-center gap-1 self-start text-xs font-semibold text-brand underline"
                  >
                    <Download size={12} aria-hidden />
                    {t("downloadInstall")}
                  </button>
                ) : null}
              </div>
            );
          })}
        </div>
      ) : null}

      <div className="grid gap-3 lg:grid-cols-2 2xl:grid-cols-3">
        {clients.map((client) => (
          <ClientCard
            key={client.id}
            client={client}
            pins={pins.filter(
              (pin) =>
                pin.outbound.kind === "client" &&
                pin.outbound.client_id === client.id,
            )}
            lists={lists.filter(
              (list) =>
                list.outbound.kind === "client" &&
                list.outbound.client_id === client.id,
            )}
            isDefault={
              settings.default_route.kind === "client" &&
              settings.default_route.client_id === client.id
            }
            phase={
              snapshot?.clients.find((item) => item.id === client.id)?.status
                .phase ?? "stopped"
            }
            exitIp={
              snapshot?.clients.find((item) => item.id === client.id)
                ?.exit_ip ?? null
            }
            deleting={pendingDelete === client.id}
            others={enabledClients(clients).filter(
              (item) => item.id !== client.id,
            )}
            moveTo={moveTo}
            onMoveTo={setMoveTo}
            onAskDelete={() => {
              setPendingDelete(client.id);
              setMoveTo(
                enabledClients(clients).find((item) => item.id !== client.id)
                  ?.id ?? "direct",
              );
            }}
            onCancelDelete={() => setPendingDelete(null)}
            onConfirmDelete={() => {
              void deleteClient(
                client.id,
                moveTo === "direct" ? undefined : moveTo,
              ).then(() => setPendingDelete(null));
            }}
            onEnabled={(enabled) => void setClientEnabled(client.id, enabled)}
            onAllowDirectWhenDown={(allow) =>
              void setClientAllowDirectWhenDown(client.id, allow)
            }
            onConfig={(next) => void updateClient(client.id, next.config)}
            onPin={(host) => void pinRoute(host, client.id)}
            onRemovePin={(host) => void removeRule(host)}
            actionPending={actionPending}
            platform={platform}
          />
        ))}
      </div>
    </section>
  );
}

function ClientCard({
  client,
  pins,
  lists,
  isDefault,
  phase,
  deleting,
  others,
  moveTo,
  onMoveTo,
  onAskDelete,
  onCancelDelete,
  onConfirmDelete,
  onEnabled,
  onAllowDirectWhenDown,
  onConfig,
  exitIp,
  onPin,
  onRemovePin,
  actionPending,
  platform,
}: {
  client: ClientInstance;
  pins: PinnedRoute[];
  lists: RuleListMeta[];
  isDefault: boolean;
  phase: string;
  deleting: boolean;
  others: ClientInstance[];
  moveTo: string;
  onMoveTo: (value: string) => void;
  onAskDelete: () => void;
  onCancelDelete: () => void;
  onConfirmDelete: () => void;
  onEnabled: (enabled: boolean) => void;
  onAllowDirectWhenDown: (allow: boolean) => void;
  exitIp: string | null;
  onConfig: (next: ClientInstance) => void;
  onPin: (host: string) => Promise<void> | void;
  onRemovePin: (host: string) => void;
  actionPending: boolean;
  platform: string;
}) {
  const { t } = useTranslation();
  const [host, setHost] = useState("");
  const spec = useMemo(
    () => presetById(client.preset as PresetId),
    [client.preset],
  );
  const domainCount = pins.filter((pin) => pin.target.kind === "domain").length;
  const ipCount = pins.length - domainCount;
  // Side tunnels need a system binary the app cannot ship; surface a
  // per-platform download link when it is missing.
  const [binaryInstalled, setBinaryInstalled] = useState<boolean | null>(null);
  useEffect(() => {
    if (client.config.kind !== "owned_side_tunnel") return;
    let cancelled = false;
    desktop
      .clientBinaryInstalled(client.preset)
      .then((installed) => {
        if (!cancelled) setBinaryInstalled(installed);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [client.preset, client.config.kind]);

  async function chooseProfile() {
    if (client.config.kind !== "owned_side_tunnel") return;
    try {
      const path = await desktop.pickProfileFile();
      if (!path) return;
      onConfig({
        ...client,
        config: {
          ...client.config,
          profile_path: path,
        },
      });
    } catch (error) {
      const message =
        error instanceof Error ? error.message : t("profileFileInvalid");
      useAppStore.setState({ error: message });
    }
  }

  return (
    <article
      data-testid={`client-card-${client.preset}`}
      className={`rounded-2xl border border-ink/10 bg-surface p-3.5 ${
        client.enabled ? "" : "opacity-70"
      }`}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <h3 className="font-semibold">{spec.title}</h3>
          {isDefault ? (
            <span className="rounded-md bg-brand/10 px-2 py-0.5 text-xs font-semibold text-brand">
              {t("matchDefault")}
            </span>
          ) : null}
          <StatusPill phase={phase as never} />
        </div>
        <label className="flex shrink-0 items-center gap-2 text-xs font-semibold">
          <input
            type="checkbox"
            checked={client.enabled}
            onChange={(event) => onEnabled(event.target.checked)}
          />
          {t("enabled")}
        </label>
      </div>

      <p className="mt-1.5 text-xs text-muted">
        {t("pinSummary", { domains: domainCount, ips: ipCount })}
        {client.config.kind === "local_proxy"
          ? ` · ${t("clientPort")} ${client.config.port}`
          : ""}
        {exitIp ? (
          <span className="font-mono">
            {" "}
            · {t("exitIp")} {exitIp}
          </span>
        ) : null}
      </p>

      {binaryInstalled === false ? (
        <p className="mt-2 flex flex-wrap items-center gap-2 rounded-xl border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-xs">
          {t("binaryMissing")}
          <button
            type="button"
            onClick={() => void desktop.openUrl(downloadUrlFor(spec, platform))}
            className="inline-flex items-center gap-1 font-semibold text-brand underline"
          >
            <Download size={12} aria-hidden />
            {t("downloadInstall")}
          </button>
        </p>
      ) : null}

      <details className="mt-2 text-sm">
        <summary className="cursor-pointer select-none text-xs font-semibold text-muted">
          {t("clientDetails")}
        </summary>

        <p className="mt-2 text-xs text-muted">{spec.installHint}</p>
        <button
          type="button"
          onClick={() => void desktop.openUrl(downloadUrlFor(spec, platform))}
          className="mt-1 inline-flex items-center gap-1 text-xs font-semibold text-brand underline"
        >
          <Download size={12} aria-hidden />
          {t("downloadInstall")}
        </button>

        <label className="mt-3 flex items-center gap-2 text-xs text-muted">
          <input
            type="checkbox"
            checked={client.allow_direct_when_down}
            onChange={(event) => onAllowDirectWhenDown(event.target.checked)}
          />
          {t("allowDirectWhenDown")}
        </label>

        {client.config.kind === "local_proxy" ? (
          <div className="mt-3 grid gap-2 sm:grid-cols-2">
            <label className="text-xs font-medium">
              {t("clientPort")}
              <input
                type="number"
                value={client.config.port}
                onChange={(event) => {
                  if (client.config.kind !== "local_proxy") return;
                  onConfig({
                    ...client,
                    config: {
                      ...client.config,
                      port: Number(event.target.value),
                    },
                  });
                }}
                className="mt-1 w-full rounded-xl border-ink/15 bg-canvas"
              />
            </label>
            <label className="flex items-center gap-2 text-xs font-medium">
              <input
                type="checkbox"
                checked={client.config.stop_with_stack}
                onChange={(event) => {
                  if (client.config.kind !== "local_proxy") return;
                  onConfig({
                    ...client,
                    config: {
                      ...client.config,
                      stop_with_stack: event.target.checked,
                    },
                  });
                }}
              />
              {t("stopWithStack")}
            </label>
          </div>
        ) : null}

        {client.config.kind === "owned_side_tunnel" ? (
          <div className="mt-3 grid gap-2">
            <div className="text-xs font-medium">
              <span id={`${client.id}-profile-label`}>
                {t("openvpnProfile")}
              </span>
              <div className="mt-1 flex items-center gap-2">
                <p
                  data-testid="profile-file-name"
                  aria-labelledby={`${client.id}-profile-label`}
                  title={client.config.profile_path ?? undefined}
                  className="min-w-0 flex-1 truncate rounded-xl border border-ink/15 bg-canvas px-3 py-2 text-sm font-normal"
                >
                  {profileFileName(client.config.profile_path) ??
                    t("noFileChosen")}
                </p>
                <button
                  type="button"
                  data-testid="choose-profile-file"
                  disabled={actionPending}
                  onClick={() => {
                    void chooseProfile();
                  }}
                  className="inline-flex shrink-0 items-center gap-2 rounded-xl bg-brand px-3 py-2 font-semibold text-white"
                >
                  <FolderOpen size={16} aria-hidden />
                  {t("chooseFile")}
                </button>
                {client.config.profile_path ? (
                  <button
                    type="button"
                    data-testid="clear-profile-file"
                    disabled={actionPending}
                    onClick={() => {
                      if (client.config.kind !== "owned_side_tunnel") return;
                      onConfig({
                        ...client,
                        config: {
                          ...client.config,
                          profile_path: null,
                        },
                      });
                    }}
                    aria-label={t("clearFile")}
                    title={t("clearFile")}
                    className="inline-flex shrink-0 items-center justify-center rounded-xl border border-ink/15 p-2 text-muted hover:text-danger"
                  >
                    <X size={16} aria-hidden />
                  </button>
                ) : null}
              </div>
            </div>
            <div className="grid gap-2 sm:grid-cols-2">
              <label className="text-xs font-medium">
                {t("openvpnUsername")}
                <input
                  value={client.config.username ?? ""}
                  autoComplete="off"
                  onChange={(event) => {
                    if (client.config.kind !== "owned_side_tunnel") return;
                    onConfig({
                      ...client,
                      config: {
                        ...client.config,
                        username: event.target.value || null,
                      },
                    });
                  }}
                  className="mt-1 w-full rounded-xl border-ink/15 bg-canvas"
                />
              </label>
              <label className="text-xs font-medium">
                {t("openvpnPassword")}
                <input
                  type="password"
                  value={
                    client.config.password === "[REDACTED]"
                      ? ""
                      : (client.config.password ?? "")
                  }
                  placeholder={
                    client.config.password === "[REDACTED]" ? "••••••••" : ""
                  }
                  autoComplete="new-password"
                  onChange={(event) => {
                    if (client.config.kind !== "owned_side_tunnel") return;
                    onConfig({
                      ...client,
                      config: {
                        ...client.config,
                        password: event.target.value || null,
                      },
                    });
                  }}
                  className="mt-1 w-full rounded-xl border-ink/15 bg-canvas"
                />
              </label>
            </div>
          </div>
        ) : null}

        {lists.length > 0 ? (
          <div className="mt-3 flex flex-wrap gap-1.5">
            {lists.map((list) => (
              <span
                key={list.id}
                className="rounded-md bg-canvas px-2 py-0.5 text-xs font-medium text-muted"
              >
                {list.name}
              </span>
            ))}
          </div>
        ) : null}

        <form
          className="mt-3 flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (!host.trim()) return;
            void Promise.resolve(onPin(host)).then(() => setHost(""));
          }}
        >
          <input
            value={host}
            onChange={(event) => setHost(event.target.value)}
            placeholder={t("clientPinPlaceholder")}
            className="min-w-0 flex-1 rounded-xl border-ink/15 bg-canvas text-sm"
          />
          <button
            type="submit"
            className="rounded-xl border border-ink/15 px-3 py-2 text-xs font-semibold"
          >
            {t("pinToClient")}
          </button>
        </form>

        <ul className="mt-2 space-y-1">
          {pins.map((pin) => (
            <li
              key={pin.target.value}
              className="flex items-center justify-between gap-2 text-sm"
            >
              <span className="break-all">{pin.target.value}</span>
              <button
                type="button"
                onClick={() => onRemovePin(pin.target.value)}
                className="text-xs font-semibold text-muted hover:text-danger"
              >
                {t("remove")}
              </button>
            </li>
          ))}
        </ul>

        <div className="mt-3 border-t border-ink/10 pt-3">
          {deleting ? (
            <div className="space-y-2 text-sm" role="dialog">
              <p>
                {t("deleteClientConfirm", {
                  name: spec.title,
                  count: pins.length,
                })}
              </p>
              {others.length > 0 ? (
                <label className="block text-xs font-medium">
                  {t("movePinsTo")}
                  <select
                    value={moveTo}
                    onChange={(event) => onMoveTo(event.target.value)}
                    className="mt-1 w-full rounded-xl border-ink/15 bg-canvas"
                  >
                    <option value="direct">{t("deletePins")}</option>
                    {others.map((item) => (
                      <option key={item.id} value={item.id}>
                        {presetById(item.preset as PresetId).title}
                      </option>
                    ))}
                  </select>
                </label>
              ) : null}
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={onConfirmDelete}
                  className="rounded-lg bg-danger px-3 py-1.5 text-xs font-semibold text-white"
                >
                  {t("deleteClient")}
                </button>
                <button
                  type="button"
                  onClick={onCancelDelete}
                  className="rounded-lg border border-ink/15 px-3 py-1.5 text-xs font-semibold"
                >
                  {t("close")}
                </button>
              </div>
            </div>
          ) : (
            <button
              type="button"
              onClick={onAskDelete}
              className="inline-flex items-center gap-1 text-xs font-semibold text-muted hover:text-danger"
            >
              <Trash2 size={14} aria-hidden />
              {t("deleteClient")}
            </button>
          )}
        </div>
      </details>
    </article>
  );
}
