# 0084 Curated Iranian CDN/ISP CIDRs

## Status

Accepted

## Context

`iran-networks.txt` is the Chocolate4U `ircidr.txt` snapshot. Hand-editing it
would be wiped by `pnpm rules:update` and would break SHA-256 provenance
(ADR 0022 / 0054). Arvancloud's published
[`ips.txt`](https://www.arvancloud.ir/en/ips.txt) is already contained in that
snapshot. RIPEstat's IR country-resource-list still reports prefixes that
Chocolate4U missed, but many of those leftovers are unannounced or originated
by LeaseWeb, OVH, or other non-Iranian networks — unioning them would
false-DIRECT off-Iran traffic.

## Decision

- Ship extras as a **curated** IP provider: `iran-cdn-networks.txt` plus
  `iran-cdn-networks.sources.json` (`prefix`, `publisher`, `official_url`,
  `verified_at`). Record it in `manifest.json` with `source: "curated"`.
  `updateSnapshot` preserves curated rows and must not fetch them from
  Chocolate4U.
- CloudRuleStore still fetches only the three upstream catalog files. The
  curated IP file is embedded, copied into every runtime generation, and
  emitted as `RULE-SET,iran-cdn-networks,DIRECT,no-resolve` next to
  `iran-networks`. Do not put CIDRs in `fake-ip-filter`.
- CIDR research uses **containment**, not exact-string match. RIPEstat IR
  gaps stay in `docs/considering/iran-coverage-out/` unless they are missing,
  not already contained, and announced by an Iranian ASN (or published on a
  first-party CDN page). Do not union the entire IR RIR table.
- `test_route` / CLI `RuleSet::decide` fold the extra prefixes into
  `DecisionReason::IranCidr`. A client IP pin still wins.

## Consequences

- Wave-1 Mobinnet `/22` leftovers (AS50810) are DIRECT without user pins.
- A later cloud refresh cannot silently drop the catalog.
- `scripts/research-iran-coverage.mjs` regenerates the review dump from
  official sources so later waves do not invent entries.
