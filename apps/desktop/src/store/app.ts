import { create } from "zustand";
import { desktop } from "../api/desktop";
import { extractHost } from "../lib/host";
import { missingConnectRequirements } from "../lib/connectRequirements";
import { ACTION_TIMEOUT_MS, controlsLocked } from "../lib/lifecycle";
import type {
  AppConfig,
  BootstrapResult,
  ClientConfig,
  CloudRulesStatus,
  DefaultRoute,
  DependencyStatus,
  DiagnosticsReport,
  DirectRulesDocument,
  InstallGuide,
  NetworkStatus,
  StackSnapshot,
  TrafficTotals,
  UpdateProgress,
  UpdateStatus,
} from "../api/models";
import {
  canAddPreset,
  createClientInstance,
  sanitizeDefaultRoute,
} from "../lib/clients";
import { presetById, type PresetId } from "../lib/presets";

type Page = "dashboard" | "rules" | "diagnostics" | "settings" | "about";

const initialUpdateProgress = (): UpdateProgress => ({
  phase: "idle",
  percent: null,
  version: null,
  error: null,
  app_available: false,
  rules_available: false,
  thirdparty_available: false,
});

interface AppStore {
  loading: boolean;
  actionPending: boolean;
  installingId: string | null;
  page: Page;
  boot: BootstrapResult | null;
  snapshot: StackSnapshot | null;
  settings: AppConfig | null;
  rules: DirectRulesDocument | null;
  cloudRules: CloudRulesStatus | null;
  dependencies: DependencyStatus[];
  networkStatus: NetworkStatus | null;
  networkRefreshing: boolean;
  trafficTotals: TrafficTotals;
  trafficRefreshing: boolean;
  diagnostics: DiagnosticsReport | null;
  error: string | null;
  installGuide: InstallGuide | null;
  update: UpdateProgress;
  setPage: (page: Page) => void;
  initialize: () => Promise<() => void>;
  toggleConnection: () => Promise<void>;
  pauseConnection: () => Promise<void>;
  resumeConnection: () => Promise<void>;
  cancel: () => Promise<void>;
  saveSettings: (draft: AppConfig) => Promise<void>;
  addRule: (input: string) => Promise<void>;
  pinRoute: (input: string, outbound: string) => Promise<void>;
  createList: (name: string, outbound: string) => Promise<void>;
  renameList: (id: string, name: string) => Promise<void>;
  deleteList: (id: string) => Promise<void>;
  setListOutbound: (id: string, outbound: string) => Promise<void>;
  pinToList: (input: string, listId: string) => Promise<void>;
  discardClientPins: (id: string) => Promise<void>;
  reassignClientPins: (from: string, to: string) => Promise<void>;
  addClient: (preset: PresetId) => Promise<void>;
  deleteClient: (id: string, moveTo?: string) => Promise<void>;
  setClientEnabled: (id: string, enabled: boolean) => Promise<void>;
  setClientAllowDirectWhenDown: (id: string, allow: boolean) => Promise<void>;
  updateClient: (id: string, config: ClientConfig) => Promise<void>;
  setDefaultRoute: (route: DefaultRoute) => Promise<void>;
  routeFallbackNotice: string | null;
  clearRouteFallbackNotice: () => void;
  removeRule: (input: string) => Promise<void>;
  refreshRules: () => Promise<void>;
  syncCloudRules: () => Promise<void>;
  refreshNetworkStatus: () => Promise<void>;
  refreshTrafficTotals: () => Promise<void>;
  installDependency: (id: string) => Promise<void>;
  installHelper: () => Promise<void>;
  runDiagnostics: () => Promise<void>;
  applyUpdateProgress: (progress: UpdateProgress) => void;
  checkForUpdate: () => Promise<void>;
  installUpdate: () => Promise<void>;
  retryUpdate: () => Promise<void>;
  openRepository: () => Promise<void>;
  clearError: () => void;
  clearInstallGuide: () => void;
}

function message(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "An unexpected error occurred";
}

