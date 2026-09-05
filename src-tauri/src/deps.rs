#![allow(clippy::module_name_repetitions)]

use flate2::read::GzDecoder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    error::Error as _,
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tracing::{info, warn};

const USER_AGENT: &str = concat!("BiFlow/", env!("CARGO_PKG_VERSION"));
const MAX_DOWNLOAD_BYTES: usize = 180 * 1024 * 1024;
const HIDDIFY_VERSION: &str = "v4.1.1";
const MIHOMO_VERSION: &str = "v1.19.29";
const MIHOMO_LINUX_SHA256: &str =
    "9c397be7489538628fae781bc005e4c5b8cd7b0961b8bb2ca815c8150f193577";
const MIHOMO_LINUX_ARCHIVE_SHA256: &str =
    "60de76a35e6cbf7b4fa4a20f5c257c24345d1d635ab1aa3877022a1997ef413c";
const MIHOMO_WINDOWS_ZIP_SHA256: &str =
    "1a8520cfe425441eba3eba8623b27b985020031243fe1ecaa1af2b92358a03f9";
const MIHOMO_WINDOWS_SHA256: &str =
    "4316ff91fecec2fca9acb5612d7400ba228c069ffd325b1f17f46f1d4ef7e0cd";
const WINTUN_WINDOWS_SHA256: &str =
    "e5da8447dc2c320edc0fc52fa01885c103de8c118481f683643cacc3220dafce";

#[derive(Debug, Error)]
pub enum DepsError {
    #[error("dependency I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("download failed: {0}")]
    Fetch(String),
    #[error("downloaded file failed integrity checks: {0}")]
    Integrity(String),
    #[error("installation failed: {0}")]
    Install(String),
    #[error("unknown dependency: {0}")]
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyId {
    Hiddify,
    Mihomo,
}

impl DependencyId {
    pub fn parse(value: &str) -> Result<Self, DepsError> {
        match value {
            "hiddify" => Ok(Self::Hiddify),
            "mihomo" => Ok(Self::Mihomo),
            other => Err(DepsError::Unknown(other.into())),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hiddify => "hiddify",
            Self::Mihomo => "mihomo",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallGuide {
    pub id: &'static str,
    pub title: String,
    pub download_url: String,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub id: &'static str,
    pub installed: bool,
    pub path: Option<String>,
    pub guide: InstallGuide,
}

#[must_use]
pub fn hiddify_candidates(data: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![
        data.join("bin").join("hiddify"),
        data.join("bin").join("hiddify-app"),
        data.join("apps").join("Hiddify.AppImage"),
        data.join("apps").join("Hiddify").join("Hiddify.exe"),
        data.join("apps").join("Hiddify").join("hiddify.exe"),
        PathBuf::from("/usr/bin/hiddify"),
        PathBuf::from("/usr/bin/hiddify-app"),
        PathBuf::from("/usr/local/bin/hiddify"),
        PathBuf::from("/usr/local/bin/hiddify-app"),
        PathBuf::from("/opt/Hiddify/Hiddify"),
        PathBuf::from("/opt/hiddify/hiddify"),
        PathBuf::from("/opt/hiddify/hiddify-app"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".local").join("bin").join("hiddify"));
        candidates.push(home.join(".local").join("bin").join("hiddify-app"));
        candidates.push(
            home.join(".local")
                .join("share")
                .join("Hiddify")
                .join("hiddify"),
        );
        candidates.extend(appimages_matching(&home.join("Applications"), "hiddify"));
        candidates.extend(appimages_matching(
            &home
                .join(".local")
                .join("share")
                .join("biflow")
                .join("apps"),
            "hiddify",
        ));
    }
    candidates.extend(appimages_matching(&data.join("apps"), "hiddify"));
    if let Some(local) = dirs::data_local_dir() {
        candidates.push(local.join("Hiddify").join("Hiddify.exe"));
        candidates.push(local.join("Hiddify").join("hiddify.exe"));
        candidates.push(local.join("Programs").join("Hiddify").join("Hiddify.exe"));
    }
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        let programs = PathBuf::from(programs);
        candidates.push(programs.join("Hiddify").join("Hiddify.exe"));
        candidates.push(programs.join("HiddifyNext").join("Hiddify.exe"));
    }
    candidates.extend(lookup_on_path(&["hiddify", "hiddify-app", "Hiddify"]));
    candidates
}

#[must_use]
pub fn mihomo_candidates(data: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    // A dev session vendors its own Mihomo; without this the dependency
    // card claims Mihomo is missing and offers a pointless install.
    if let Some(dev) = std::env::var_os("BIFLOW_DEV_MIHOMO_BINARY") {
        candidates.push(PathBuf::from(dev));
    }
    candidates.extend([
        mihomo_install_path(data),
        data.join("bin").join("clash-meta"),
        PathBuf::from("/usr/lib/biflow/mihomo"),
        PathBuf::from("/opt/biflow/mihomo"),
        PathBuf::from("/opt/iran-split/mihomo"),
        PathBuf::from("/usr/local/bin/mihomo"),
        PathBuf::from("/usr/local/bin/clash-meta"),
        PathBuf::from("/usr/bin/mihomo"),
        PathBuf::from("/usr/bin/clash-meta"),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".local").join("bin").join("mihomo"));
        candidates.push(home.join(".local").join("bin").join("clash-meta"));
    }
    candidates.extend(lookup_on_path(&["mihomo", "clash-meta"]));
    candidates
}

fn lookup_on_path(names: &[&str]) -> Vec<PathBuf> {
    files_named_in(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        names,
    )
}

#[cfg(any(windows, test))]
fn has_extension(name: &str, extension: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn files_named_in(dirs: impl IntoIterator<Item = PathBuf>, names: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in dirs {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                found.push(candidate);
            }
            #[cfg(windows)]
            {
                if !has_extension(name, "exe") {
                    let exe = dir.join(format!("{name}.exe"));
                    if exe.is_file() {
                        found.push(exe);
                    }
                }
            }
        }
    }
    found
}

