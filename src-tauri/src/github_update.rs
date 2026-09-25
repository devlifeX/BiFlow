//! GitHub Releases in-app updater, following the `DBack` About-page flow.
//!
//! Check uses the public Releases API. Install downloads the platform package
//! and applies it with `pkexec apt-get` (`.deb`), a replace helper (`AppImage`),
//! or an elevated NSIS installer (Windows). Do not log asset URLs.

use serde::Deserialize;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
use tracing::{info, warn};

pub const LATEST_RELEASE_API: &str = "https://api.github.com/repos/devlifeX/BiFlow/releases/latest";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Deb,
    AppImage,
    Nsis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag_name: String,
    pub version: String,
    pub notes: String,
    pub html_url: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub notes: String,
    pub asset: Option<Asset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Linux `.deb`: package is installed; the running process is still old.
    ManualRestart,
    /// A helper script will replace this process after exit.
    HelperRestart,
}

#[must_use]
pub fn normalize_version(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('v').trim_start_matches('V');
    if trimmed.is_empty() {
        return "0.0.0".into();
    }
    let mut parts: Vec<&str> = trimmed.split('.').take(3).collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    parts.join(".")
}

#[must_use]
pub fn compare_versions(left: &str, right: &str) -> i32 {
    let left_parts = normalize_version(left);
    let right_parts = normalize_version(right);
    let left_nums = version_parts(&left_parts);
    let right_nums = version_parts(&right_parts);
    for index in 0..3 {
        if left_nums[index] < right_nums[index] {
            return -1;
        }
        if left_nums[index] > right_nums[index] {
            return 1;
        }
    }
    0
}

#[must_use]
pub fn is_newer(current: &str, candidate: &str) -> bool {
    compare_versions(candidate, current) > 0
}

fn version_parts(normalized: &str) -> [u64; 3] {
    let mut parts = [0_u64; 3];
    for (index, piece) in normalized.split('.').take(3).enumerate() {
        parts[index] = piece.parse().unwrap_or(0);
    }
    parts
}

#[must_use]
pub fn user_agent(current_version: &str) -> String {
    format!("BiFlow/{}", normalize_version(current_version))
}

#[must_use]
pub fn install_kind_from(appimage: Option<&OsStr>, windows: bool) -> InstallKind {
    if windows {
        InstallKind::Nsis
    } else if appimage.is_some() {
        InstallKind::AppImage
    } else {
        InstallKind::Deb
    }
}

#[must_use]
pub fn detect_install_kind() -> InstallKind {
    install_kind_from(
        std::env::var_os("APPIMAGE").as_deref(),
        cfg!(target_os = "windows"),
    )
}

/// Picks the GitHub Release asset for this install kind.
///
/// # Errors
///
/// Returns an error when the release has no matching `.deb`, `AppImage`, or NSIS
/// installer.
pub fn pick_asset(release: &Release, kind: InstallKind) -> Result<Asset, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err("automatic updates are not available for this architecture".into());
    }
    let version = &release.version;
    let exact = match kind {
        InstallKind::Deb => format!("BiFlow_{version}_amd64.deb"),
        InstallKind::AppImage => format!("BiFlow_{version}_amd64.AppImage"),
        InstallKind::Nsis => format!("BiFlow_{version}_x64-setup.exe"),
    };
    release
        .assets
        .iter()
        .find(|asset| asset.name == exact && asset.size > 0)
        .cloned()
        .ok_or_else(|| {
            "no exact nonempty update package for this version and platform is attached to the latest GitHub Release".into()
        })
}

#[must_use]
pub const fn signed_target(kind: InstallKind) -> &'static str {
    match kind {
        InstallKind::Deb => "linux-deb-x86_64",
        InstallKind::AppImage => "linux-x86_64",
        InstallKind::Nsis => "windows-x86_64",
    }
}

