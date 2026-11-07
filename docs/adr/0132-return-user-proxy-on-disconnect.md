# ADR 0132: Return the original live local proxy after releasing owned routing

Status: Accepted (6.2.83)

## Context

A Windows report showed internet access and HTTPS failing after Disconnect,
then working again after Connect. The saved baseline contained an enabled
Hiddify proxy. Disconnect removed the owned TUN and disabled the desktop proxy
but never returned that baseline. Network access that had depended on the
user's existing proxy therefore stayed disabled. This establishes a handoff
defect; it does not identify every earlier browser certificate error.

ADR 0120 correctly requires removing a dead loopback proxy. Removing every
previously working proxy indefinitely is a different behavior. Also, restoring
on Resume would bypass the active TUN and break split routing (ADR 0062).

## Decision

- Capture the original managed proxy before starting any clients. Persist a
  disabled marker when the original settings are not a managed local proxy,
  so a client launched by BiFlow cannot become the apparent original owner.
  Repeated clears retain the first baseline. Publish it atomically through a
  private temporary file; keep the record if publication or restoration fails.
- On Pause, Disconnect, rollback, and startup reconciliation, first verify that
  the owned capture is gone. Then restore the baseline only if its configured
  loopback endpoints actually accept TCP connections and settings still match
  the state left by BiFlow. Do not replace intervening proxy or PAC choices.
  Never restore a proxy while the TUN is running.
- Leave a borrowed Hiddify client running when the baseline shows that the user
  was already using it. The stop-with-stack option applies to clients owned by
  BiFlow. Never re-enable a dead proxy or take ownership of a corporate server.
- Retire the baseline after a verified restore or a deliberate safe skip.
  Restoration failures retain it for retry. Quit without taking over the
  network must not clear an unrelated working user proxy.
- On macOS, clear only proxy kinds and network services pointing at the managed
  endpoint; preserve other services' settings during the connected session.
- Emit structured baseline and handoff events without raw settings. Diagnostics
  reports `saved_user_proxy_handoff_pending` for an idle stack with a saved
  baseline and disabled system proxies.

## Validation

Core tests cover returning a user proxy after detaching capture, restoring on
Pause but clearing on Resume, cancellation/rollback, and unused Quit. Shared
policy tests cover live/dead sockets, concurrent edits, and unmanaged endpoints.
Each native backend tests baseline preservation and its settings comparison;
macOS also tests preserving an unrelated service. The existing primary-flow
e2e asserts connectivity after Pause and Disconnect.

The user's current connection stays running during development. Build and CI
validation do not replace a live disconnect acceptance test with the new
installed version. No certificate-validation settings are weakened.
