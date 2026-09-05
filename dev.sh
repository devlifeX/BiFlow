#!/usr/bin/env bash
set -Eeuo pipefail

PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
SOURCE_STACK_DIR="/home/devlife/dev/dariush/clash"
SOURCE_MIHOMO="${SOURCE_STACK_DIR}/.tools/mihomo"
VENDORED_MIHOMO="${PROJECT_DIR}/vendor/mihomo/linux-x86_64/mihomo"
EXPECTED_MIHOMO_SHA256="9c397be7489538628fae781bc005e4c5b8cd7b0961b8bb2ca815c8150f193577"
TARGET_DIR="${PROJECT_DIR}/target"
# One shared definition: the app (BIFLOW_DEV_PROFILE) and the helper staging
# path must agree, or the helper cannot read staged generations.
DEV_PROFILE_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/biflow-dev-profile"
DEV_HELPER_UNIT=""
DEV_HELPER_ROOT=""
DEV_HELPER_LIB_ROOT=""
DEV_HELPER_SOCKET=""
DEV_HELPER_RUNTIME=""
DEV_HELPER_MIHOMO=""
DEV_HELPER_EXECUTABLE=""
DEV_HELPER_CONFIG_TEMP=""
DEV_HELPER_STARTED=0
DEV_HELPER_ROOT_CREATED=0
DEV_HELPER_LIB_ROOT_CREATED=0
DEV_HELPER_LOCK_FD=""

usage() {
  command cat <<'EOF'
BiFlow development helper

Usage: ./dev.sh [command]

Commands:
  dev       Compile and start the complete native Tauri application (default)
  desktop   Alias for dev
  web       Start only the React UI with the safe in-browser mock backend
  check     Run frontend and Rust formatting, linting, type checks, and tests
  e2e       Run Playwright primary-flow tests against the mock UI
  build     Build optimized frontend, helper, CLI, and Tauri desktop binaries
  package   Build the Linux .deb via ./build.sh linux
  assets    Refresh the local Mihomo development asset after checksum validation
  paths     Print expected Linux output paths
  clean     Remove generated build outputs after an explicit confirmation
  help      Show this message

Release packages for Linux (.deb) and Windows (.exe + NSIS installer):

  ./build.sh
  ./build.sh linux
  ./build.sh windows

The dev/desktop/package commands require Rust 1.88 and WebKitGTK 4.1
development packages. Native Linux development asks for sudo to run a
per-user transient helper; the helper stops and its /run files are removed
when dev.sh exits. Windows packages also need NSIS and a Windows Rust target.
The web command only requires Node.js and pnpm and never touches TUN, routes,
DNS, or system services.
EOF
}

die() {
  command printf 'error: %s\n' "$*" >&2
  exit 1
}

need_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

sha256_of() {
  command sha256sum "$1" | command awk '{print $1}'
}

verify_mihomo() {
  local candidate="$1"
  [[ -x "${candidate}" ]] || die "Mihomo is missing or not executable: ${candidate}"
  local actual
  actual="$(sha256_of "${candidate}")"
  [[ "${actual}" == "${EXPECTED_MIHOMO_SHA256}" ]] || \
    die "Mihomo checksum mismatch: expected ${EXPECTED_MIHOMO_SHA256}, got ${actual}"
  "${candidate}" -v | command sed -n '1,2p'
}

run_root() {
  if [[ "$(command id -u)" -eq 0 ]]; then
    command "$@"
  elif command -v sudo >/dev/null 2>&1; then
    command sudo -- "$@"
  else
    die "sudo is required to start the privileged development helper"
  fi
}

toml_escape() {
  local value="$1"
  [[ "${value}" != *$'\n'* && "${value}" != *$'\r'* ]] || \
    die "development helper paths must not contain newlines"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  command printf '%s' "${value}"
}

