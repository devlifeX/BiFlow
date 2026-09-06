#!/usr/bin/env bash
set -Eeuo pipefail

PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
TARGET_DIR="${PROJECT_DIR}/target"
CRATE_BIN="iran-split-desktop"
BUILD_VERSION=""
WINDOWS_TARGET=""
WINDOWS_TAURI_ARGS=()
TAURI_SIGNING_CONFIG_ARGS=()
FROM_STAGE=""
FORCE=0
# Which Linux bundles to produce: all (default), deb, or appimage.
LINUX_BUNDLES="all"
NODE_VERSION="24.11.1"
PNPM_VERSION="9.0.1"
# cargo-xwin 0.20+ requires rustc 1.89; pin the last release that builds on 1.88.
CARGO_XWIN_VERSION="0.19.2"
TOOL_PREFIX="${HOME}/.local/share/biflow-tools"
RUST_VERSION="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "${PROJECT_DIR}/rust-toolchain.toml" | head -n 1)"
[[ -n "${RUST_VERSION}" ]] || RUST_VERSION="1.88.0"

die() {
  command printf 'error: %s\n' "$*" >&2
  exit 1
}

log() {
  command printf '%s\n' "$*"
}

have() {
  command -v "$1" >/dev/null 2>&1
}

plan() {
  node "${PROJECT_DIR}/scripts/build-plan.mjs" "$1"
}

assert_build_version() {
  local current
  current="$(plan version)"
  [[ -n "${BUILD_VERSION}" && "${current}" == "${BUILD_VERSION}" ]] || \
    die "version changed during the build: started with ${BUILD_VERSION:-unset}, now ${current}"
}

linux_deb_name() {
  command printf 'BiFlow_%s_amd64.deb\n' "${BUILD_VERSION}"
}

linux_appimage_name() {
  command printf 'BiFlow_%s_amd64.AppImage\n' "${BUILD_VERSION}"
}

windows_installer_name() {
  command printf 'BiFlow_%s_x64-setup.exe\n' "${BUILD_VERSION}"
}

usage() {
  command cat <<'EOF'
BiFlow release builder

Usage: ./build.sh [mode] [--from STAGE] [--force]

Focused local verification (default developer gate):
  check-frontend   pnpm check and pnpm build only
  check-rust       cargo test + clippy for named workspace crates

GitHub-hosted packaging entry points:
  ci-linux       Native Linux .deb and AppImage
  ci-windows     Native Windows .exe and NSIS installer

Full packaging (machines with spare disk; not the local default):
  linux          Native Linux .deb and AppImage
  linux deb      Only the Linux .deb
  linux appimage Only the Linux AppImage
  windows        Windows app .exe and NSIS installer
  all            Linux and Windows (default when no mode is given)

Resume (linux stages: compile, deb, appimage, collect;
        windows stages: compile, nsis, collect):
  --from STAGE   Start at STAGE. Earlier finished work is left in place.
  --force        Rebuild every packaging stage for this version.

Without --from, packaging skips stages that already produced this version
(for example a .deb after AppImage download failed). Re-run the same
command to continue.

One-shot packaging installs missing tools, then builds.
Installs Node.js 24, pnpm, Rust (from rust-toolchain.toml), Linux desktop
libraries, NSIS, and cargo-xwin as needed.

Version is read from the root version file. Do not edit package.json,
Cargo.toml, or tauri.conf.json by hand.
EOF
}

host_os() {
  case "$(uname -s)" in
    Linux*) echo linux ;;
    Darwin*) echo macos ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT) echo windows ;;
    *) echo unknown ;;
  esac
}

host_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo x64 ;;
    aarch64|arm64) echo arm64 ;;
    *) die "unsupported architecture: $(uname -m)" ;;
  esac
}

refresh_path() {
  export PATH="${HOME}/.cargo/bin:${TOOL_PREFIX}/node/bin:${HOME}/.local/share/pnpm:${PATH}"
  if [[ -f "${HOME}/.cargo/env" ]]; then
    set +u
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
    set -u
  fi
}

run_root() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  elif have sudo; then
    sudo "$@"
  else
    die "root or sudo is required to install: $*"
  fi
}

APT_UPDATED=0

