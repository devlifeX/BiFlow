# 0086 Pinning google.com to a client also pins Search companions

## Status

Accepted

## Context

Operators pin `google.com` to Windscribe so Google Search shows the full
(AI) experience. Iranian DIRECT and typical Hiddify exits serve the
restricted “basic” Google page.

`DOMAIN-SUFFIX,google.com` covers `www.google.com` and
`gemini.google.com`, but Search still loads scripts and APIs from
`gstatic.com`, `googleapis.com`, `googleusercontent.com`, and
`googletagmanager.com`. Those stay on `MATCH` (Hiddify). Live traffic
then looks split: `safebrowsing.google.com` on Windscribe, `google.com`
on Hiddify (often a stale MATCH socket), and the page stays basic.

A PUT `/configs` relative-path 400 (ADR 0083) also left old
`google.com` sockets on Hiddify after the pin was stored.

## Decision

- Pinning the exact name `google.com` to a **client** also pins the
  Search companion registrable roots onto the same list, unless a host
  is already pinned (so a DIRECT `gstatic.com` is not stolen).
- Loading an existing document with a `google.com` client pin fills any
  missing companions (revision bump).
- Pinning `google.com` to DIRECT does not add companions.
- Subdomain pins (`developer.google.com`) stay surgical.
- After a live apply, close connections for `google.com` and the
  companions so stale MATCH sockets reconnect on the new outbound.

## Consequences

The Windscribe (or other client) list grows by those four names when
`google.com` is pinned. Other sites that share `gstatic.com` then follow
that client too. Chrome Secure DNS can still bypass Mihomo fake-ip and
land `google.com` on MATCH; use system DNS (BiFlow) for this split.
