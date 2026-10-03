//! Authoritative runtime-profile resolution (ADR 0115).
//!
//! Profile identity used to be reconstructed independently by the config
//! store, the desktop path discovery, diagnostics, and every platform helper
//! backend. A development run and the installed application could therefore
//! disagree about their own locations, and the one site that actually *changed*
//! behavior (the Mihomo port/TUN remap) hid an environment read inside a
//! reusable library.
//!
//! This module is the single boundary where profile inputs are read and
//! turned into a resource policy. Consumers receive a [`ResolvedProfile`]
//! instead of re-deriving paths or re-reading the environment.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

/// Environment variable naming the development profile root. `dev.sh` exports
/// it before the helper, so the app and the transient helper agree.
pub const DEV_PROFILE_VAR: &str = "BIFLOW_DEV_PROFILE";
/// Endpoint (Unix socket path or Windows pipe name) of a development helper.
pub const DEV_HELPER_ENDPOINT_VAR: &str = "BIFLOW_DEV_HELPER_SOCKET";
/// Privileged runtime root of a development helper.
pub const DEV_SYSTEM_RUNTIME_VAR: &str = "BIFLOW_DEV_SYSTEM_RUNTIME";
/// Mihomo executable a development run should use instead of the installed one.
pub const DEV_MIHOMO_BINARY_VAR: &str = "BIFLOW_DEV_MIHOMO_BINARY";

/// Which profile this process runs under.
///
/// The semantics are deliberately unchanged: an unset or empty
/// `BIFLOW_DEV_PROFILE` means the installed application, and any nonempty
/// value means a development run rooted at that directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeProfile {
    /// The packaged/installed application. Uses the host's own directories
    /// and the production helper endpoint.
    Production,
    /// A development run isolated under `root`.
    Development { root: PathBuf },
}

impl RuntimeProfile {
    /// Resolve the profile from a raw environment value.
    #[must_use]
    pub fn resolve(raw: Option<&OsStr>) -> Self {
        match raw.filter(|value| !value.is_empty()) {
            Some(root) => Self::Development {
                root: PathBuf::from(root),
            },
            None => Self::Production,
        }
    }

    #[must_use]
    pub fn is_development(&self) -> bool {
        matches!(self, Self::Development { .. })
    }

    /// Root of a development profile, or `None` in production.
    #[must_use]
    pub fn root(&self) -> Option<&Path> {
        match self {
            Self::Development { root } => Some(root),
            Self::Production => None,
        }
    }
}

impl Default for RuntimeProfile {
    fn default() -> Self {
        Self::Production
    }
}

/// Raw profile-related process inputs.
///
/// This is the only place in the workspace that reads the profile
/// environment. Everything downstream takes the resolved values, so tests can
/// supply explicit inputs instead of mutating process-global state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileEnv {
    pub dev_profile: Option<OsString>,
    pub dev_helper_endpoint: Option<OsString>,
    pub dev_system_runtime: Option<OsString>,
    pub dev_mihomo_binary: Option<OsString>,
}

impl ProfileEnv {
    /// Read the profile environment once, at an explicit process boundary.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            dev_profile: std::env::var_os(DEV_PROFILE_VAR),
            dev_helper_endpoint: std::env::var_os(DEV_HELPER_ENDPOINT_VAR),
            dev_system_runtime: std::env::var_os(DEV_SYSTEM_RUNTIME_VAR),
            dev_mihomo_binary: std::env::var_os(DEV_MIHOMO_BINARY_VAR),
        }
    }

    #[must_use]
    pub fn profile(&self) -> RuntimeProfile {
        RuntimeProfile::resolve(self.dev_profile.as_deref())
    }
}

/// The host's own application directories, supplied by the caller so this
/// crate never depends on a directory-locating library or the host OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseDirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

/// Writable, unprivileged state owned by this user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserResources {
    pub config_root: PathBuf,
    pub data_root: PathBuf,
    pub cache_root: PathBuf,
    pub config_file: PathBuf,
    /// Permanent newline-delimited diagnostics log for this profile.
    pub debug_log: PathBuf,
}

/// Privileged, development-only resources owned by the transient helper.
///
/// These are *never* relocated into a mutable workspace and never relaxed:
/// they are only ever the explicit overrides `dev.sh` exports for a
/// per-user transient helper. In production every field is `None` and the
/// platform's own production endpoint is used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivilegedOverrides {
    pub helper_endpoint: Option<PathBuf>,
    pub system_runtime: Option<PathBuf>,
    pub mihomo_binary: Option<PathBuf>,
    /// Environment variables a development run still needs. A development run
    /// never inherits the production helper, so a missing override must be
    /// reported rather than silently downgraded to it.
    pub missing: Vec<&'static str>,
}

impl PrivilegedOverrides {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// One resolved, authoritative resource policy for this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProfile {
    profile: RuntimeProfile,
    user: UserResources,
    privileged: PrivilegedOverrides,
}

