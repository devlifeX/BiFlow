import { Download, LoaderCircle } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { ComponentStatus, StackSnapshot } from "../api/models";
import { presetById, type PresetId } from "../lib/presets";
import { listPanelClassName, listRowClassName } from "../lib/listPanel";
import { StatusPill } from "./StatusPill";

export function ComponentStatusList({
  snapshot,
  dependencies,
  installingId,
  onInstallHelper,
  onInstallDependency,
}: {
  snapshot: StackSnapshot;
  dependencies: Array<{ id: string; installed: boolean }>;
  installingId?: string | null;
  onInstallHelper: () => void;
  onInstallDependency: (id: "hiddify" | "mihomo") => void;
}) {
  const { t } = useTranslation();

  const rows: Array<{
    key: string;
    name: string;
    status: ComponentStatus;
    disabled?: boolean;
    detail?: string;
    install?: ReactNode;
  }> = [
    {
      key: "helper",
      name: t("helper"),
      status: snapshot.helper,
      install:
        snapshot.helper.phase === "unavailable" ||
        snapshot.helper.phase === "error" ? (
          <InstallAction
            installing={installingId === "helper"}
            onClick={onInstallHelper}
          />
        ) : null,
    },
    ...snapshot.clients.map((client) => ({
      key: client.id,
      name: presetById(client.preset as PresetId).title,
      status: client.status,
      disabled: !client.enabled,
      detail: client.exit_ip ?? undefined,
      install:
        client.preset === "hiddify" &&
        !dependencies.find((item) => item.id === "hiddify")?.installed ? (
          <InstallAction
            installing={installingId === "hiddify"}
            onClick={() => onInstallDependency("hiddify")}
          />
        ) : null,
    })),
    {
      key: "mihomo",
      name: "Mihomo",
      status: snapshot.mihomo,
      install: !dependencies.find((item) => item.id === "mihomo")?.installed ? (
        <InstallAction
          installing={installingId === "mihomo"}
          onClick={() => onInstallDependency("mihomo")}
        />
      ) : null,
    },
    {
      key: "tun",
      name: "TUN",
      status: snapshot.tun,
    },
    {
      key: "dns",
      name: "DNS",
      status: snapshot.dns,
    },
  ];

  return (
    <div data-testid="connection-status-strip" className={listPanelClassName()}>
      {rows.map((row) => (
        <div key={row.key} className={listRowClassName()}>
          <span className="min-w-[64px] max-w-[140px] shrink-0 truncate font-medium">
            {row.name}
          </span>
          <span className="min-w-0 flex-1 truncate text-[11px] text-muted">
            {row.detail ?? row.status.message ?? t("statusDetailUnavailable")}
          </span>
          {row.install}
          <StatusPill phase={row.status.phase} disabled={row.disabled} />
        </div>
      ))}
    </div>
  );
}

function InstallAction({
  installing,
  onClick,
}: {
  installing?: boolean;
  onClick: () => void;
}) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      disabled={installing}
      onClick={onClick}
      className="inline-flex shrink-0 items-center gap-1 rounded-[5px] bg-accent px-2 py-0.5 text-[10px] font-semibold text-white disabled:opacity-50"
    >
      {installing ? (
        <LoaderCircle className="animate-spin" size={12} aria-hidden />
      ) : (
        <Download size={12} aria-hidden />
      )}
      {installing ? t("installing") : t("install")}
    </button>
  );
}