/// Checks that the signed manifest identifies the exact GitHub asset selected
/// for this release. Signature verification of the bytes is done by Tauri's
/// updater before this package is written or the stack is paused.
///
/// # Errors
///
/// Returns an error when any release identity differs.
pub fn validate_signed_metadata(
    expected_version: &str,
    asset: &Asset,
    kind: InstallKind,
    signed_version: &str,
    signed_url: &str,
    target: &str,
) -> Result<(), String> {
    if signed_version != expected_version
        || signed_url != asset.url
        || target != signed_target(kind)
    {
        return Err("signed update manifest does not match the selected release package".into());
    }
    Ok(())
}

#[derive(Deserialize)]
struct GithubReleasePayload {
    tag_name: Option<String>,
    body: Option<String>,
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAssetPayload>,
}

#[derive(Deserialize)]
struct GithubAssetPayload {
    name: Option<String>,
    browser_download_url: Option<String>,
    #[serde(default)]
    size: u64,
}

/// Parses a GitHub Releases JSON body.
///
/// # Errors
///
/// Returns an error when JSON is invalid or `tag_name` is missing.
pub fn parse_release(body: &[u8]) -> Result<Release, String> {
    let payload: GithubReleasePayload =
        serde_json::from_slice(body).map_err(|error| error.to_string())?;
    let tag_name = payload.tag_name.unwrap_or_default();
    let raw_version = tag_name.strip_prefix('v').unwrap_or(&tag_name);
    let parsed_version = semver::Version::parse(raw_version)
        .map_err(|error| format!("github release tag is not a semantic version: {error}"))?;
    if !parsed_version.pre.is_empty() || !parsed_version.build.is_empty() {
        return Err("github release tag must be a stable X.Y.Z version".into());
    }
    let version = parsed_version.to_string();
    let notes = payload.body.unwrap_or_default();
    let notes = notes.trim();
    let notes = if notes.chars().count() > 300 {
        let clipped: String = notes.chars().take(300).collect();
        format!("{clipped}…")
    } else {
        notes.to_owned()
    };
    Ok(Release {
        tag_name,
        version,
        notes,
        html_url: payload.html_url.unwrap_or_default(),
        assets: payload
            .assets
            .into_iter()
            .filter_map(|asset| {
                Some(Asset {
                    name: asset.name?,
                    url: asset.browser_download_url?,
                    size: asset.size,
                })
            })
            .collect(),
    })
}

/// Fetches `/releases/latest` and decides whether `current_version` is behind.
///
/// # Errors
///
/// Returns a network, HTTP, or parse error. A newer tag without a platform
/// asset is also an error, matching `DBack`.
pub async fn check(
    current_version: &str,
    kind: InstallKind,
    timeout: std::time::Duration,
) -> Result<UpdateInfo, String> {
    let current_version = {
        let trimmed = current_version.trim();
        if trimmed.is_empty() {
            "0.0.0"
        } else {
            trimmed
        }
    };
    let release = fetch_latest(current_version, timeout).await?;
    let mut info = UpdateInfo {
        available: is_newer(current_version, &release.version),
        current_version: normalize_version(current_version),
        latest_version: release.version.clone(),
        notes: release.notes.clone(),
        asset: None,
    };
    if !info.available {
        return Ok(info);
    }
    info.asset = Some(pick_asset(&release, kind)?);
    Ok(info)
}

async fn fetch_latest(
    current_version: &str,
    timeout: std::time::Duration,
) -> Result<Release, String> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(user_agent(current_version))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "github releases API HTTP {status}: could not read the latest release"
        ));
    }
    parse_release(&bytes)
}

