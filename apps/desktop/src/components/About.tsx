import {
  Download,
  ExternalLink,
  Info,
  RefreshCw,
  RotateCw,
} from "lucide-react";
import { AppButton, BUTTON_ICON_PX } from "./AppButton";
import { useTranslation } from "react-i18next";
import { APP_VERSION } from "../version";
import { useAppStore } from "../store/app";

const REPOSITORY_URL = "https://github.com/devlifeX/BiFlow";

export function About() {
  const { t } = useTranslation();
  const update = useAppStore((state) => state.update);
  const checkForUpdate = useAppStore((state) => state.checkForUpdate);
  const installUpdate = useAppStore((state) => state.installUpdate);
  const retryUpdate = useAppStore((state) => state.retryUpdate);
  const openRepository = useAppStore((state) => state.openRepository);

  const busy =
    update.phase === "checking" ||
    update.phase === "downloading" ||
    update.phase === "installing" ||
    update.phase === "restarting";
  const showInstall = update.phase === "available";
  const showRetry = update.phase === "failed";

  return (
    <section className="flex flex-col gap-3 pb-2">
      <header className="flex items-start gap-3">
        <Info className="mt-1 text-brand" size={24} aria-hidden />
        <div>
          <h1 className="text-2xl font-semibold">{t("about")}</h1>
          <p className="mt-1 text-sm text-muted">{t("aboutUpdatesHelp")}</p>
        </div>
      </header>

      <dl className="grid gap-4 rounded-2xl border border-ink/10 bg-surface p-3.5 sm:grid-cols-2">
        <div>
          <dt className="text-xs uppercase tracking-wide text-muted">
            {t("appName")}
          </dt>
          <dd className="mt-1 font-medium">BiFlow</dd>
        </div>
        <div>
          <dt className="text-xs uppercase tracking-wide text-muted">
            {t("aboutVersion", { version: APP_VERSION })}
          </dt>
          <dd className="mt-1 font-medium">{APP_VERSION}</dd>
        </div>
        <div>
          <dt className="text-xs uppercase tracking-wide text-muted">
            {t("aboutAuthorLabel")}
          </dt>
          <dd className="mt-1 font-medium">{t("aboutAuthor")}</dd>
        </div>
        <div>
          <dt className="text-xs uppercase tracking-wide text-muted">
            {t("aboutRepositoryLabel")}
          </dt>
          <dd className="mt-1">
            <AppButton
              icon={<ExternalLink size={16} aria-hidden />}
              className="font-medium text-brand underline-offset-2 hover:underline"
              onClick={() => void openRepository()}
            >
              devlifeX/BiFlow
            </AppButton>
          </dd>
        </div>
      </dl>

      <div
        className="rounded-2xl border border-ink/10 bg-surface p-3.5"
        aria-live="polite"
      >
        <h2 className="text-lg font-semibold">{t("aboutUpdatesTitle")}</h2>
        <p className="mt-1 text-sm text-muted">{t("aboutUpdatesHelp")}</p>

        <p className="mt-4 text-sm" role="status">
          {updateMessage(t, update)}
        </p>
        {update.rules_available ? (
          <p className="mt-2 text-sm text-muted">{t("updateRulesAvailable")}</p>
        ) : null}
        {update.thirdparty_available ? (
          <p className="mt-2 text-sm text-muted">
            {t("updateThirdpartyAvailable")}
          </p>
        ) : null}

        {update.phase === "downloading" && update.percent !== null ? (
          <div className="mt-4">
            <div
              className="h-2 overflow-hidden rounded-full bg-canvas"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={update.percent}
              aria-label={t("updateDownloadProgress")}
            >
              <div
                className="h-full rounded-full bg-brand transition-[width]"
                style={{ width: `${update.percent}%` }}
              />
            </div>
            <p className="mt-2 text-xs text-muted">{update.percent}%</p>
          </div>
        ) : null}

        {update.error ? (
          <p className="mt-3 text-sm text-danger">{update.error}</p>
        ) : null}

        <div className="mt-5 flex flex-wrap gap-2">
          <button
            type="button"
            className="inline-flex items-center gap-2 rounded-xl bg-brand px-4 py-2.5 font-semibold text-white disabled:opacity-60"
            disabled={busy}
            onClick={() => void checkForUpdate()}
          >
            <RefreshCw size={16} aria-hidden />
            {update.phase === "checking"
              ? t("updateChecking")
              : t("updateCheck")}
          </button>
          {showInstall ? (
            <AppButton
              icon={<Download size={BUTTON_ICON_PX} aria-hidden />}
              className="rounded-xl border border-brand px-4 py-2.5 font-semibold text-brand disabled:opacity-60"
              disabled={busy}
              onClick={() => void installUpdate()}
            >
              {t("updateInstall", { version: update.version ?? "" })}
            </AppButton>
          ) : null}
          {showRetry ? (
            <AppButton
              icon={<RotateCw size={BUTTON_ICON_PX} aria-hidden />}
              className="rounded-xl border border-ink/15 px-4 py-2.5 font-semibold"
              onClick={() => void retryUpdate()}
            >
              {t("updateRetry")}
            </AppButton>
          ) : null}
        </div>
      </div>

      <p className="text-xs text-muted">
        {t("aboutNoticesLabel")}{" "}
        <a
          className="text-brand underline-offset-2 hover:underline"
          href={`${REPOSITORY_URL}/blob/main/resources/licenses/NOTICE.txt`}
          onClick={(event) => {
            event.preventDefault();
            void openRepository();
          }}
        >
          {t("aboutNoticesLink")}
        </a>
      </p>
    </section>
  );
}

function updateMessage(
  t: (key: string, options?: Record<string, string>) => string,
  update: ReturnType<typeof useAppStore.getState>["update"],
) {
  switch (update.phase) {
    case "checking":
      return t("updateChecking");
    case "current":
      return t("updateCurrent");
    case "available":
      return t("updateAvailable", { version: update.version ?? "" });
    case "downloading":
      return t("updateDownloading", { version: update.version ?? "" });
    case "installing":
      return t("updateInstalling");
    case "restarting":
      return t("updateRestarting");
    case "installed":
      return t("updateInstalled");
    case "failed":
      return t("updateFailed");
    default:
      return t("updateIdle");
  }
}