fn appimages_matching(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        let lower = name.to_ascii_lowercase();
                        lower.starts_with(prefix) && lower.ends_with(".appimage")
                    })
        })
        .collect()
}

#[must_use]
pub fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

#[must_use]
pub fn dependency_status(data: &Path) -> Vec<DependencyStatus> {
    vec![
        status_for(
            "hiddify",
            "Hiddify",
            first_existing(&hiddify_candidates(data)),
        ),
        status_for("mihomo", "Mihomo", first_existing(&mihomo_candidates(data))),
    ]
}

fn status_for(id: &'static str, name: &'static str, path: Option<PathBuf>) -> DependencyStatus {
    DependencyStatus {
        id,
        name,
        installed: path.is_some(),
        version: None,
        path: path.map(|value| value.to_string_lossy().into_owned()),
    }
}

#[must_use]
pub fn install_guide(id: DependencyId) -> InstallGuide {
    match (id, std::env::consts::OS) {
        (DependencyId::Hiddify, "windows") => InstallGuide {
            id: id.as_str(),
            title: "Install Hiddify on Windows".into(),
            download_url: hiddify_windows_url(),
            steps: vec![
                "Open the official Hiddify release page and download Hiddify-Windows-Setup-x64.exe.".into(),
                "Run the installer and accept the Windows permission prompt.".into(),
                "Finish setup, then restart BiFlow and press Connect.".into(),
            ],
        },
        (DependencyId::Hiddify, _) => InstallGuide {
            id: id.as_str(),
            title: "Install Hiddify on Linux".into(),
            download_url: hiddify_linux_appimage_url(),
            steps: vec![
                "Download Hiddify-Linux-x64-AppImage.AppImage from the official GitHub release.".into(),
                "Make it executable: chmod +x Hiddify-Linux-x64-AppImage.AppImage".into(),
                "Move it to ~/.local/share/biflow/apps/Hiddify.AppImage".into(),
                "Restart BiFlow and press Connect.".into(),
            ],
        },
        (DependencyId::Mihomo, "windows") => InstallGuide {
            id: id.as_str(),
            title: "Install Mihomo on Windows".into(),
            download_url: mihomo_windows_url(),
            steps: vec![
                "Download mihomo-windows-amd64 zip from the MetaCubeX GitHub release.".into(),
                "Extract mihomo.exe into %LOCALAPPDATA%\\biflow\\bin\\mihomo.exe".into(),
                "Restart BiFlow and press Connect.".into(),
            ],
        },
        (DependencyId::Mihomo, _) => InstallGuide {
            id: id.as_str(),
            title: "Install Mihomo on Linux".into(),
            download_url: mihomo_linux_url(),
            steps: vec![
                "Download mihomo-linux-amd64 gzip from the MetaCubeX GitHub release.".into(),
                "Decompress it: gzip -dc mihomo-linux-amd64-*.gz > ~/.local/share/biflow/bin/mihomo".into(),
                "Make it executable: chmod +x ~/.local/share/biflow/bin/mihomo".into(),
                "Restart BiFlow and press Connect.".into(),
            ],
        },
    }
}

