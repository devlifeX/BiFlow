//! Windows named-pipe ACL helpers for the privileged helper process.
//!
//! Isolated from workspace `unsafe_code = "forbid"` because Tokio and Win32
//! require an `unsafe` `SECURITY_ATTRIBUTES` pointer when creating a pipe that
//! Medium-integrity desktop clients can open.

/// SDDL allowing SYSTEM, Administrators, and local Users at Medium integrity.
///
/// `ME` (Medium) is required so a non-elevated desktop process can connect to a
/// pipe created by SYSTEM. Remote clients are rejected by the pipe mode flags.
pub const HELPER_PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;BU)S:(ML;;NW;;;ME)";
/// SDDL granting the runtime pipe owner and SYSTEM full access.
pub const RUNTIME_PIPE_SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

#[cfg(windows)]
mod windows_impl;

#[cfg(windows)]
pub use windows_impl::{create_helper_server, create_runtime_server};

#[cfg(test)]
mod tests {
    use super::HELPER_PIPE_SDDL;

    #[test]
    fn packaged_sddl_allows_users_at_medium_integrity() {
        assert!(HELPER_PIPE_SDDL.contains("BU"));
        assert!(HELPER_PIPE_SDDL.contains("ME"));
        assert!(HELPER_PIPE_SDDL.contains("SY"));
    }

    #[test]
    fn runtime_pipe_acl_is_limited_to_owner_and_system() {
        assert!(super::RUNTIME_PIPE_SDDL.contains("OW"));
        assert!(super::RUNTIME_PIPE_SDDL.contains("SY"));
        assert!(!super::RUNTIME_PIPE_SDDL.contains("BU"));
    }
}
