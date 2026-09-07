# Offline rule snapshot

These immutable installer inputs were downloaded on 2026-08-12 from upstream
commit `767ef8bf56739c436f72e7489cc86b5f79a926e6` in
[Chocolate4U/Iran-clash-rules](https://github.com/Chocolate4U/Iran-clash-rules)
(`GPL-3.0`). Installed clients refresh only from
[devlifeX/BiFlow](https://github.com/devlifeX/BiFlow)
(`resources/rules/manifest.json`, then the files in that same commit).
Upstream hosts stay in this snapshot for maintainer provenance and are not
runtime fallbacks.

| File                        |  Lines | SHA-256                                                            |
| --------------------------- | -----: | ------------------------------------------------------------------ |
| `iran-domains.txt`          | 62,828 | `ae533f8bf147877bb97efd24a3dd708695f10289462d0746c30af0d9442f2581` |
| `iran-networks.txt`         |  2,888 | `e72076c81b372dcd6ecb6e8fb17b63b0d33bdd1dc53f1b795875a3f811d6561e` |
| `private.txt`               |     18 | `aed134cc43c2414cb3df5a10fcb3e215e64fac0249579a112c163674df4ddd36` |
| `iran-business-domains.txt` |     40 | `a7c17dc46102a692b4133d1e9b993c28be538d2fc578bde0427aea107027ab7d` |
| `iran-cdn-networks.txt`     |      3 | `ef103916c3188c242031060a35d55a8a14553ddfd359123218c6c785d475da4e` |

The installed copy is never modified. Live refreshes are validated and written
to the application data directory, and a failed refresh keeps the last known
good cache. Lines beginning with `#` are metadata. Domain entries may use the
Mihomo text-provider `+.` suffix form.

Run `pnpm rules:update` (`./scripts/update-rules.sh`) to create a fresh
single-commit snapshot. The script does not commit or push. The generated
`manifest.json` is authoritative; `pnpm rules:check` validates its hashes and
minimum entry counts without accessing the network.

Bundled rule files use LF bytes only. Root `.gitattributes` marks
`resources/rules/*` as `-text` so Windows Git checkout does not rewrite CRLF and
break SHA-256 verification during `bundle:check`.

Curated files (`iran-business-domains.txt`, `iran-cdn-networks.txt`) are
BiFlow-owned DIRECT catalogs, not Chocolate4U. Cloud refresh must not
overwrite them. Provenance lives in the matching `*.sources.json`. Extra
CIDRs need containment-diff against `iran-networks.txt` plus a first-party
or Iranian-ASN source; do not union the IR RIR table.
