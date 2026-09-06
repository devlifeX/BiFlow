import { useTranslation } from "react-i18next";
import type { StackSnapshot } from "../api/models";

export function StatStrip({ snapshot }: { snapshot: StackSnapshot }) {
  const { t } = useTranslation();
  const enabledClients = snapshot.clients.filter((client) => client.enabled);
  const activeClients = enabledClients.filter(
    (client) => client.status.phase === "running",
  ).length;

  return (
    <div
      data-testid="provider-summary"
      className="flex gap-2 rounded-md border border-[rgb(var(--border-default))] bg-surface px-3 py-2 text-[12px] leading-snug"
    >
      <div className="min-w-0 flex-[2]">
        <p className="text-[11px] text-muted">{t("exitIp")}</p>
        <p className="mt-0.5 truncate font-semibold tabular-nums">
          {snapshot.exit_ip ?? t("noExitIp")}
        </p>
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[11px] text-muted">{t("providers")}</p>
        <p className="mt-0.5 font-semibold tabular-nums">
          {snapshot.providers.ready} / {snapshot.providers.total}
        </p>
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[11px] text-muted">{t("activeClients")}</p>
        <p className="mt-0.5 font-semibold tabular-nums">
          {activeClients} / {enabledClients.length}
        </p>
      </div>
    </div>
  );
}
