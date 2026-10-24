use iran_split_core::{LifecycleBusy, StackPhase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayLabels {
    pub connection_id: &'static str,
    pub connection_label: &'static str,
    pub pause_id: &'static str,
    pub pause_label: &'static str,
}

/// One item from each pair: Connect/Disconnect and Pause/Resume.
#[must_use]
pub fn labels_for(phase: StackPhase) -> TrayLabels {
    let connected = matches!(
        phase,
        StackPhase::Running | StackPhase::Degraded | StackPhase::Paused
    );
    let paused = matches!(phase, StackPhase::Paused);
    TrayLabels {
        connection_id: if connected { "disconnect" } else { "connect" },
        connection_label: if connected { "Disconnect" } else { "Connect" },
        pause_id: if paused { "resume" } else { "pause" },
        pause_label: if paused { "Resume" } else { "Pause" },
    }
}

#[must_use]
pub const fn actions_enabled(busy: Option<LifecycleBusy>) -> bool {
    busy.is_none()
}

/// Phase and in-flight action that the tray menu already describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayMenuKey {
    pub phase: StackPhase,
    pub busy: Option<LifecycleBusy>,
}

#[must_use]
pub const fn menu_key(phase: StackPhase, busy: Option<LifecycleBusy>) -> TrayMenuKey {
    TrayMenuKey { phase, busy }
}

/// A snapshot that still describes the menu on screen does not need a rebuild.
#[must_use]
pub fn menu_changed(previous: Option<TrayMenuKey>, next: TrayMenuKey) -> bool {
    match previous {
        Some(previous) => previous.phase != next.phase || previous.busy != next.busy,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_shows_connect_and_pause() {
        let labels = labels_for(StackPhase::Stopped);
        assert_eq!(labels.connection_id, "connect");
        assert_eq!(labels.connection_label, "Connect");
        assert_eq!(labels.pause_id, "pause");
        assert_eq!(labels.pause_label, "Pause");
    }

    #[test]
    fn running_shows_disconnect_and_pause() {
        let labels = labels_for(StackPhase::Running);
        assert_eq!(labels.connection_id, "disconnect");
        assert_eq!(labels.pause_id, "pause");
    }

    #[test]
    fn paused_shows_disconnect_and_resume() {
        let labels = labels_for(StackPhase::Paused);
        assert_eq!(labels.connection_id, "disconnect");
        assert_eq!(labels.connection_label, "Disconnect");
        assert_eq!(labels.pause_id, "resume");
        assert_eq!(labels.pause_label, "Resume");
    }

    #[test]
    fn menu_rebuilds_only_when_the_phase_or_busy_flag_changes() {
        let running = menu_key(StackPhase::Running, None);
        assert!(menu_changed(None, running));
        assert!(!menu_changed(Some(running), running));
        let connecting = menu_key(StackPhase::Running, Some(LifecycleBusy::Connecting));
        assert!(menu_changed(Some(running), connecting));
        assert!(!menu_changed(Some(connecting), connecting));
        let paused = menu_key(StackPhase::Paused, None);
        assert!(menu_changed(Some(running), paused));
    }

    #[test]
    fn never_emits_both_options_from_the_same_pair() {
        for phase in [
            StackPhase::Uninitialized,
            StackPhase::Stopped,
            StackPhase::StartingClient,
            StackPhase::PreparingRuntime,
            StackPhase::ValidatingConfig,
            StackPhase::StartingCore,
            StackPhase::CheckingReadiness,
            StackPhase::Running,
            StackPhase::Paused,
            StackPhase::Degraded,
            StackPhase::Stopping,
            StackPhase::Recovering,
            StackPhase::Error,
        ] {
            let labels = labels_for(phase);
            assert!((labels.connection_id == "connect") ^ (labels.connection_id == "disconnect"));
            assert!((labels.pause_id == "pause") ^ (labels.pause_id == "resume"));
        }
    }

    #[test]
    fn tray_actions_disable_while_a_lifecycle_lock_is_held() {
        assert!(actions_enabled(None));
        assert!(!actions_enabled(Some(LifecycleBusy::Connecting)));
        assert!(!actions_enabled(Some(LifecycleBusy::Disconnecting)));
        assert!(!actions_enabled(Some(LifecycleBusy::Pausing)));
        assert!(!actions_enabled(Some(LifecycleBusy::Resuming)));
        assert!(!actions_enabled(Some(LifecycleBusy::ApplyingRules)));
    }

    #[test]
    fn dashboard_navigation_is_not_gated_by_lifecycle_busy() {
        assert!(actions_enabled(None));
        assert!(!actions_enabled(Some(LifecycleBusy::ApplyingRules)));
        assert!(!actions_enabled(Some(LifecycleBusy::Connecting)));
    }
}
