import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { StackSnapshot } from "../api/models";
import {
  connectionButtonProgress,
  type ConnectionAction,
} from "../lib/connectionProgress";
import { INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT } from "../lib/sideTunnelConnect";
import { useAppStore } from "../store/app";

export const CONNECTION_BUTTON_ICON_PX = 14;
export const CONNECTION_BUTTON_WIDTH_CLASS = "w-32";
export const CONNECTION_BUTTON_HEIGHT_CLASS = "h-[30px]";

export function ConnectionActionButton({
  action,
  snapshot,
  installingId,
  actionPending,
  disabled,
  onClick,
  icon,
  variant,
}: {
  action: ConnectionAction;
  snapshot: StackSnapshot;
  installingId?: string | null;
  actionPending?: boolean;
  disabled: boolean;
  onClick: () => void;
  icon: ReactNode;
  variant: "primary" | "secondary";
}) {
  const { t } = useTranslation();
  const sideTunnelLastTimeout = useAppStore(
    (state) => state.sideTunnelLastTimeout,
  );
  const progress = connectionButtonProgress(
    snapshot,
    action,
    installingId,
    actionPending,
  );
  const label = t(progress.labelKey);
  const glow = action === "connect" && !disabled && !progress.processing;
  const countdownSeconds =
    progress.processing && action === "connect"
      ? (sideTunnelLastTimeout ?? INITIAL_SIDE_TUNNEL_CONNECT_TIMEOUT)
      : null;

  return (
    <button
      type="button"
      data-connection-action={action}
      data-progress={String(progress.percent)}
      data-processing={progress.processing ? "true" : "false"}
      data-connect-glow={glow ? "available" : "off"}
      aria-busy={progress.processing}
      disabled={disabled}
      onClick={onClick}
      className={`connection-action connection-action-${variant} ${
        progress.processing ? "connection-action-processing" : ""
      } ${glow ? "connect-button-glow" : ""} relative isolate inline-flex ${CONNECTION_BUTTON_WIDTH_CLASS} ${CONNECTION_BUTTON_HEIGHT_CLASS} shrink-0 items-center justify-center gap-1 rounded-[5px] px-2 text-[11px] font-semibold leading-none disabled:cursor-not-allowed disabled:opacity-55`}
    >
      <span className="connection-action-fill-clip" aria-hidden>
        <span
          className="connection-action-fill"
          style={{ width: `${progress.percent}%` }}
        />
      </span>
      <span className="relative z-10 flex min-w-0 flex-1 items-center justify-center gap-1 overflow-hidden">
        <span className="shrink-0">{icon}</span>
        <span className="connection-action-label min-w-0 flex-1 truncate whitespace-nowrap text-center">
          {label}
        </span>
        <span
          className={`connection-action-countdown w-[3ch] shrink-0 tabular-nums text-[10px] leading-none ${
            countdownSeconds === null ? "opacity-0" : "opacity-80"
          }`}
          aria-hidden={countdownSeconds === null}
        >
          {countdownSeconds === null ? "60s" : `${countdownSeconds}s`}
        </span>
      </span>
    </button>
  );
}
