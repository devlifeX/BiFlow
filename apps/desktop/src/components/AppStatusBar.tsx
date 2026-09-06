import {
  ArrowDown,
  ArrowUp,
  LoaderCircle,
  MapPin,
  Wifi,
  WifiOff,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type { NetworkStatus } from "../api/models";
import { formatTrafficBytes } from "../lib/formatTraffic";
import { useAppStore } from "../store/app";

function countryName(code: string | null, language: string): string | null {
  if (!code) return null;
  try {
    return (
      new Intl.DisplayNames([language], { type: "region" }).of(code) ?? code
    );
  } catch {
    return code;
  }
}

function locationLabel(status: NetworkStatus, language: string): string | null {
  const country = countryName(status.country_code, language);
  return [status.city, country].filter(Boolean).join(", ") || null;
}

export function AppStatusBar() {
  const { i18n, t } = useTranslation();
  const status = useAppStore((state) => state.networkStatus);
  const refreshing = useAppStore((state) => state.networkRefreshing);
  const refreshNetworkStatus = useAppStore(
    (state) => state.refreshNetworkStatus,
  );
  const traffic = useAppStore((state) => state.trafficTotals);
  const current: NetworkStatus = status ?? {
    state: "checking",
    public_ip: null,
    country_code: null,
    city: null,
    checked_at: new Date().toISOString(),
    detail: null,
  };
  const location = locationLabel(current, i18n.language);
  const state = refreshing ? "checking" : current.state;

  return (
    <footer
      className="app-status-bar sticky bottom-0 z-20 shrink-0 border-t border-[rgb(var(--border-default))] bg-surface px-4 py-2 text-[11px] text-muted"
      role="status"
      aria-live="polite"
      title={current.detail ?? undefined}
    >
      <div className="mx-auto flex w-full max-w-6xl flex-wrap items-center gap-x-5 gap-y-2">
        <button
          type="button"
          className="inline-flex items-center gap-2 font-semibold text-ink disabled:opacity-70"
          onClick={() => void refreshNetworkStatus()}
          disabled={refreshing}
          aria-label={t("refreshNetwork")}
        >
          {state === "online" ? (
            <Wifi className="text-success" size={16} aria-hidden />
          ) : state === "offline" ? (
            <WifiOff className="text-danger" size={16} aria-hidden />
          ) : (
            <LoaderCircle
              className="animate-spin text-muted"
              size={16}
              aria-hidden
            />
          )}
          {t(`internet.${state}`)}
        </button>
        {current.public_ip ? (
          <button
            type="button"
            className="text-start disabled:opacity-70"
            onClick={() => void refreshNetworkStatus()}
            disabled={refreshing}
            aria-label={t("refreshNetwork")}
          >
            {t("currentIp")}:{" "}
            <bdi className="font-semibold tabular-nums text-ink">
              {current.public_ip}
            </bdi>
          </button>
        ) : null}
        {location ? (
          <span className="inline-flex items-center gap-1.5">
            <MapPin size={14} aria-hidden className="shrink-0 text-muted" />
            <span>{location}</span>
          </span>
        ) : null}
        <span
          className="inline-flex items-center gap-3 tabular-nums text-ink"
          data-testid="traffic-totals"
          title={`${traffic.sent} / ${traffic.received}`}
        >
          <span
            className="inline-flex items-center gap-1"
            title={t("trafficSent")}
          >
            <ArrowUp size={14} aria-hidden />
            <span>
              {t("trafficSent")}: {formatTrafficBytes(traffic.sent)}
            </span>
          </span>
          <span
            className="inline-flex items-center gap-1"
            title={t("trafficReceived")}
          >
            <ArrowDown size={14} aria-hidden />
            <span>
              {t("trafficReceived")}: {formatTrafficBytes(traffic.received)}
            </span>
          </span>
        </span>
      </div>
    </footer>
  );
}