pub async fn install_dependency(
    id: DependencyId,
    data: &Path,
    bundled_dependencies: &Path,
) -> Result<InstallResult, DepsError> {
    info!(
        event = "dependency.install_started",
        section = "dependencies",
        initiator = "dependency_installer",
        cause = "user_request",
        trace_route = "tauri_command->dependency_installer->bundled_or_network_source",
        dependency_id = id.as_str(),
        "dependency installation started"
    );
    fs::create_dir_all(data.join("bin"))?;
    fs::create_dir_all(data.join("apps"))?;
    let path = match id {
        DependencyId::Hiddify => install_hiddify(data).await?,
        DependencyId::Mihomo => install_mihomo(data, bundled_dependencies).await?,
    };
    let result = InstallResult {
        id: id.as_str(),
        installed: path.is_file(),
        path: Some(path.to_string_lossy().into_owned()),
        guide: install_guide(id),
    };
    info!(
        event = "dependency.install_completed",
        section = "dependencies",
        initiator = "dependency_installer",
        cause = "none",
        trace_route = "tauri_command->dependency_installer->installed_binary",
        dependency_id = id.as_str(),
        installed = result.installed,
        "dependency installation completed"
    );
    Ok(result)
}

async fn install_hiddify(data: &Path) -> Result<PathBuf, DepsError> {
    if cfg!(target_os = "windows") {
        install_hiddify_windows(data).await
    } else {
        install_hiddify_linux(data).await
    }
}

async fn install_hiddify_linux(data: &Path) -> Result<PathBuf, DepsError> {
    let bytes = download_first(&[
        hiddify_linux_appimage_url(),
        hiddify_linux_appimage_pinned_url(),
    ])
    .await?;
    if !is_elf(&bytes) {
        return Err(DepsError::Integrity(
            "Hiddify AppImage is not an ELF binary".into(),
        ));
    }
    let appimage = data.join("apps").join("Hiddify.AppImage");
    write_executable(&appimage, &bytes)?;
    let link = data.join("bin").join("hiddify");
    if let Err(cause) = fs::remove_file(&link) {
        if cause.kind() != std::io::ErrorKind::NotFound {
            return Err(cause.into());
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&appimage, &link)?;
    }
    #[cfg(not(unix))]
    {
        fs::copy(&appimage, &link)?;
    }
    Ok(appimage)
}