apt_update_once() {
  [[ "${APT_UPDATED}" -eq 1 ]] && return 0
  run_root env DEBIAN_FRONTEND=noninteractive apt-get update -y
  APT_UPDATED=1
}

ensure_curl() {
  have curl && return 0
  log "Installing curl..."
  case "$(host_os)" in
    linux)
      apt_update_once
      run_root env DEBIAN_FRONTEND=noninteractive apt-get install -y curl ca-certificates
      ;;
    *) die "curl is required to install build tools" ;;
  esac
}

node_major() {
  have node || return 1
  node -p "process.versions.node.split('.')[0]"
}

install_node() {
  local os arch tarball name url
  os="$(host_os)"
  arch="$(host_arch)"
  [[ "${os}" == "linux" ]] || die "automatic Node.js install is supported on Linux; install Node.js ${NODE_VERSION}+ and re-run"
  ensure_curl
  have tar || run_root env DEBIAN_FRONTEND=noninteractive apt-get install -y tar xz-utils
  name="node-v${NODE_VERSION}-linux-${arch}"
  tarball="${name}.tar.xz"
  url="https://nodejs.org/dist/v${NODE_VERSION}/${tarball}"
  log "Installing Node.js ${NODE_VERSION} into ${TOOL_PREFIX}/node..."
  command mkdir -p -- "${TOOL_PREFIX}"
  command curl -fsSL "${url}" | command tar -xJ -C "${TOOL_PREFIX}"
  command rm -rf -- "${TOOL_PREFIX}/node"
  command mv -- "${TOOL_PREFIX}/${name}" "${TOOL_PREFIX}/node"
  refresh_path
  have node || die "Node.js install finished but node is not on PATH"
}

ensure_node() {
  refresh_path
  local major
  if have node; then
    major="$(node_major)"
    if [[ "${major}" -ge 24 ]]; then
      log "Node.js $(node -v) is ready"
      return 0
    fi
    log "Node.js $(node -v) is too old; installing ${NODE_VERSION}"
  else
    log "Node.js is missing; installing ${NODE_VERSION}"
  fi
  install_node
  log "Node.js $(node -v) is ready"
}

ensure_pnpm() {
  refresh_path
  if have pnpm; then
    log "pnpm $(pnpm --version) is ready"
    return 0
  fi
  log "Installing pnpm ${PNPM_VERSION}..."
  if have corepack; then
    corepack enable
    corepack prepare "pnpm@${PNPM_VERSION}" --activate
  else
    ensure_curl
    command curl -fsSL https://get.pnpm.io/install.sh | env PNPM_VERSION="${PNPM_VERSION}" SHELL="$(command -v bash)" sh -
  fi
  refresh_path
  have pnpm || die "pnpm install finished but pnpm is not on PATH"
  log "pnpm $(pnpm --version) is ready"
}

ensure_node_modules() {
  if [[ ! -d "${PROJECT_DIR}/node_modules" ]]; then
    log "Installing pinned frontend dependencies..."
    (cd -- "${PROJECT_DIR}" && pnpm install --frozen-lockfile)
  fi
}

ensure_rust() {
  refresh_path
  if ! have rustup || ! have cargo || ! have rustc; then
    ensure_curl
    log "Installing Rust ${RUST_VERSION} with rustup..."
    command curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
      sh -s -- -y --default-toolchain "${RUST_VERSION}" --profile minimal -c rustfmt -c clippy
    refresh_path
  fi
  have cargo && have rustc || die "Rust install finished but cargo is not on PATH"
  local actual
  actual="$(cd -- "${PROJECT_DIR}" && rustc --version | command awk '{print $2}')"
  if [[ "${actual}" == "${RUST_VERSION}" ]]; then
    log "Rust ${actual} is ready"
    return 0
  fi
  rustup toolchain install "${RUST_VERSION}" --profile minimal --component rustfmt,clippy
  actual="$(cd -- "${PROJECT_DIR}" && rustc --version | command awk '{print $2}')"
  [[ "${actual}" == "${RUST_VERSION}" ]] || \
    die "Rust ${RUST_VERSION} is required by rust-toolchain.toml; active version is ${actual}"
  log "Rust ${actual} is ready"
}

dpkg_installed() {
  dpkg-query -W -f '${Status}' "$1" 2>/dev/null | grep -q 'install ok installed'
}

