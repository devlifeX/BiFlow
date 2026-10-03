//! Process boundary for the resolved runtime profile (ADR 0115).
//!
//! Profile identity used to be re-derived from the environment by path
//! discovery, diagnostics, the Mihomo binary lookup, every helper backend,
//! and the production-install guard. Each site could disagree with the others,
//! and none of them were testable without mutating process-global state.
//!
//! [`iran_split_config::ResolvedProfile`] is the authoritative representation.
//! This module owns exactly one job: read the environment once, at a real
//! process boundary, resolve the policy, and hand the result to consumers.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use iran_split_config::{BaseDirs, ProfileEnv, ResolvedProfile, RuntimeProfile};

static RESOLVED: OnceLock<ResolvedProfile> = OnceLock::new();

/// The host's own application directories.
///
/// Supplied here rather than inside `iran-split-config` so that crate stays
/// free of a directory-locating dependency and of any host assumption.
///
/// A missing host directory is a hard error. Substituting a relative path
/// would silently write the configuration document and the permanent
/// `debug.log` somewhere the Diagnostics card cannot reason about, which is
/// exactly the class of path drift this module exists to remove (ADR 0115).
fn host_base_dirs() -> Result<BaseDirs, String> {
    let config = dirs::config_dir()
        .ok_or("configuration directory is unavailable for the runtime profile")?
        .join("biflow");
    let data = dirs::data_local_dir()
        .ok_or("local data directory is unavailable for the runtime profile")?
        .join("biflow");
    let cache = dirs::cache_dir()
        .ok_or("cache directory is unavailable for the runtime profile")?
        .join("biflow");
    Ok(BaseDirs {
        config,
        data,
        cache,
    })
}

/// Resolve the profile for this process, once, reporting failure.
///
/// A development run never consults the host directories, so it resolves even
/// when they are unavailable. Only production can fail here.
pub fn try_resolved() -> Result<&'static ResolvedProfile, String> {
    if let Some(existing) = RESOLVED.get() {
        return Ok(existing);
    }
    let env = ProfileEnv::from_process();
    // Resolve against a placeholder first: a development profile discards the
    // base directories entirely, so it must not depend on them existing.
    let base = host_base_dirs().unwrap_or_else(|_| placeholder_base_dirs());
    let resolved = ResolvedProfile::resolve(&env, base);
    if !resolved.is_development() {
        host_base_dirs()?;
    }
    Ok(RESOLVED.get_or_init(|| resolved))
}

fn placeholder_base_dirs() -> BaseDirs {
    let unusable = PathBuf::from("\u{0}biflow-profile-unused");
    BaseDirs {
        config: unusable.clone(),
        data: unusable.clone(),
        cache: unusable,
    }
}

/// Resolve the profile for this process, once.
///
/// Called at the top of `run()` before any consumer runs. The result is
/// process-wide because the profile is a property of the process, not of a
/// request, and several consumers (diagnostics initialization, the Linux
/// `WebKit` re-exec, Tauri setup) need it before any Tauri state exists.
///
/// # Panics
///
/// Panics when a production run cannot locate the host's application
/// directories. There is no safe fallback location: writing the config
/// document and `debug.log` relative to the working directory would put user
/// state somewhere nothing reports it.
#[must_use]
pub fn resolved() -> &'static ResolvedProfile {
    try_resolved()
        .unwrap_or_else(|cause| panic!("BiFlow runtime profile could not be resolved: {cause}"))
}

/// Profile identity of this process.
#[must_use]
pub fn runtime() -> &'static RuntimeProfile {
    resolved().profile()
}

/// `true` for a development run. Production provisioning (the elevated helper
/// installer) must refuse under this flag.
#[must_use]
pub fn is_development() -> bool {
    resolved().is_development()
}

/// Outcome of the pre-provisioning gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisioningGate {
    /// The installed application may modify machine-wide state.
    Allowed,
    /// A development run must not touch the installed app's helper.
    RefusedDevelopmentProfile,
}