async fn install_hiddify_windows(data: &Path) -> Result<PathBuf, DepsError> {
    let bytes =
        download_first(&[hiddify_windows_portable_url(), hiddify_windows_setup_url()]).await?;
    if is_zip(&bytes) {
        let dest = data.join("apps").join("Hiddify");
        extract_zip(&bytes, &dest)?;
        return first_existing(&hiddify_candidates(data)).ok_or_else(|| {
            DepsError::Install("Hiddify portable archive did not contain Hiddify.exe".into())
        });
    }
    if !is_pe(&bytes) {
        return Err(DepsError::Integrity(
            "Hiddify installer is not a Windows executable".into(),
        ));
    }
    let installer = data.join("apps").join("Hiddify-Setup.exe");
    fs::write(&installer, bytes)?;
    let status = std::process::Command::new(&installer)
        .args(["/VERYSILENT", "/NORESTART", "/SUPPRESSMSGBOXES"])
        .status()
        .map_err(|error| DepsError::Install(error.to_string()))?;
    if !status.success() {
        std::process::Command::new(&installer).spawn().map_err(|cause| {
            DepsError::Install(format!(
                "silent Hiddify setup failed with {status}, and its interactive fallback could not start: {cause}"
            ))
        })?;
        warn!(
            event = "dependency.interactive_installer_started",
            section = "dependencies",
            initiator = "dependency_installer",
            cause = %status,
            trace_route = "tauri_command->dependency_installer->hiddify_setup_fallback",
            dependency_id = "hiddify",
            "silent setup failed, so the interactive installer was started"
        );
        return Err(DepsError::Install(
            "silent Hiddify setup did not finish; the installer window was opened instead".into(),
        ));
    }
    first_existing(&hiddify_candidates(data)).ok_or_else(|| {
        DepsError::Install("Hiddify installed but the executable was not found".into())
    })
}

async fn install_mihomo(data: &Path, bundled_dependencies: &Path) -> Result<PathBuf, DepsError> {
    let dest = mihomo_install_path(data);
    if cfg!(target_os = "windows") {
        let bundled = bundled_dependencies.join("mihomo.exe");
        if bundled.is_file() {
            let bytes = fs::read(bundled)?;
            install_windows_mihomo_bytes(&dest, &bytes, MIHOMO_WINDOWS_SHA256)?;
            install_windows_wintun(dest.parent().unwrap_or(data), bundled_dependencies)?;
            return Ok(dest);
        }
        let bytes = download_first(&[mihomo_windows_url()]).await?;
        verify_sha256(&bytes, MIHOMO_WINDOWS_ZIP_SHA256)?;
        let staging = data.join("apps").join("mihomo-extract");
        extract_zip(&bytes, &staging)?;
        let found = find_file(&staging, "mihomo.exe")
            .ok_or_else(|| DepsError::Install("Mihomo zip did not contain mihomo.exe".into()))?;
        fs::create_dir_all(dest.parent().unwrap_or(data))?;
        let executable = fs::read(found)?;
        install_windows_mihomo_bytes(&dest, &executable, MIHOMO_WINDOWS_SHA256)?;
        install_windows_wintun(dest.parent().unwrap_or(data), bundled_dependencies)?;
        if let Err(cause) = fs::remove_dir_all(staging) {
            warn!(
                event = "dependency.staging_cleanup_failed",
                section = "dependencies",
                initiator = "dependency_installer",
                cause = %cause,
                trace_route = "tauri_command->dependency_installer->mihomo_staging_cleanup",
                dependency_id = "mihomo",
                "Mihomo was installed but its staging directory could not be removed"
            );
        }
    } else {
        let bundled = bundled_dependencies.join("mihomo");
        if bundled.is_file() {
            let bytes = fs::read(bundled)?;
            install_linux_mihomo_bytes(&dest, &bytes, MIHOMO_LINUX_SHA256)?;
            return Ok(dest);
        }
        let bytes = download_first(&[mihomo_linux_url()]).await?;
        verify_sha256(&bytes, MIHOMO_LINUX_ARCHIVE_SHA256)?;
        let decoded = gunzip(&bytes)?;
        install_linux_mihomo_bytes(&dest, &decoded, MIHOMO_LINUX_SHA256)?;
    }
    Ok(dest)
}