apt_install_missing() {
  local packages=()
  local pkg
  for pkg in "$@"; do
    dpkg_installed "${pkg}" || packages+=("${pkg}")
  done
  if [[ "${#packages[@]}" -eq 0 ]]; then
    return 0
  fi
  log "Installing packages: ${packages[*]}"
  apt_update_once
  run_root env DEBIAN_FRONTEND=noninteractive apt-get install -y "${packages[@]}"
}

ensure_linux_desktop_dependencies() {
  [[ "$(host_os)" == "linux" ]] || return 0
  [[ -r /etc/os-release ]] || die "cannot detect Linux distribution"
  # shellcheck disable=SC1091
  source /etc/os-release
  case "${ID:-}" in
    ubuntu|debian|linuxmint|pop)
      local indicator="libappindicator3-dev"
      local fuse2="libfuse2"
      if apt-cache show libayatana-appindicator3-dev >/dev/null 2>&1; then
        indicator="libayatana-appindicator3-dev"
      fi
      if apt-cache show libfuse2t64 >/dev/null 2>&1; then
        fuse2="libfuse2t64"
      fi
      apt_install_missing \
        build-essential \
        curl \
        pkg-config \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        libgtk-3-dev \
        "${indicator}" \
        librsvg2-dev \
        patchelf \
        xdg-utils \
        "${fuse2}"
      ;;
    *)
      die "automatic desktop-library install supports Debian/Ubuntu; install WebKitGTK 4.1 and GTK 3 development packages"
      ;;
  esac
  have pkg-config || die "pkg-config is missing after package install"
  pkg-config --exists webkit2gtk-4.1 || die "webkit2gtk-4.1 development files are still missing"
  pkg-config --exists gtk+-3.0 || die "GTK 3 development files are still missing"
  log "Linux desktop libraries are ready"
}

ensure_tauri_appimage_tools() {
  [[ "$(host_os)" == "linux" ]] || return 0
  ensure_curl
  log "Prefetching Tauri AppImage tools with curl..."
  "${PROJECT_DIR}/scripts/prefetch-appimage-tools.sh"
}

ensure_rust_target() {
  local triple="$1"
  refresh_path
  if rustup target list --installed | grep -qx "${triple}"; then
    return 0
  fi
  log "Adding Rust target ${triple}..."
  rustup target add "${triple}"
}

ensure_windows_cross_from_linux() {
  [[ "$(host_os)" == "linux" ]] || return 0
  apt_install_missing nsis llvm clang lld gcc-mingw-w64
  have makensis || die "NSIS (makensis) is missing after package install"
  ensure_rust_target x86_64-pc-windows-msvc
  if ! have cargo-xwin; then
    log "Installing cargo-xwin ${CARGO_XWIN_VERSION} (compatible with rustc ${RUST_VERSION})..."
    cargo install cargo-xwin --locked --version "${CARGO_XWIN_VERSION}"
    refresh_path
  fi
  if have cargo-xwin; then
    WINDOWS_TARGET="x86_64-pc-windows-msvc"
    # On a Unix host, Tauri selects NSIS from the Windows target. Passing
    # `--bundles nsis` is rejected by the host CLI before target detection.
    WINDOWS_TAURI_ARGS=(--runner cargo-xwin --target x86_64-pc-windows-msvc)
    # cargo-xwin 0.19.2 defaults to x86_64,aarch64 and downloads the CRT with
    # ureq. Pin the arch we actually package and prefetch with curl first.
    export XWIN_ARCH="${XWIN_ARCH:-x86_64}"
    export XWIN_VERSION="${XWIN_VERSION:-16}"
    export XWIN_ACCEPT_LICENSE=1
    # Parallel ureq GETs against Microsoft CDNs often end mid-body
    # (Jake-Shadle/xwin#141). One download thread plus curl prefetch
    # is the combination that actually finishes on Linux.
    export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-1}"
    log "Prefetching MSVC CRT payloads for ${XWIN_ARCH} with curl..."
    "${PROJECT_DIR}/scripts/prefetch-msvc-crt.sh"
    log "Windows cross-compile toolchain (cargo-xwin) is ready"
    return 0
  fi
  ensure_rust_target x86_64-pc-windows-gnu
  have x86_64-w64-mingw32-gcc || die "MinGW-w64 is missing after package install"
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
  WINDOWS_TARGET="x86_64-pc-windows-gnu"
  WINDOWS_TAURI_ARGS=(--target x86_64-pc-windows-gnu)
  log "Windows cross-compile toolchain (MinGW-w64) is ready"
}

