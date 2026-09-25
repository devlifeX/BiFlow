#!/bin/sh
set -eu

HELPER="/usr/lib/biflow/iran-split-helper"
MIHOMO="/usr/lib/biflow/mihomo"
UNIT="/usr/lib/biflow/iran-split-helper.service"
CONFIG="/etc/iran-split/helper.toml"

# Debian calls postinst with "configure" after both a fresh install and an
# upgrade. Do not infer the authorized desktop user from a root or GUI package
# manager when no previous Helper configuration exists.
case "${1:-}" in
  configure) ;;
  *) exit 0 ;;
esac

for required in "${HELPER}" "${MIHOMO}" "${UNIT}"; do
  if [ ! -f "${required}" ]; then
    printf 'BiFlow Helper setup failed: packaged payload is missing: %s\n' "${required}" >&2
    exit 1
  fi
done

if [ -f "${CONFIG}" ]; then
  # Keep the previously authorized UID, GID, staging path, socket, and TUN
  # name. Only the packaged Mihomo hash changes across an application upgrade.
  # Replace the root-owned config atomically so a crash cannot leave TOML torn.
  mihomo_hash="$(sha256sum -- "${MIHOMO}" | awk '{print $1}')"
  valid_hash_fields="$(awk '
    /^mihomo_sha256[[:space:]]*=/ {
      count += 1
      value = $0
      sub(/^[^=]*=[[:space:]]*"/, "", value)
      sub(/"$/, "", value)
      if (length(value) != 64 || value ~ /[^0-9a-f]/) invalid = 1
    }
    END { print (count == 1 && !invalid) ? 1 : 0 }
  ' "${CONFIG}")"
  if [ "${valid_hash_fields}" -ne 1 ]; then
    printf 'BiFlow Helper upgrade failed: %s has no single valid Mihomo digest\n' "${CONFIG}" >&2
    exit 1
  fi
  config_temp="$(mktemp /etc/iran-split/helper.toml.XXXXXX)"
  trap 'rm -f -- "${config_temp}"' EXIT
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM
  sed "s/^mihomo_sha256 = \"[0-9a-f]\{64\}\"$/mihomo_sha256 = \"${mihomo_hash}\"/" "${CONFIG}" >"${config_temp}"
  chown root:root -- "${config_temp}"
  chmod 0600 -- "${config_temp}"
  mv -f -- "${config_temp}" "${CONFIG}"
  trap - EXIT HUP INT TERM

  if ! command -v systemctl >/dev/null 2>&1; then
    printf 'BiFlow Helper upgrade failed: systemctl is unavailable\n' >&2
    exit 1
  fi
  systemctl daemon-reload
  systemctl enable iran-split-helper.service
  systemctl restart iran-split-helper.service
  if ! systemctl is-active --quiet iran-split-helper.service; then
    printf 'BiFlow Helper upgrade failed: the service is not active after restart\n' >&2
    exit 1
  fi
  exit 0
fi

# A first install may run under apt, a GUI package manager, or pkexec. Only
# provision automatically when sudo provides a complete identity that agrees
# with the local passwd database. Otherwise the desktop's explicit Install
# Helper action requests authorization from the actual user on first launch.
if [ -z "${SUDO_UID:-}" ] || [ -z "${SUDO_GID:-}" ] || [ -z "${SUDO_USER:-}" ]; then
  printf '%s\n' 'BiFlow is installed. Open the app and choose Install Helper to authorize the service.' >&2
  exit 0
fi
case "${SUDO_UID}:${SUDO_GID}" in
  *[!0-9:]*)
    printf '%s\n' 'BiFlow Helper setup deferred: sudo identity is invalid; install it from the app.' >&2
    exit 0
    ;;
esac
if [ "${SUDO_UID}" = "0" ]; then
  printf '%s\n' 'BiFlow Helper setup deferred: root is not an authorized desktop user.' >&2
  exit 0
fi
passwd_entry="$(getent passwd "${SUDO_UID}" || true)"
passwd_user="$(printf '%s\n' "${passwd_entry}" | cut -d: -f1)"
passwd_gid="$(printf '%s\n' "${passwd_entry}" | cut -d: -f4)"
home="$(printf '%s\n' "${passwd_entry}" | cut -d: -f6)"
if [ "${passwd_user}" != "${SUDO_USER}" ] || [ "${passwd_gid}" != "${SUDO_GID}" ] || [ -z "${home}" ]; then
  printf '%s\n' 'BiFlow Helper setup deferred: sudo identity does not match the local user database; install it from the app.' >&2
  exit 0
fi

staging="${home}/.local/share/biflow/runtime/generations"
helper_hash="$(sha256sum -- "${HELPER}" | awk '{print $1}')"
mihomo_hash="$(sha256sum -- "${MIHOMO}" | awk '{print $1}')"
mkdir -p -- "${staging}"
/usr/lib/biflow/install-helper.sh \
  --authorized-uid "${SUDO_UID}" \
  --authorized-gid "${SUDO_GID}" \
  --staging-dir "${staging}" \
  --helper-src "${HELPER}" \
  --mihomo-src "${MIHOMO}" \
  --helper-sha256 "${helper_hash}" \
  --mihomo-sha256 "${mihomo_hash}" \
  --tun-name clash-iran \
  --unit-src "${UNIT}"
