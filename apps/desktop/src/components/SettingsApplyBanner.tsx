import { RotateCcw, Undo2, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/app";

export function SettingsApplyBanner() {
  const { t } = useTranslation();
  const notice = useAppStore((state) => state.settingsApplyNotice);
  const actionPending = useAppStore((state) => state.actionPending);
  const applyPendingSettings = useAppStore(
    (state) => state.applyPendingSettings,
  );
  const revertPendingSettings = useAppStore(
    (state) => state.revertPendingSettings,
  );
  const dismissSettingsApplyNotice = useAppStore(
    (state) => state.dismissSettingsApplyNotice,
  );

  if (!notice) return null;

  return (
    <div
      data-testid="settings-apply-banner"
      className="sticky top-0 z-40 mb-4 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-sm shadow-card backdrop-blur-md"
      role="status"
    >
      <p className="min-w-0 flex-1">{t("restartMihomoToApply")}</p>
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={actionPending}
          onClick={() => void applyPendingSettings()}
          className="inline-flex items-center gap-2 rounded-lg bg-brand px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-50"
        >
          <RotateCcw size={14} aria-hidden />
          {t("restartMihomo")}
        </button>
        <button
          type="button"
          disabled={actionPending}
          onClick={() => void revertPendingSettings()}
          className="inline-flex items-center gap-2 rounded-lg border border-ink/15 px-3 py-1.5 text-xs font-semibold disabled:opacity-50"
        >
          <Undo2 size={14} aria-hidden />
          {t("revertChanges")}
        </button>
        <button
          type="button"
          disabled={actionPending}
          onClick={() => dismissSettingsApplyNotice()}
          aria-label={t("close")}
          title={t("close")}
          className="inline-flex items-center justify-center rounded-lg p-1.5 text-muted hover:text-ink disabled:opacity-50"
        >
          <X size={16} aria-hidden />
        </button>
      </div>
    </div>
  );
}