copy_one() {
  local source="$1"
  local destination="$2"
  [[ -f "${source}" ]] || die "expected artifact is missing: ${source}"
  command mkdir -p -- "$(dirname -- "${destination}")"
  command cp -f -- "${source}" "${destination}"
  log "wrote ${destination}"
}

tauri_signing_config_args() {
  TAURI_SIGNING_CONFIG_ARGS=()
  if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
    return 0
  fi
  log "TAURI_SIGNING_PRIVATE_KEY is unset; building unsigned local packages"
  TAURI_SIGNING_CONFIG_ARGS=(--config '{"bundle":{"createUpdaterArtifacts":false}}')
}

linux_binary_path() {
  command printf '%s/release/%s\n' "${TARGET_DIR}" "${CRATE_BIN}"
}

linux_deb_path() {
  command printf '%s/release/bundle/deb/%s\n' "${TARGET_DIR}" "$(linux_deb_name)"
}

linux_appimage_path() {
  command printf '%s/release/bundle/appimage/%s\n' "${TARGET_DIR}" "$(linux_appimage_name)"
}

windows_prefix() {
  local triple="${1:-}"
  if [[ -n "${triple}" ]]; then
    command printf '%s/%s/release\n' "${TARGET_DIR}" "${triple}"
  else
    command printf '%s/release\n' "${TARGET_DIR}"
  fi
}

frontend_dist_ready() {
  [[ -f "${PROJECT_DIR}/apps/desktop/dist/index.html" ]]
}

stage_index() {
  local needle="$1"
  shift
  local i=0
  local item
  for item in "$@"; do
    if [[ "${item}" == "${needle}" ]]; then
      command printf '%s\n' "${i}"
      return 0
    fi
    i=$((i + 1))
  done
  return 1
}

stamp_dir() {
  command printf '%s/biflow-build\n' "${TARGET_DIR}"
}

stamp_file() {
  command printf '%s/%s.stamp\n' "$(stamp_dir)" "$1"
}

write_stamp() {
  command mkdir -p -- "$(stamp_dir)"
  command printf '%s\n' "${BUILD_VERSION}" > "$(stamp_file "$1")"
}

stamp_is_current() {
  local file
  file="$(stamp_file "$1")"
  [[ -f "${file}" ]] || return 1
  [[ "$(command cat "${file}")" == "${BUILD_VERSION}" ]]
}

linux_bundle_selected() {
  [[ "${LINUX_BUNDLES}" == "all" || "${LINUX_BUNDLES}" == "$1" ]]
}

linux_stage_done() {
  local stage="$1"
  local deb
  case "${stage}" in
    compile)
      [[ -x "$(linux_binary_path)" ]] || return 1
      stamp_is_current linux-compile && return 0
      linux_stage_done deb && return 0
      linux_stage_done appimage
      ;;
    deb)
      deb="$(linux_deb_path)"
      [[ -f "${deb}" ]] || return 1
      [[ "$(dpkg-deb -f "${deb}" Version)" == "${BUILD_VERSION}" ]]
      ;;
    appimage)
      [[ -s "$(linux_appimage_path)" ]]
      ;;
    collect)
      if linux_bundle_selected deb; then
        [[ -f "${PROJECT_DIR}/$(plan linux.dir)/$(linux_deb_name)" ]] || return 1
      fi
      if linux_bundle_selected appimage; then
        [[ -s "${PROJECT_DIR}/$(plan linux.dir)/$(linux_appimage_name)" ]] || return 1
      fi
      ;;
    *) return 1 ;;
  esac
}

windows_stage_done() {
  local stage="$1"
  local prefix
  prefix="$(windows_prefix "${WINDOWS_TARGET}")"
  case "${stage}" in
    compile)
      [[ -f "${prefix}/${CRATE_BIN}.exe" ]] || return 1
      stamp_is_current windows-compile && return 0
      windows_stage_done nsis
      ;;
    nsis)
      [[ -f "${prefix}/bundle/nsis/$(windows_installer_name)" ]]
      ;;
    collect)
      [[ -f "${PROJECT_DIR}/$(plan windows.dir)/$(plan windows.exe)" ]] && \
        [[ -f "${PROJECT_DIR}/$(plan windows.dir)/$(windows_installer_name)" ]]
      ;;
    *) return 1 ;;
  esac
}

