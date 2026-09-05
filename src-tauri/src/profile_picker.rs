//! Native file dialog for `OwnedSideTunnel` profile paths.

use std::path::Path;
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;
use tracing::info;

/// `.ovpn` is `OpenVPN` and Windscribe. `.conf` is reserved for `WireGuard`.
const PROFILE_EXTENSIONS: &[&str] = &["ovpn", "conf"];

/// Extensions the native picker accepts.
pub fn accepted_profile_extensions() -> &'static [&'static str] {
    PROFILE_EXTENSIONS
}

/// True when `path` has an accepted profile extension.
pub fn is_accepted_profile(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        PROFILE_EXTENSIONS
            .iter()
            .any(|allowed| ext.eq_ignore_ascii_case(allowed))
    })
}

/// Opens a native file dialog and returns the chosen path.
///
/// `Ok(None)` means the operator cancelled. The path itself is never logged.
pub fn pick_profile<R: Runtime>(app: &AppHandle<R>) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .add_filter("VPN profile", accepted_profile_extensions())
        .blocking_pick_file();
    let Some(file) = picked else {
        info!(chosen = false, "client profile picker cancelled");
        return Ok(None);
    };
    let Some(path) = file.as_path() else {
        return Err("profile must be a local file".into());
    };
    accepted_profile_path(path)
}

fn accepted_profile_path(path: &Path) -> Result<Option<String>, String> {
    if !is_accepted_profile(path) {
        return Err("choose an .ovpn or .conf profile".into());
    }
    info!(
        chosen = true,
        "client profile picker completed without logging the selection"
    );
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::{accepted_profile_extensions, accepted_profile_path, is_accepted_profile};
    use std::path::Path;

    #[test]
    fn accepts_ovpn_and_conf_case_insensitively() {
        assert_eq!(accepted_profile_extensions(), ["ovpn", "conf"]);
        assert!(is_accepted_profile(Path::new("/tmp/office.ovpn")));
        assert!(is_accepted_profile(Path::new("windscribe.OVPN")));
        assert!(is_accepted_profile(Path::new("/tmp/wg.conf")));
        assert!(!is_accepted_profile(Path::new("/tmp/notes.txt")));
        assert!(!is_accepted_profile(Path::new("/tmp/profile")));
    }

    #[test]
    fn rejected_extension_does_not_yield_a_path() {
        let error = accepted_profile_path(Path::new("/tmp/notes.txt")).expect_err("txt");
        assert!(error.contains(".ovpn"));
        assert!(accepted_profile_path(Path::new("/tmp/office.ovpn"))
            .expect("ovpn")
            .is_some());
    }
}
