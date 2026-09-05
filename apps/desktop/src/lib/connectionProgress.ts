import type {
  LifecycleBusy,
  OperationStage,
  StackSnapshot,
} from "../api/models";

export type ConnectionAction = "connect" | "disconnect" | "pause" | "resume";

export interface ConnectionButtonProgress {
  labelKey: string;
  percent: number;
  processing: boolean;
}

const STAGE_META: Record<
  OperationStage,
  { percent: number; labelKey: string }
> = {
  preparing: { percent: 10, labelKey: "stages.preparing" },
  starting_client: { percent: 25, labelKey: "stages.startClient" },
  preparing_runtime: { percent: 40, labelKey: "stages.prepareRuntime" },
  validating_config: { percent: 55, labelKey: "stages.validateConfig" },
  starting_core: { percent: 70, labelKey: "stages.startMihomo" },
  checking_readiness: { percent: 85, labelKey: "stages.checkReadiness" },
  stopping_core: { percent: 35, labelKey: "stages.stopMihomo" },
  stopping_proxy: { percent: 65, labelKey: "stages.stopHiddify" },
  cleaning_up: { percent: 85, labelKey: "stages.cleaningUp" },
  recovering: { percent: 50, labelKey: "stages.recovering" },
};

const INSTALL_STAGES: Record<string, { percent: number; labelKey: string }> = {
  helper: { percent: 12, labelKey: "stages.installHelper" },
  hiddify: { percent: 16, labelKey: "stages.installHiddify" },
  mihomo: { percent: 20, labelKey: "stages.installMihomo" },
};

export function busyAction(
  busy: LifecycleBusy | null | undefined,
): ConnectionAction | null {
  switch (busy) {
    case "connecting":
      return "connect";
    case "disconnecting":
      return "disconnect";
    case "pausing":
      return "pause";
    case "resuming":
      return "resume";
    default:
      return null;
  }
}

export function idleLabelKey(action: ConnectionAction): ConnectionAction {
  return action;
}

export function resolveOperationStage(
  snapshot: StackSnapshot,
  installingId?: string | null,
): { percent: number; labelKey: string } | null {
  if (installingId && INSTALL_STAGES[installingId]) {
    return INSTALL_STAGES[installingId];
  }
  if (snapshot.operation_stage) {
    return STAGE_META[snapshot.operation_stage];
  }
  return derivedStage(snapshot);
}

function derivedStage(
  snapshot: StackSnapshot,
): { percent: number; labelKey: string } | null {
  const busy = snapshot.busy ?? null;
  if (busy === "connecting" || busy === "resuming") {
    switch (snapshot.phase) {
      case "starting_client":
        return STAGE_META.starting_client;
      case "preparing_runtime":
        return STAGE_META.preparing_runtime;
      case "validating_config":
        return STAGE_META.validating_config;
      case "starting_core":
        return STAGE_META.starting_core;
      case "checking_readiness":
        return STAGE_META.checking_readiness;
      case "recovering":
        return STAGE_META.recovering;
      case "running":
        return { percent: 100, labelKey: "stages.checkReadiness" };
      default:
        return STAGE_META.preparing;
    }
  }
  if (busy === "disconnecting") {
    if (snapshot.phase === "stopped") {
      return { percent: 100, labelKey: "stages.cleaningUp" };
    }
    if (snapshot.mihomo.phase !== "stopped") {
      return STAGE_META.stopping_core;
    }
    if (snapshot.clients.some((client) => client.status.phase !== "stopped")) {
      return STAGE_META.stopping_proxy;
    }
    return STAGE_META.cleaning_up;
  }
  if (busy === "pausing") {
    if (snapshot.phase === "paused") {
      return { percent: 100, labelKey: "stages.cleaningUp" };
    }
    if (snapshot.mihomo.phase !== "stopped") {
      return STAGE_META.stopping_core;
    }
    return STAGE_META.cleaning_up;
  }
  return null;
}

function optimisticStage(action: ConnectionAction): {
  percent: number;
  labelKey: string;
} {
  if (action === "connect" || action === "resume") {
    return STAGE_META.preparing;
  }
  return STAGE_META.stopping_core;
}

export function connectionButtonProgress(
  snapshot: StackSnapshot,
  action: ConnectionAction,
  installingId?: string | null,
  actionPending = false,
): ConnectionButtonProgress {
  const active = busyAction(snapshot.busy);
  if (active === action) {
    const stage =
      resolveOperationStage(snapshot, installingId) ?? STAGE_META.preparing;
    return {
      labelKey: stage.labelKey,
      percent: stage.percent,
      processing: true,
    };
  }
  if (
    active === null &&
    actionPending &&
    installingId &&
    action === "connect" &&
    INSTALL_STAGES[installingId]
  ) {
    const stage = INSTALL_STAGES[installingId];
    return {
      labelKey: stage.labelKey,
      percent: stage.percent,
      processing: true,
    };
  }
  if (
    active === null &&
    actionPending &&
    isLikelyClickedAction(snapshot, action)
  ) {
    const stage = optimisticStage(action);
    return {
      labelKey: stage.labelKey,
      percent: stage.percent,
      processing: true,
    };
  }
  return {
    labelKey: idleLabelKey(action),
    percent: 0,
    processing: false,
  };
}

function isLikelyClickedAction(
  snapshot: StackSnapshot,
  action: ConnectionAction,
): boolean {
  switch (action) {
    case "connect":
      return (
        snapshot.phase === "stopped" ||
        snapshot.phase === "error" ||
        snapshot.phase === "uninitialized"
      );
    case "resume":
      return snapshot.phase === "paused";
    case "pause":
      return snapshot.phase === "running" || snapshot.phase === "degraded";
    case "disconnect":
      return (
        snapshot.phase === "running" ||
        snapshot.phase === "degraded" ||
        snapshot.phase === "paused"
      );
  }
}