impl ResolvedProfile {
    /// Resolve the profile identity and the resources it owns.
    ///
    /// `base` supplies the production locations; a development profile
    /// replaces them with subdirectories of its own root.
    #[must_use]
    pub fn resolve(env: &ProfileEnv, base: BaseDirs) -> Self {
        let profile = env.profile();
        Self::from_parts(env, profile, base)
    }

    /// Resolve from an already-known profile identity, keeping the raw
    /// overrides from `env`. Used when a caller owns the profile decision.
    #[must_use]
    pub fn from_parts(env: &ProfileEnv, profile: RuntimeProfile, base: BaseDirs) -> Self {
        let (config_root, data_root, cache_root) = if let Some(root) = profile.root() {
            (root.join("config"), root.join("data"), root.join("cache"))
        } else {
            (base.config, base.data, base.cache)
        };
        let user = UserResources {
            config_file: config_root.join("config.toml"),
            debug_log: data_root.join("debug.log"),
            config_root,
            data_root,
            cache_root,
        };
        let privileged = if profile.is_development() {
            let mut missing = Vec::new();
            let helper_endpoint = override_path(
                env.dev_helper_endpoint.as_deref(),
                &mut missing,
                DEV_HELPER_ENDPOINT_VAR,
            );
            let system_runtime = override_path(
                env.dev_system_runtime.as_deref(),
                &mut missing,
                DEV_SYSTEM_RUNTIME_VAR,
            );
            let mihomo_binary = override_path(
                env.dev_mihomo_binary.as_deref(),
                &mut missing,
                DEV_MIHOMO_BINARY_VAR,
            );
            PrivilegedOverrides {
                helper_endpoint,
                system_runtime,
                mihomo_binary,
                missing,
            }
        } else {
            PrivilegedOverrides::default()
        };
        Self {
            profile,
            user,
            privileged,
        }
    }

    #[must_use]
    pub fn profile(&self) -> &RuntimeProfile {
        &self.profile
    }

    #[must_use]
    pub fn is_development(&self) -> bool {
        self.profile.is_development()
    }

    #[must_use]
    pub fn user(&self) -> &UserResources {
        &self.user
    }

    #[must_use]
    pub fn privileged(&self) -> &PrivilegedOverrides {
        &self.privileged
    }

    /// Configuration document for this profile.
    #[must_use]
    pub fn config_file(&self) -> &Path {
        &self.user.config_file
    }

    /// Permanent diagnostics log for this profile. Derived from the same data
    /// root as the rest of the user's state, so the app can never write into
    /// a directory the diagnostics card does not also report.
    #[must_use]
    pub fn debug_log(&self) -> &Path {
        &self.user.debug_log
    }

    /// Development-only Mihomo executable override.
    #[must_use]
    pub fn dev_mihomo_binary(&self) -> Option<&Path> {
        self.privileged.mihomo_binary.as_deref()
    }
}