should_run_linux_stage() {
  local stage="$1"
  local stages=(compile deb appimage collect)
  local from_i stage_i
  if [[ "${FORCE}" -eq 1 ]]; then
    return 0
  fi
  if [[ -n "${FROM_STAGE}" ]]; then
    if from_i="$(stage_index "${FROM_STAGE}" "${stages[@]}")"; then
      stage_i="$(stage_index "${stage}" "${stages[@]}")"
      [[ "${stage_i}" -ge "${from_i}" ]]
      return
    fi
  fi
  if linux_stage_done "${stage}"; then
    return 1
  fi
  return 0
}

should_run_windows_stage() {
  local stage="$1"
  local stages=(compile nsis collect)
  local from_i stage_i
  if [[ "${FORCE}" -eq 1 ]]; then
    return 0
  fi
  if [[ -n "${FROM_STAGE}" ]]; then
    if from_i="$(stage_index "${FROM_STAGE}" "${stages[@]}")"; then
      stage_i="$(stage_index "${stage}" "${stages[@]}")"
      [[ "${stage_i}" -ge "${from_i}" ]]
      return
    fi
  fi
  if windows_stage_done "${stage}"; then
    return 1
  fi
  return 0
}

run_tauri_build() {
  local frontend_mode="$1"
  shift
  local configs=("${TAURI_SIGNING_CONFIG_ARGS[@]}")
  if [[ "${frontend_mode}" == "skip-frontend" ]]; then
    frontend_dist_ready || \
      die "cannot skip the frontend build: apps/desktop/dist/index.html is missing"
    configs+=(--config '{"build":{"beforeBuildCommand":""}}')
  fi
  (cd -- "${PROJECT_DIR}" && pnpm tauri build "$@" "${configs[@]}")
}

run_windows_cross_tauri_build() {
  local attempt=1
  local max_attempts=5
  local log_file status
  log_file="$(command mktemp)"
  while true; do
    set +e
    run_tauri_build "$@" >"${log_file}" 2>&1
    status=$?
    set -e
    command cat -- "${log_file}"
    if [[ "${status}" -eq 0 ]]; then
      command rm -f -- "${log_file}"
      return 0
    fi
    if ! command grep -Eq 'Failed to setup MSVC CRT|unexpected end of file' "${log_file}"; then
      command rm -f -- "${log_file}"
      die "Windows cross-compile failed"
    fi
    if [[ "${attempt}" -ge "${max_attempts}" ]]; then
      command rm -f -- "${log_file}"
      die "Windows cross-compile failed after ${max_attempts} attempts (MSVC CRT download)"
    fi
    log "Windows cross-compile failed on MSVC CRT/CDN; prefetching and retrying (${attempt}/${max_attempts})..."
    "${PROJECT_DIR}/scripts/prefetch-msvc-crt.sh"
    attempt=$((attempt + 1))
  done
}

validate_from_stage() {
  local target="$1"
  [[ -n "${FROM_STAGE}" ]] || return 0
  case "${target}" in
    linux)
      case "${FROM_STAGE}" in
        compile|deb|appimage|collect) ;;
        *) die "linux --from must be compile, deb, appimage, or collect" ;;
      esac
      ;;
    windows)
      case "${FROM_STAGE}" in
        compile|nsis|collect) ;;
        *) die "windows --from must be compile, nsis, or collect" ;;
      esac
      ;;
    all)
      case "${FROM_STAGE}" in
        compile|deb|appimage|nsis|collect) ;;
        *) die "--from must be compile, deb, appimage, nsis, or collect" ;;
      esac
      ;;
  esac
}