/// Decide whether this run may provision privileged, machine-wide state.
///
/// Every path that can rewrite the installed helper â€” the Install button, the
/// Connect flow's missing-helper branch, and its version-mismatch reinstall â€”
/// goes through this one decision. Guarding only the Tauri command left
/// `prepare_stack_start` free to run the production installer from a
/// development window, which is the exact failure ADR 0115 exists to prevent.
#[must_use]
pub fn provisioning_gate(is_development: bool) -> ProvisioningGate {
    if is_development {
        ProvisioningGate::RefusedDevelopmentProfile
    } else {
        ProvisioningGate::Allowed
    }
}

/// Gate shared by every privileged provisioning path.
///
/// `initiator` names the caller so the diagnostic and the operator message say
/// which route was refused.
pub fn ensure_provisioning_allowed(initiator: &str) -> Result<(), String> {
    ensure_provisioning_allowed_for(is_development(), initiator, missing_development_overrides())
}

/// Pure core of [`ensure_provisioning_allowed`].
///
/// Separated so the refusal can be asserted on every host without a Tauri
/// `AppHandle` and without reading the process environment — a test that
/// cannot run under `BIFLOW_DEV_PROFILE` does not actually protect the rule.
pub fn ensure_provisioning_allowed_for(
    is_development: bool,
    initiator: &str,
    missing_overrides: &[&'static str],
) -> Result<(), String> {
    if provisioning_gate(is_development) == ProvisioningGate::Allowed {
        return Ok(());
    }
    let detail = if missing_overrides.is_empty() {
        String::new()
    } else {
        format!(
            " missing development helper overrides: {}.",
            missing_overrides.join(", ")
        )
    };
    tracing::warn!(
        event = "profile.provisioning_refused",
        section = "helper_install",
        initiator = "profile::ensure_provisioning_allowed",
        cause = "development_profile_cannot_provision_production",
        trace_route = "application_process->profile->provisioning_gate",
        route = initiator,
        missing = ?missing_overrides,
        "refused to run the production helper installer from a development run"
    );
    Err(format!(
        "development run: restart ./dev.sh to provision the transient helper; \
         the production helper installer is disabled in dev \
         (refused route: {initiator}){detail}"
    ))
}

/// Development-only Mihomo executable override, if `dev.sh` supplied one.
#[must_use]
pub fn dev_mihomo_binary() -> Option<&'static std::path::Path> {
    resolved().dev_mihomo_binary()
}

/// Environment variables this development run still needs, so the operator can
/// be told exactly what `./dev.sh` did not export.
#[must_use]
pub fn missing_development_overrides() -> &'static [&'static str] {
    &resolved().privileged().missing
}

/// Resolve the privileged helper endpoint and runtime root for this process.
///
/// Shared by every platform backend so "a development run never inherits
/// production" has exactly one implementation (ADR 0115). A development run
/// without explicit overrides gets an unreachable placeholder, and the
/// operator is told which variables are missing instead of being silently
/// pointed at the installed app's helper.
///
/// `production_*` are the platform's installed-application locations and
/// `missing_*` are per-platform placeholders no helper ever serves.
#[must_use]
pub fn helper_paths(
    production_endpoint: &str,
    production_runtime: &str,
    missing_endpoint: &str,
    missing_runtime: &str,
) -> (PathBuf, PathBuf) {
    let resolved = resolved();
    if resolved.is_development() {
        let missing = missing_development_overrides();
        if !missing.is_empty() {
            tracing::warn!(
                event = "profile.development_overrides_missing",
                section = "startup",
                initiator = "create_services",
                cause = "development_helper_not_provisioned",
                trace_route = "application_process->profile->helper_paths",
                missing = ?missing,
                "development run has no isolated helper; the installed helper is never used as a fallback"
            );
        }
    }
    helper_paths_for(
        resolved.is_development(),
        resolved.privileged().helper_endpoint.as_deref(),
        resolved.privileged().system_runtime.as_deref(),
        production_endpoint,
        production_runtime,
        missing_endpoint,
        missing_runtime,
    )
}

