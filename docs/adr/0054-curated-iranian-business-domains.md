# ADR 0054: Curated Iranian business domains

## Status

Accepted

## Context

The Chocolate4U snapshot is GPL-licensed and Iran-centric, but several active
Iranian businesses publish on non-`.ir` domains that are missing from
`iran-domains.txt`. Mixing a hand-curated list into that upstream file would
erase provenance on the next `pnpm rules:update`.

## Decision

- Ship `resources/rules/iran-business-domains.txt` (`+.domain` lines) and
  `iran-business-domains.sources.json` (`domain`, `category`, `official_url`,
  `discovered_from`, `verified_at`, `status`) as a **curated** provider.
- Record it in `manifest.json` with `source: "curated"`. `scripts/sync-rules.mjs`
  validates hash, count, uniqueness, and metadata offline. `updateSnapshot`
  preserves curated rows and must not fetch or overwrite the catalog from
  Chocolate4U.
- CloudRuleStore still fetches only the three upstream catalog files. It never
  writes `direct-rules.json` or the curated provider. The file is embedded and
  copied into every runtime generation (helper `GENERATION_FILES`, both
  platform backends, Mihomo `RULE-SET,iran-business-domains,DIRECT` after
  `iran-domains` and before `MATCH`).
- Do not add `.ir` names (already covered by `+.ir`), shared CDNs, public
  analytics, or domains whose first-party ownership is unclear.
- Companion curated IP prefixes live in `iran-cdn-networks.txt` (ADR 0084)
  so Chocolate4U refreshes cannot wipe them either.
- Removing or changing more than 25% of curated entries in one snapshot
  requires the same review gate as upstream count deltas.

## Consequences

- Wave-1 domains (technolife, azkivam, and the rest of the considering list)
  plus later first-party roots such as `kavenegar.com` and the Tehran Index
  `/companies` harvest (ADR 0085) are DIRECT in mock and runtime
  `test_route` without user pins.
- A later cloud refresh cannot silently drop the catalog.
