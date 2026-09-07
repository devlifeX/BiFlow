import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function rustSources(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (
      entry.name === "target" ||
      entry.name === "node_modules" ||
      entry.name === ".git"
    ) {
      continue;
    }
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...rustSources(path));
    } else if (entry.name.endsWith(".rs")) {
      files.push(path);
    }
  }
  return files;
}

function requiredPlatformBackendMethods(traitSource) {
  return traitSource
    .split("async fn ")
    .slice(1)
    .filter((chunk) => {
      const semicolon = chunk.indexOf(";");
      const brace = chunk.indexOf("{");
      return semicolon !== -1 && (brace === -1 || semicolon < brace);
    })
    .map((chunk) => /^(\w+)/.exec(chunk)[1]);
}

describe("Tauri frontend contract", () => {
  it("registers every Rust command invoked by the production UI", () => {
    const frontend = readFileSync(
      join(root, "apps/desktop/src/api/desktop.ts"),
      "utf8",
    );
    const rust = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
    const invoked = [...frontend.matchAll(/\binvoke\("([a-z0-9_]+)"/g)].map(
      (match) => match[1],
    );
    const handlerBlock = rust.match(/tauri::generate_handler!\[([\s\S]*?)\]\)/);

    assert.ok(handlerBlock, "Rust invoke handler registration is missing");
    const registered = handlerBlock[1]
      .split(",")
      .map((command) => command.trim())
      .filter(Boolean);
    const missing = [...new Set(invoked)].filter(
      (command) => !registered.includes(command),
    );

    assert.deepEqual(missing, []);
    assert.match(frontend, /window\.__TAURI_INTERNALS__ !== undefined/);
    assert.match(frontend, /listen<StackSnapshot>\("stack-snapshot"/);
    assert.match(rust, /emit\("stack-snapshot"/);
    assert.match(rust, /emit\("update-progress"/);
  });

  it("picks side-tunnel profiles through the Tauri dialog plugin", () => {
    const rust = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
    const cargo = readFileSync(join(root, "src-tauri/Cargo.toml"), "utf8");
    const frontend = readFileSync(
      join(root, "apps/desktop/src/api/desktop.ts"),
      "utf8",
    );
    const picker = readFileSync(
      join(root, "src-tauri/src/profile_picker.rs"),
      "utf8",
    );
    assert.match(cargo, /tauri-plugin-dialog/);
    assert.match(rust, /tauri_plugin_dialog::init\(\)/);
    assert.match(rust, /fn pick_client_profile/);
    assert.match(frontend, /invoke\("pick_client_profile"\)/);
    assert.match(
      picker,
      /add_filter\("VPN profile", accepted_profile_extensions\(\)\)/,
    );
    assert.doesNotMatch(picker, /info!\(\s*path\b/);
    assert.doesNotMatch(picker, /blocking_pick_file/);
    assert.match(picker, /\.pick_file\(/);
    assert.match(rust, /async fn pick_client_profile/);
    assert.match(frontend, /invoke\("apply_live_settings"\)/);
    assert.match(rust, /async fn apply_live_settings/);
  });

  it("reads the native clipboard through the Tauri plugin", () => {
    const rust = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
    const cargo = readFileSync(join(root, "src-tauri/Cargo.toml"), "utf8");
    const capabilities = JSON.parse(
      readFileSync(join(root, "src-tauri/capabilities/main.json"), "utf8"),
    );
    assert.match(cargo, /tauri-plugin-clipboard-manager/);
    assert.match(rust, /tauri_plugin_clipboard_manager::init\(\)/);
    assert.ok(
      capabilities.permissions.includes("clipboard-manager:allow-read-text"),
    );
    assert.ok(
      capabilities.permissions.includes("clipboard-manager:allow-write-text"),
    );
  });

  it("points NSIS helper staging at ProgramData iran-split staging", () => {
    const hook = readFileSync(
      join(root, "packaging/windows/installer-hooks.nsh"),
      "utf8",
    );
    const desktop = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
    const install = readFileSync(
      join(root, "src-tauri/src/helper_install.rs"),
      "utf8",
    );
    const helper = readFileSync(
      join(root, "crates/iran-split-helper/src/windows.rs"),
      "utf8",
    );
    assert.match(hook, /\$PROGRAMDATA\\iran-split\\staging/);
    assert.doesNotMatch(hook, /\$LOCALAPPDATA\\biflow\\runtime\\generations/);
    assert.ok(install.includes(String.raw`C:\ProgramData\iran-split\staging`));
    assert.match(desktop, /generation_staging_dir/);
    assert.match(helper, /root\.join\("staging"\)/);
    assert.match(helper, /fn grant_users_modify/);
    const windows = readFileSync(
      join(root, "crates/iran-split-platform-win/src/lib.rs"),
      "utf8",
    );
    // `#![cfg(windows)]` hides this crate from Linux `cargo test`. Scan the
    // production half so a source-contract `!contains` cannot match itself.
    const production = windows.split("mod tests {")[0];
    assert.match(production, /\.generation_staging_dir/);
    assert.doesNotMatch(
      production,
      /user_data_dir\.join\("runtime"\)\.join\("generations"\)/,
    );
  });

  it("keeps a resizable main window with a 390x640 minimum", () => {
    const config = JSON.parse(
      readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"),
    );
    const window = config.app.windows[0];
    assert.equal(window.width, 1120);
    assert.equal(window.height, 760);
    assert.equal(window.minWidth, 390);
    assert.equal(window.minHeight, 640);
    assert.equal(window.maxWidth, undefined);
    assert.equal(window.maxHeight, undefined);
    assert.equal(window.resizable, true);
    const rust = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
    assert.match(rust, /restore_main_window_size/);
    assert.match(rust, /persist_main_window_size/);
  });

  it("assigns the default application icon to the tray builder", () => {
    const rust = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
    assert.match(rust, /default_window_icon\(\)/);
    assert.match(
      rust,
      /TrayIconBuilder::with_id\("main"\)[\s\S]*?\.icon\(icon\)[\s\S]*?\.menu\(&menu\)/,
    );
    assert.match(rust, /MenuItem::with_id\(\s*app,\s*"dashboard"/);
    assert.match(rust, /app.emit\("app-navigate", "dashboard"\)/);
    assert.match(rust, /apply_tray_menu/);
    const tray = readFileSync(join(root, "src-tauri/src/tray.rs"), "utf8");
    assert.match(tray, /fn labels_for/);
    assert.match(tray, /fn actions_enabled/);
    assert.match(rust, /reserve_lifecycle/);
  });

  it("disables WebKitGTK DMA-BUF rendering before the Linux webview starts", () => {
    const rust = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
    assert.match(rust, /WEBKIT_DISABLE_DMABUF_RENDERER/);
    assert.match(rust, /WEBKIT_DISABLE_COMPOSITING_MODE/);
    assert.match(rust, /LIBGL_ALWAYS_SOFTWARE/);
    assert.match(rust, /apply_linux_webview_workarounds\(\)/);
    assert.match(rust, /BIFLOW_WEBKIT_WORKAROUNDS/);
    const applyFn = rust.match(
      /fn apply_linux_webview_workarounds\(\) \{[\s\S]*?\n\}/,
    );
    assert.ok(applyFn, "apply_linux_webview_workarounds is missing");
    assert.match(applyFn[0], /command\.exec\(\)/);
    assert.doesNotMatch(applyFn[0], /\.status\(\)/);
    assert.match(rust, /current_exe\(\)/);
    assert.match(rust, /single_instance_dbus_id/);
    assert.match(rust, /dbus_id\(single_instance_dbus_id/);
    const run = rust.match(/pub fn run\(\) \{([\s\S]*?)let builder =/);
    assert.ok(run, "run() is missing");
    const apply = run[1].indexOf("apply_linux_webview_workarounds");
    const diagnostics = run[1].indexOf("initialize_diagnostics");
    assert.ok(
      apply >= 0 && diagnostics > apply,
      "WebKit env must be set before diagnostics and GTK start",
    );
  });

  it("does not attach a console or leftover terminal when the GUI starts", () => {
    const desktopMain = readFileSync(
      join(root, "src-tauri/src/main.rs"),
      "utf8",
    );
    const helperMain = readFileSync(
      join(root, "crates/iran-split-helper/src/main.rs"),
      "utf8",
    );
    const config = JSON.parse(
      readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"),
    );
    const desktop = readFileSync(
      join(root, "packaging/linux/app.desktop"),
      "utf8",
    );
    assert.match(
      desktopMain,
      /#!\[cfg_attr\(windows, windows_subsystem = "windows"\)\]/,
    );
    assert.match(
      helperMain,
      /#!\[cfg_attr\(windows, windows_subsystem = "windows"\)\]/,
    );
    assert.equal(
      config.bundle.linux.deb.desktopTemplate,
      "../packaging/linux/app.desktop",
    );
    assert.match(desktop, /^Terminal=false$/m);
  });

  it("commits the current updater public key and verifies signing via the Tauri CLI", () => {
    const config = JSON.parse(
      readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"),
    );
    const prepare = readFileSync(
      join(root, "scripts/prepare-tauri-signing.mjs"),
      "utf8",
    );
    const decoded = Buffer.from(
      config.plugins.updater.pubkey,
      "base64",
    ).toString("utf8");
    assert.match(
      decoded,
      /untrusted comment: minisign public key: 4D42AC0D9C21345C/,
    );
    assert.match(prepare, /node_modules\/@tauri-apps\/cli\/tauri\.js/);
    assert.doesNotMatch(prepare, /spawn\("pnpm"/);
  });

  it("passes Win32 security-descriptor out-params as raw pointers", () => {
    const source = readFileSync(
      join(root, "crates/iran-split-helper-winacl/src/windows_impl.rs"),
      "utf8",
    );
    assert.match(source, /&raw mut descriptor/);
    assert.doesNotMatch(
      source,
      /ConvertStringSecurityDescriptorToSecurityDescriptorW\([\s\S]*?&mut descriptor/,
    );
  });

  it("keeps Unix-only OpenOptions inside the Unix sync_directory function", () => {
    const config = readFileSync(
      join(root, "crates/iran-split-config/src/lib.rs"),
      "utf8",
    );
    assert.doesNotMatch(config, /use std::fs::OpenOptions/);
    assert.doesNotMatch(config, /fs::\{self, OpenOptions\}/);
    assert.match(config, /fs::OpenOptions::new\(\)/);
  });

  it("keeps both platform backends staging the same runtime generation", () => {
    const linux = readFileSync(
      join(root, "crates/iran-split-platform-linux/src/lib.rs"),
      "utf8",
    );
    const windows = readFileSync(
      join(root, "crates/iran-split-platform-win/src/lib.rs"),
      "utf8",
    );
    // The staging helpers are duplicated per platform (ADR 0033). A rule file
    // added on one side and not the other ships a config Mihomo cannot load.
    const providers = [
      "private.txt",
      "iran-domains.txt",
      "iran-networks.txt",
      "iran-business-domains.txt",
      "iran-cdn-networks.txt",
      "custom-direct-domains.txt",
      "custom-direct-ips.txt",
      "config.yaml",
    ];
    for (const name of providers) {
      assert.ok(linux.includes(name), `${name} missing from the Linux backend`);
      assert.ok(
        windows.includes(name),
        `${name} missing from the Windows backend`,
      );
    }
    assert.ok(
      linux.includes("provider_files"),
      "Linux backend must emit per-client provider files",
    );
    assert.ok(
      windows.includes("provider_files"),
      "Windows backend must emit per-client provider files",
    );
    // Each backend must pin its own Platform value or the generated config
    // silently loses the Windows-only strict-route flag.
    assert.match(
      linux,
      /generate_config_with_handles\(\s*&config,\s*Platform::Linux,/,
    );
    assert.match(
      windows,
      /generate_config_with_handles\(\s*&config,\s*Platform::Windows,/,
    );
    // A readiness timeout used to be CoreError::Platform, which the UI maps
    // to "An internal error occurred." Keep both backends on the typed error.
    assert.match(linux, /fn readiness_error/);
    assert.match(windows, /fn readiness_error/);
    assert.match(linux, /CoreError::ControllerTimeout/);
    assert.match(windows, /CoreError::ControllerTimeout/);
    assert.match(linux, /CoreError::ControllerUnauthorized/);
    assert.match(windows, /CoreError::ControllerUnauthorized/);
    // Both compile only for their own OS, so host Clippy never sees the other.
    assert.match(linux, /^#!\[cfg\(target_os = "linux"\)\]/m);
    assert.match(windows, /^#!\[cfg\(windows\)\]/m);
  });

  it("implements every PlatformBackend method on Windows", () => {
    const core = readFileSync(
      join(root, "crates/iran-split-core/src/lib.rs"),
      "utf8",
    );
    const windows = readFileSync(
      join(root, "crates/iran-split-platform-win/src/lib.rs"),
      "utf8",
    );
    const trait = core.match(/pub trait PlatformBackend[\s\S]*?\n\}/);
    assert.ok(trait, "PlatformBackend trait is missing");
    // A signature ending in `;` has no default body, so every implementor must
    // provide it. Methods with a default (`stop_user_proxy`,
    // `verify_not_intercepting`) are checked separately.
    const required = requiredPlatformBackendMethods(trait[0]);
    assert.ok(required.length > 8, "trait parse found too few methods");

    const impl = windows.match(
      /impl PlatformBackend for WindowsBackend \{[\s\S]*?\n\}\n/,
    );
    assert.ok(impl, "WindowsBackend does not implement PlatformBackend");
    const missing = required.filter(
      (name) => !new RegExp(`async fn ${name}\\(`).test(impl[0]),
    );
    assert.deepEqual(missing, [], "Windows backend is missing trait methods");
    // Defaulted in the trait, but a backend that never stops the user's proxy
    // leaves Hiddify running after Disconnect.
    assert.match(impl[0], /async fn stop_user_proxy\(/);
    assert.match(impl[0], /async fn clear_hiddify_system_proxy\(/);
    assert.match(impl[0], /async fn restore_hiddify_system_proxy\(/);
    // The pre-2.2.0 stub answered every call with one canned platform error.
    assert.doesNotMatch(windows, /fn unavailable<T>\(\)/);
  });

  it("implements every required PlatformBackend method on the CLI demo", () => {
    const core = readFileSync(
      join(root, "crates/iran-split-core/src/lib.rs"),
      "utf8",
    );
    const cli = readFileSync(
      join(root, "crates/iran-split-cli/src/main.rs"),
      "utf8",
    );
    const trait = core.match(/pub trait PlatformBackend[\s\S]*?\n\}/);
    assert.ok(trait, "PlatformBackend trait is missing");
    const required = requiredPlatformBackendMethods(trait[0]);
    const impl = cli.match(
      /impl PlatformBackend for DemoBackend \{[\s\S]*?\n\}\n/,
    );
    assert.ok(impl, "DemoBackend does not implement PlatformBackend");
    const missing = required.filter(
      (name) => !new RegExp(`async fn ${name}\\(`).test(impl[0]),
    );
    assert.deepEqual(missing, [], "CLI DemoBackend is missing trait methods");
  });

  it("checks GitHub Releases from About and never polls in the background", () => {
    const rust = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
    const githubUpdate = readFileSync(
      join(root, "src-tauri/src/github_update.rs"),
      "utf8",
    );

    assert.match(
      githubUpdate,
      /https:\/\/api\.github\.com\/repos\/devlifeX\/BiFlow\/releases\/latest/,
    );
    assert.match(
      rust,
      /async fn check_for_update\([\s\S]*?collect_update_status\(&app, "tauri_command"\)/,
    );
    assert.match(
      rust,
      /async fn collect_update_status\([\s\S]*?fetch_github_update\(app, initiator\)/,
    );
    assert.match(rust, /fn fetch_github_update\(/);
    assert.match(githubUpdate, /async fn download_asset\(/);
    assert.match(githubUpdate, /pkexec/);
    assert.match(githubUpdate, /apt-get/);
    assert.match(githubUpdate, /apply-update\.bat/);
    assert.match(rust, /\.try_begin\(/);
    assert.match(rust, /phase: "installed"/);
    assert.doesNotMatch(rust, /fn spawn_background_update_checks\(/);
    assert.doesNotMatch(rust, /fn fetch_signed_update\(/);
    assert.doesNotMatch(rust, /open_linux_deb_release/);
    assert.doesNotMatch(rust, /an update is already in progress/);
    assert.doesNotMatch(githubUpdate, /trace_route = .*browser_download_url/);
  });

  it("never reports a previous attempt's Windows install reason", () => {
    const source = readFileSync(
      join(root, "src-tauri/src/helper_install.rs"),
      "utf8",
    );
    // The elevated helper writes install.log only when it reaches its own
    // error path, so a stale file must be cleared before each elevation.
    const install = source.match(/async fn install_windows\([\s\S]*?\n\}\n/);
    assert.ok(install, "install_windows is missing");
    const cleared = install[0].indexOf("discard_stale_install_log()");
    const elevated = install[0].indexOf('Command::new("powershell")');
    assert.ok(cleared >= 0, "the stale install log is never cleared");
    assert.ok(
      elevated > cleared,
      "install.log must be cleared before the elevation runs",
    );
    assert.match(source, /fn discard_stale_install_log\(/);
  });

  it("gates Linux helper-install paths so Windows dead_code stays clean", () => {
    const source = readFileSync(
      join(root, "src-tauri/src/helper_install.rs"),
      "utf8",
    );
    assert.match(
      source,
      /#\[cfg\(target_os = "linux"\)\]\s*const LINUX_HELPER_ROOT/,
    );
    assert.match(
      source,
      /#\[cfg\(target_os = "linux"\)\]\s*fn helper_binary_candidates/,
    );
    assert.match(
      source,
      /#\[cfg\(target_os = "linux"\)\]\s*#\[must_use\]\s*pub\(crate\) fn parse_proc_status_ids/,
    );
    assert.match(
      source,
      /#\[cfg\(target_os = "linux"\)\]\s*use std::path::\{Path, PathBuf\};/,
    );
    assert.match(
      source,
      /#\[cfg\(target_os = "linux"\)\]\s*\{\s*let staging_dir[\s\S]*?let payload_dir = /,
    );
    assert.match(
      source,
      /#\[cfg\(target_os = "linux"\)\]\s*use std::os::unix::fs::PermissionsExt;/,
    );
  });

  it("stages the helper payload out of the AppImage FUSE mount for pkexec", () => {
    const source = readFileSync(
      join(root, "src-tauri/src/helper_install.rs"),
      "utf8",
    );
    // root cannot read /tmp/.mount_* without allow_other, so pkexec must be
    // handed copies rather than the paths inside the mount.
    assert.match(source, /\.arg\(&payload\.script\)/);
    assert.match(source, /\.arg\(&payload\.helper\)/);
    assert.match(source, /\.arg\(&payload\.mihomo\)/);
    assert.doesNotMatch(source, /\.arg\(&script\)/);
    assert.doesNotMatch(source, /\.arg\(&helper_src\)/);
    assert.doesNotMatch(source, /\.arg\(&mihomo_src\)/);
    // 126 is a dismissed polkit dialog; 127 is a real failure to execute.
    assert.match(source, /output\.status\.code\(\) == Some\(126\)/);
    assert.doesNotMatch(
      source,
      /Some\(126\)\s*\|\|\s*output\.status\.code\(\) == Some\(127\)/,
    );
    assert.match(source, /fs::Permissions::from_mode\(0o700\)/);
  });

  it("re-raises the elevated Windows exit code instead of PowerShell's", () => {
    const source = readFileSync(
      join(root, "src-tauri/src/helper_install.rs"),
      "utf8",
    );
    assert.match(source, /-Verb RunAs -Wait -PassThru/);
    assert.match(source, /exit \$process\.ExitCode/);
    assert.match(source, /\$ErrorActionPreference = 'Stop'/);
    assert.match(source, /const UAC_CANCELLED: i32 = 1223;/);
    // ADR 0030: the GUI is a windows-subsystem binary, so the elevation shell
    // must not flash a console over the UI.
    assert.match(source, /\.creation_flags\(CREATE_NO_WINDOW\)/);
    assert.match(source, /const CREATE_NO_WINDOW: u32 = 0x0800_0000;/);
    // Start-Process -Wait without -PassThru reports PowerShell's own status, so
    // a failed helper and a refused UAC prompt both looked like success.
    assert.doesNotMatch(source, /-Verb RunAs -Wait"/);
  });

  it("uses tail expressions in cfg blocks so host Clippy needless_return stays clean", () => {
    const cfgReturn = /#\[cfg\([^\]]+\)\]\s*\{\s*return /;
    const offenders = rustSources(root).filter((path) =>
      cfgReturn.test(readFileSync(path, "utf8")),
    );

    assert.deepEqual(
      offenders,
      [],
      "cfg blocks compiled by host Clippy must be tail expressions, not return statements",
    );
  });
});
