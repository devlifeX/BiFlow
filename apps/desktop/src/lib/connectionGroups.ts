import type { ActiveConnection } from "../api/models";

export interface ConnectionGroup {
  key: string;
  host: string;
  ips: string[];
  outbound: string;
  rule: string;
  count: number;
}

/**
 * Collapses per-socket connections into one row per domain and route.
 *
 * A single page load opens many sockets to the same host, which made the
 * live list unreadably long. Connections that share a host, outbound, and
 * matched rule become one group; a host that is somehow split between
 * DIRECT and VPN stays visible as two rows. Hostless (pure IP) entries
 * group by destination IP instead.
 */
export function groupConnections(rows: ActiveConnection[]): ConnectionGroup[] {
  const groups = new Map<string, ConnectionGroup>();
  for (const row of rows) {
    const host = row.host.trim();
    const label = host || row.destination_ip;
    const key = `${label}|${row.outbound}|${row.rule}`;
    const group = groups.get(key);
    if (group) {
      group.count += 1;
      if (row.destination_ip && !group.ips.includes(row.destination_ip)) {
        group.ips.push(row.destination_ip);
      }
    } else {
      groups.set(key, {
        key,
        host,
        ips: row.destination_ip ? [row.destination_ip] : [],
        outbound: row.outbound,
        rule: row.rule,
        count: 1,
      });
    }
  }
  return [...groups.values()].sort(
    (a, b) =>
      b.count - a.count ||
      (a.host || a.ips[0] || "").localeCompare(b.host || b.ips[0] || ""),
  );
}

export function formatGroupIps(ips: string[]): string {
  if (ips.length === 0) return "";
  if (ips.length === 1) return ips[0] ?? "";
  return `${ips[0]} +${ips.length - 1}`;
}
