# ADR 0094: Remember a side-tunnel address without editing the profile

- Status: Accepted
- Date: 2026-09-25

## Context

Windscribe and OpenVPN profiles name a server that filtered DNS answers with
an unroutable address. Looking that name up through Hiddify finds the real
address, but writing it back into the imported `.ovpn` would change a file
the operator may replace, and the next start would still need Hiddify if the
lookup was not kept.

Linux and Windows must use the same lookup. A platform-specific copy drifted
the first time this was added.

## Decision

- Read the profile. Never write the resolved address into it.
- On a successful lookup, store host, port, and address in
  `side-tunnel-remotes.json` under the user data directory.
- The next start uses that store when Hiddify is not running. A failed lookup
  while Hiddify is up also falls back to the store.
- Poisoned answers (`10.0.0.0/8`, fake-ip, and the rest of the existing
  routable-public check) are not stored and are ignored if already stored.
- `pin_profile_remote` in `iran-split-clients` is the only implementation.
  Both platform backends call it.

## Consequences

One successful connect with Hiddify is enough for later connects without it.
Replacing the `.ovpn` does not erase the store; a new hostname is looked up
again the next time a client can resolve it.
