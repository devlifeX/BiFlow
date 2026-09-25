#!/bin/sh
# Privileged installer for the BiFlow helper. Must run as root (pkexec or postinst).
# Copies helper/Mihomo into root-owned paths, writes helper.toml, and starts systemd.
set -eu

HELPER_DEST="/usr/lib/biflow/iran-split-helper"
MIHOMO_DEST="/usr/lib/biflow/mihomo"
CONFIG_DEST="/etc/iran-split/helper.toml"
UNIT_DEST="/etc/systemd/system/iran-split-helper.service"
SOCKET_PATH="/run/iran-split/helper.sock"
RUNTIME_DIR="/var/lib/iran-split"
PRINT_PLAN=0
AUTHORIZED_UID=""
AUTHORIZED_GID=""
STAGING_DIR=""
HELPER_SRC=""
MIHOMO_SRC=""
HELPER_SHA256=""
MIHOMO_SHA256=""
TUN_NAME=""
UNIT_SRC=""

usage() {
  printf '%s\n' "usage: install-helper.sh --authorized-uid UID --authorized-gid GID --staging-dir PATH --helper-src PATH --mihomo-src PATH --helper-sha256 HASH --mihomo-sha256 HASH --tun-name NAME [--unit-src PATH] [--print-plan]" >&2
  exit 2
}

is_numeric() {
  case "${1:-}" in
    ''|*[!0-9]*) return 1 ;;
    *) return 0 ;;
  esac
}

is_sha256() {
  value="${1:-}"
  [ "${#value}" -eq 64 ] || return 1
  case "${value}" in
    *[!0-9a-f]*) return 1 ;;
    *) return 0 ;;
  esac
}