fn override_path(
    value: Option<&OsStr>,
    missing: &mut Vec<&'static str>,
    variable: &'static str,
) -> Option<PathBuf> {
    let Some(path) = value.filter(|value| !value.is_empty()) else {
        missing.push(variable);
        return None;
    };
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> BaseDirs {
        BaseDirs {
            config: PathBuf::from("/home/u/.config/biflow"),
            data: PathBuf::from("/home/u/.local/share/biflow"),
            cache: PathBuf::from("/home/u/.cache/biflow"),
        }
    }

    fn dev_env() -> ProfileEnv {
        ProfileEnv {
            dev_profile: Some(OsString::from("/tmp/dev-profile")),
            dev_helper_endpoint: Some(OsString::from("/run/biflow-dev-1000/helper.sock")),
            dev_system_runtime: Some(OsString::from("/var/lib/biflow-dev-1000")),
            dev_mihomo_binary: Some(OsString::from("/var/lib/biflow-dev-1000/bin/mihomo")),
        }
    }

    #[test]
    fn unset_profile_is_production() {
        assert_eq!(RuntimeProfile::resolve(None), RuntimeProfile::Production);
    }

    #[test]
    fn empty_profile_is_production() {
        assert_eq!(
            RuntimeProfile::resolve(Some(OsStr::new(""))),
            RuntimeProfile::Production
        );
    }

    #[test]
    fn nonempty_profile_is_development_at_that_root() {
        assert_eq!(
            RuntimeProfile::resolve(Some(OsStr::new("/tmp/dev"))),
            RuntimeProfile::Development {
                root: PathBuf::from("/tmp/dev")
            }
        );
    }

    #[test]
    fn production_resources_use_the_host_directories() {
        let resolved = ResolvedProfile::resolve(&ProfileEnv::default(), base());
        assert!(!resolved.is_development());
        assert_eq!(
            resolved.config_file(),
            Path::new("/home/u/.config/biflow/config.toml")
        );
        assert_eq!(
            resolved.debug_log(),
            Path::new("/home/u/.local/share/biflow/debug.log")
        );
        assert_eq!(
            resolved.user().cache_root,
            PathBuf::from("/home/u/.cache/biflow")
        );
    }

    #[test]
    fn production_never_carries_privileged_overrides() {
        // Overrides present in the environment must not leak into a
        // production run just because they happen to be exported.
        let env = ProfileEnv {
            dev_helper_endpoint: Some(OsString::from("/run/somewhere/helper.sock")),
            dev_mihomo_binary: Some(OsString::from("/tmp/mihomo")),
            ..ProfileEnv::default()
        };
        let resolved = ResolvedProfile::resolve(&env, base());
        assert_eq!(resolved.privileged(), &PrivilegedOverrides::default());
        assert!(resolved.dev_mihomo_binary().is_none());
    }

    #[test]
    fn development_resources_stay_inside_the_profile_root() {
        let resolved = ResolvedProfile::resolve(&dev_env(), base());
        assert!(resolved.is_development());
        assert_eq!(
            resolved.config_file(),
            Path::new("/tmp/dev-profile/config/config.toml")
        );
        assert_eq!(
            resolved.debug_log(),
            Path::new("/tmp/dev-profile/data/debug.log")
        );
        assert_eq!(
            resolved.user().cache_root,
            PathBuf::from("/tmp/dev-profile/cache")
        );
    }

    #[test]
    fn development_shares_no_path_with_production() {
        let production = ResolvedProfile::resolve(&ProfileEnv::default(), base());
        let development = ResolvedProfile::resolve(&dev_env(), base());
        for (name, left, right) in [
            (
                "config",
                production.config_file(),
                development.config_file(),
            ),
            (
                "data",
                &production.user().data_root,
                &development.user().data_root,
            ),
            (
                "cache",
                &production.user().cache_root,
                &development.user().cache_root,
            ),
        ] {
            assert!(!left.starts_with(right), "{name} shares a root");
            assert!(!right.starts_with(left), "{name} shares a root");
        }
    }

    #[test]
    fn development_reports_missing_privileged_overrides_instead_of_inheriting_production() {
        let env = ProfileEnv {
            dev_profile: Some(OsString::from("/tmp/dev-profile")),
            ..ProfileEnv::default()
        };
        let resolved = ResolvedProfile::resolve(&env, base());
        let privileged = resolved.privileged();
        assert!(!privileged.is_complete());
        assert!(privileged.helper_endpoint.is_none());
        assert!(privileged.system_runtime.is_none());
        assert_eq!(
            privileged.missing,
            vec![
                DEV_HELPER_ENDPOINT_VAR,
                DEV_SYSTEM_RUNTIME_VAR,
                DEV_MIHOMO_BINARY_VAR
            ]
        );
    }

    #[test]
    fn a_single_missing_override_is_still_reported() {
        let mut env = dev_env();
        env.dev_system_runtime = None;
        let resolved = ResolvedProfile::resolve(&env, base());
        assert_eq!(resolved.privileged().missing, vec![DEV_SYSTEM_RUNTIME_VAR]);
        assert!(resolved.privileged().helper_endpoint.is_some());
    }

    #[test]
    fn an_empty_override_counts_as_missing() {
        let mut env = dev_env();
        env.dev_helper_endpoint = Some(OsString::new());
        let resolved = ResolvedProfile::resolve(&env, base());
        assert_eq!(resolved.privileged().missing, vec![DEV_HELPER_ENDPOINT_VAR]);
    }

    #[test]
    fn development_keeps_the_explicit_privileged_overrides() {
        let resolved = ResolvedProfile::resolve(&dev_env(), base());
        let privileged = resolved.privileged();
        assert!(privileged.is_complete());
        assert_eq!(
            privileged.helper_endpoint.as_deref(),
            Some(Path::new("/run/biflow-dev-1000/helper.sock"))
        );
        assert_eq!(
            privileged.system_runtime.as_deref(),
            Some(Path::new("/var/lib/biflow-dev-1000"))
        );
        assert_eq!(
            resolved.dev_mihomo_binary(),
            Some(Path::new("/var/lib/biflow-dev-1000/bin/mihomo"))
        );
    }

    #[test]
    fn dev_profile_env_reads_the_documented_variables() {
        // The variable names are a contract with `dev.sh`; keep them pinned.
        assert_eq!(DEV_PROFILE_VAR, "BIFLOW_DEV_PROFILE");
        assert_eq!(DEV_HELPER_ENDPOINT_VAR, "BIFLOW_DEV_HELPER_SOCKET");
        assert_eq!(DEV_SYSTEM_RUNTIME_VAR, "BIFLOW_DEV_SYSTEM_RUNTIME");
        assert_eq!(DEV_MIHOMO_BINARY_VAR, "BIFLOW_DEV_MIHOMO_BINARY");
    }
}