collect_linux() {
  assert_build_version
  local source dest package_version
  if linux_bundle_selected deb; then
    source="${TARGET_DIR}/release/bundle/deb/$(linux_deb_name)"
    [[ -f "${source}" ]] || die "expected Linux package is missing: ${source}"
    package_version="$(dpkg-deb -f "${source}" Version)"
    [[ "${package_version}" == "${BUILD_VERSION}" ]] || \
      die "Linux package version mismatch: expected ${BUILD_VERSION}, got ${package_version}"
    dest="${PROJECT_DIR}/$(plan linux.dir)/$(linux_deb_name)"
    copy_one "${source}" "${dest}"
  fi
  if linux_bundle_selected appimage; then
    source="${TARGET_DIR}/release/bundle/appimage/$(linux_appimage_name)"
    dest="${PROJECT_DIR}/$(plan linux.dir)/$(linux_appimage_name)"
    copy_one "${source}" "${dest}"
  fi
}

collect_windows() {
  assert_build_version
  local triple="${1:-}"
  local prefix="${TARGET_DIR}/release"
  if [[ -n "${triple}" ]]; then
    prefix="${TARGET_DIR}/${triple}/release"
  fi
  local exe setup dest_exe dest_setup
  exe="${prefix}/${CRATE_BIN}.exe"
  setup="${prefix}/bundle/nsis/$(windows_installer_name)"
  dest_exe="${PROJECT_DIR}/$(plan windows.dir)/$(plan windows.exe)"
  dest_setup="${PROJECT_DIR}/$(plan windows.dir)/$(windows_installer_name)"
  copy_one "${exe}" "${dest_exe}"
  copy_one "${setup}" "${dest_setup}"
  local dest_dir
  dest_dir="$(dirname "${dest_exe}")"
  mkdir -p "${dest_dir}/rules" "${dest_dir}/dependencies"
  cp -a "${PROJECT_DIR}/resources/rules/." "${dest_dir}/rules/"
  if [[ -f "${PROJECT_DIR}/vendor/mihomo/windows-x86_64/mihomo.exe" ]]; then
    cp -a "${PROJECT_DIR}/vendor/mihomo/windows-x86_64/mihomo.exe" "${dest_dir}/dependencies/mihomo.exe"
  fi
  if [[ -f "${PROJECT_DIR}/vendor/wintun/windows-x86_64/wintun.dll" ]]; then
    cp -a "${PROJECT_DIR}/vendor/wintun/windows-x86_64/wintun.dll" "${dest_dir}/dependencies/wintun.dll"
  fi
  if [[ -f "${PROJECT_DIR}/packaging/staged/iran-split-helper.exe" ]]; then
    mkdir -p "${dest_dir}/helper"
    cp -a "${PROJECT_DIR}/packaging/staged/iran-split-helper.exe" "${dest_dir}/helper/iran-split-helper.exe"
  fi
}

build_linux() {
  [[ "$(host_os)" == "linux" ]] || die "Linux .deb packages must be built on Linux"
  assert_build_version
  tauri_signing_config_args
  local frontend_mode="run-frontend"
  if frontend_dist_ready && ! should_run_linux_stage compile; then
    frontend_mode="skip-frontend"
  fi
  if should_run_linux_stage compile; then
    log "Stage compile: release binary for BiFlow ${BUILD_VERSION}"
    "${PROJECT_DIR}/scripts/stage-helper.sh"
    run_tauri_build run-frontend --no-bundle
    [[ -x "$(linux_binary_path)" ]] || die "compile did not produce $(linux_binary_path)"
    write_stamp linux-compile
    frontend_mode="skip-frontend"
  else
    log "Skipping compile; already have $(linux_binary_path)"
  fi
  if ! linux_bundle_selected deb; then
    log "Skipping deb; only the ${LINUX_BUNDLES} bundle was requested"
  elif should_run_linux_stage deb; then
    log "Stage deb: $(linux_deb_name)"
    [[ -x "${PROJECT_DIR}/packaging/staged/iran-split-helper" ]] || \
      "${PROJECT_DIR}/scripts/stage-helper.sh"
    run_tauri_build "${frontend_mode}" --bundles deb
    write_stamp linux-deb
    frontend_mode="skip-frontend"
  else
    log "Skipping deb; already have $(linux_deb_path)"
  fi
  if ! linux_bundle_selected appimage; then
    log "Skipping appimage; only the ${LINUX_BUNDLES} bundle was requested"
  elif should_run_linux_stage appimage; then
    [[ -x "$(linux_binary_path)" ]] || die "stage appimage needs a compiled binary; run ./build.sh linux --from compile"
    log "Stage appimage: $(linux_appimage_name)"
    [[ -x "${PROJECT_DIR}/packaging/staged/iran-split-helper" ]] || \
      "${PROJECT_DIR}/scripts/stage-helper.sh"
    ensure_tauri_appimage_tools
    run_tauri_build "${frontend_mode}" --bundles appimage
    write_stamp linux-appimage
  else
    log "Skipping appimage; already have $(linux_appimage_path)"
  fi
  if should_run_linux_stage collect; then
    log "Stage collect: copying Linux artifacts"
    collect_linux
    write_stamp linux-collect
  else
    log "Skipping collect; Linux artifacts already in $(plan linux.dir)"
  fi
}

