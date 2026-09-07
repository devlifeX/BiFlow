#!/usr/bin/env node
import assert from "node:assert/strict";
import {
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { isIP } from "node:net";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const rulesDir = join(root, "resources/rules");
const manifestPath = join(rulesDir, "manifest.json");
const snapshotPath = join(rulesDir, "SNAPSHOT.md");
const repository = "Chocolate4U/Iran-clash-rules";
const branch = "release";
export const UPSTREAM_LICENSE = "GPL-3.0";
export const RUNTIME_REPOSITORY = "devlifeX/BiFlow";
export const MAX_DELTA_RATIO = 0.25;
const catalog = [
  {
    id: "iran-domains",
    localName: "iran-domains.txt",
    remoteName: "ir.txt",
    kind: "domain",
    minimumEntries: 1_000,
  },
  {
    id: "iran-networks",
    localName: "iran-networks.txt",
    remoteName: "ircidr.txt",
    kind: "ip_cidr",
    minimumEntries: 100,
  },
  {
    id: "private",
    localName: "private.txt",
    remoteName: "private.txt",
    kind: "ip_cidr",
    minimumEntries: 5,
  },
];

const curatedCatalog = [
  {
    id: "iran-business-domains",
    localName: "iran-business-domains.txt",
    kind: "domain",
    minimumEntries: 10,
    source: "curated",
    sourcesFile: "iran-business-domains.sources.json",
  },
  {
    id: "iran-cdn-networks",
    localName: "iran-cdn-networks.txt",
    kind: "ip_cidr",
    minimumEntries: 1,
    source: "curated",
    sourcesFile: "iran-cdn-networks.sources.json",
  },
];

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function entryCount(bytes) {
  return bytes
    .toString("utf8")
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter(
      (line) => line.length > 0 && !line.startsWith("#") && line !== "payload:",
    ).length;
}

export function parseProviderLine(kind, line) {
  const value = line.trim();
  if (!value) {
    throw new Error("empty provider line");
  }
  if (/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/.test(value)) {
    throw new Error("unsafe control character in provider line");
  }
  if (
    /\s/u.test(value) ||
    /https?:\/\//iu.test(value) ||
    value.includes("\\")
  ) {
    throw new Error(`unsafe provider line: ${value.slice(0, 80)}`);
  }
  if (kind === "domain") {
    const normalized = value.toLowerCase();
    if (
      !/^(?:\+\.)?[a-z0-9._*-]+$/u.test(normalized) ||
      normalized.includes("..")
    ) {
      throw new Error(`malformed domain: ${value.slice(0, 80)}`);
    }
    return normalized;
  }
  const slash = value.lastIndexOf("/");
  const addr = slash === -1 ? value : value.slice(0, slash);
  const prefix = slash === -1 ? null : value.slice(slash + 1);
  const version = isIP(addr);
  if (!version) {
    throw new Error(`malformed CIDR: ${value.slice(0, 80)}`);
  }
  if (prefix !== null) {
    const bits = Number(prefix);
    const max = version === 4 ? 32 : 128;
    if (!Number.isInteger(bits) || bits < 0 || bits > max) {
      throw new Error(`malformed CIDR prefix: ${value.slice(0, 80)}`);
    }
  }
  return value;
}

export function normalizeProviderBytes(kind, bytes) {
  const text = bytes
    .toString("utf8")
    .replace(/\r\n/gu, "\n")
    .replace(/\r/gu, "\n");
  const comments = [];
  const unique = [];
  const seen = new Set();
  let inBody = false;
  for (const raw of text.split("\n")) {
    if (/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/.test(raw)) {
      throw new Error("unsafe control character in provider file");
    }
    const line = raw.trim();
    if (!inBody && (line.length === 0 || line.startsWith("#"))) {
      if (line.length > 0) comments.push(line);
      continue;
    }
    inBody = true;
    if (line.length === 0 || line.startsWith("#") || line === "payload:") {
      continue;
    }
    const parsed = parseProviderLine(kind, line);
    if (seen.has(parsed)) continue;
    seen.add(parsed);
    unique.push(parsed);
  }
  const header = comments.length > 0 ? `${comments.join("\n")}\n` : "";
  return Buffer.from(`${header}${unique.join("\n")}\n`, "utf8");
}

export function assertCountDelta(previous, next, ratio = MAX_DELTA_RATIO) {
  if (!previous) return;
  const prior = previous.rules.find((rule) => rule.id === next.id);
  if (!prior) return;
  const allowed = Math.max(1, Math.round(prior.entry_count * ratio));
  const delta = Math.abs(next.entry_count - prior.entry_count);
  if (delta > allowed) {
    throw new Error(
      `${next.file} changed by ${delta} entries from ${prior.entry_count}; max allowed is ${allowed} (${ratio * 100}%)`,
    );
  }
}

export function renderSnapshotMarkdown(manifest) {
  const rows = manifest.rules
    .map(
      (rule) =>
        `| \`${rule.file}\` | ${rule.entry_count.toLocaleString("en-US")} | \`${rule.sha256}\` |`,
    )
    .join("\n");
  const fetched = String(manifest.fetched_at).slice(0, 10);
  return `# Offline rule snapshot

These immutable installer inputs were downloaded on ${fetched} from upstream
commit \`${manifest.commit}\` in [${manifest.repository}](https://github.com/${manifest.repository})
(\`${manifest.license}\`). Installed clients refresh only from
[${manifest.runtime_repository}](https://github.com/${manifest.runtime_repository})
(\`resources/rules/manifest.json\`, then the files in that same commit).
Upstream hosts stay in this snapshot for maintainer provenance and are not
runtime fallbacks.

| File | Lines | SHA-256 |
| --- | ---: | --- |
${rows}

The installed copy is never modified. Live refreshes are validated and written
to the application data directory, and a failed refresh keeps the last known
good cache. Lines beginning with \`#\` are metadata. Domain entries may use the
Mihomo text-provider \`+.\` suffix form.

Run \`pnpm rules:update\` (\`./scripts/update-rules.sh\`) to create a fresh
single-commit snapshot. The script does not commit or push. The generated
\`manifest.json\` is authoritative; \`pnpm rules:check\` validates its hashes and
minimum entry counts without accessing the network.

Bundled rule files use LF bytes only. Root \`.gitattributes\` marks
\`resources/rules/*\` as \`-text\` so Windows Git checkout does not rewrite CRLF and
break SHA-256 verification during \`bundle:check\`.

Curated files (\`source: "curated"\`) are BiFlow-owned DIRECT catalogs, not
Chocolate4U. Cloud refresh must not overwrite them. Provenance lives in the
matching \`*.sources.json\`. Extra CIDRs need containment-diff against
\`iran-networks.txt\` plus a first-party or Iranian-ASN source; do not union the
IR RIR table.
`;
}

function validateEntry(entry, bytes) {
  const count = entryCount(bytes);
  assert.ok(
    count >= entry.minimumEntries,
    `${entry.localName} has ${count} entries; expected at least ${entry.minimumEntries}`,
  );
  return count;
}

export function readManifest() {
  return JSON.parse(readFileSync(manifestPath, "utf8"));
}

export function checkSnapshot() {
  const manifest = readManifest();
  assert.equal(manifest.schema_version, 1);
  assert.equal(manifest.repository, repository);
  assert.equal(manifest.license, UPSTREAM_LICENSE);
  assert.equal(manifest.runtime_repository, RUNTIME_REPOSITORY);
  assert.match(manifest.commit, /^[0-9a-f]{40}$/u);
  const upstreamRules = manifest.rules.filter((rule) =>
    catalog.some((entry) => entry.id === rule.id),
  );
  assert.equal(upstreamRules.length, catalog.length);

  const snapshotMd = readFileSync(snapshotPath, "utf8");
  assert.match(snapshotMd, /devlifeX\/BiFlow/);
  assert.match(snapshotMd, new RegExp(manifest.commit, "u"));
  assert.doesNotMatch(snapshotMd, /Live updates come from\s*\n\[Chocolate4U/u);

  for (const entry of catalog) {
    const recorded = manifest.rules.find((rule) => rule.id === entry.id);
    assert.ok(recorded, `manifest is missing ${entry.id}`);
    assert.equal(recorded.file, entry.localName);
    assert.equal(recorded.upstream_file, entry.remoteName);
    assert.equal(recorded.kind, entry.kind);
    const bytes = readFileSync(join(rulesDir, entry.localName));
    assert.equal(recorded.sha256, sha256(bytes));
    assert.equal(recorded.entry_count, validateEntry(entry, bytes));
    assert.match(snapshotMd, new RegExp(recorded.sha256, "u"));
  }
  checkCuratedCatalog(manifest, snapshotMd);
  process.stdout.write(
    `verified ${catalog.length} bundled rule sets from ${manifest.commit.slice(0, 12)}\n`,
  );
}

export function checkCuratedCatalog(manifest, snapshotMd) {
  for (const entry of curatedCatalog) {
    const recorded = manifest.rules.find((rule) => rule.id === entry.id);
    assert.ok(recorded, `manifest is missing curated ${entry.id}`);
    assert.equal(recorded.file, entry.localName);
    assert.equal(recorded.kind, entry.kind);
    assert.equal(recorded.source, "curated");
    assert.notEqual(recorded.upstream_file, recorded.file);
    const bytes = readFileSync(join(rulesDir, entry.localName));
    assert.equal(recorded.sha256, sha256(bytes));
    assert.equal(recorded.entry_count, validateEntry(entry, bytes));
    assert.match(snapshotMd, new RegExp(recorded.sha256, "u"));
    const sources = JSON.parse(
      readFileSync(join(rulesDir, entry.sourcesFile), "utf8"),
    );
    assert.equal(sources.source, "curated");
    if (entry.kind === "domain") {
      const domains = bytes
        .toString("utf8")
        .split(/\n/u)
        .map((line) => line.trim())
        .filter((line) => line.startsWith("+."))
        .map((line) => line.slice(2));
      assert.equal(sources.entries.length, domains.length);
      for (const item of sources.entries) {
        assert.ok(domains.includes(item.domain), `missing ${item.domain}`);
        assert.ok(
          !item.domain.endsWith(".ir"),
          `${item.domain} is already +.ir`,
        );
        assert.ok(item.official_url, `${item.domain} needs official_url`);
        assert.ok(item.verified_at, `${item.domain} needs verified_at`);
        assert.ok(item.status, `${item.domain} needs status`);
      }
      continue;
    }
    const prefixes = bytes
      .toString("utf8")
      .split(/\n/u)
      .map((line) => line.trim())
      .filter((line) => line.length > 0 && !line.startsWith("#"));
    assert.equal(sources.entries.length, prefixes.length);
    for (const item of sources.entries) {
      assert.ok(prefixes.includes(item.prefix), `missing ${item.prefix}`);
      assert.ok(item.publisher, `${item.prefix} needs publisher`);
      assert.ok(item.official_url, `${item.prefix} needs official_url`);
      assert.ok(item.verified_at, `${item.prefix} needs verified_at`);
    }
  }
}

async function fetchBytes(url) {
  const response = await fetch(url, {
    headers: { "user-agent": "BiFlow rule snapshot builder" },
    redirect: "follow",
    signal: AbortSignal.timeout(60_000),
  });
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}`);
  }
  return Buffer.from(await response.arrayBuffer());
}

async function resolveCommit() {
  const bytes = await fetchBytes(
    `https://api.github.com/repos/${repository}/commits/${branch}`,
  );
  const response = JSON.parse(bytes.toString("utf8"));
  assert.match(response.sha, /^[0-9a-f]{40}$/u);
  return response.sha;
}

