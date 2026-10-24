//! Two-line menu bar title. A plain status-item title stays one proportional
//! line, so the item changes width every time the rate changes.
use std::cell::Cell;
use std::sync::Mutex;

use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{
    NSApplication, NSButton, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSImage, NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSView, NSWindow,
};
use objc2_foundation::{NSAttributedString, NSDictionary, NSRect, NSString};

use crate::{MenuBarRecord, MenuBarUpdate};

#[derive(Clone)]
enum SavedLabel {
    Hidden,
    Shown(String),
}

static SAVED_LABEL: Mutex<Option<SavedLabel>> = Mutex::new(None);
thread_local! {
    static APPLYING: Cell<bool> = const { Cell::new(false) };
}

/// True when `AppKit` calls are legal on this thread.
#[must_use]
pub fn menu_bar_is_main_thread() -> bool {
    MainThreadMarker::new().is_some()
}

/// Draws `label` on `BiFlow`'s status item. `None` clears the title.
///
/// Returns whether that status item was found, or whether nothing needed to change.
#[must_use]
pub fn apply_menu_bar_label(label: Option<&str>) -> bool {
    let Some(marker) = MainThreadMarker::new() else {
        return false;
    };
    if APPLYING.with(Cell::get) {
        tracing::warn!(
            event = "taskbar.menu_bar_reentered",
            section = "taskbar",
            initiator = "interface_sampler",
            cause = "appkit_reentry",
            trace_route = "shared_stats->tray_label->menu_bar",
            "menu bar update skipped because AppKit reentered it"
        );
        return true;
    }
    let previous = saved_record();
    let record = match previous.as_ref() {
        None => MenuBarRecord::Unknown,
        Some(SavedLabel::Hidden) => MenuBarRecord::Hidden,
        Some(SavedLabel::Shown(text)) => MenuBarRecord::Shown(text),
    };
    let update = crate::menu_bar_update(record, label);
    if update == MenuBarUpdate::Unchanged {
        return true;
    }
    let Some(button) = status_button(marker) else {
        return false;
    };
    let _guard = ApplyGuard::enter();
    // A title-only change must not call `sizeToFit`. If the logo is visible
    // again, the status item was recreated and needs one layout.
    if update == MenuBarUpdate::Title && button.image().is_none() {
        let Some(label) = label else {
            return false;
        };
        set_stacked_title(&button, label);
        remember(label);
        return true;
    }
    button.setToolTip(Some(&NSString::from_str("BiFlow")));
    if let Some(label) = label {
        set_stacked_title(&button, label);
        show_logo(&button, false);
    } else {
        button.setTitle(&NSString::from_str(""));
        show_logo(&button, true);
    }
    fit_click_target(&button);
    remember_layout(label);
    true
}

fn saved_record() -> Option<SavedLabel> {
    SAVED_LABEL.lock().expect("menu bar label").clone()
}

fn remember(label: &str) {
    *SAVED_LABEL.lock().expect("menu bar label") = Some(SavedLabel::Shown(label.to_owned()));
}

fn remember_layout(label: Option<&str>) {
    *SAVED_LABEL.lock().expect("menu bar label") = Some(match label {
        Some(label) => SavedLabel::Shown(label.to_owned()),
        None => SavedLabel::Hidden,
    });
}

struct ApplyGuard;

impl ApplyGuard {
    fn enter() -> Self {
        APPLYING.with(|applying| applying.set(true));
        Self
    }
}

impl Drop for ApplyGuard {
    fn drop(&mut self) {
        APPLYING.with(|applying| applying.set(false));
    }
}