build_windows() {
  local os
  os="$(host_os)"
  if [[ "${os}" == "windows" ]]; then
    WINDOWS_TARGET=""
    WINDOWS_TAURI_ARGS=(--bundles nsis)
    log "Building Windows .exe and NSIS installer for BiFlow ${BUILD_VERSION}"
  elif [[ "${os}" == "linux" ]]; then
    [[ -n "${WINDOWS_TARGET:-}" ]] || die "Windows cross-compile toolchain was not prepared"
    log "Cross-compiling Windows .exe and NSIS installer for BiFlow ${BUILD_VERSION} (${WINDOWS_TARGET})"
  else
    die "Windows packages require Windows or Linux"
  fi
  tauri_signing_config_args
  local frontend_mode="run-frontend"
  local prefix compile_args
  prefix="$(windows_prefix "${WINDOWS_TARGET}")"
  if frontend_dist_ready && ! should_run_windows_stage compile; then
    frontend_mode="skip-frontend"
  fi
  if should_run_windows_stage compile; then
    log "Stage compile: Windows app for BiFlow ${BUILD_VERSION}"
    if [[ "${os}" == "linux" ]]; then
      STAGE_HELPER_TARGET="${WINDOWS_TARGET}" "${PROJECT_DIR}/scripts/stage-helper.sh" windows
    else
      "${PROJECT_DIR}/scripts/stage-helper.sh" windows
    fi
    compile_args=(--no-bundle)
    if [[ "${os}" == "linux" ]]; then
      compile_args+=("${WINDOWS_TAURI_ARGS[@]}")
      run_windows_cross_tauri_build run-frontend "${compile_args[@]}"
    else
      run_tauri_build run-frontend "${compile_args[@]}"
    fi
    write_stamp windows-compile
    frontend_mode="skip-frontend"
  else
    log "Skipping compile; already have ${prefix}/${CRATE_BIN}.exe"
  fi
  if should_run_windows_stage nsis; then
    log "Stage nsis: $(windows_installer_name)"
    if [[ ! -f "${PROJECT_DIR}/packaging/staged/iran-split-helper.exe" ]]; then
      if [[ "${os}" == "linux" ]]; then
        STAGE_HELPER_TARGET="${WINDOWS_TARGET}" "${PROJECT_DIR}/scripts/stage-helper.sh" windows
      else
        "${PROJECT_DIR}/scripts/stage-helper.sh" windows
      fi
    fi
    if [[ "${os}" == "linux" ]]; then
      run_windows_cross_tauri_build "${frontend_mode}" "${WINDOWS_TAURI_ARGS[@]}"
    else
      run_tauri_build "${frontend_mode}" "${WINDOWS_TAURI_ARGS[@]}"
    fi
    write_stamp windows-nsis
  else
    log "Skipping nsis; installer already built"
  fi
  if should_run_windows_stage collect; then
    log "Stage collect: copying Windows artifacts"
    collect_windows "${WINDOWS_TARGET}"
    write_stamp windows-collect
  else
    log "Skipping collect; Windows artifacts already in $(plan windows.dir)"
  fi
}

print_summary() {
  assert_build_version
  log ""
  log "BiFlow ${BUILD_VERSION} artifacts:"
  if [[ -f "${PROJECT_DIR}/$(plan linux.dir)/$(linux_deb_name)" ]]; then
    log "  Linux deb:          $(plan linux.dir)/$(linux_deb_name)"
  fi
  if [[ -f "${PROJECT_DIR}/$(plan linux.dir)/$(linux_appimage_name)" ]]; then
    log "  Linux AppImage:     $(plan linux.dir)/$(linux_appimage_name)"
  fi
  if [[ -f "${PROJECT_DIR}/$(plan windows.dir)/$(plan windows.exe)" ]]; then
    log "  Windows app:        $(plan windows.dir)/$(plan windows.exe)"
  fi
  if [[ -f "${PROJECT_DIR}/$(plan windows.dir)/$(windows_installer_name)" ]]; then
    log "  Windows installer:  $(plan windows.dir)/$(windows_installer_name)"
  fi
}