fn install_linux_mihomo_bytes(
    destination: &Path,
    bytes: &[u8],
    expected_sha256: &str,
) -> Result<(), DepsError> {
    verify_sha256(bytes, expected_sha256)?;
    if !is_elf(bytes) {
        return Err(DepsError::Integrity("Mihomo is not an ELF binary".into()));
    }
    write_executable(destination, bytes)
}

fn install_windows_mihomo_bytes(
    destination: &Path,
    bytes: &[u8],
    expected_sha256: &str,
) -> Result<(), DepsError> {
    verify_sha256(bytes, expected_sha256)?;
    if !is_pe(bytes) {
        return Err(DepsError::Integrity(
            "Mihomo is not a Windows executable".into(),
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(destination, bytes)?;
    Ok(())
}

fn install_windows_wintun(
    destination_dir: &Path,
    bundled_dependencies: &Path,
) -> Result<(), DepsError> {
    let bundled = bundled_dependencies.join("wintun.dll");
    if !bundled.is_file() {
        return Ok(());
    }
    let bytes = fs::read(bundled)?;
    verify_sha256(&bytes, WINTUN_WINDOWS_SHA256)?;
    fs::create_dir_all(destination_dir)?;
    fs::write(destination_dir.join("wintun.dll"), bytes)?;
    Ok(())
}

fn mihomo_install_path(data: &Path) -> PathBuf {
    data.join("bin").join(mihomo_file_name())
}

fn mihomo_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "mihomo.exe"
    } else {
        "mihomo"
    }
}

fn hiddify_linux_appimage_url() -> String {
    "https://github.com/hiddify/hiddify-app/releases/latest/download/Hiddify-Linux-x64-AppImage.AppImage"
        .into()
}

fn hiddify_linux_appimage_pinned_url() -> String {
    format!(
        "https://github.com/hiddify/hiddify-app/releases/download/{HIDDIFY_VERSION}/Hiddify-Linux-x64-AppImage.AppImage"
    )
}

fn hiddify_windows_url() -> String {
    format!(
        "https://github.com/hiddify/hiddify-app/releases/download/{HIDDIFY_VERSION}/Hiddify-Windows-Setup-x64.exe"
    )
}

fn hiddify_windows_setup_url() -> String {
    hiddify_windows_url()
}

fn hiddify_windows_portable_url() -> String {
    format!(
        "https://github.com/hiddify/hiddify-app/releases/download/{HIDDIFY_VERSION}/Hiddify-Windows-Portable-x64.zip"
    )
}

fn mihomo_linux_url() -> String {
    format!(
        "https://github.com/MetaCubeX/mihomo/releases/download/{MIHOMO_VERSION}/mihomo-linux-amd64-{MIHOMO_VERSION}.gz"
    )
}

fn mihomo_windows_url() -> String {
    format!(
        "https://github.com/MetaCubeX/mihomo/releases/download/{MIHOMO_VERSION}/mihomo-windows-amd64-{MIHOMO_VERSION}.zip"
    )
}

fn url_allowed(url: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "https://github.com/hiddify/hiddify-app/releases/",
        "https://github.com/MetaCubeX/mihomo/releases/",
    ];
    PREFIXES.iter().any(|prefix| url.starts_with(prefix))
}

async fn download_first(urls: &[String]) -> Result<Vec<u8>, DepsError> {
    let mut last = DepsError::Fetch("no allowlisted download URL succeeded".into());
    for url in urls {
        match download(url).await {
            Ok(bytes) => return Ok(bytes),
            Err(error) => last = error,
        }
    }
    Err(last)
}

async fn download(url: &str) -> Result<Vec<u8>, DepsError> {
    if !url_allowed(url) {
        return Err(DepsError::Fetch("download URL is not allowlisted".into()));
    }
    match download_with_client(url, false).await {
        Ok(bytes) => Ok(bytes),
        Err(proxy_error) => {
            warn!(
                event = "dependency.proxy_download_failed",
                section = "dependencies",
                initiator = "dependency_downloader",
                cause = %proxy_error,
                trace_route = "dependency_downloader->proxy_aware_client->direct_retry",
                "proxy-aware dependency download failed; retrying directly"
            );
            download_with_client(url, true)
                .await
                .map_err(|direct_error| {
                    DepsError::Fetch(format!(
                        "proxy-aware request failed ({proxy_error}); direct retry failed ({direct_error})"
                    ))
                })
        }
    }
}

