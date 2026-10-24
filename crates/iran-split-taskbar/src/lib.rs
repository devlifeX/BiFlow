//! Native Windows taskbar boundary. No statistics collection or `WebView`.
use iran_split_config::SpeedUnit;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod native;
#[cfg(target_os = "macos")]
pub use macos::{apply_menu_bar_label, menu_bar_is_main_thread};
#[cfg(windows)]
pub use native::{diagnostic_renderer, diagnostic_status, Display, Status};

#[cfg(any(windows, test))]
fn rows(upload: Option<f64>, download: Option<f64>) -> [(&'static str, Option<f64>); 2] {
    [("↑", upload), ("↓", download)]
}

/// Chooses the rightmost verified gap, adjacent to the notification area.
/// All coordinates are taskbar-client physical pixels. Unknown layouts fail closed.
#[must_use]
pub fn find_gap(width: i32, occupied: &[(i32, i32)], requested: i32, margin: i32) -> Option<i32> {
    if width <= 0 || requested <= 0 || margin < 0 || occupied.is_empty() {
        return None;
    }
    let mut spans = occupied.to_vec();
    spans.sort_unstable();
    let mut cursor = margin;
    let mut best = None;
    for (left, right) in spans {
        if left < 0 || right < left || right > width {
            return None;
        }
        if left - cursor >= requested + margin {
            best = Some(left - requested - margin);
        }
        cursor = cursor.max(right + margin);
    }
    if width - cursor >= requested + margin {
        best = Some(width - requested - margin);
    }
    best
}

/// Decimal bits/second, matching Windows Task Manager's throughput units.
/// The shared provider supplies byte rates; this is the sole bit conversion.
#[must_use]
pub fn format_rate(rate: Option<f64>) -> String {
    format_rate_in(rate, SpeedUnit::AutoBits)
}

/// Rates arrive in bytes/second; display units use decimal factors of 1000.
#[must_use]
pub fn format_rate_in(rate: Option<f64>, unit: SpeedUnit) -> String {
    let (bits, mut index, automatic) = match unit {
        SpeedUnit::AutoBits => (true, 0, true),
        SpeedUnit::AutoBytes => (false, 0, true),
        SpeedUnit::Bits => (true, 0, false),
        SpeedUnit::Kilobits => (true, 1, false),
        SpeedUnit::Megabits => (true, 2, false),
        SpeedUnit::Gigabits => (true, 3, false),
        SpeedUnit::Bytes => (false, 0, false),
        SpeedUnit::Kilobytes => (false, 1, false),
        SpeedUnit::Megabytes => (false, 2, false),
        SpeedUnit::Gigabytes => (false, 3, false),
    };
    let units = if bits {
        ["bps", "Kbps", "Mbps", "Gbps", "Tbps", "Pbps"]
    } else {
        ["B/s", "KB/s", "MB/s", "GB/s", "TB/s", "PB/s"]
    };
    let Some(mut value) = rate
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|bytes| bytes * if bits { 8.0 } else { 1.0 })
        .filter(|value| value.is_finite())
    else {
        return format!("— {}", units[index]);
    };
    if automatic {
        while value >= 1000.0 && index < units.len() - 1 {
            value /= 1000.0;
            index += 1;
        }
    } else {
        for _ in 0..index {
            value /= 1000.0;
        }
    }
    if index <= 1 || value >= 100.0 {
        format!("{value:.0} {}", units[index])
    } else {
        format!("{value:.1} {}", units[index])
    }
}

/// Columns for one auto-scaled rate, a space, and the arrow. `99.9 Kbps`
/// is the longest automatic rate, so normal updates do not resize the item.
const MENU_BAR_COLUMNS: usize = 11;

/// What the menu bar already shows, if this process has drawn it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuBarRecord<'a> {
    /// No title has been applied in this process.
    Unknown,
    /// The status icon is visible and the speed title is clear.
    Hidden,
    /// The stacked speed title currently on the status item.
    Shown(&'a str),
}

/// How the next menu-bar sample may touch `AppKit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuBarUpdate {
    /// The visible title already matches. Do not call `AppKit`.
    Unchanged,
    /// Replace the attributed title. The item width stays the same.
    Title,
    /// Show or hide the speeds, or change the line width.
    Layout,
}

/// Digit changes inside the fixed column width must not resize the status item.
///
/// Resizing enters the status-item scene update, which can block the main
/// thread until the process is force-quit.
#[must_use]
pub fn menu_bar_update(previous: MenuBarRecord<'_>, next: Option<&str>) -> MenuBarUpdate {
    match (previous, next) {
        (MenuBarRecord::Unknown | MenuBarRecord::Hidden, None) => MenuBarUpdate::Unchanged,
        (MenuBarRecord::Shown(previous), Some(next)) if previous == next => {
            MenuBarUpdate::Unchanged
        }
        (MenuBarRecord::Shown(previous), Some(next))
            if menu_bar_line_width(previous) == menu_bar_line_width(next) =>
        {
            MenuBarUpdate::Title
        }
        _ => MenuBarUpdate::Layout,
    }
}