/// Applies a downloaded package. The caller must pause the stack first.
///
/// # Errors
///
/// Returns a platform error when `pkexec`, the helper script, or NSIS cannot
/// start.
pub async fn apply_package(
    kind: InstallKind,
    package: &Path,
    current_exe: &Path,
    expected_version: &str,
) -> Result<ApplyOutcome, String> {
    match kind {
        InstallKind::Deb => {
            install_deb(package).await?;
            Ok(ApplyOutcome::ManualRestart)
        }
        InstallKind::AppImage => {
            install_appimage(package).await?;
            Ok(ApplyOutcome::HelperRestart)
        }
        InstallKind::Nsis => {
            install_nsis(package, current_exe, expected_version).await?;
            Ok(ApplyOutcome::HelperRestart)
        }
    }
}

async fn install_deb(package: &Path) -> Result<(), String> {
    info!(
        event = "update.deb_install_started",
        section = "updates",
        initiator = "github_update",
        cause = "linux_deb",
        trace_route = "tauri_command->github_update->pkexec",
        "installing the downloaded .deb with pkexec apt-get"
    );
    let output = tokio::process::Command::new("pkexec")
        .args([
            "env",
            "DEBIAN_FRONTEND=noninteractive",
            "apt-get",
            "install",
            "-y",
        ])
        .arg(package)
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else if output.status.code() == Some(126) {
        Err("package install was cancelled".into())
    } else {
        Err("package install failed".into())
    }
}

async fn install_appimage(package: &Path) -> Result<(), String> {
    let Some(appimage) = std::env::var_os("APPIMAGE") else {
        return Err("APPIMAGE is not set".into());
    };
    let dest = PathBuf::from(appimage);
    let script = std::env::temp_dir()
        .join("biflow-update")
        .join("apply-update.sh");
    if let Some(parent) = script.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    let body = format!(
        "#!/bin/sh\nPID={}\nSRC={}\nDST={}\nwhile kill -0 \"$PID\" 2>/dev/null; do sleep 1; done\nmv -f \"$SRC\" \"$DST\"\nchmod +x \"$DST\"\nexec \"$DST\"\n",
        std::process::id(),
        sh_single_quote(&package.to_string_lossy()),
        sh_single_quote(&dest.to_string_lossy()),
    );
    tokio::fs::write(&script, body)
        .await
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = tokio::fs::metadata(&script)
            .await
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o700);
        tokio::fs::set_permissions(&script, permissions)
            .await
            .map_err(|error| error.to_string())?;
    }
    tokio::process::Command::new("sh")
        .arg(&script)
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn install_nsis(
    package: &Path,
    current_exe: &Path,
    expected_version: &str,
) -> Result<(), String> {
    let script = std::env::temp_dir()
        .join("biflow-update")
        .join("apply-update.ps1");
    let body = nsis_update_script(std::process::id(), package, current_exe, expected_version);
    if let Some(parent) = script.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    tokio::fs::write(&script, body)
        .await
        .map_err(|error| error.to_string())?;
    let mut command = tokio::process::Command::new("powershell");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]);
    command.arg(&script);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map_err(|error| error.to_string())?;
    warn!(
        event = "update.nsis_helper_started",
        section = "updates",
        initiator = "github_update",
        cause = "windows_nsis",
        trace_route = "tauri_command->github_update->apply_helper",
        "NSIS updater will verify elevation and installer status after this process exits"
    );
    Ok(())
}

