use iran_split_core::ComponentPhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectRequirement {
    Helper,
    Hiddify,
    Mihomo,
}

impl ConnectRequirement {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Helper => "helper",
            Self::Hiddify => "hiddify",
            Self::Mihomo => "mihomo",
        }
    }
}

#[must_use]
pub fn helper_is_ready(phase: ComponentPhase) -> bool {
    !matches!(phase, ComponentPhase::Unavailable | ComponentPhase::Error)
}

/// `hiddify_satisfied` is "installed, or not required at all": Hiddify is
/// only a connect requirement while an enabled Hiddify client exists. An
/// operator who disabled Hiddify and promoted another client (e.g. Happ)
/// to the default route must not be forced to install it. Mirrors
/// `missingConnectRequirements` in the desktop frontend.
#[must_use]
pub fn missing_requirements(
    helper_ready: bool,
    hiddify_satisfied: bool,
    mihomo_installed: bool,
) -> Vec<ConnectRequirement> {
    let mut missing = Vec::new();
    if !helper_ready {
        missing.push(ConnectRequirement::Helper);
    }
    if !hiddify_satisfied {
        missing.push(ConnectRequirement::Hiddify);
    }
    if !mihomo_installed {
        missing.push(ConnectRequirement::Mihomo);
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_helper_then_hiddify_then_mihomo() {
        assert_eq!(
            missing_requirements(false, false, false),
            [
                ConnectRequirement::Helper,
                ConnectRequirement::Hiddify,
                ConnectRequirement::Mihomo
            ]
        );
        assert_eq!(
            missing_requirements(true, true, true),
            [] as [ConnectRequirement; 0]
        );
    }

    #[test]
    fn hiddify_satisfied_covers_the_no_enabled_client_case() {
        // Not installed but also not required: an operator without an
        // enabled Hiddify client connects without being forced to install.
        assert_eq!(
            missing_requirements(true, true, false),
            [ConnectRequirement::Mihomo]
        );
    }

    #[test]
    fn treats_only_unavailable_or_error_helpers_as_missing() {
        assert!(!helper_is_ready(ComponentPhase::Unavailable));
        assert!(!helper_is_ready(ComponentPhase::Error));
        assert!(helper_is_ready(ComponentPhase::Running));
        assert!(helper_is_ready(ComponentPhase::Stopped));
        assert!(helper_is_ready(ComponentPhase::Degraded));
    }
}