async function downloadRule(entry, commit) {
  const urls = [
    `https://raw.githubusercontent.com/${repository}/${commit}/${entry.remoteName}`,
    `https://cdn.jsdelivr.net/gh/${repository}@${commit}/${entry.remoteName}`,
    `https://fastly.jsdelivr.net/gh/${repository}@${commit}/${entry.remoteName}`,
  ];
  let lastError;
  for (const url of urls) {
    try {
      return { bytes: await fetchBytes(url), source: url };
    } catch (error) {
      lastError = error;
    }
  }
  throw lastError;
}

async function updateSnapshot() {
  const previous = readManifest();
  const commit = await resolveCommit();
  const temporary = mkdtempSync(join(tmpdir(), "biflow-rules-"));
  try {
    const rules = [];
    for (const entry of catalog) {
      const { bytes, source } = await downloadRule(entry, commit);
      const normalized = normalizeProviderBytes(entry.kind, bytes);
      const count = validateEntry(entry, normalized);
      const next = {
        id: entry.id,
        file: entry.localName,
        upstream_file: entry.remoteName,
        kind: entry.kind,
        entry_count: count,
        sha256: sha256(normalized),
        source,
      };
      assertCountDelta(previous, next);
      writeFileSync(join(temporary, entry.localName), normalized);
      rules.push(next);
    }
    const curated = previous.rules.filter(
      (rule) =>
        rule.source === "curated" ||
        curatedCatalog.some((entry) => entry.id === rule.id),
    );
    rules.push(...curated);
    const manifest = {
      schema_version: 1,
      repository,
      runtime_repository: RUNTIME_REPOSITORY,
      license: UPSTREAM_LICENSE,
      branch,
      commit,
      fetched_at: new Date().toISOString(),
      rules,
    };
    writeFileSync(
      join(temporary, basename(manifestPath)),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );
    writeFileSync(
      join(temporary, "SNAPSHOT.md"),
      renderSnapshotMarkdown(manifest),
    );
    for (const entry of catalog) {
      renameSync(
        join(temporary, entry.localName),
        join(rulesDir, entry.localName),
      );
    }
    renameSync(join(temporary, basename(manifestPath)), manifestPath);
    renameSync(join(temporary, "SNAPSHOT.md"), snapshotPath);
  } finally {
    rmSync(temporary, { force: true, recursive: true });
  }
  checkSnapshot();
}

function isDirectRun() {
  return Boolean(
    process.argv[1] &&
      import.meta.url === pathToFileURL(resolve(process.argv[1])).href,
  );
}

export async function main(argv = process.argv) {
  if (argv.includes("--check")) {
    checkSnapshot();
    return;
  }
  await updateSnapshot();
}

if (isDirectRun()) {
  await main();
}
