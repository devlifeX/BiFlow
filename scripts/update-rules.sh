#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MIHOMO="${ROOT}/vendor/mihomo/linux-x86_64/mihomo"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '%s\n' "$*"
}

validate_with_mihomo() {
  [[ -x "${MIHOMO}" ]] || die "pinned Mihomo is missing at ${MIHOMO}"
  local tmp output
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/biflow-rules-mihomo.XXXXXX")"
  cp "${ROOT}/resources/rules/private.txt" "${tmp}/private.txt"
  cp "${ROOT}/resources/rules/iran-domains.txt" "${tmp}/iran-domains.txt"
  cp "${ROOT}/resources/rules/iran-networks.txt" "${tmp}/iran-networks.txt"
  cp "${ROOT}/resources/rules/iran-business-domains.txt" "${tmp}/iran-business-domains.txt"
  cp "${ROOT}/resources/rules/iran-cdn-networks.txt" "${tmp}/iran-cdn-networks.txt"
  : > "${tmp}/custom-direct-domains.txt"
  : > "${tmp}/custom-direct-ips.txt"
  cat > "${tmp}/config.yaml" <<'YAML'
mixed-port: 7890
allow-lan: false
mode: rule
log-level: silent
ipv6: true
dns:
  enable: true
  listen: 127.0.0.1:0
  enhanced-mode: fake-ip
  nameserver:
    - 1.1.1.1
rule-providers:
  private-networks:
    type: file
    behavior: ipcidr
    path: private.txt
  iran-domains:
    type: file
    behavior: domain
    path: iran-domains.txt
  iran-business-domains:
    type: file
    behavior: domain
    path: iran-business-domains.txt
  iran-networks:
    type: file
    behavior: ipcidr
    path: iran-networks.txt
  iran-cdn-networks:
    type: file
    behavior: ipcidr
    path: iran-cdn-networks.txt
rules:
  - RULE-SET,private-networks,DIRECT
  - RULE-SET,iran-domains,DIRECT
  - RULE-SET,iran-business-domains,DIRECT
  - RULE-SET,iran-networks,DIRECT
  - RULE-SET,iran-cdn-networks,DIRECT
  - MATCH,DIRECT
YAML
  if ! output="$("${MIHOMO}" -t -d "${tmp}" -f "${tmp}/config.yaml" 2>&1)"; then
    printf '%s\n' "${output}" >&2
    rm -rf "${tmp}"
    die "pinned Mihomo rejected the snapshot"
  fi
  printf '%s\n' "${output}"
  rm -rf "${tmp}"
}

cd "${ROOT}"
log "Downloading an immutable upstream snapshot (does not commit or push)..."
node "${ROOT}/scripts/sync-rules.mjs"
log "Running the offline snapshot check..."
node "${ROOT}/scripts/sync-rules.mjs" --check
log "Validating provider files with the pinned Mihomo binary..."
validate_with_mihomo
log "Human review diff for resources/rules:"
git --no-pager diff --stat -- resources/rules || true
log "Review with: git diff -- resources/rules"
log "This script does not commit or push. Commit only after you review the diff."