/// Pure core of [`helper_paths`], with explicit inputs.
///
/// The rule this encodes is the whole point: a development run resolves to its
/// own override, or to a placeholder that no helper serves. It never returns
/// the production endpoint or runtime, whatever the caller passes.
#[must_use]
#[allow(
    clippy::fn_params_excessive_bools,
    reason = "one flag, two options, two fallbacks"
)]
pub fn helper_paths_for(
    is_development: bool,
    development_endpoint: Option<&std::path::Path>,
    development_runtime: Option<&std::path::Path>,
    production_endpoint: &str,
    production_runtime: &str,
    missing_endpoint: &str,
    missing_runtime: &str,
) -> (PathBuf, PathBuf) {
    if !is_development {
        return (
            PathBuf::from(production_endpoint),
            PathBuf::from(production_runtime),
        );
    }
    (
        development_endpoint.map_or_else(|| PathBuf::from(missing_endpoint), Path::to_path_buf),
        development_runtime.map_or_else(|| PathBuf::from(missing_runtime), Path::to_path_buf),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn development_helper_paths_accept_explicit_overrides() {
        let endpoint = Path::new("/run/biflow-dev-test/helper.sock");
        let runtime = Path::new("/var/lib/biflow-dev-test/runtime");
        let paths = helper_paths_for(
            true,
            Some(endpoint),
            Some(runtime),
            "/run/iran-split/helper.sock",
            "/var/lib/iran-split",
            "/run/biflow-dev/missing-helper.sock",
            "/run/biflow-dev/missing-runtime",
        );
        assert_eq!(paths, (endpoint.to_path_buf(), runtime.to_path_buf()));
    }

    /// The policy for explicit inputs, without touching the process-wide
    /// `OnceLock` or the ambient environment.
    fn resolve(env: &ProfileEnv) -> ResolvedProfile {
        ResolvedProfile::resolve(env, host_base_dirs().expect("host base dirs"))
    }

    #[test]
    fn production_run_resolves_to_the_host_biflow_directories() {
        let resolved = resolve(&ProfileEnv::default());
        assert!(!resolved.is_development());
        assert!(resolved.config_file().ends_with("biflow/config.toml"));
        assert!(resolved.debug_log().ends_with("biflow/debug.log"));
    }

    #[test]
    fn development_run_keeps_every_resource_under_the_profile_root() {
        let root = PathBuf::from("/tmp/biflow-dev-profile");
        let resolved = resolve(&ProfileEnv {
            dev_profile: Some(OsString::from(&root)),
            dev_helper_endpoint: Some(OsString::from("/run/biflow-dev-1000/helper.sock")),
            dev_system_runtime: Some(OsString::from("/var/lib/biflow-dev-1000")),
            dev_mihomo_binary: Some(OsString::from("/var/lib/biflow-dev-1000/bin/mihomo")),
        });
        assert!(resolved.is_development());
        for path in [
            resolved.config_file(),
            resolved.debug_log(),
            &resolved.user().cache_root,
        ] {
            assert!(
                path.starts_with(&root),
                "{} escaped the profile",
                path.display()
            );
        }
    }

    #[test]
    fn development_run_reports_missing_helper_overrides() {
        let resolved = resolve(&ProfileEnv {
            dev_profile: Some(OsString::from("/tmp/biflow-dev-profile")),
            ..ProfileEnv::default()
        });
        assert!(resolved.is_development());
        assert!(!resolved.privileged().is_complete());
        assert_eq!(
            resolved.privileged().missing,
            vec![
                iran_split_config::DEV_HELPER_ENDPOINT_VAR,
                iran_split_config::DEV_SYSTEM_RUNTIME_VAR,
                iran_split_config::DEV_MIHOMO_BINARY_VAR,
            ]
        );
    }

    #[test]
    fn production_run_reports_no_missing_overrides() {
        let resolved = resolve(&ProfileEnv::default());
        assert!(resolved.privileged().is_complete());
        assert!(resolved.dev_mihomo_binary().is_none());
    }

    #[test]
    fn process_resolution_is_stable_and_cached() {
        // The process-wide accessor must return the same policy every time;
        // a second resolution that disagreed with the first would reintroduce
        // exactly the drift this module exists to remove.
        let first = resolved() as *const ResolvedProfile;
        let second = resolved() as *const ResolvedProfile;
        assert!(std::ptr::eq(first, second));
        assert_eq!(runtime().is_development(), is_development());
    }
}
