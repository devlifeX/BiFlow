#!/usr/bin/env node
import { isIP } from "node:net";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const rulesDir = join(root, "resources/rules");
const outDir = join(root, "docs/considering/iran-coverage-out");
const USER_AGENT =
  "BiFlow iran-coverage research (https://github.com/devlifeX/BiFlow)";

const TEHRAN_INDEX_PAGES = [
  {
    id: "tehranindex-companies",
    path: "/companies",
    note: "Tehran Index company directory (all published records).",
  },
  {
    id: "tehranindex-ecommerce",
    path: "/sectors/ecommerce",
    note: "Tehran Index ecommerce sector page.",
  },
  {
    id: "tehranindex-fintech",
    path: "/sectors/fintech",
    note: "Tehran Index fintech sector page.",
  },
  {
    id: "tehranindex-crypto",
    path: "/sectors/crypto",
    note: "Tehran Index digital-assets sector page.",
  },
  {
    id: "tehranindex-cloud",
    path: "/sectors/cloud",
    note: "Tehran Index cloud sector page.",
  },
  {
    id: "tehranindex-mobility",
    path: "/sectors/mobility",
    note: "Tehran Index mobility sector page.",
  },
  {
    id: "tehranindex-travel",
    path: "/sectors/travel",
    note: "Tehran Index travel sector page.",
  },
  {
    id: "tehranindex-media",
    path: "/sectors/media",
    note: "Tehran Index media sector page.",
  },
  {
    id: "tehranindex-health",
    path: "/sectors/health",
    note: "Tehran Index healthtech sector page.",
  },
  {
    id: "tehranindex-foodtech",
    path: "/sectors/foodtech",
    note: "Tehran Index foodtech sector page.",
  },
  {
    id: "tehranindex-edtech",
    path: "/sectors/edtech",
    note: "Tehran Index edtech sector page.",
  },
  {
    id: "tehranindex-hrtech",
    path: "/sectors/hrtech",
    note: "Tehran Index HR-tech sector page.",
  },
  {
    id: "tehranindex-saas",
    path: "/sectors/saas",
    note: "Tehran Index SaaS sector page.",
  },
  {
    id: "tehranindex-ai",
    path: "/sectors/ai",
    note: "Tehran Index AI sector page.",
  },
  {
    id: "tehranindex-telecom",
    path: "/sectors/telecom",
    note: "Tehran Index telecom sector page.",
  },
  {
    id: "tehranindex-logistics",
    path: "/sectors/logistics",
    note: "Tehran Index logistics sector page.",
  },
  {
    id: "tehranindex-cybersecurity",
    path: "/sectors/cybersecurity",
    note: "Tehran Index cybersecurity sector page.",
  },
  {
    id: "tehranindex-gaming",
    path: "/sectors/gaming",
    note: "Tehran Index gaming sector page.",
  },
  {
    id: "tehranindex-classifieds",
    path: "/sectors/classifieds",
    note: "Tehran Index classifieds sector page.",
  },
  {
    id: "tehranindex-enterprise-software",
    path: "/sectors/enterprise-software",
    note: "Tehran Index enterprise-software sector page.",
  },
  {
    id: "tehranindex-accelerator",
    path: "/sectors/accelerator",
    note: "Tehran Index accelerator sector page.",
  },
  {
    id: "tehranindex-appdist",
    path: "/sectors/appdist",
    note: "Tehran Index app-distribution sector page.",
  },
  {
    id: "tehranindex-agri",
    path: "/sectors/agri",
    note: "Tehran Index agritech sector page.",
  },
];

export const DOMAIN_SOURCES = [
  {
    id: "bootmortis-other",
    url: "https://github.com/bootmortis/iran-hosted-domains/releases/latest/download/clash_rules_other.txt",
    kind: "clash_payload",
    note: "bootmortis `other` category: non-.ir domains used as DIRECT. Never ads/proxy.",
  },
  {
    id: "bootmortis-custom-direct",
    url: "https://raw.githubusercontent.com/bootmortis/iran-hosted-domains/main/src/data/custom_domains.py",
    kind: "python_direct_set",
    note: "bootmortis custom_domains.py `direct` set only.",
  },
  ...TEHRAN_INDEX_PAGES.map((page) => ({
    id: page.id,
    url: `https://tehranindex.com${page.path}`,
    kind: "html_hosts",
    note: page.note,
  })),
];

