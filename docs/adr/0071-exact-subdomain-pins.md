# ADR 0071: Exact subdomain pins with longest-match precedence

## Status

Accepted (supersedes the root-collapse pin rule in ADR 0034)

## Context

Pins used to collapse to the registrable root: typing
`developer.google.com` stored `google.com`, so one root pin owned the whole
tree and a specific subdomain could never live in a different list. Users
want DNS-style behaviour: `google.com` in one list covers every subdomain,
while `developer.google.com` in another list carves out its own subtree.

## Decision

- `canonical_target` keeps the exact (IDNA-normalized) name typed. Bare
  public suffixes (`co.uk`, `github.io`) are still rejected via the
  registrable-root check.
- A pin covers itself and all of its subdomains. Two pins on the same tree
  (`google.com`, `developer.google.com`) may coexist in different lists
  with different outbounds.
- **Longest match wins everywhere.** Generation emits user domain pins as
  inline `DOMAIN-SUFFIX` rules ordered by label count (descending), so
  Mihomo's first-match evaluation gives the most specific pin priority
  across lists and outbounds. `RuleSet::decide` (test-route) and the mock
  use the same rule.
- Domain rule-set references (`RULE-SET,custom-*-domains`) are gone from
  the rules chain; IP pins keep their per-outbound rule-set files and
  positions. The `custom-direct-domains` provider file is still written and
  declared because `fake-ip-filter` needs it (ADR 0058).
- Pins of disabled clients stay unemitted; fail-closed (ADR 0070) applies
  per pin through the same target resolution.

## Consequences

- User pin counts are small, so inline rules add negligible config size and
  restore live-reload semantics unchanged (the whole generation is re-staged
  on apply).
- Existing documents keep working: previously stored roots are already
  valid exact pins.
