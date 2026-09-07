# 0085 Tehran Index company directory harvest

## Status

Accepted

## Context

Wave-1/2 curated DIRECT roots came from four Tehran Index sector pages
(ecommerce, fintech, crypto, cloud). Those HTML pages under-count the
registry: Tehran Index publishes 23 sectors and a `/companies` directory
with hundreds of records. Several first-party non-`.ir` sites (Ketabrah,
Sibche, Golrang System, IFA Crowd, …) never appeared in the four-sector
scrape.

## Decision

- `scripts/research-iran-coverage.mjs` fetches
  `https://tehranindex.com/companies` plus the remaining live sector
  slugs. Sector HTML stays as attribution; the company directory is the
  coverage harvest.
- Ship only first-party, non-`.ir` hosts whose official pages verify.
  TLS-dead, 500, 404, inactive (`hoshno.com`), or ownership-unclear
  leftovers stay in `docs/considering/iran-coverage-out/` and are not
  invented into Chocolate4U files (ADR 0054 / 0084).
- Append verified roots to `iran-business-domains.txt` with matching
  `sources.json` rows. `pnpm rules:update` still cannot wipe them.

## Consequences

- Mock and runtime `test_route` treat the new catalog roots as DIRECT
  without user pins.
- A later research run against `/companies` should not rediscover the
  shipped names as gaps.