async fn download_with_client(url: &str, direct: bool) -> Result<Vec<u8>, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::limited(8));
    if direct {
        builder = builder.no_proxy();
    }
    let client = builder.build().map_err(|error| error_chain(&error))?;
    let response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error_chain(&error))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES as u64)
    {
        return Err("download exceeded the size limit".into());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| error_chain(&error))?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return Err("download exceeded the size limit".into());
    }
    Ok(bytes.to_vec())
}

fn error_chain(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, DepsError> {
    let mut decoder = GzDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|error| DepsError::Integrity(error.to_string()))?;
    Ok(output)
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), DepsError> {
    fs::create_dir_all(dest)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| DepsError::Integrity(error.to_string()))?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|error| DepsError::Integrity(error.to_string()))?;
        let Some(relative) = file.enclosed_name() else {
            continue;
        };
        let out = dest.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut target = fs::File::create(&out)?;
        std::io::copy(&mut file, &mut target)?;
    }
    Ok(())
}

fn find_file(root: &Path, file_name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|name| name == file_name) {
                return Some(path);
            }
        }
    }
    None
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), DepsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn verify_sha256(bytes: &[u8], expected: &str) -> Result<(), DepsError> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(DepsError::Integrity(format!(
            "SHA-256 mismatch: expected {expected}, got {actual}"
        )))
    }
}

fn is_elf(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0] == 0x7f && bytes[1] == b'E' && bytes[2] == b'L' && bytes[3] == b'F'
}

fn is_pe(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == b'M' && bytes[1] == b'Z'
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0] == b'P' && bytes[1] == b'K'
}

pub fn open_allowlisted_url(url: &str) -> Result<(), DepsError> {
    if !open_url_allowed(url) {
        return Err(DepsError::Fetch("URL is not allowlisted".into()));
    }
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    result
        .map(|_| ())
        .map_err(|error| DepsError::Install(error.to_string()))
}

