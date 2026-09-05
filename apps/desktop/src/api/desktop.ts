import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { mockApi } from "./mock";
import type {
  AppConfig,
  BootstrapResult,
  CloudRulesStatus,
  DebugLogStatus,
  DependencyStatus,
  DiagnosticsReport,
  DirectRulesDocument,
  ExportResult,
  FreshStartReport,
  InstallGuide,
  InstallResult,
  ListCheckEntry,
  LogEntry,
  NetworkStatus,
  OperationAccepted,
  ReachabilityResult,
  TrafficTotals,
  RouteTestResult,
  StackSnapshot,
  UpdateProgress,
  UpdateStatus,
  ValidationIssue,
  ActiveConnection,
} from "./models";

const native =
  typeof window !== "undefined" && window.__TAURI_INTERNALS__ !== undefined;

export const desktop = {
  bootstrap(): Promise<BootstrapResult> {
    return native ? invoke("bootstrap_app") : mockApi.bootstrap();
  },
  getSnapshot(): Promise<StackSnapshot> {
    return native ? invoke("get_stack_snapshot") : mockApi.getSnapshot();
  },
  getNetworkStatus(): Promise<NetworkStatus> {
    return native ? invoke("get_network_status") : mockApi.getNetworkStatus();
  },
  checkReachability(): Promise<ReachabilityResult[]> {
    return native ? invoke("check_reachability") : mockApi.checkReachability();
  },
  getTrafficTotals(): Promise<TrafficTotals> {
    return native ? invoke("get_traffic_totals") : mockApi.getTrafficTotals();
  },
  listActiveConnections(): Promise<ActiveConnection[]> {
    return native
      ? invoke("list_active_connections")
      : mockApi.listActiveConnections();
  },
  start(): Promise<OperationAccepted> {
    return native ? invoke("start_stack") : mockApi.start();
  },
  stop(): Promise<OperationAccepted> {
    return native ? invoke("stop_stack") : mockApi.stop();
  },
  pause(): Promise<OperationAccepted> {
    return native ? invoke("pause_stack") : mockApi.pause();
  },
  resume(): Promise<OperationAccepted> {
    return native ? invoke("resume_stack") : mockApi.resume();
  },
  cancel(operationId: string): Promise<boolean> {
    return native
      ? invoke("cancel_operation", { operationId })
      : mockApi.cancel(operationId);
  },
  getSettings(): Promise<AppConfig> {
    return native ? invoke("get_settings") : mockApi.getSettings();
  },
  validateSettings(draft: AppConfig): Promise<ValidationIssue[]> {
    return native
      ? invoke("validate_settings", { draft })
      : mockApi.validateSettings(draft);
  },
  saveSettings(draft: AppConfig, expectedRevision: number): Promise<AppConfig> {
    return native
      ? invoke("save_settings", { draft, expectedRevision })
      : mockApi.saveSettings(draft, expectedRevision);
  },
  listRules(): Promise<DirectRulesDocument> {
    return native ? invoke("list_direct_rules") : mockApi.listRules();
  },
  addRule(
    input: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("add_direct_rule", { input, expectedRevision })
      : mockApi.addRule(input, expectedRevision);
  },
  removeRule(
    input: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("remove_direct_rule", { input, expectedRevision })
      : mockApi.removeRule(input, expectedRevision);
  },
  pinRoute(
    input: string,
    outbound: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("pin_route", { input, outbound, expectedRevision })
      : mockApi.pinRoute(input, outbound, expectedRevision);
  },
  discardClientPins(
    id: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("discard_client_pins", { id, expectedRevision })
      : mockApi.discardClientPins(id, expectedRevision);
  },
  reassignClientPins(
    from: string,
    to: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("reassign_client_pins", { from, to, expectedRevision })
      : mockApi.reassignClientPins(from, to, expectedRevision);
  },
  refreshRules(): Promise<DirectRulesDocument> {
    return native ? invoke("refresh_direct_rules") : mockApi.refreshRules();
  },
  createRuleList(
    name: string,
    outbound: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("create_rule_list", { name, outbound, expectedRevision })
      : mockApi.createRuleList(name, outbound, expectedRevision);
  },
  renameRuleList(
    listId: string,
    name: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("rename_rule_list", { listId, name, expectedRevision })
      : mockApi.renameRuleList(listId, name, expectedRevision);
  },
  deleteRuleList(
    listId: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("delete_rule_list", { listId, expectedRevision })
      : mockApi.deleteRuleList(listId, expectedRevision);
  },
  setRuleListOutbound(
    listId: string,
    outbound: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("set_rule_list_outbound", { listId, outbound, expectedRevision })
      : mockApi.setRuleListOutbound(listId, outbound, expectedRevision);
  },
  pinToRuleList(
    input: string,
    listId: string,
    expectedRevision: number,
  ): Promise<DirectRulesDocument> {
    return native
      ? invoke("pin_to_rule_list", { input, listId, expectedRevision })
      : mockApi.pinToRuleList(input, listId, expectedRevision);
  },
  checkRuleList(listId: string): Promise<ListCheckEntry[]> {
    return native
      ? invoke("check_rule_list", { listId })
      : mockApi.checkRuleList(listId);
  },
  getCloudRules(): Promise<CloudRulesStatus> {
    return native ? invoke("get_cloud_rules_status") : mockApi.getCloudRules();
  },
  syncCloudRules(): Promise<CloudRulesStatus> {
    return native ? invoke("sync_cloud_rules") : mockApi.syncCloudRules();
  },
  listDependencies(): Promise<DependencyStatus[]> {
    return native ? invoke("list_dependencies") : mockApi.listDependencies();
  },
  installDependency(id: string): Promise<InstallResult> {
    return native
      ? invoke("install_dependency", { id })
      : mockApi.installDependency(id);
  },
  installHelper(): Promise<{ installed: boolean }> {
    return native ? invoke("install_helper") : mockApi.installHelper();
  },
  freshHiddifyStart(): Promise<FreshStartReport> {
    return native ? invoke("fresh_hiddify_start") : mockApi.freshHiddifyStart();
  },
  getInstallGuide(id: string): Promise<InstallGuide> {
    return native
      ? invoke("get_install_guide", { id })
      : mockApi.getInstallGuide(id);
  },
  openUrl(url: string): Promise<void> {
    return native ? invoke("open_external_url", { url }) : mockApi.openUrl(url);
  },
  testRoute(target: string): Promise<RouteTestResult> {
    return native
      ? invoke("test_route", { target })
      : mockApi.testRoute(target);
  },
  diagnostics(): Promise<DiagnosticsReport> {
    return native ? invoke("run_full_diagnostics") : mockApi.diagnostics();
  },
  queryLogs(): Promise<LogEntry[]> {
    return native
      ? invoke("query_logs", { maximum: 500 })
      : mockApi.queryLogs();
  },
  debugLogStatus(): Promise<DebugLogStatus> {
    return native ? invoke("get_debug_log_status") : mockApi.debugLogStatus();
  },
  revealDebugLog(): Promise<DebugLogStatus> {
    return native ? invoke("reveal_debug_log") : mockApi.revealDebugLog();
  },
  deleteDebugLog(): Promise<DebugLogStatus> {
    return native ? invoke("delete_debug_log") : mockApi.deleteDebugLog();
  },
  exportBundle(): Promise<ExportResult> {
    return native ? invoke("export_support_bundle") : mockApi.exportBundle();
  },
  checkUpdate(): Promise<UpdateStatus> {
    return native ? invoke("check_for_update") : mockApi.checkUpdate();
  },
  getUpdateState(): Promise<UpdateProgress> {
    return native ? invoke("get_update_state") : mockApi.getUpdateState();
  },
  installUpdate(): Promise<OperationAccepted> {
    return native ? invoke("install_update") : mockApi.installUpdate();
  },
  async subscribeUpdateProgress(
    listener: (progress: UpdateProgress) => void,
  ): Promise<() => void> {
    if (!native) return mockApi.subscribeUpdateProgress(listener);
    const unlisten = await listen<UpdateProgress>("update-progress", (event) =>
      listener(event.payload),
    );
    return unlisten;
  },
  async subscribe(
    listener: (snapshot: StackSnapshot) => void,
  ): Promise<() => void> {
    if (!native) return mockApi.subscribe(listener);
    const unlisten = await listen<StackSnapshot>("stack-snapshot", (event) =>
      listener(event.payload),
    );
    return unlisten;
  },
  async subscribeNavigation(
    listener: (
      page: "dashboard" | "rules" | "diagnostics" | "settings" | "about",
    ) => void,
  ): Promise<() => void> {
    if (!native) return () => undefined;
    const unlisten = await listen<string>("app-navigate", (event) => {
      const page = event.payload;
      if (
        page === "dashboard" ||
        page === "rules" ||
        page === "diagnostics" ||
        page === "settings" ||
        page === "about"
      ) {
        listener(page);
      }
    });
    return unlisten;
  },
};

export type { AppConfig, StackSnapshot } from "./models";
