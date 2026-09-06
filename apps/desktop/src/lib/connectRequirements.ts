import type { DependencyStatus, StackSnapshot } from "../api/models";

export type ConnectRequirement = "helper" | "hiddify" | "mihomo";

export function helperNeedsInstall(
  snapshot: StackSnapshot | null | undefined,
): boolean {
  const phase = snapshot?.helper.phase;
  return phase === "unavailable" || phase === "error";
}

/**
 * Hiddify is only a connect requirement while an enabled Hiddify client
 * exists. An operator who removed Hiddify and promoted another client
 * (e.g. Happ) to the default route must not be forced to install it.
 * Before the first snapshot arrives the client list is unknown, so the
 * legacy requirement stands.
 */
function hiddifyClientEnabled(
  snapshot: StackSnapshot | null | undefined,
): boolean {
  const clients = snapshot?.clients;
  if (!clients) return true;
  return clients.some(
    (client) => client.preset === "hiddify" && client.enabled,
  );
}

export function missingConnectRequirements(
  snapshot: StackSnapshot | null | undefined,
  dependencies: DependencyStatus[],
): ConnectRequirement[] {
  const missing: ConnectRequirement[] = [];
  if (helperNeedsInstall(snapshot)) {
    missing.push("helper");
  }
  const hiddify = dependencies.find((item) => item.id === "hiddify");
  const mihomo = dependencies.find((item) => item.id === "mihomo");
  if (hiddify && !hiddify.installed && hiddifyClientEnabled(snapshot)) {
    missing.push("hiddify");
  }
  if (mihomo && !mihomo.installed) {
    missing.push("mihomo");
  }
  return missing;
}
