import { Download, Plus, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { desktop } from "../api/desktop";
import type { ClientInstance, PinnedRoute, RuleListMeta } from "../api/models";
import { canAddPreset, enabledClients } from "../lib/clients";
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
              <button
                key={preset.id}
                type="button"
                disabled={disabled || actionPending}
                onClick={() => {
                  void addClient(preset.id).then(() => setCatalogOpen(false));
                }}
                className="rounded-xl border border-ink/10 p-3 text-start disabled:opacity-50"
              >
                <p className="font-semibold">{preset.title}</p>
                <p className="mt-1 text-xs text-muted">{preset.installHint}</p>
                {preset.status === "working" ? (
                  <span
                    role="link"
                    tabIndex={0}
                    onClick={(event) => {
                      event.stopPropagation();
                      void desktop.openUrl(downloadUrlFor(preset, platform));
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.stopPropagation();
                        void desktop.openUrl(downloadUrlFor(preset, platform));
                      }
                    }}
                    className="mt-2 inline-flex items-center gap-1 text-xs font-semibold text-brand underline"
                  >
                    <Download size={12} aria-hidden />
                    {t("downloadInstall")}
                  </span>
                ) : null}
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
            );
          })}
        </div>
      ) : null}

      <div className="grid gap-3 lg:grid-cols-2">
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
            onConfig={(next) => void updateClient(client.id, next.config)}
            onPin={(host) => void pinRoute(host, client.id)}
            onRemovePin={(host) => void removeRule(host)}
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
  onConfig,
  onPin,
  onRemovePin,
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
  onConfig: (next: ClientInstance) => void;
  onPin: (host: string) => Promise<void> | void;
  onRemovePin: (host: string) => void;
  platform: string;
}) {
  const { t } = useTranslation();
  const [host, setHost] = useState("");
  const spec = useMemo(
    () => presetById(client.preset as PresetId),
    [client.preset],
  );

  return (
    <article
      data-testid={`client-card-${client.preset}`}
      className={`rounded-2xl border border-ink/10 bg-surface p-4 ${
        client.enabled ? "" : "opacity-70"
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="font-semibold">{spec.title}</h3>
            {isDefault ? (
              <span className="rounded-md bg-brand/10 px-2 py-0.5 text-xs font-semibold text-brand">
                {t("matchDefault")}
              </span>
            ) : null}
            <StatusPill phase={phase as never} />
          </div>
          <p className="mt-1 text-xs text-muted">{spec.installHint}</p>
          <button
            type="button"
            onClick={() => void desktop.openUrl(downloadUrlFor(spec, platform))}
            className="mt-2 inline-flex items-center gap-1 text-xs font-semibold text-brand underline"
          >
            <Download size={12} aria-hidden />
            {t("downloadInstall")}
          </button>
        </div>
        <label className="flex items-center gap-2 text-xs font-semibold">
          <input
            type="checkbox"
            checked={client.enabled}
            onChange={(event) => onEnabled(event.target.checked)}
          />
          {t("enabled")}
        </label>
      </div>

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
          <label className="text-xs font-medium">
            {t("openvpnProfile")}
            <input
              value={client.config.profile_path ?? ""}
              onChange={(event) => {
                if (client.config.kind !== "owned_side_tunnel") return;
                onConfig({
                  ...client,
                  config: {
                    ...client.config,
                    profile_path: event.target.value || null,
                  },
                });
              }}
              className="mt-1 w-full rounded-xl border-ink/15 bg-canvas"
              placeholder="profile.ovpn"
            />
          </label>
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
    </article>
  );
}