export const IP_SOURCES = [
  {
    id: "arvancloud-en",
    url: "https://www.arvancloud.ir/en/ips.txt",
    kind: "cidr_lines",
    publisher: "Arvancloud",
    note: "Official Arvancloud IPv4/IPv6 prefix list.",
  },
  {
    id: "arvancloud-fa",
    url: "https://www.arvancloud.ir/fa/ips.txt",
    kind: "cidr_lines",
    publisher: "Arvancloud",
    note: "Persian alias of the same official prefix list (fallback).",
  },
  {
    id: "ripestat-ir",
    url: "https://stat.ripe.net/data/country-resource-list/data.json?resource=IR&v4_format=prefix",
    kind: "ripestat",
    publisher: "RIPEstat",
    note: "IR country-resource-list gap report only. Do not union blindly.",
  },
];

export function parseCidr(value) {
  const trimmed = value.trim();
  const slash = trimmed.lastIndexOf("/");
  const addr = slash === -1 ? trimmed : trimmed.slice(0, slash);
  const version = isIP(addr);
  if (!version) {
    throw new Error(`malformed CIDR: ${trimmed.slice(0, 80)}`);
  }
  const width = version === 4 ? 32 : 128;
  const bits =
    slash === -1 ? width : Number.parseInt(trimmed.slice(slash + 1), 10);
  if (!Number.isInteger(bits) || bits < 0 || bits > width) {
    throw new Error(`malformed CIDR prefix: ${trimmed.slice(0, 80)}`);
  }
  const start = ipToBigInt(addr, version);
  const hostBits = BigInt(width - bits);
  const mask =
    hostBits === BigInt(width)
      ? 0n
      : ((1n << BigInt(width)) - 1n) ^ ((1n << hostBits) - 1n);
  const network = start & mask;
  const end = network | ((1n << hostBits) - 1n);
  return { cidr: `${addr}/${bits}`, version, bits, network, end };
}

export function cidrContains(outer, inner) {
  if (outer.version !== inner.version) return false;
  return inner.network >= outer.network && inner.end <= outer.end;
}

export function isCoveredByCidrs(candidate, existing) {
  const parsed =
    typeof candidate === "string" ? parseCidr(candidate) : candidate;
  return existing.some((network) => cidrContains(network, parsed));
}

export function normalizeDomain(value) {
  return value.trim().toLowerCase().replace(/^\+\./u, "").replace(/\.$/u, "");
}

export function domainCoveredBySuffixes(host, suffixes) {
  const domain = normalizeDomain(host);
  if (!domain) return false;
  return suffixes.some(
    (suffix) => domain === suffix || domain.endsWith(`.${suffix}`),
  );
}

export function parseProviderLines(text) {
  return text
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter(
      (line) => line.length > 0 && !line.startsWith("#") && line !== "payload:",
    );
}

export function extractDomainsFromClashPayload(text) {
  const domains = [];
  for (const raw of parseProviderLines(text)) {
    const line = raw.replace(/^-\s+/u, "");
    const suffix = line.match(/^DOMAIN-SUFFIX,([^,]+)/iu);
    const keyword = line.match(/^DOMAIN-KEYWORD,/iu);
    if (keyword) continue;
    if (suffix) {
      domains.push(normalizeDomain(suffix[1]));
      continue;
    }
    if (line.startsWith("DOMAIN,")) {
      domains.push(normalizeDomain(line.slice("DOMAIN,".length).split(",")[0]));
      continue;
    }
    if (line.startsWith("+.") || /^[a-z0-9.-]+$/iu.test(line)) {
      domains.push(normalizeDomain(line));
    }
  }
  return uniqueSorted(domains.filter(Boolean));
}