ensure_requirements() {
  local target="$1"
  log "Checking build requirements..."
  refresh_path
  ensure_node
  ensure_pnpm
  ensure_node_modules
  ensure_rust
  case "${target}" in
    check-frontend|check-rust)
      ;;
    linux|ci-linux)
      ensure_linux_desktop_dependencies
      ;;
    windows|ci-windows)
      if [[ "$(host_os)" == "linux" ]]; then
        ensure_windows_cross_from_linux
      fi
      ;;
    all)
      ensure_linux_desktop_dependencies
      if [[ "$(host_os)" == "linux" ]]; then
        ensure_windows_cross_from_linux
      fi
      ;;
  esac
  log "All build requirements are ready"
}

check_frontend() {
  log "Running frontend done gate..."
  (cd -- "${PROJECT_DIR}" && pnpm version:sync)
  (cd -- "${PROJECT_DIR}" && pnpm check)
  (cd -- "${PROJECT_DIR}" && pnpm build)
  log "Frontend check passed"
}

check_rust() {
  shift
  [[ "$#" -gt 0 ]] || die "check-rust requires at least one workspace crate name"
  refresh_path
  ensure_rust
  local crate
  for crate in "$@"; do
    log "Testing crate ${crate}..."
    (cd -- "${PROJECT_DIR}" && cargo test -p "${crate}")
    log "Clippy for crate ${crate}..."
    (cd -- "${PROJECT_DIR}" && cargo clippy -p "${crate}" --all-targets -- -D warnings)
  done
  log "Rust check passed for: $*"
}

main() {
  local target="all"
  local -a rest=()
  FROM_STAGE=""
  FORCE=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --from)
        [[ $# -ge 2 ]] || die "--from requires a stage name"
        FROM_STAGE="$(command printf '%s' "$2" | command tr '[:upper:]' '[:lower:]')"
        shift 2
        ;;
      --force)
        FORCE=1
        shift
        ;;
      -h|--help|help)
        usage
        return 0
        ;;
      --)
        shift
        rest+=("$@")
        break
        ;;
      -*)
        die "unknown option: $1"
        ;;
      *)
        rest+=("$1")
        shift
        ;;
    esac
  done
  if [[ "${#rest[@]}" -gt 0 ]]; then
    target="${rest[0]}"
  fi

  case "${target}" in
    check-frontend)
      ensure_requirements "${target}"
      check_frontend
      return 0
      ;;
    check-rust)
      ensure_requirements "${target}"
      check_rust "${rest[@]}"
      return 0
      ;;
    ci-linux)
      [[ "$(host_os)" == "linux" ]] || die "ci-linux requires a native Linux runner"
      target="linux"
      ;;
    ci-windows)
      [[ "$(host_os)" == "windows" ]] || die "ci-windows requires a native Windows runner"
      target="windows"
      ;;
    linux|windows|all) ;;
    *) usage >&2; die "unknown target: ${target}" ;;
  esac

  if [[ "${target}" == "linux" && "${#rest[@]}" -gt 1 ]]; then
    case "${rest[1]}" in
      deb|appimage) LINUX_BUNDLES="${rest[1]}" ;;
      *) usage >&2; die "linux bundle must be deb or appimage, got: ${rest[1]}" ;;
    esac
  elif [[ "${#rest[@]}" -gt 1 ]]; then
    usage >&2
    die "unexpected extra argument: ${rest[1]}"
  fi

  validate_from_stage "${target}"
  ensure_requirements "${target}"
  (cd -- "${PROJECT_DIR}" && pnpm version:sync)
  BUILD_VERSION="$(plan version)"
  assert_build_version

  case "${target}" in
    linux) build_linux ;;
    windows) build_windows ;;
    all)
      build_linux
      build_windows
      ;;
  esac
  print_summary
}

main "$@"