fn open_url_allowed(url: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "https://github.com/hiddify/hiddify-app/releases/",
        "https://github.com/MetaCubeX/mihomo/releases/",
        "https://github.com/devlifeX/BiFlow",
        "https://raw.githubusercontent.com/devlifeX/BiFlow/",
    ];
    if PREFIXES.iter().any(|prefix| url.starts_with(prefix)) {
        return true;
    }
    // Catalog download pages are part of the shipped preset table, so the
    // allowlist can never drift from the buttons that use it.
    iran_split_config::PresetId::all().iter().any(|preset| {
        let downloads = preset.spec().downloads;
        url == downloads.linux || url == downloads.windows
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_only_allow_official_release_hosts() {
        assert!(url_allowed(&hiddify_linux_appimage_url()));
        assert!(url_allowed(&mihomo_linux_url()));
        assert!(!url_allowed("https://example.com/hiddify"));
    }

    #[test]
    fn opens_biflow_repository_links() {
        assert!(open_url_allowed("https://github.com/devlifeX/BiFlow"));
        assert!(open_url_allowed(
            "https://github.com/devlifeX/BiFlow/releases/latest"
        ));
        assert!(!open_url_allowed("https://github.com/example/not-biflow"));
    }

    #[test]
    fn executable_extensions_are_case_insensitive() {
        assert!(has_extension("Hiddify.exe", "exe"));
        assert!(has_extension("Hiddify.EXE", "exe"));
        assert!(!has_extension("Hiddify.exe.zip", "exe"));
    }

    #[test]
    fn linux_install_paths_are_under_user_data() {
        let data = PathBuf::from("/tmp/biflow-user");
        let hiddify = hiddify_candidates(&data);
        assert!(hiddify.iter().any(|path| path.ends_with("Hiddify.AppImage")
            && path.parent().is_some_and(|parent| parent.ends_with("apps"))));
        assert_eq!(mihomo_candidates(&data)[0], mihomo_install_path(&data));
    }

    #[test]
    fn install_guides_name_biflow_paths() {
        let hiddify = install_guide(DependencyId::Hiddify);
        assert!(hiddify
            .download_url
            .starts_with("https://github.com/hiddify/hiddify-app/releases/"));
        assert!(!hiddify.steps.is_empty());
        let mihomo = install_guide(DependencyId::Mihomo);
        assert!(mihomo.download_url.contains("MetaCubeX/mihomo"));
        assert!(hiddify
            .steps
            .iter()
            .any(|step| step.contains("biflow") || step.contains("Hiddify")));
    }

    #[test]
    fn unknown_dependency_id_is_rejected() {
        assert!(DependencyId::parse("wireguard").is_err());
        assert_eq!(
            DependencyId::parse("hiddify").expect("id").as_str(),
            "hiddify"
        );
    }

    #[test]
    fn existing_binaries_in_data_dir_hide_install() {
        let dir = std::env::temp_dir().join(format!("biflow-deps-{}", std::process::id()));
        let apps = dir.join("apps");
        let bin = dir.join("bin");
        fs::create_dir_all(&apps).expect("apps");
        fs::create_dir_all(&bin).expect("bin");
        fs::write(apps.join("Hiddify.AppImage"), b"elf").expect("hiddify");
        fs::write(mihomo_install_path(&dir), b"elf").expect("mihomo");
        let status = dependency_status(&dir);
        fs::remove_dir_all(&dir).expect("cleanup dependency fixture");
        assert!(status
            .iter()
            .any(|item| item.id == "hiddify" && item.installed));
        assert!(status
            .iter()
            .any(|item| item.id == "mihomo" && item.installed));
    }

    #[test]
    fn files_on_a_search_path_count_as_installed() {
        let dir = std::env::temp_dir().join(format!("biflow-path-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("hiddify"), b"ok").expect("hiddify");
        fs::write(dir.join("mihomo"), b"ok").expect("mihomo");
        let found = files_named_in(std::iter::once(dir.clone()), &["hiddify", "mihomo"]);
        fs::remove_dir_all(&dir).expect("cleanup path fixture");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn bundled_wintun_is_optional_and_integrity_checked() {
        let directory = tempfile::tempdir().expect("tempdir");
        let bundled = directory.path().join("bundled");
        let dest = directory.path().join("bin");
        fs::create_dir_all(&bundled).expect("bundled");
        install_windows_wintun(&dest, &bundled).expect("missing wintun is skipped");
        assert!(!dest.join("wintun.dll").is_file());
        fs::write(bundled.join("wintun.dll"), b"not-wintun").expect("write");
        assert!(install_windows_wintun(&dest, &bundled).is_err());
    }

    #[test]
    fn bundled_linux_mihomo_is_verified_before_install() {
        let directory = tempfile::tempdir().expect("tempdir");
        let destination = directory.path().join("bin").join("mihomo");
        let bytes = b"\x7fELF bundled mihomo fixture";
        let expected = hex::encode(Sha256::digest(bytes));

        install_linux_mihomo_bytes(&destination, bytes, &expected).expect("install");

        assert_eq!(fs::read(destination).expect("installed bytes"), bytes);
    }

    #[test]
    fn dependency_paths_join_components_separately() {
        let data = PathBuf::from("data");
        assert_eq!(
            mihomo_install_path(&data),
            data.join("bin").join(mihomo_file_name())
        );
        let source = include_str!("deps.rs");
        assert!(!source.contains(".join(\"bin/"));
        assert!(!source.contains(".join(\"apps/"));
        assert!(!source.contains(".join(\"Hiddify/"));
        assert!(!source.contains(".join(\".local/"));
        assert!(!source.contains(".join(\"Programs/"));
    }
}