read_dev_tun_name() {
  local account_home="$1"
  local config_base config_path setting_line tun_name="clash-iran"
  config_base="${XDG_CONFIG_HOME:-${account_home}/.config}"
  config_path="${config_base}/biflow/config.toml"
  if [[ -f "${config_path}" ]]; then
    setting_line="$(command awk '
      /^\[mihomo\][[:space:]]*$/ { in_mihomo = 1; next }
      /^\[/ { in_mihomo = 0 }
      in_mihomo && /^[[:space:]]*tun_name[[:space:]]*=/ { print; exit }
    ' "${config_path}")"
    if [[ -n "${setting_line}" ]]; then
      tun_name="$(command sed -n 's/^[[:space:]]*tun_name[[:space:]]*=[[:space:]]*"\([^"]*\)".*$/\1/p' <<<"${setting_line}")"
    fi
  fi
  [[ "${tun_name}" =~ ^[A-Za-z0-9_-]{1,15}$ ]] || \
    die "configured TUN name must contain 1-15 letters, digits, '-' or '_' for the Linux helper"
  command printf '%s' "${tun_name}"
}

validate_dev_helper_targets() {
  [[ "${DEV_HELPER_ROOT}" =~ ^/run/biflow-dev-[0-9]+$ ]] || \
    die "refusing unsafe development helper root: ${DEV_HELPER_ROOT}"
  [[ "${DEV_HELPER_LIB_ROOT}" =~ ^/var/lib/biflow-dev-[0-9]+$ ]] || \
    die "refusing unsafe development helper lib root: ${DEV_HELPER_LIB_ROOT}"
  [[ "${DEV_HELPER_UNIT}" =~ ^biflow-dev-helper-[0-9]+\.service$ ]] || \
    die "refusing unsafe development helper unit: ${DEV_HELPER_UNIT}"
}

reset_stale_dev_session() {
  local developer_uid="$1" stale_unit vite_pids
  if command pkill -f "${TARGET_DIR}/debug/iran-split-desktop" 2>/dev/null; then
    command printf 'Closed an orphaned dev app window from a previous run.\n'
    command sleep 0.3
  fi
  vite_pids="$(command pgrep -f "${PROJECT_DIR}/node_modules/.*vite" 2>/dev/null || true)"
  if [[ -n "${vite_pids}" ]]; then
    # shellcheck disable=SC2086
    command kill ${vite_pids} 2>/dev/null || true
    command printf 'Closed a stale dev Vite server (port 1420).\n'
    command sleep 0.3
  fi
  stale_unit="biflow-dev-helper-${developer_uid}.service"
  if command systemctl is-active --quiet "${stale_unit}"; then
    command printf 'Stopping a leftover transient helper from a previous run...\n'
    run_root systemctl stop "${stale_unit}" || \
      die "could not stop the leftover transient helper ${stale_unit}"
  fi
}

cleanup_dev_helper() {
  local original_status="$?" cleanup_status=0
  set +e
  if [[ -n "${DEV_HELPER_CONFIG_TEMP}" ]]; then
    command rm -f -- "${DEV_HELPER_CONFIG_TEMP}"
    DEV_HELPER_CONFIG_TEMP=""
  fi
  if [[ "${DEV_HELPER_STARTED}" -eq 1 ]]; then
    # Take the dev app window down with the session: an orphaned window
    # keeps running against a helper that no longer exists and then offers
    # actions (like helper install) that cannot work.
    command pkill -f "${TARGET_DIR}/debug/iran-split-desktop" 2>/dev/null
    command printf 'Stopping transient BiFlow development helper...\n'
    if command systemctl is-active --quiet "${DEV_HELPER_UNIT}"; then
      run_root systemctl stop "${DEV_HELPER_UNIT}" || cleanup_status=1
    fi
    DEV_HELPER_STARTED=0
  fi
  if [[ "${DEV_HELPER_ROOT_CREATED}" -eq 1 ]]; then
    if [[ "${DEV_HELPER_ROOT}" =~ ^/run/biflow-dev-[0-9]+$ ]]; then
      run_root rm -rf -- "${DEV_HELPER_ROOT}" || cleanup_status=1
    else
      command printf 'warning: refused unsafe helper cleanup target: %s\n' \
        "${DEV_HELPER_ROOT}" >&2
      cleanup_status=1
    fi
    DEV_HELPER_ROOT_CREATED=0
  fi
  if [[ "${DEV_HELPER_LIB_ROOT_CREATED}" -eq 1 ]]; then
    if [[ "${DEV_HELPER_LIB_ROOT}" =~ ^/var/lib/biflow-dev-[0-9]+$ ]]; then
      run_root rm -rf -- "${DEV_HELPER_LIB_ROOT}" || cleanup_status=1
    else
      command printf 'warning: refused unsafe helper lib cleanup target: %s\n' \
        "${DEV_HELPER_LIB_ROOT}" >&2
      cleanup_status=1
    fi
    DEV_HELPER_LIB_ROOT_CREATED=0
  fi
  if [[ "${original_status}" -eq 0 && "${cleanup_status}" -ne 0 ]]; then
    return "${cleanup_status}"
  fi
  return "${original_status}"
}

show_dev_helper_failure() {
  command printf 'error: transient helper did not create an accessible socket: %s\n' \
    "${DEV_HELPER_SOCKET}" >&2
  run_root journalctl --unit "${DEV_HELPER_UNIT}" --lines 50 --no-pager >&2 || true
  return 1
}

prepare_dev_helper() {
  need_command awk
  need_command flock
  need_command install
  need_command journalctl
  need_command mktemp
  need_command sed
  need_command sha256sum
  need_command stat
  need_command systemctl
  need_command systemd-run

  local developer_uid developer_gid account_home data_base staging_dir tun_name
  local helper_source helper_hash lock_directory mihomo_hash
  developer_uid="$(command id -u)"
  developer_gid="$(command id -g)"
  [[ "${developer_uid}" =~ ^[0-9]+$ && "${developer_gid}" =~ ^[0-9]+$ ]] || \
    die "could not determine the developer UID/GID"
  [[ "${developer_uid}" -ne 0 ]] || \
    die "run dev.sh as your normal account; it elevates only the helper"
  lock_directory="${XDG_RUNTIME_DIR:-/run/user/${developer_uid}}"
  [[ -d "${lock_directory}" && -O "${lock_directory}" ]] || \
    die "private user runtime directory is unavailable: ${lock_directory}"
  exec {DEV_HELPER_LOCK_FD}>"${lock_directory}/biflow-dev.lock"
  command flock -n "${DEV_HELPER_LOCK_FD}" || \
    die "another native BiFlow development session is already running for this user"
  # The session lock is held, so anything still running from this repo is a
  # leftover of a crashed or abandoned run. Close it all for a fresh start:
  # an orphaned dev window keeps acting against a helper that no longer
  # exists, and a stale Vite server blocks port 1420.
  reset_stale_dev_session "${developer_uid}"
  account_home="$(command getent passwd "${developer_uid}" | command cut -d: -f6)"
  [[ -n "${account_home}" && "${account_home}" == /* ]] || \
    die "could not determine the developer home directory"
  data_base="${XDG_DATA_HOME:-${account_home}/.local/share}"
  [[ "${data_base}" == /* ]] || die "XDG_DATA_HOME must be an absolute path"
  # The dev app stages generations inside its isolated profile, so the dev
  # helper must read from the same place — never the installed app's data.
  staging_dir="${DEV_PROFILE_DIR}/data/runtime/generations"
  tun_name="$(read_dev_tun_name "${account_home}")"

  DEV_HELPER_UNIT="biflow-dev-helper-${developer_uid}.service"
  DEV_HELPER_ROOT="/run/biflow-dev-${developer_uid}"
  DEV_HELPER_LIB_ROOT="/var/lib/biflow-dev-${developer_uid}"
  DEV_HELPER_SOCKET="${DEV_HELPER_ROOT}/helper.sock"
  DEV_HELPER_RUNTIME="${DEV_HELPER_ROOT}/runtime"
  DEV_HELPER_EXECUTABLE="${DEV_HELPER_LIB_ROOT}/bin/iran-split-helper"
  DEV_HELPER_MIHOMO="${DEV_HELPER_LIB_ROOT}/bin/mihomo"
  validate_dev_helper_targets

  command printf 'Building the privileged development helper...\n'
  cargo build -p iran-split-helper
  helper_source="${TARGET_DIR}/debug/iran-split-helper"
  [[ -x "${helper_source}" ]] || die "development helper build is missing: ${helper_source}"
  verify_mihomo "${VENDORED_MIHOMO}" >/dev/null
  helper_hash="$(sha256_of "${helper_source}")"
  mihomo_hash="$(sha256_of "${VENDORED_MIHOMO}")"
  [[ "${mihomo_hash}" == "${EXPECTED_MIHOMO_SHA256}" ]] || \
    die "development Mihomo checksum changed during helper setup"
  command mkdir -p -- "${staging_dir}"
  command chmod 700 -- "${DEV_PROFILE_DIR}/data/runtime" "${staging_dir}"

  DEV_HELPER_CONFIG_TEMP="$(command mktemp "/tmp/biflow-helper-${developer_uid}.XXXXXX.toml")"
  command chmod 600 -- "${DEV_HELPER_CONFIG_TEMP}"
  {
    command printf 'authorized_uid = %s\n' "${developer_uid}"
    command printf 'authorized_gid = %s\n' "${developer_gid}"
    command printf 'socket_path = "%s"\n' "$(toml_escape "${DEV_HELPER_SOCKET}")"
    command printf 'staging_dir = "%s"\n' "$(toml_escape "${staging_dir}")"
    command printf 'runtime_dir = "%s"\n' "$(toml_escape "${DEV_HELPER_RUNTIME}")"
    command printf 'mihomo_binary = "%s"\n' "$(toml_escape "${DEV_HELPER_MIHOMO}")"
    command printf 'mihomo_sha256 = "%s"\n' "${mihomo_hash}"
    command printf 'tun_name = "%s"\n' "${tun_name}"
  } >"${DEV_HELPER_CONFIG_TEMP}"

  if command systemctl is-active --quiet "${DEV_HELPER_UNIT}"; then
    command printf 'Replacing a stale BiFlow development helper...\n'
    run_root systemctl stop "${DEV_HELPER_UNIT}"
  fi
  run_root systemctl reset-failed "${DEV_HELPER_UNIT}" >/dev/null 2>&1 || true
  run_root rm -rf -- "${DEV_HELPER_ROOT}" "${DEV_HELPER_LIB_ROOT}"
  run_root install -d -o root -g "${developer_gid}" -m 0750 "${DEV_HELPER_ROOT}"
  DEV_HELPER_ROOT_CREATED=1
  run_root install -d -o root -g root -m 0755 "${DEV_HELPER_LIB_ROOT}/bin"
  DEV_HELPER_LIB_ROOT_CREATED=1
  run_root install -d -o root -g root -m 0700 "${DEV_HELPER_RUNTIME}"
  run_root install -o root -g root -m 0755 "${helper_source}" \
    "${DEV_HELPER_EXECUTABLE}"
  run_root install -o root -g root -m 0755 "${VENDORED_MIHOMO}" \
    "${DEV_HELPER_MIHOMO}"
  run_root install -o root -g root -m 0600 "${DEV_HELPER_CONFIG_TEMP}" \
    "${DEV_HELPER_ROOT}/helper.toml"
  command rm -f -- "${DEV_HELPER_CONFIG_TEMP}"
  DEV_HELPER_CONFIG_TEMP=""
  [[ "$(sha256_of "${DEV_HELPER_EXECUTABLE}")" == "${helper_hash}" ]] || \
    die "root-owned development helper copy failed checksum verification"
  [[ "$(sha256_of "${DEV_HELPER_MIHOMO}")" == "${mihomo_hash}" ]] || \
    die "root-owned development Mihomo copy failed checksum verification"
  [[ "$(command stat -c '%u:%a' "${DEV_HELPER_ROOT}/helper.toml")" == "0:600" ]] || \
    die "development helper config is not root-owned mode 0600"

  command printf 'Starting transient helper %s (helper SHA-256 %s)...\n' \
    "${DEV_HELPER_UNIT}" "${helper_hash}"
  run_root systemd-run \
    --quiet \
    --collect \
    --unit "${DEV_HELPER_UNIT}" \
    --uid root \
    --gid "${developer_gid}" \
    --property Type=simple \
    --property Restart=no \
    --property KillMode=control-group \
    --property TimeoutStopSec=15s \
    --property UMask=0007 \
    --property NoNewPrivileges=yes \
    --property PrivateTmp=yes \
    --property ProtectSystem=strict \
    --property ProtectHome=read-only \
    --property DeviceAllow="/dev/net/tun rw" \
    --property "ReadWritePaths=${DEV_HELPER_ROOT} ${DEV_HELPER_LIB_ROOT}" \
    -- "${DEV_HELPER_EXECUTABLE}" \
    --config "${DEV_HELPER_ROOT}/helper.toml"
  DEV_HELPER_STARTED=1

  local attempt
  for attempt in {1..100}; do
    if [[ -S "${DEV_HELPER_SOCKET}" && -r "${DEV_HELPER_SOCKET}" && -w "${DEV_HELPER_SOCKET}" ]]; then
      [[ "$(command stat -c '%u:%g:%a' "${DEV_HELPER_SOCKET}")" == \
        "0:${developer_gid}:660" ]] || \
        die "development helper socket ownership or mode is unsafe"
      export BIFLOW_DEV_HELPER_SOCKET="${DEV_HELPER_SOCKET}"
      export BIFLOW_DEV_SYSTEM_RUNTIME="${DEV_HELPER_RUNTIME}"
      export BIFLOW_DEV_MIHOMO_BINARY="${DEV_HELPER_MIHOMO}"
      command printf 'Transient helper is ready: %s\n' "${DEV_HELPER_SOCKET}"
      return 0
    fi
    command systemctl is-active --quiet "${DEV_HELPER_UNIT}" || break
    command sleep 0.05
  done
  show_dev_helper_failure
}

refresh_assets() {
  need_command rsync
  need_command sha256sum
  verify_mihomo "${SOURCE_MIHOMO}"
  command mkdir -p -- "$(dirname -- "${VENDORED_MIHOMO}")"
  command rsync --checksum --chmod=F755 "${SOURCE_MIHOMO}" "${VENDORED_MIHOMO}"
  verify_mihomo "${VENDORED_MIHOMO}"
  command printf 'Mihomo development asset is ready: %s\n' "${VENDORED_MIHOMO}"
}

ensure_node_dependencies() {
  need_command node
  need_command pnpm
  if [[ ! -d "${PROJECT_DIR}/node_modules" ]]; then
    command printf 'Installing pinned frontend dependencies...\n'
    (cd -- "${PROJECT_DIR}" && pnpm install --frozen-lockfile)
  fi
}

activate_rust_path() {
  local account_home rust_bin_dir
  account_home="$(command getent passwd "$(command id -u)" | command cut -d: -f6)"
  [[ -n "${account_home}" ]] || die "could not determine the current account home directory"
  rust_bin_dir="${account_home}/.cargo/bin"
  if [[ -x "${rust_bin_dir}/cargo" ]]; then
    export PATH="${rust_bin_dir}:${PATH}"
  fi
}

ensure_rust() {
  activate_rust_path
  need_command cargo
  need_command rustc
  local actual
  actual="$(rustc --version | command awk '{print $2}')"
  [[ "${actual}" == "1.88.0" ]] || \
    die "Rust 1.88.0 is required by rust-toolchain.toml; active version is ${actual}"
}

ensure_linux_desktop_dependencies() {
  need_command pkg-config
  pkg-config --exists webkit2gtk-4.1 || die \
    "webkit2gtk-4.1 development files are missing; install libwebkit2gtk-4.1-dev"
  pkg-config --exists gtk+-3.0 || die \
    "GTK 3 development files are missing; install libgtk-3-dev"
}

print_paths() {
  command printf '%s\n' \
    "Frontend: ${PROJECT_DIR}/apps/desktop/dist/" \
    "Desktop:  ${TARGET_DIR}/release/iran-split-desktop" \
    "Helper:   ${TARGET_DIR}/release/iran-split-helper" \
    "CLI:      ${TARGET_DIR}/release/iran-split-cli" \
    "Debian:   ${PROJECT_DIR}/artifacts/linux/" \
    "Windows:  ${PROJECT_DIR}/artifacts/windows/"
}

run_web() {
  ensure_node_dependencies
  command printf 'Starting the safe UI mock at http://127.0.0.1:1420\n'
  command printf 'This mode does not start Mihomo, create a TUN, or change routes.\n'
  cd -- "${PROJECT_DIR}"
  exec pnpm dev --host 127.0.0.1
}

run_dev() {
  ensure_node_dependencies
  ensure_rust
  ensure_linux_desktop_dependencies
  verify_mihomo "${VENDORED_MIHOMO}"
  cd -- "${PROJECT_DIR}"
  trap cleanup_dev_helper EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  trap 'exit 129' HUP
  prepare_dev_helper
  # Keep the development profile away from the installed app's profile:
  # config schema migration is one-way, and a dev run must never break the
  # release build the developer relies on.
  export BIFLOW_DEV_PROFILE="${DEV_PROFILE_DIR}"
  command mkdir -p -- "${BIFLOW_DEV_PROFILE}"
  command printf 'Development profile: %s\n' "${BIFLOW_DEV_PROFILE}"
  command pnpm tauri dev
}

run_check() {
  ensure_node_dependencies
  ensure_rust
  cd -- "${PROJECT_DIR}"
  pnpm check
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
}

run_e2e() {
  ensure_node_dependencies
  cd -- "${PROJECT_DIR}"
  pnpm exec playwright install chromium
  pnpm test:e2e
}

run_build() {
  ensure_node_dependencies
  ensure_rust
  ensure_linux_desktop_dependencies
  verify_mihomo "${VENDORED_MIHOMO}"
  cd -- "${PROJECT_DIR}"
  pnpm build
  cargo build --release -p iran-split-helper -p iran-split-cli
  cargo build --release -p iran-split-desktop
  print_paths
}

run_package() {
  exec "${PROJECT_DIR}/build.sh" linux
}

run_clean() {
  command printf 'This removes only generated target, dist, and coverage directories. Continue? [y/N] '
  local answer
  IFS= read -r answer
  [[ "${answer}" == "y" || "${answer}" == "Y" ]] || die "clean cancelled"
  command rm -rf -- \
    "${PROJECT_DIR}/target" \
    "${PROJECT_DIR}/apps/desktop/dist" \
    "${PROJECT_DIR}/apps/desktop/coverage"
}

main() {
  cd -- "${PROJECT_DIR}"
  case "${1:-dev}" in
    dev) run_dev ;;
    desktop) run_dev ;;
    web) run_web ;;
    check) run_check ;;
    e2e) run_e2e ;;
    build) run_build ;;
    package) run_package ;;
    assets) refresh_assets ;;
    paths) print_paths ;;
    clean) run_clean ;;
    help|-h|--help) usage ;;
    *) usage >&2; die "unknown command: $1" ;;
  esac
}

main "$@"
