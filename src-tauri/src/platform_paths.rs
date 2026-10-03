//! Platform-specific resource locations for this run (ADR 0115).
//!
//! The installed application's locations, the Mihomo executable, and the
//! generation staging root are three separate concerns that all depend on the
//! same thing: which profile this process runs under. They lived inline in
//! `lib.rs`, spread across three `cfg` blocks that each re-derived profile
//! identity.
//!
//! Every decision here reads the resolved profile from [`crate::profile`], so
//! there is exactly one rule ط·آ·ط¢آ£ط·آ¢ط¢آ¢ط·آ£ط¢آ¢ط£آ¢أ¢â€ڑآ¬ط¹â€کط·آ¢ط¢آ¬ط·آ£ط¢آ¢ط£آ¢أ¢â‚¬ع‘ط¢آ¬ط£آ¢أ¢â€ڑآ¬ط¥â€™ *a development run never inherits a production
//! location* ط·آ·ط¢آ£ط·آ¢ط¢آ¢ط·آ£ط¢آ¢ط£آ¢أ¢â€ڑآ¬ط¹â€کط·آ¢ط¢آ¬ط·آ£ط¢آ¢ط£آ¢أ¢â‚¬ع‘ط¢آ¬ط£آ¢أ¢â€ڑآ¬ط¥â€™ instead of one per platform.

use std::path::{Path, PathBuf};

use crate::profile;

#[cfg(target_os = "linux")]
const PRODUCTION_ENDPOINT: &str = "/run/iran-split/helper.sock";
#[cfg(target_os = "linux")]
const PRODUCTION_RUNTIME: &str = "/var/lib/iran-split";
#[cfg(target_os = "linux")]
const MISSING_DEV_ENDPOINT: &str = "/run/biflow-dev/missing-helper.sock";
#[cfg(target_os = "linux")]
const MISSING_DEV_RUNTIME: &str = "/run/biflow-dev/missing-runtime";

/// Written by the elevated installer in `iran-split-helper::install` (ADR 0029).
#[cfg(target_os = "windows")]
const PRODUCTION_ENDPOINT: &str = iran_split_platform_win::HELPER_PIPE;
#[cfg(target_os = "windows")]
const PRODUCTION_RUNTIME: &str = r"C:\ProgramData\iran-split\runtime";
#[cfg(target_os = "windows")]
const WINDOWS_PROGRAMDATA_MIHOMO: &str = r"C:\ProgramData\iran-split\bin\mihomo.exe";
/// No per-user Windows development helper exists, so a development run is
/// pointed at an endpoint nothing serves. Both placeholders must stay outside
/// the production `iran-split` tree, so a later bug can never create or write
/// inside a directory the installed app owns.
#[cfg(target_os = "windows")]
const MISSING_DEV_ENDPOINT: &str = r"\\.\pipe\iran-split-helper-dev-missing";
#[cfg(target_os = "windows")]
const MISSING_DEV_RUNTIME: &str = r"C:\ProgramData\biflow-dev-missing-runtime";

/// The privileged helper runs as a launchd daemon under
/// `/Library/Application Support/BiFlow`; the socket lives next to it so only
/// root and the authorized group can reach it.
#[cfg(target_os = "macos")]
const PRODUCTION_ENDPOINT: &str = "/Library/Application Support/BiFlow/helper.sock";
#[cfg(target_os = "macos")]
const PRODUCTION_RUNTIME: &str = "/Library/Application Support/BiFlow/runtime";
#[cfg(target_os = "macos")]
const MISSING_DEV_ENDPOINT: &str = "/tmp/biflow-dev-missing-helper.sock";
#[cfg(target_os = "macos")]
const MISSING_DEV_RUNTIME: &str = "/tmp/biflow-dev-missing-runtime";

/// Helper endpoint and system runtime root on Linux.
///
/// A development run uses only the explicit overrides `dev.sh` exported for
/// its transient per-user helper. It never inherits the production endpoint: a
/// missing override resolves to an unreachable placeholder so the helper reads
/// as unavailable instead of the installed app being reconfigured (ADR 0080).
#[cfg(target_os = "linux")]
#[must_use]
pub fn linux_helper_paths() -> (PathBuf, PathBuf) {
    resolved_helper_paths()
}

/// Helper endpoint and system runtime root on macOS.
#[cfg(target_os = "macos")]
#[must_use]
pub fn macos_helper_paths() -> (PathBuf, PathBuf) {
    resolved_helper_paths()
}