is_safe_absolute() {
  path="${1:-}"
  case "${path}" in
    /*) ;;
    *) return 1 ;;
  esac
  case "${path}" in
    *..*) return 1 ;;
  esac
  return 0
}

is_safe_tun() {
  name="${1:-}"
  [ -n "${name}" ] || return 1
  [ "${#name}" -le 15 ] || return 1
  case "${name}" in
    *[!A-Za-z0-9_-]*) return 1 ;;
    *) return 0 ;;
  esac
}

toml_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

sha256_of() {
  command sha256sum -- "$1" | command awk '{print $1}'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --authorized-uid)
      AUTHORIZED_UID="${2:-}"
      shift 2
      ;;
    --authorized-gid)
      AUTHORIZED_GID="${2:-}"
      shift 2
      ;;
    --staging-dir)
      STAGING_DIR="${2:-}"
      shift 2
      ;;
    --helper-src)
      HELPER_SRC="${2:-}"
      shift 2
      ;;
    --mihomo-src)
      MIHOMO_SRC="${2:-}"
      shift 2
      ;;
    --helper-sha256)
      HELPER_SHA256="${2:-}"
      shift 2
      ;;
    --mihomo-sha256)
      MIHOMO_SHA256="${2:-}"
      shift 2
      ;;
    --tun-name)
      TUN_NAME="${2:-}"
      shift 2
      ;;
    --unit-src)
      UNIT_SRC="${2:-}"
      shift 2
      ;;
    --print-plan)
      PRINT_PLAN=1
      shift
      ;;
    -h|--help)
      usage
      ;;
    *)
      printf 'unknown argument: %s\n' "$1" >&2
      usage
      ;;
  esac
done

is_numeric "${AUTHORIZED_UID}" || { printf 'authorized-uid must be numeric\n' >&2; exit 2; }
is_numeric "${AUTHORIZED_GID}" || { printf 'authorized-gid must be numeric\n' >&2; exit 2; }
[ "${AUTHORIZED_UID}" != "0" ] || { printf 'authorized-uid must not be root\n' >&2; exit 2; }
is_safe_absolute "${STAGING_DIR}" || { printf 'staging-dir must be an absolute path without ..\n' >&2; exit 2; }
is_safe_absolute "${HELPER_SRC}" || { printf 'helper-src must be an absolute path without ..\n' >&2; exit 2; }
is_safe_absolute "${MIHOMO_SRC}" || { printf 'mihomo-src must be an absolute path without ..\n' >&2; exit 2; }
is_sha256 "${HELPER_SHA256}" || { printf 'helper-sha256 must be lowercase SHA-256\n' >&2; exit 2; }
is_sha256 "${MIHOMO_SHA256}" || { printf 'mihomo-sha256 must be lowercase SHA-256\n' >&2; exit 2; }
is_safe_tun "${TUN_NAME}" || { printf 'tun-name is not a safe interface name\n' >&2; exit 2; }
if [ -n "${UNIT_SRC}" ]; then
  is_safe_absolute "${UNIT_SRC}" || { printf 'unit-src must be an absolute path without ..\n' >&2; exit 2; }
fi

if [ "${PRINT_PLAN}" -eq 1 ]; then
  printf 'HELPER_DEST=%s\n' "${HELPER_DEST}"
  printf 'MIHOMO_DEST=%s\n' "${MIHOMO_DEST}"
  printf 'CONFIG_DEST=%s\n' "${CONFIG_DEST}"
  printf 'UNIT_DEST=%s\n' "${UNIT_DEST}"
  printf 'SOCKET_PATH=%s\n' "${SOCKET_PATH}"
  printf 'RUNTIME_DIR=%s\n' "${RUNTIME_DIR}"
  printf 'AUTHORIZED_UID=%s\n' "${AUTHORIZED_UID}"
  printf 'AUTHORIZED_GID=%s\n' "${AUTHORIZED_GID}"
  printf 'STAGING_DIR=%s\n' "${STAGING_DIR}"
  printf 'TUN_NAME=%s\n' "${TUN_NAME}"
  exit 0
fi

if [ "$(id -u)" -ne 0 ]; then
  printf 'install-helper.sh must run as root\n' >&2
  exit 1
fi

[ -f "${HELPER_SRC}" ] || { printf 'helper source is missing\n' >&2; exit 1; }
[ -f "${MIHOMO_SRC}" ] || { printf 'mihomo source is missing\n' >&2; exit 1; }
[ ! -L "${HELPER_SRC}" ] || { printf 'helper source must not be a symlink\n' >&2; exit 1; }
[ ! -L "${MIHOMO_SRC}" ] || { printf 'mihomo source must not be a symlink\n' >&2; exit 1; }

actual_helper="$(sha256_of "${HELPER_SRC}")"
actual_mihomo="$(sha256_of "${MIHOMO_SRC}")"
[ "${actual_helper}" = "${HELPER_SHA256}" ] || { printf 'helper source failed checksum verification\n' >&2; exit 1; }
[ "${actual_mihomo}" = "${MIHOMO_SHA256}" ] || { printf 'mihomo source failed checksum verification\n' >&2; exit 1; }

if [ -z "${UNIT_SRC}" ]; then
  script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
  UNIT_SRC="${script_dir}/iran-split-helper.service"
fi
[ -f "${UNIT_SRC}" ] || { printf 'systemd unit template is missing\n' >&2; exit 1; }

install -d -o root -g root -m 0755 /usr/lib/biflow
install -d -o root -g root -m 0750 /etc/iran-split
install -d -o root -g root -m 0700 "${RUNTIME_DIR}"
install -d -o root -g "${AUTHORIZED_GID}" -m 0750 /run/iran-split
# A .deb already placed these under /usr/lib/biflow, so the source and the
# destination can be the same path. `install` fails with "are the same file";
# the file is already where it belongs, so only re-apply ownership and mode.
install_or_reown() {
  mode="$1"
  source_path="$2"
  destination="$3"
  if [ "$(readlink -f -- "${source_path}")" = "$(readlink -f -- "${destination}")" ]; then
    chown root:root -- "${destination}"
    chmod "${mode}" -- "${destination}"
    return 0
  fi
  install -o root -g root -m "${mode}" "${source_path}" "${destination}"
}

install_or_reown 0755 "${HELPER_SRC}" "${HELPER_DEST}"
install_or_reown 0755 "${MIHOMO_SRC}" "${MIHOMO_DEST}"
install_or_reown 0644 "${UNIT_SRC}" "${UNIT_DEST}"
install_or_reown 0755 "$0" /usr/lib/biflow/install-helper.sh

config_temp="$(mktemp /etc/iran-split/helper.toml.XXXXXX)"
trap 'rm -f -- "${config_temp}"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
chmod 0600 -- "${config_temp}"
{
  printf 'authorized_uid = %s\n' "${AUTHORIZED_UID}"
  printf 'authorized_gid = %s\n' "${AUTHORIZED_GID}"
  printf 'socket_path = "%s"\n' "$(toml_escape "${SOCKET_PATH}")"
  printf 'staging_dir = "%s"\n' "$(toml_escape "${STAGING_DIR}")"
  printf 'runtime_dir = "%s"\n' "$(toml_escape "${RUNTIME_DIR}")"
  printf 'mihomo_binary = "%s"\n' "$(toml_escape "${MIHOMO_DEST}")"
  printf 'mihomo_sha256 = "%s"\n' "${actual_mihomo}"
  printf 'tun_name = "%s"\n' "${TUN_NAME}"
} >"${config_temp}"
chown root:root -- "${config_temp}"
mv -f -- "${config_temp}" "${CONFIG_DEST}"
trap - EXIT HUP INT TERM

[ "$(sha256_of "${HELPER_DEST}")" = "${actual_helper}" ] || { printf 'installed helper failed checksum verification\n' >&2; exit 1; }
[ "$(sha256_of "${MIHOMO_DEST}")" = "${actual_mihomo}" ] || { printf 'installed mihomo failed checksum verification\n' >&2; exit 1; }

systemctl daemon-reload
systemctl enable iran-split-helper.service
systemctl restart iran-split-helper.service
if ! systemctl is-active --quiet iran-split-helper.service; then
  printf 'installed Helper service did not become active\n' >&2
  exit 1
fi
