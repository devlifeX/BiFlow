# ADR 0034: Bidirectional route pins

## Status

Accepted

## Context

User rules were additive DIRECT only. `DirectRulesDocument` could pin a host to
DIRECT, and `DecisionReason` had no force-VPN case, so "Add to VPN" could only
call `removeRule`. For a host the bundled Iran list keeps direct — `iran.ir`
matching `ir` — there is no user rule to remove, so the action was a silent
no-op and the UI had to explain that exclusions were unsupported.

## Decision

- A user pin is a **registrable domain** (eTLD+1, including the PSL private
  section) or a literal IP. `api.shop.example.com` stores `example.com` and
  matches every subdomain; `user.github.io` stays a tenant and cannot pin all
  of `github.io`. `notexample.com` does not match `example.com`.
- Writers emit `+.example.com` in custom domain providers and never copy
  `resolved_ips` into IP providers (CDN addresses must not leak onto DIRECT).
- `RuleManager::load` migrates exact-host documents to canonical roots, merges
  duplicates, bumps `revision` once, and keeps `direct-rules.json.last-good`.
- Schema 3 stores one list: `RoutePinsDocument.pins` with
  `Outbound::Direct | Client { client_id }`. Legacy `rules` / `vpn_rules`
  migrate through `RuleManager::load_with_legacy`.
- A host lives in at most one pin. `RuleManager::pin(input, outbound,
revision)` moves it. `add` stays as the DIRECT shorthand.
- Precedence, identical in `RuleSet::decide` and the generated Mihomo rules:
  1. process bypass, localhost, private/LAN/CGNAT → DIRECT
  2. enabled client pins
  3. user DIRECT pins
  4. bundled Iran domains and CIDRs → DIRECT
  5. curated `iran-business-domains` and `iran-cdn-networks` → DIRECT
  6. `MATCH` → `default_route` (a client group or DIRECT)
- Private, loopback, and CGNAT addresses are rejected from the VPN list with a
  clear error rather than silently ignored: routing them through the tunnel
  cuts the machine off from its own network. `decide_ip` checks that range
  first, mirroring `private-networks` staying ahead of the VPN rule sets.
- Two providers, `custom-vpn-domains.txt` and `custom-vpn-ips.txt`, are written
  on every generation even when empty, registered in `providers()`, and listed
  in `OPTIONAL_RULE_PROVIDERS` so an empty file counts as ready. Both platform
  backends write them and the helper allowlists them in `GENERATION_FILES`.
- `refresh()` re-resolves domains on both lists.
- The Diagnostics flow card offers the opposite move for every host except
  private/local, and re-tests after the change so the card shows the outbound
  that now applies. The Direct rules page lists both sets with a DIRECT/VPN
  badge and moves entries either way.

## Consequences

- The two staging helpers now emit nine generation files (the eighth pair is
  custom VPN providers; the ninth is the curated Iranian business catalog);
  the script contract test pins that list for both platforms.
- A pinned registrable domain covers its subdomains. Removing a VPN pin restores
  whatever the bundled list and catalog decide.
- The mock API implements the same precedence and mutual exclusivity, so the
  e2e flow exercises the real ordering rather than a simplified one.