async function ensureRequiredServices(
  get: () => AppStore,
  set: (partial: Partial<AppStore>) => void,
): Promise<void> {
  const missing = missingConnectRequirements(
    get().snapshot,
    get().dependencies,
  );
  for (const id of missing) {
    if (id === "helper") {
      set({ installingId: "helper" });
      await desktop.installHelper();
      const snapshot = await desktop.getSnapshot();
      set({ snapshot });
      if (
        snapshot.helper.phase === "unavailable" ||
        snapshot.helper.phase === "error"
      ) {
        throw new Error(
          "privileged helper is still unavailable after installation",
        );
      }
      continue;
    }
    set({ installingId: id });
    const result = await desktop.installDependency(id);
    const dependencies = await desktop.listDependencies();
    set({
      dependencies,
      installGuide: result.installed ? null : result.guide,
    });
    if (!result.installed) {
      throw new Error(`${id} installation did not complete`);
    }
  }
  set({ installingId: null });
}

export const useAppStore = create<AppStore>((set, get) => ({
  loading: true,
  actionPending: false,
  installingId: null,
  page: "dashboard",
  boot: null,
  snapshot: null,
  settings: null,
  rules: null,
  cloudRules: null,
  dependencies: [],
  networkStatus: null,
  networkRefreshing: false,
  trafficTotals: { sent: 0, received: 0 },
  trafficRefreshing: false,
  diagnostics: null,
  error: null,
  routeFallbackNotice: null,
  installGuide: null,
  update: initialUpdateProgress(),
  setPage: (page) => set({ page }),
  initialize: async () => {
    try {
      const boot = await desktop.bootstrap();
      set({
        loading: false,
        boot,
        snapshot: boot.snapshot,
        settings: boot.settings,
        rules: boot.direct_rules,
        cloudRules: boot.cloud_rules,
        dependencies: boot.dependencies,
        networkStatus: boot.network_status,
        update: initialUpdateProgress(),
      });
      void get().refreshNetworkStatus();
      void get().refreshTrafficTotals();
      const unsubscribeSnapshot = await desktop.subscribe((snapshot) => {
        set({
          snapshot,
          actionPending: snapshot.busy != null,
        });
        void get().refreshTrafficTotals();
      });
      const unsubscribeUpdate = await desktop.subscribeUpdateProgress(
        (progress) => {
          get().applyUpdateProgress(progress);
        },
      );
      return () => {
        unsubscribeSnapshot();
        unsubscribeUpdate();
      };
    } catch (error) {
      set({ loading: false, error: message(error) });
      return () => undefined;
    }
  },
  toggleConnection: async () => {
    const snapshot = get().snapshot;
    if (!snapshot || controlsLocked(snapshot, get().actionPending)) return;
    set({ actionPending: true, error: null, installGuide: null });
    const timeout = window.setTimeout(() => {
      const current = get().snapshot;
      if (get().actionPending) {
        set({
          actionPending: false,
          installingId: null,
          error: "The connection operation timed out.",
          snapshot: current ? { ...current, busy: null } : current,
        });
      }
    }, ACTION_TIMEOUT_MS);
    try {
      if (
        snapshot.phase === "running" ||
        snapshot.phase === "degraded" ||
        snapshot.phase === "paused"
      ) {
        await desktop.stop();
      } else {
        await ensureRequiredServices(get, set);
        await desktop.start();
      }
    } catch (error) {
      set({
        actionPending: false,
        installingId: null,
        error: message(error),
      });
    } finally {
      window.clearTimeout(timeout);
    }
  },
  pauseConnection: async () => {
    if (controlsLocked(get().snapshot, get().actionPending)) return;
    set({ actionPending: true, error: null });
    try {
      await desktop.pause();
    } catch (error) {
      set({ actionPending: false, error: message(error) });
    }
  },
  resumeConnection: async () => {
    if (controlsLocked(get().snapshot, get().actionPending)) return;
    set({ actionPending: true, error: null });
    try {
      await desktop.resume();
    } catch (error) {
      set({ actionPending: false, error: message(error) });
    }
  },
  cancel: async () => {
    const operationId = get().snapshot?.operation_id;
    if (operationId) await desktop.cancel(operationId);
  },
  saveSettings: async (draft) => {
    const current = get().settings;
    if (!current) return;
    set({ actionPending: true, error: null });
    try {
      const sanitized = sanitizeDefaultRoute(draft);
      const fallback =
        draft.default_route.kind === "client" &&
        sanitized.default_route.kind === "direct";
      const settings = await desktop.saveSettings(sanitized, current.revision);
      set({
        settings,
        actionPending: false,
        routeFallbackNotice: fallback ? "defaultRouteFallback" : null,
      });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
    }
  },
  addRule: async (input) => {
    const rules = get().rules;
    if (!rules) return;
    const host = extractHost(input);
    if (!host) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.addRule(host, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  pinRoute: async (input, outbound) => {
    const rules = get().rules;
    if (!rules) return;
    const host = extractHost(input);
    if (!host) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.pinRoute(host, outbound, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  createList: async (name, outbound) => {
    const rules = get().rules;
    if (!rules) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.createRuleList(name, outbound, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  renameList: async (id, name) => {
    const rules = get().rules;
    if (!rules) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.renameRuleList(id, name, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  deleteList: async (id) => {
    const rules = get().rules;
    if (!rules) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.deleteRuleList(id, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  setListOutbound: async (id, outbound) => {
    const rules = get().rules;
    if (!rules) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.setRuleListOutbound(
        id,
        outbound,
        rules.revision,
      );
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  pinToList: async (input, listId) => {
    const rules = get().rules;
    if (!rules) return;
    const host = extractHost(input);
    if (!host) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.pinToRuleList(host, listId, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  discardClientPins: async (id) => {
    const rules = get().rules;
    if (!rules) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.discardClientPins(id, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  reassignClientPins: async (from, to) => {
    const rules = get().rules;
    if (!rules) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.reassignClientPins(from, to, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  addClient: async (preset) => {
    const current = get().settings;
    if (!current) return;
    if (!canAddPreset(preset, current.clients)) {
      throw new Error("that client is already added or is not available yet");
    }
    const instance = createClientInstance(preset);
    const next = {
      ...current,
      clients: [...current.clients, instance],
    };
    await get().saveSettings(next);
    // Every client starts with its own named list so the pin flow has an
    // obvious destination. Best-effort: the client itself is already saved.
    try {
      await get().createList(presetById(preset).title, instance.id);
    } catch {
      // The registry stays usable without the list; the user can add one.
    }
  },
  deleteClient: async (id, moveTo) => {
    if (moveTo) {
      await get().reassignClientPins(id, moveTo);
    } else {
      await get().discardClientPins(id);
    }
    const current = get().settings;
    if (!current) return;
    await get().saveSettings({
      ...current,
      clients: current.clients.filter((client) => client.id !== id),
    });
  },
  setClientEnabled: async (id, enabled) => {
    const current = get().settings;
    if (!current) return;
    const next = {
      ...current,
      clients: current.clients.map((client) =>
        client.id === id ? { ...client, enabled } : client,
      ),
    };
    await get().saveSettings(next);
  },
  setClientAllowDirectWhenDown: async (id, allow) => {
    const current = get().settings;
    if (!current) return;
    const next = {
      ...current,
      clients: current.clients.map((client) =>
        client.id === id
          ? { ...client, allow_direct_when_down: allow }
          : client,
      ),
    };
    await get().saveSettings(next);
  },
  updateClient: async (id, config) => {
    const current = get().settings;
    if (!current) return;
    const next = {
      ...current,
      clients: current.clients.map((client) =>
        client.id === id ? { ...client, config } : client,
      ),
    };
    await get().saveSettings(next);
  },
  setDefaultRoute: async (route) => {
    const current = get().settings;
    if (!current) return;
    await get().saveSettings({ ...current, default_route: route });
  },
  clearRouteFallbackNotice: () => set({ routeFallbackNotice: null }),
  removeRule: async (input) => {
    const rules = get().rules;
    if (!rules) return;
    set({ actionPending: true, error: null });
    try {
      const next = await desktop.removeRule(input, rules.revision);
      set({ rules: next, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
      throw error;
    }
  },
  refreshRules: async () => {
    set({ actionPending: true, error: null });
    try {
      const rules = await desktop.refreshRules();
      set({ rules, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
    }
  },
  syncCloudRules: async () => {
    if (get().actionPending) {
      return;
    }
    set({ actionPending: true, error: null });
    try {
      const cloudRules = await desktop.syncCloudRules();
      set({ cloudRules, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
    }
  },
  refreshTrafficTotals: async () => {
    if (get().trafficRefreshing) {
      return;
    }
    set({ trafficRefreshing: true });
    try {
      const trafficTotals = await desktop.getTrafficTotals();
      set({ trafficTotals, trafficRefreshing: false });
    } catch {
      set({ trafficRefreshing: false });
    }
  },
  refreshNetworkStatus: async () => {
    if (get().networkRefreshing) {
      return;
    }
    set({ networkRefreshing: true });
    try {
      const networkStatus = await desktop.getNetworkStatus();
      set({ networkStatus, networkRefreshing: false });
    } catch (error) {
      set({
        networkRefreshing: false,
        networkStatus: {
          state: "offline",
          public_ip: null,
          country_code: null,
          city: null,
          checked_at: new Date().toISOString(),
          detail: message(error),
        },
      });
    }
  },
  installDependency: async (id) => {
    set({ installingId: id, error: null, installGuide: null });
    try {
      const result = await desktop.installDependency(id);
      const dependencies = await desktop.listDependencies();
      set({
        dependencies,
        installingId: null,
        installGuide: result.installed ? null : result.guide,
      });
    } catch (error) {
      const guide = await desktop.getInstallGuide(id).catch(() => null);
      set({ installingId: null, error: message(error), installGuide: guide });
    }
  },
  installHelper: async () => {
    set({ installingId: "helper", error: null });
    try {
      await desktop.installHelper();
      const snapshot = await desktop.getSnapshot();
      set({ installingId: null, snapshot });
    } catch (error) {
      set({ installingId: null, error: message(error) });
    }
  },
  runDiagnostics: async () => {
    set({ actionPending: true, diagnostics: null, error: null });
    try {
      const diagnostics = await desktop.diagnostics();
      set({ diagnostics, actionPending: false });
    } catch (error) {
      set({ actionPending: false, error: message(error) });
    }
  },
  applyUpdateProgress: (progress) => {
    const current = get().update;
    if (
      progress.operation_id &&
      current.operation_id &&
      progress.operation_id !== current.operation_id &&
      (current.phase === "downloading" ||
        current.phase === "installing" ||
        current.phase === "restarting")
    ) {
      return;
    }
    set({ update: { ...current, ...progress, error: progress.error ?? null } });
  },
  checkForUpdate: async () => {
    const phase = get().update.phase;
    if (
      phase === "checking" ||
      phase === "downloading" ||
      phase === "installing" ||
      phase === "restarting"
    ) {
      return;
    }
    set({
      update: {
        phase: "checking",
        percent: null,
        version: null,
        error: null,
      },
    });
    try {
      const status = await desktop.checkUpdate();
      set({
        update: updateStatusToProgress(status),
      });
    } catch (error) {
      set({
        update: {
          phase: "failed",
          percent: null,
          version: null,
          error: message(error),
        },
      });
    }
  },
  installUpdate: async () => {
    const phase = get().update.phase;
    if (
      phase === "checking" ||
      phase === "downloading" ||
      phase === "installing" ||
      phase === "restarting"
    ) {
      return;
    }
    const current = get().update;
    set({
      update: {
        phase: "downloading",
        percent: 0,
        version: current.version,
        error: null,
      },
    });
    try {
      await desktop.installUpdate();
    } catch (error) {
      set({
        update: {
          phase: "failed",
          percent: null,
          version: current.version,
          error: message(error),
        },
      });
    }
  },
  retryUpdate: async () => {
    const { update, checkForUpdate, installUpdate } = get();
    if (update.version) {
      set({
        update: {
          phase: "available",
          percent: null,
          version: update.version,
          error: null,
        },
      });
      await installUpdate();
      return;
    }
    await checkForUpdate();
    if (get().update.phase === "available") {
      await installUpdate();
    }
  },
  openRepository: async () => {
    await desktop.openUrl("https://github.com/devlifeX/BiFlow");
  },
  clearError: () => set({ error: null, routeFallbackNotice: null }),
  clearInstallGuide: () => set({ installGuide: null, error: null }),
}));

function updateStatusToProgress(status: UpdateStatus): UpdateProgress {
  if (!status.available) {
    return {
      phase: "current",
      percent: null,
      version: null,
      error: null,
      app_available: false,
      rules_available: false,
      thirdparty_available: false,
    };
  }
  return {
    phase: "available",
    percent: null,
    version: status.version,
    error: null,
    app_available: status.app_available,
    rules_available: status.rules_available,
    thirdparty_available: status.thirdparty_available,
  };
}