fn menu_bar_line_width(label: &str) -> usize {
    label
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

/// Upload above download, each line the same width, arrows aligned.
#[must_use]
pub fn menu_bar_label(upload: Option<f64>, download: Option<f64>, unit: SpeedUnit) -> String {
    let upload = format_rate_in(upload, unit);
    let download = format_rate_in(download, unit);
    let width = MENU_BAR_COLUMNS
        .max(aligned_width(&upload))
        .max(aligned_width(&download));
    format!(
        "{}\n{}",
        align_line('↑', &upload, width),
        align_line('↓', &download, width)
    )
}

fn aligned_width(rate: &str) -> usize {
    2 + rate.chars().count()
}

fn align_line(arrow: char, rate: &str, width: usize) -> String {
    let body = aligned_width(rate);
    let pad = width.saturating_sub(body);
    format!("{}{rate} {arrow}", " ".repeat(pad))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_never_overlap_widgets_apps_or_tray() {
        let controls = [(0, 220), (500, 900), (1100, 1200)];
        assert_eq!(find_gap(1200, &controls, 160, 8), Some(932));
        assert_eq!(find_gap(800, &[(0, 650), (700, 800)], 160, 8), None);
        assert_eq!(find_gap(800, &[], 160, 8), None);
        assert_eq!(find_gap(800, &[(0, 900)], 160, 8), None);
        assert_eq!(
            find_gap(800, &[(0, 200), (100, 500), (700, 800)], 160, 8),
            Some(532)
        );
    }

    #[test]
    fn formatting_matches_task_manager_bits_and_unavailable_state() {
        assert_eq!(format_rate(Some(0.0)), "0 bps");
        assert_eq!(format_rate(Some(14_000.0)), "112 Kbps");
        assert_eq!(format_rate(Some(450_000.0)), "3.6 Mbps");
        assert_eq!(format_rate(None), "— bps");
        assert_eq!(format_rate(Some(f64::NAN)), "— bps");
    }

    #[test]
    fn selected_units_distinguish_bits_bytes_and_fixed_scales() {
        let rate = Some(1_000_000.0);
        for (unit, expected) in [
            (SpeedUnit::AutoBits, "8.0 Mbps"),
            (SpeedUnit::AutoBytes, "1.0 MB/s"),
            (SpeedUnit::Bits, "8000000 bps"),
            (SpeedUnit::Kilobits, "8000 Kbps"),
            (SpeedUnit::Megabits, "8.0 Mbps"),
            (SpeedUnit::Gigabits, "0.0 Gbps"),
            (SpeedUnit::Bytes, "1000000 B/s"),
            (SpeedUnit::Kilobytes, "1000 KB/s"),
            (SpeedUnit::Megabytes, "1.0 MB/s"),
            (SpeedUnit::Gigabytes, "0.0 GB/s"),
        ] {
            assert_eq!(format_rate_in(rate, unit), expected);
        }
        assert_eq!(
            format_rate_in(Some(2_000_000_000.0), SpeedUnit::Gigabytes),
            "2.0 GB/s"
        );
        for invalid in [None, Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
            assert_eq!(format_rate_in(invalid, SpeedUnit::Megabytes), "— MB/s");
        }
    }

    #[test]
    fn menu_bar_lines_share_one_width_and_keep_the_arrows_aligned() {
        let label = menu_bar_label(Some(14_000.0), Some(450_000.0), SpeedUnit::AutoBits);
        let lines: Vec<_> = label.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].chars().count(), lines[1].chars().count());
        assert_eq!(lines[0].chars().count(), MENU_BAR_COLUMNS);
        assert!(lines[0].ends_with("112 Kbps ↑"));
        assert!(lines[1].ends_with("3.6 Mbps ↓"));
        let quiet = menu_bar_label(None, Some(0.0), SpeedUnit::AutoBits);
        let quiet: Vec<_> = quiet.lines().collect();
        assert_eq!(quiet[0].chars().count(), quiet[1].chars().count());
        assert_eq!(quiet[0].chars().count(), lines[0].chars().count());
        assert!(quiet[0].ends_with("— bps ↑"));
        assert!(quiet[1].ends_with("0 bps ↓"));
        let wide = menu_bar_label(Some(2_000_000_000.0), Some(1.0), SpeedUnit::Bits);
        let wide: Vec<_> = wide.lines().collect();
        assert_eq!(wide[0].chars().count(), wide[1].chars().count());
        assert!(wide[0].chars().count() > MENU_BAR_COLUMNS);
    }

    #[test]
    fn digit_changes_do_not_resize_the_menu_bar_item() {
        let idle = menu_bar_label(None, Some(0.0), SpeedUnit::AutoBits);
        let live = menu_bar_label(Some(14_000.0), Some(450_000.0), SpeedUnit::AutoBits);
        assert_eq!(
            menu_bar_update(MenuBarRecord::Shown(&idle), Some(&live)),
            MenuBarUpdate::Title
        );
        assert_eq!(
            menu_bar_update(MenuBarRecord::Shown(&live), Some(&live)),
            MenuBarUpdate::Unchanged
        );
        assert_eq!(
            menu_bar_update(MenuBarRecord::Unknown, Some(&live)),
            MenuBarUpdate::Layout
        );
        assert_eq!(
            menu_bar_update(MenuBarRecord::Shown(&live), None),
            MenuBarUpdate::Layout
        );
        assert_eq!(
            menu_bar_update(MenuBarRecord::Hidden, None),
            MenuBarUpdate::Unchanged
        );
        assert_eq!(
            menu_bar_update(MenuBarRecord::Unknown, None),
            MenuBarUpdate::Unchanged
        );
        let wide = menu_bar_label(Some(2_000_000_000.0), Some(1.0), SpeedUnit::Bits);
        assert_eq!(
            menu_bar_update(MenuBarRecord::Shown(&live), Some(&wide)),
            MenuBarUpdate::Layout
        );
    }

    #[test]
    fn upload_is_above_download_with_distinct_values() {
        assert_eq!(
            rows(Some(14_000.0), Some(13_000.0)),
            [("↑", Some(14_000.0)), ("↓", Some(13_000.0))]
        );
    }
}