export function extractDirectSetFromPython(text) {
  const named = text.match(/(?:^|\n)direct\s*=\s*\{([\s\S]*?)\n\}/u);
  const keyed = text.match(/(?:^|\n)\s*["']direct["']\s*:\s*\[([\s\S]*?)\]/u);
  const body = named?.[1] ?? keyed?.[1];
  if (!body) return [];
  const domains = [];
  for (const hit of body.matchAll(/['"]([a-z0-9.-]+)['"]/giu)) {
    domains.push(normalizeDomain(hit[1]));
  }
  return uniqueSorted(domains.filter(Boolean));
}

export function extractHostsFromHtml(text) {
  const domains = [];
  for (const hit of text.matchAll(
    /https?:\/\/(?:www\.)?([a-z0-9-]+(?:\.[a-z0-9-]+)+)/giu,
  )) {
    domains.push(normalizeDomain(hit[1]));
  }
  return uniqueSorted(domains.filter(Boolean));
}

export function extractCidrsFromText(text) {
  const cidrs = [];
  for (const line of parseProviderLines(text)) {
    const value = line.replace(/^-\s+/u, "").split(/\s+/u)[0];
    if (!value.includes("/") && !isIP(value)) continue;
    try {
      cidrs.push(parseCidr(value).cidr);
    } catch {
      // Skip prose that is not a prefix.
    }
  }
  return uniqueSorted(cidrs);
}

export function extractRipestatPrefixes(payload) {
  const resources = payload?.data?.resources ?? {};
  const v4 = Array.isArray(resources.ipv4) ? resources.ipv4 : [];
  const v6 = Array.isArray(resources.ipv6) ? resources.ipv6 : [];
  return uniqueSorted(
    [...v4, ...v6]
      .map((value) => String(value).trim())
      .filter(Boolean)
      .map((value) => parseCidr(value).cidr),
  );
}

function ipToBigInt(addr, version) {
  if (version === 4) {
    return addr
      .split(".")
      .reduce((sum, octet) => (sum << 8n) + BigInt(octet), 0n);
  }
  const hex = expandIpv6(addr);
  return BigInt(`0x${hex}`);
}

function expandIpv6(addr) {
  const [head, tail] = addr.split("::");
  const headParts = head ? head.split(":") : [];
  const tailParts = tail ? tail.split(":") : [];
  const missing = 8 - headParts.length - tailParts.length;
  const parts = [
    ...headParts,
    ...Array.from({ length: Math.max(missing, 0) }, () => "0"),
    ...tailParts,
  ].map((part) => part.padStart(4, "0"));
  if (parts.length !== 8) {
    throw new Error(`malformed IPv6: ${addr}`);
  }
  return parts.join("");
}

function uniqueSorted(values) {
  return [...new Set(values)].sort((left, right) => left.localeCompare(right));
}

function loadSuffixes(fileName) {
  const text = readFileSync(join(rulesDir, fileName), "utf8");
  return uniqueSorted(
    parseProviderLines(text).map((line) => normalizeDomain(line)),
  );
}

function loadCidrs(fileName) {
  const text = readFileSync(join(rulesDir, fileName), "utf8");
  return parseProviderLines(text).map((line) => parseCidr(line));
}

async function fetchText(url) {
  const response = await fetch(url, {
    headers: { "user-agent": USER_AGENT, accept: "*/*" },
    redirect: "follow",
    signal: AbortSignal.timeout(60_000),
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return {
    text: await response.text(),
    finalUrl: response.url,
    contentType: response.headers.get("content-type") ?? "",
  };
}

async function fetchJson(url) {
  const fetched = await fetchText(url);
  return { ...fetched, json: JSON.parse(fetched.text) };
}

const HTML_NOISE_HOSTS = new Set([
  "cloudflare.com",
  "facebook.com",
  "github.com",
  "google.com",
  "googleapis.com",
  "googletagmanager.com",
  "gstatic.com",
  "instagram.com",
  "jsdelivr.net",
  "linkedin.com",
  "schema.org",
  "tehranindex.com",
  "twitter.com",
  "vercel.app",
  "w3.org",
  "wikipedia.org",
  "x.com",
  "youtube.com",
]);

function isHtmlNoiseHost(host) {
  return [...HTML_NOISE_HOSTS].some(
    (suffix) => host === suffix || host.endsWith(`.${suffix}`),
  );
}

export async function researchIranCoverage({
  fetchDomain = fetchText,
  fetchIp = fetchText,
  fetchRipestat = fetchJson,
} = {}) {
  const iranDomains = loadSuffixes("iran-domains.txt");
  const businessDomains = loadSuffixes("iran-business-domains.txt");
  const coveredSuffixes = uniqueSorted([...iranDomains, ...businessDomains]);
  const iranCidrs = loadCidrs("iran-networks.txt");

  const domainReports = [];
  const missingDomains = new Map();
  for (const source of DOMAIN_SOURCES) {
    const report = {
      id: source.id,
      url: source.url,
      note: source.note,
      fetched: 0,
      covered: 0,
      missing: 0,
      error: null,
    };
    try {
      const fetched = await fetchDomain(source.url);
      report.finalUrl = fetched.finalUrl;
      let hosts = [];
      if (source.kind === "clash_payload") {
        hosts = extractDomainsFromClashPayload(fetched.text);
      } else if (source.kind === "python_direct_set") {
        hosts = extractDirectSetFromPython(fetched.text);
      } else {
        hosts = extractHostsFromHtml(fetched.text).filter(
          (host) => !isHtmlNoiseHost(host),
        );
      }
      report.fetched = hosts.length;
      for (const host of hosts) {
        if (domainCoveredBySuffixes(host, coveredSuffixes)) {
          report.covered += 1;
          continue;
        }
        report.missing += 1;
        const current = missingDomains.get(host) ?? [];
        if (!current.includes(source.id)) current.push(source.id);
        missingDomains.set(host, current);
      }
    } catch (error) {
      report.error = error instanceof Error ? error.message : String(error);
    }
    domainReports.push(report);
  }

  const ipReports = [];
  const missingCidrs = new Map();
  for (const source of IP_SOURCES) {
    const report = {
      id: source.id,
      url: source.url,
      publisher: source.publisher,
      note: source.note,
      fetched: 0,
      covered: 0,
      missing: 0,
      error: null,
    };
    try {
      let prefixes = [];
      if (source.kind === "ripestat") {
        const fetched = await fetchRipestat(source.url);
        report.finalUrl = fetched.finalUrl;
        prefixes = extractRipestatPrefixes(fetched.json);
      } else {
        const fetched = await fetchIp(source.url);
        report.finalUrl = fetched.finalUrl;
        prefixes = extractCidrsFromText(fetched.text);
      }
      report.fetched = prefixes.length;
      for (const prefix of prefixes) {
        if (isCoveredByCidrs(prefix, iranCidrs)) {
          report.covered += 1;
          continue;
        }
        report.missing += 1;
        const current = missingCidrs.get(prefix) ?? [];
        if (!current.includes(source.id)) current.push(source.id);
        missingCidrs.set(prefix, current);
      }
    } catch (error) {
      report.error = error instanceof Error ? error.message : String(error);
    }
    ipReports.push(report);
  }

  const domainLines = [...missingDomains.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([domain, sources]) => `${domain}\t${sources.join(",")}`);
  const cidrLines = [...missingCidrs.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([cidr, sources]) => `${cidr}\t${sources.join(",")}`);

  return {
    snapshot: {
      iran_domains: iranDomains.length,
      business_domains: businessDomains.length,
      iran_networks: iranCidrs.length,
    },
    domainReports,
    ipReports,
    missingDomains: domainLines,
    missingCidrs: cidrLines,
  };
}

function renderReport(result) {
  const domainRows = result.domainReports
    .map((item) => {
      const status = item.error
        ? `error: ${item.error}`
        : `${item.fetched} fetched / ${item.covered} covered / ${item.missing} missing`;
      return `- \`${item.id}\` — ${item.url}\n  ${item.note}\n  ${status}`;
    })
    .join("\n");
  const ipRows = result.ipReports
    .map((item) => {
      const status = item.error
        ? `error: ${item.error}`
        : `${item.fetched} fetched / ${item.covered} covered / ${item.missing} missing`;
      return `- \`${item.id}\` (${item.publisher}) — ${item.url}\n  ${item.note}\n  ${status}`;
    })
    .join("\n");
  return `# Iran DIRECT coverage research

Generated by \`scripts/research-iran-coverage.mjs\`. This dump is a review
input, not a shipping list. Do not invent entries. Do not merge extras into
Chocolate4U snapshots.

## Current snapshot

- \`iran-domains.txt\`: ${result.snapshot.iran_domains.toLocaleString("en-US")} suffix rules (\`+.ir\` already covers every \`.ir\` name)
- \`iran-business-domains.txt\`: ${result.snapshot.business_domains.toLocaleString("en-US")} curated roots
- \`iran-networks.txt\`: ${result.snapshot.iran_networks.toLocaleString("en-US")} CIDRs

## Domain sources

${domainRows}

**New domain candidates:** ${result.missingDomains.length.toLocaleString("en-US")} (see \`domains-missing.txt\`)

Ship only first-party, non-\`.ir\`, non-CDN roots whose ownership is clear
(ADR 0054). Shared analytics, Cloudflare/Google, and unclear tenants stay out.

## IP sources

${ipRows}

**New CIDR candidates:** ${result.missingCidrs.length.toLocaleString("en-US")} (see \`cidrs-missing.txt\`)

CIDR comparison uses containment, not exact-string match. RIPEstat IR prefixes
are a gap report: only add a prefix to the curated CDN file when it is missing,
not already contained, and published on a first-party CDN/cloud page. Do not
union the entire IR RIR table.
`;
}

function isDirectRun() {
  return Boolean(
    process.argv[1] &&
      import.meta.url === pathToFileURL(resolve(process.argv[1])).href,
  );
}

export async function main() {
  const result = await researchIranCoverage();
  mkdirSync(outDir, { recursive: true });
  writeFileSync(
    join(outDir, "domains-missing.txt"),
    `${result.missingDomains.join("\n")}${result.missingDomains.length ? "\n" : ""}`,
  );
  writeFileSync(
    join(outDir, "cidrs-missing.txt"),
    `${result.missingCidrs.join("\n")}${result.missingCidrs.length ? "\n" : ""}`,
  );
  writeFileSync(join(outDir, "REPORT.md"), renderReport(result));
  process.stdout.write(
    `wrote ${result.missingDomains.length} missing domains and ${result.missingCidrs.length} missing CIDRs to ${outDir}\n`,
  );
}

if (isDirectRun()) {
  await main();
}