fn nsis_update_script(
    process_id: u32,
    package: &Path,
    current_exe: &Path,
    expected_version: &str,
) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
    $processId = {process_id}\n\
    $installerPath = '{installer}'\n\
    $applicationPath = '{application}'\n\
    $expectedVersion = '{version}'\n\
    $deadline = [DateTime]::UtcNow.AddMinutes(3)\n\
    $failure = $null\n\
    try {{\n\
    while (Get-Process -Id $processId -ErrorAction SilentlyContinue) {{\n\
    if ([DateTime]::UtcNow -ge $deadline) {{ throw 'The previous BiFlow process did not exit within 3 minutes.' }}\n\
    Start-Sleep -Seconds 1\n\
    }}\n\
    $process = Start-Process -FilePath $installerPath -ArgumentList '/S' -Verb RunAs -Wait -PassThru -ErrorAction Stop\n\
    if ($process.ExitCode -ne 0) {{ throw \"The installer returned exit code $($process.ExitCode).\" }}\n\
    if (-not (Test-Path -LiteralPath $applicationPath -PathType Leaf)) {{ throw 'The installer reported success but BiFlow.exe is missing.' }}\n\
    $installedVersion = (Get-Item -LiteralPath $applicationPath).VersionInfo.ProductVersion\n\
    $versionMatch = [regex]::Match($installedVersion, '^(?<semver>\\d+\\.\\d+\\.\\d+)(?:\\.\\d+)?(?:\\s|$)')\n\
    if (-not $versionMatch.Success -or $versionMatch.Groups['semver'].Value -ne $expectedVersion) {{ throw \"Expected BiFlow $expectedVersion but found '$installedVersion'.\" }}\n\
    }} catch {{\n\
    $errorCode = $_.Exception.HResult -band 65535\n\
    if ($errorCode -eq 1223) {{\n\
    $failure = 'Administrator approval was cancelled. BiFlow was not updated.'\n\
    }} else {{\n\
    $failure = \"BiFlow could not complete the update: $($_.Exception.Message)\"\n\
    }}\n\
    try {{\n\
    Add-Type -AssemblyName System.Windows.Forms\n\
    [System.Windows.Forms.MessageBox]::Show($failure, 'BiFlow update failed', 'OK', 'Error') | Out-Null\n\
    }} catch {{}}\n\
    if (Test-Path -LiteralPath $applicationPath -PathType Leaf) {{ try {{ Start-Process -FilePath $applicationPath -ErrorAction Stop }} catch {{}} }}\n\
    exit 1\n\
    }}\n\
    try {{ Start-Process -FilePath $applicationPath -ErrorAction Stop }} catch {{\n\
    Add-Type -AssemblyName System.Windows.Forms\n\
    [System.Windows.Forms.MessageBox]::Show(\"BiFlow was updated to $expectedVersion but could not be started: $($_.Exception.Message)\", 'BiFlow update failed', 'OK', 'Error') | Out-Null\n\
    exit 1\n\
    }}\n",
        installer = powershell_single_quote(&package.to_string_lossy()),
        application = powershell_single_quote(&current_exe.to_string_lossy()),
        version = normalize_version(expected_version),
    )
}

fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sh_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_v_and_pads() {
        assert_eq!(normalize_version("v3.6"), "3.6.0");
        assert_eq!(normalize_version("3.5.0"), "3.5.0");
        assert_eq!(normalize_version(""), "0.0.0");
    }

    #[test]
    fn is_newer_requires_a_greater_triple() {
        assert!(is_newer("3.5.0", "3.6.0"));
        assert!(!is_newer("3.6.0", "3.6.0"));
        assert!(!is_newer("3.6.0", "3.5.9"));
    }

    #[test]
    fn parse_release_reads_tag_and_assets() {
        let release = parse_release(
            br#"{
              "tag_name": "v3.6.0",
              "body": "notes",
              "html_url": "https://github.com/devlifeX/BiFlow/releases/tag/v3.6.0",
              "assets": [
                {
                  "name": "BiFlow_3.6.0_amd64.deb",
                  "browser_download_url": "https://example.invalid/BiFlow_3.6.0_amd64.deb",
                  "size": 12
                }
              ]
            }"#,
        )
        .expect("release");
        assert_eq!(release.version, "3.6.0");
        assert!(release.html_url.contains("releases/tag"));
        let asset = pick_asset(&release, InstallKind::Deb).expect("deb");
        assert_eq!(asset.name, "BiFlow_3.6.0_amd64.deb");
    }

    #[test]
    fn release_tag_must_be_a_stable_semantic_version() {
        for tag in ["", "v3.6", "v3.6.0-rc.1", "v3.6.0+local", "v3.6.0-notes"] {
            let body = format!(r#"{{"tag_name":"{tag}","assets":[]}}"#);
            assert!(parse_release(body.as_bytes()).is_err(), "accepted {tag}");
        }
    }

    #[test]
    fn pick_asset_requires_exact_release_version_and_architecture() {
        let release = Release {
            tag_name: "v9.9.9".into(),
            version: "9.9.9".into(),
            notes: String::new(),
            html_url: String::new(),
            assets: vec![Asset {
                name: "BiFlow_9.9.8_x64-setup.exe".into(),
                url: "https://example.invalid/setup.exe".into(),
                size: 1,
            }],
        };
        assert!(pick_asset(&release, InstallKind::Deb).is_err());
        assert!(pick_asset(&release, InstallKind::Nsis).is_err());
        let mut release = release;
        release.assets.push(Asset {
            name: "BiFlow_9.9.9_arm64.deb".into(),
            url: "https://example.invalid/arm64.deb".into(),
            size: 1,
        });
        assert!(pick_asset(&release, InstallKind::Deb).is_err());
        release.assets.push(Asset {
            name: "BiFlow_9.9.9_x64-setup.exe".into(),
            url: "https://example.invalid/exact-setup.exe".into(),
            size: 1,
        });
        assert_eq!(
            pick_asset(&release, InstallKind::Nsis)
                .expect("exact NSIS")
                .url,
            "https://example.invalid/exact-setup.exe"
        );
    }

    #[test]
    fn signed_metadata_must_match_selected_package() {
        let asset = Asset {
            name: "BiFlow_9.9.9_x64-setup.exe".into(),
            url: "https://github.com/devlifeX/BiFlow/releases/download/v9.9.9/BiFlow_9.9.9_x64-setup.exe".into(),
            size: 42,
        };
        assert!(validate_signed_metadata(
            "9.9.9",
            &asset,
            InstallKind::Nsis,
            "9.9.9",
            &asset.url,
            "windows-x86_64",
        )
        .is_ok());
        for (version, url, target) in [
            ("9.9.8", asset.url.as_str(), "windows-x86_64"),
            (
                "9.9.9",
                "https://example.invalid/other.exe",
                "windows-x86_64",
            ),
            ("9.9.9", asset.url.as_str(), "linux-x86_64"),
        ] {
            assert!(validate_signed_metadata(
                "9.9.9",
                &asset,
                InstallKind::Nsis,
                version,
                url,
                target
            )
            .is_err());
        }
    }

    #[test]
    fn install_kind_prefers_appimage_env_on_linux() {
        assert_eq!(
            install_kind_from(Some(OsStr::new("/tmp/BiFlow.AppImage")), false),
            InstallKind::AppImage
        );
        assert_eq!(install_kind_from(None, false), InstallKind::Deb);
        assert_eq!(install_kind_from(None, true), InstallKind::Nsis);
    }

    #[test]
    fn nsis_update_waits_for_elevation_and_never_masks_installer_failure() {
        let script = nsis_update_script(
            42,
            Path::new(r"C:\Users\A User\BiFlow's setup.exe"),
            Path::new(r"C:\Program Files\BiFlow\BiFlow.exe"),
            "6.2.27",
        );

        assert!(script.contains("AddMinutes(3)"));
        assert!(script.contains("-Verb RunAs -Wait -PassThru"));
        assert!(script.contains("$process.ExitCode -ne 0"));
        assert!(script.contains("$expectedVersion = '6.2.27'"));
        assert!(script.contains("$versionMatch.Groups['semver'].Value -ne $expectedVersion"));
        assert!(script.contains("1223"));
        assert!(script.contains("BiFlow''s setup.exe"));
        assert!(script.contains("BiFlow update failed"));
        assert!(script.contains("if (Test-Path -LiteralPath $applicationPath -PathType Leaf)"));
    }
}