/// Pipe name and system runtime root on Windows.
///
/// The production values are fixed by the SYSTEM scheduled task, so there is
/// no per-user development helper to point at. A development run must *not*
/// fall back to them: doing so would stage generations into the machine-wide
/// `C:\ProgramData\iran-split\staging` and drive the installed SYSTEM helper
/// with development paths. It gets an unreachable pipe instead, so the helper
/// reads as unavailable and nothing privileged is touched.
#[cfg(target_os = "windows")]
#[must_use]
pub fn windows_helper_paths() -> (String, PathBuf) {
    let (endpoint, runtime) = resolved_helper_paths();
    (endpoint.to_string_lossy().into_owned(), runtime)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn resolved_helper_paths() -> (PathBuf, PathBuf) {
    profile::helper_paths(
        PRODUCTION_ENDPOINT,
        PRODUCTION_RUNTIME,
        MISSING_DEV_ENDPOINT,
        MISSING_DEV_RUNTIME,
    )
}

/// Choose the Mihomo executable for this run.
///
/// `candidates` is what the dependency search found, which deliberately
/// includes `PATH`. A development run must not inherit a binary from that
/// search, so the discovered result is *not* the answer in a development
/// profile: only the `dev.sh` override is. Without one, the path points inside
/// the profile, so the dependency reads as missing and `./dev.sh` is the fix,
/// rather than running production's binary against development configuration
/// (ADR 0115).
#[must_use]
pub fn mihomo_binary(
    candidates: &[PathBuf],
    development_data: &Path,
    production: PathBuf,
) -> PathBuf {
    mihomo_binary_for(
        profile::is_development(),
        profile::dev_mihomo_binary(),
        crate::deps::first_existing(candidates),
        development_data,
        production,
    )
}

/// Pure core of [`mihomo_binary`], with explicit inputs so the policy is
/// testable without the process environment.
#[must_use]
pub fn mihomo_binary_for(
    is_development: bool,
    development_override: Option<&Path>,
    discovered: Option<PathBuf>,
    development_data: &Path,
    production: PathBuf,
) -> PathBuf {
    // The override is honoured only in a development run. `PrivilegedOverrides`
    // already drops every override for a production profile, and the binary
    // choice must follow the same rule: an exported `BIFLOW_DEV_MIHOMO_BINARY`
    // must not make the installed app run a development binary.
    if is_development {
        return development_override.map_or_else(
            || development_data.join("bin").join("mihomo"),
            Path::to_path_buf,
        );
    }
    discovered.unwrap_or(production)
}

/// Privileged generation root the elevated installer records in `helper.toml`,
/// so the helper can publish the generation (ADR 0029, ADR 0064).
#[cfg(target_os = "windows")]
const PRODUCTION_GENERATION_STAGING: &str = crate::helper_install::WINDOWS_HELPER_STAGING;
/// Linux and macOS stage under the user's data root while the helper owns the
/// machine-wide runtime. The tests still need a production value to contrast a
/// development root against, so it is defined for `test` on every host.
#[cfg(all(not(target_os = "windows"), test))]
const PRODUCTION_GENERATION_STAGING: &str = "/var/lib/iran-split/runtime";

/// The helper copies Mihomo next to itself, so that copy is the production
/// fallback when the search finds nothing. A development run never reaches it.
#[cfg(target_os = "windows")]
#[must_use]
pub fn windows_programdata_mihomo() -> PathBuf {
    PathBuf::from(WINDOWS_PROGRAMDATA_MIHOMO)
}

/// Generation staging root for this run.
///
/// Packaged Connect stages where the elevated installer recorded in
/// `helper.toml`, so SYSTEM can publish the generation. That is a
/// machine-wide, privileged location, so a development run stages inside its
/// own unprivileged profile instead and the privileged directory is never
/// written to.
#[cfg(target_os = "windows")]
#[must_use]
pub fn windows_generation_staging(data: &Path) -> PathBuf {
    generation_staging_for(profile::is_development(), data)
}

/// Pure core of [`windows_generation_staging`].
///
/// Only Windows needs this, but it is compiled under `test` on every host so
/// the development rule is exercised everywhere. On Linux and macOS it would
/// otherwise be dead code, and `-D warnings` is the gate (ADR 0115).
#[cfg(any(target_os = "windows", test))]
#[must_use]
pub fn generation_staging_for(is_development: bool, data: &Path) -> PathBuf {
    if is_development {
        return data.join("generations");
    }
    PathBuf::from(PRODUCTION_GENERATION_STAGING)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn production_linux_helper_paths_are_fixed() {
        assert_eq!(PRODUCTION_ENDPOINT, "/run/iran-split/helper.sock");
        assert_eq!(PRODUCTION_RUNTIME, "/var/lib/iran-split");
    }

    /// A development run must never be handed a privileged, machine-wide
    /// staging root. `WindowsPaths.generation_staging_dir` goes straight to the
    /// helper, so this is the assertion that keeps a development Connect from
    /// writing into `C:\ProgramData`.
    ///
    /// Drives the pure core with explicit inputs ط·آ·ط¢آ£ط·آ¢ط¢آ¢ط·آ£ط¢آ¢ط£آ¢أ¢â€ڑآ¬ط¹â€کط·آ¢ط¢آ¬ط·آ£ط¢آ¢ط£آ¢أ¢â‚¬ع‘ط¢آ¬ط£آ¢أ¢â€ڑآ¬ط¥â€™ a test that read the
    /// process environment failed for anyone running the suite with
    /// `BIFLOW_DEV_PROFILE` set ط·آ·ط¢آ£ط·آ¢ط¢آ¢ط·آ£ط¢آ¢ط£آ¢أ¢â€ڑآ¬ط¹â€کط·آ¢ط¢آ¬ط·آ£ط¢آ¢ط£آ¢أ¢â‚¬ع‘ط¢آ¬ط£آ¢أ¢â€ڑآ¬ط¥â€™ and compares against *this platform's*
    /// production root rather than a hardcoded Windows path, which is a
    /// contradiction on Linux and macOS.
    #[test]
    fn generation_staging_depends_only_on_the_supplied_profile() {
        let data = PathBuf::from("/profile/data");
        assert_eq!(
            generation_staging_for(false, &data),
            PathBuf::from(PRODUCTION_GENERATION_STAGING)
        );
        assert_eq!(
            generation_staging_for(true, &data),
            data.join("generations")
        );
    }

    /// Whatever the platform's production root is, a development run stays
    /// inside its own profile and never reaches it.
    #[test]
    fn development_staging_never_leaves_the_profile() {
        let data = PathBuf::from("/profile/data");
        let development = generation_staging_for(true, &data);
        assert_eq!(development, data.join("generations"));
        assert!(development.starts_with(&data));
        assert_ne!(development, PathBuf::from(PRODUCTION_GENERATION_STAGING));
    }

    /// A placeholder must not be the production location, and must not sit
    /// inside the production privileged tree either: if it did, "no isolated
    /// helper" would silently become a write into the installed app's
    /// directory, which is the failure ADR 0115 exists to prevent.
    #[test]
    fn development_placeholders_never_equal_or_nest_in_production_locations() {
        assert_ne!(MISSING_DEV_ENDPOINT, PRODUCTION_ENDPOINT);
        assert_ne!(MISSING_DEV_RUNTIME, PRODUCTION_RUNTIME);
        assert!(!MISSING_DEV_ENDPOINT.contains(PRODUCTION_ENDPOINT));
        assert!(
            !MISSING_DEV_RUNTIME.contains(PRODUCTION_RUNTIME),
            "the development placeholder {MISSING_DEV_RUNTIME} is inside the production runtime {PRODUCTION_RUNTIME}"
        );
    }

    /// A development run must never adopt a production Mihomo, whatever the
    /// dependency search found. The search includes `PATH`, so an installed
    /// `mihomo.exe` on the developer's `PATH` was previously preferred over the
    /// profile policy and ran production's binary against development
    /// configuration.
    #[test]
    fn a_development_run_never_uses_a_discovered_production_mihomo() {
        let production = PathBuf::from("/usr/lib/biflow/mihomo");
        let data = PathBuf::from("/profile/data");
        let on_path = Some(PathBuf::from("/usr/bin/mihomo"));

        // No override, but a production binary is discoverable: the search
        // result must be discarded.
        let without_override =
            mihomo_binary_for(true, None, on_path.clone(), &data, production.clone());
        assert_ne!(without_override, PathBuf::from("/usr/bin/mihomo"));
        assert!(without_override.starts_with(&data));

        // An override whose file is missing must not promote the discovered
        // production binary into its place. The override path is returned and
        // reads as missing downstream, which is the point: the operator is
        // told to run `./dev.sh` instead of silently getting production's
        // binary.
        let stale_override = mihomo_binary_for(
            true,
            Some(Path::new("/gone/mihomo")),
            on_path,
            &data,
            production.clone(),
        );
        assert_eq!(stale_override, PathBuf::from("/gone/mihomo"));
        assert_ne!(stale_override, PathBuf::from("/usr/bin/mihomo"));
        assert!(!stale_override.starts_with(&data));

        // A real override still wins.
        let with_override = mihomo_binary_for(
            true,
            Some(Path::new("/dev/mihomo")),
            None,
            &data,
            production,
        );
        assert_eq!(with_override, PathBuf::from("/dev/mihomo"));
    }

    /// A production run keeps every behaviour it had: the search result wins,
    /// the fallback is used only when nothing was found, and a development
    /// override in the environment is ignored.
    #[test]
    fn a_production_run_still_prefers_the_discovered_mihomo() {
        let fallback = PathBuf::from("/usr/lib/biflow/mihomo");
        let data = PathBuf::from("/host/data/biflow");

        let discovered = mihomo_binary_for(
            false,
            Some(Path::new("/dev/mihomo")),
            Some(PathBuf::from("/usr/bin/mihomo")),
            &data,
            fallback.clone(),
        );
        assert_eq!(discovered, PathBuf::from("/usr/bin/mihomo"));

        let none_found = mihomo_binary_for(false, None, None, &data, fallback.clone());
        assert_eq!(none_found, fallback);
    }

    /// The development fallback must not exist by accident: a missing Mihomo
    /// has to read as missing so the operator runs `./dev.sh`.
    #[test]
    fn the_development_mihomo_fallback_is_inside_the_profile() {
        let data = PathBuf::from("/profile/data");
        let fallback = mihomo_binary_for(
            true,
            None,
            Some(PathBuf::from("/usr/lib/biflow/mihomo")),
            &data,
            PathBuf::from("/opt/biflow/mihomo"),
        );
        assert!(fallback.starts_with(&data));
        assert!(!fallback.starts_with("/usr/lib"));
        assert!(!fallback.starts_with("/opt"));
    }
}
