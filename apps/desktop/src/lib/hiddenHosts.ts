/**
 * Google hosts are a debug-only probe and a common Happ pin. Production
 * packages still route them; they just omit the hostname from the UI.
 */
export function isGoogleHost(host: string): boolean {
  const normalized = host.trim().replace(/\.$/u, "").toLowerCase();
  return normalized === "google.com" || normalized.endsWith(".google.com");
}

export function isHiddenProductionHost(
  host: string,
  production = import.meta.env.PROD,
): boolean {
  return Boolean(production) && isGoogleHost(host);
}
