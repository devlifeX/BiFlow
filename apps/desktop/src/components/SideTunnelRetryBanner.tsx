import { useTranslation } from "react-i18next";
import type { ClientInstance, StackSnapshot } from "../api/models";
import {
  failedSideTunnelClients,
  INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT,
  nextSideTunnelRetryTimeout,
} from "../lib/sideTunnelConnect";
import { useAppStore } from "../store/app";

export function SideTunnelRetryBanner({
  snapshot,
  clients,
}: {
  snapshot: StackSnapshot;
  clients: ClientInstance[];
}) {
  const { t } = useTranslation();
  const { actionPending, sideTunnelLastTimeout, retrySideTunnelConnect } =
    useAppStore();
  const failedSideTunnels = failedSideTunnelClients(snapshot, clients);
  const sideTunnelRetryTimeout = sideTunnelLastTimeout
    ? nextSideTunnelRetryTimeout(sideTunnelLastTimeout)
    : nextSideTunnelRetryTimeout(INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT);

  if (failedSideTunnels.length === 0) return null;

  if (!sideTunnelRetryTimeout) {
    return (
      <p
        data-testid="side-tunnel-retry-exhausted"
        className="rounded-md border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-[12px] leading-snug"
        role="status"
      >
        {t("sideTunnelStartExhausted", { seconds: 60 })}
      </p>
    );
  }

  return (
    <div
      data-testid="side-tunnel-retry-banner"
      className="rounded-md border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-[12px] leading-snug"
      role="status"
    >
      <p>
        {t("sideTunnelStartFailed", {
          seconds: sideTunnelLastTimeout ?? INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT,
        })}
      </p>
      <button
        type="button"
        className="mt-2 inline-flex h-[30px] items-center rounded-[5px] bg-brand px-2 text-[11px] font-semibold text-white disabled:opacity-55"
        disabled={actionPending}
        onClick={() => void retrySideTunnelConnect()}
      >
        {t("sideTunnelRetryWithTimeout", {
          seconds: sideTunnelRetryTimeout,
        })}
      </button>
    </div>
  );
}