fn set_stacked_title(button: &NSButton, label: &str) {
    let font = NSFont::monospacedSystemFontOfSize_weight(9.0, 0.0);
    let style = NSMutableParagraphStyle::new();
    style.setMaximumLineHeight(9.0);
    style.setLineSpacing(-2.0);
    let text = NSString::from_str(label);
    let attrs = attributes(&font, &style, &NSColor::labelColor());
    // SAFETY: the dictionary uses the font, paragraph, and foreground
    // attribute names, and each value is the matching AppKit object.
    let title = unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &text,
            Some(&attrs),
        )
    };
    button.setAttributedTitle(&title);
}

fn attributes(
    font: &NSFont,
    style: &NSMutableParagraphStyle,
    color: &NSColor,
) -> objc2::rc::Retained<NSDictionary<NSString, AnyObject>> {
    // SAFETY: Apple's attribute-name constants are non-null for the process lifetime.
    let keys: [&NSString; 3] = unsafe {
        [
            NSFontAttributeName,
            NSParagraphStyleAttributeName,
            NSForegroundColorAttributeName,
        ]
    };
    let objects = [object_ref(font), object_ref(style), object_ref(color)];
    NSDictionary::from_slices(&keys, &objects)
}

fn object_ref(object: &impl Message) -> &AnyObject {
    // SAFETY: every `Message` is an Objective-C object, which is the layout
    // `AnyObject` describes. The reference stays borrowed from `object`.
    unsafe { &*std::ptr::from_ref(object).cast::<AnyObject>() }
}

fn status_button(marker: MainThreadMarker) -> Option<objc2::rc::Retained<NSButton>> {
    let mut buttons = Vec::new();
    for window in windows(marker) {
        if let Some(root) = window.contentView() {
            collect_buttons(&root, &mut buttons);
        }
    }
    buttons
        .iter()
        .find(|button| tooltip(button).as_deref() == Some("BiFlow"))
        .cloned()
        .or_else(|| (buttons.len() == 1).then(|| buttons.remove(0)))
}

fn windows(marker: MainThreadMarker) -> Vec<objc2::rc::Retained<NSWindow>> {
    let list = NSApplication::sharedApplication(marker).windows();
    (0..list.count())
        .map(|index| list.objectAtIndex(index))
        .collect()
}

fn collect_buttons(view: &NSView, buttons: &mut Vec<objc2::rc::Retained<NSButton>>) {
    let children = view.subviews();
    for index in 0..children.count() {
        let child = children.objectAtIndex(index);
        if child.class().name() == c"TaoTrayTarget" {
            // SAFETY: the overlay is retained for this call and is only read
            // on the main thread. Its superview is the status button.
            let Some(parent) = (unsafe { child.superview() }) else {
                continue;
            };
            if let Ok(button) = parent.downcast::<NSButton>() {
                buttons.push(button);
            }
        } else {
            collect_buttons(&child, buttons);
        }
    }
}

fn tooltip(button: &NSButton) -> Option<String> {
    button.toolTip().map(|tip| tip.to_string())
}

fn show_logo(button: &NSButton, visible: bool) {
    std::thread_local! {
        static MENU_BAR_LOGO: std::cell::RefCell<Option<objc2::rc::Retained<NSImage>>> =
            const { std::cell::RefCell::new(None) };
    }
    if visible {
        let restored = MENU_BAR_LOGO.with(|logo| logo.borrow().clone());
        if let Some(image) = restored {
            button.setImage(Some(&image));
        }
        return;
    }
    if let Some(image) = button.image() {
        MENU_BAR_LOGO.with(|logo| *logo.borrow_mut() = Some(image));
    }
    button.setImage(None);
}

fn fit_click_target(button: &NSButton) {
    // A leftover overlay frame can keep `sizeToFit` from shrinking the item.
    place_overlay(button, NSRect::ZERO);
    button.sizeToFit();
    place_overlay(button, button.bounds());
}

fn place_overlay(button: &NSButton, frame: NSRect) {
    let children = button.subviews();
    for index in 0..children.count() {
        let child = children.objectAtIndex(index);
        if child.class().name() == c"TaoTrayTarget" {
            child.setFrame(frame);
        }
    }
}
