/**
 * Google hosts are a debug-only reachability probe. Production packages
 * omit them from that card; live connections and Dashboard packets still
 * show whatever Mihomo is handling, including a google.com pin.
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
