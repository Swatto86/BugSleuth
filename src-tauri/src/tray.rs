//! The system tray icon and its menu.
//!
//! BugSleuth is resident because a sweep takes tens of minutes: you start one,
//! close the window, and want to be told when it lands. That is the only reason
//! for a tray icon, and it is why closing the window hides rather than exits.
//!
//! **Quit here is the one real exit path.** The window's close handler always
//! hides, so without this menu item there would be no way out but the task
//! manager.

use tauri::Emitter;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

/// Long-running work whose completion the user may be waiting for with the
/// window closed to the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundWork {
    Review,
    Apply,
}

/// What a finished piece of background work should show: always a tray tooltip,
/// and a native notification only when the window is hidden — a visible window
/// already shows the result, so notifying then would be a duplicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionPlan {
    pub tooltip: String,
    /// `(title, body)` when a notification should be sent.
    pub notification: Option<(String, String)>,
}

/// Decide what to show for finished work. Pure, so the tooltip/notification
/// choice can be tested without a running app.
pub(crate) fn completion_plan(
    work: BackgroundWork,
    success: bool,
    window_hidden: bool,
) -> CompletionPlan {
    let (tooltip, body) = match (work, success) {
        (BackgroundWork::Review, true) => ("BugSleuth — review finished", "Your review finished."),
        (BackgroundWork::Review, false) => ("BugSleuth — review failed", "Your review failed."),
        (BackgroundWork::Apply, true) => ("BugSleuth — fixes applied", "The fixes were applied."),
        (BackgroundWork::Apply, false) => {
            ("BugSleuth — apply failed", "Applying the fixes failed.")
        }
    };
    CompletionPlan {
        tooltip: tooltip.to_string(),
        notification: window_hidden.then(|| ("BugSleuth".to_string(), body.to_string())),
    }
}

fn set_tray_tooltip<R: Runtime>(app: &AppHandle<R>, tooltip: &str) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

/// Mark background work as started: the tray tooltip says what is running, so a
/// user who closed the window can see the app is busy.
pub fn work_started<R: Runtime>(app: &AppHandle<R>, work: BackgroundWork) {
    let tooltip = match work {
        BackgroundWork::Review => "BugSleuth — review running",
        BackgroundWork::Apply => "BugSleuth — applying fixes",
    };
    set_tray_tooltip(app, tooltip);
}

/// Mark background work as finished: always update the tray tooltip, and when
/// the window is hidden send a native notification so the completion is visible
/// without reopening the window.
pub fn work_finished<R: Runtime>(app: &AppHandle<R>, work: BackgroundWork, success: bool) {
    let hidden = app
        .get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .map(|visible| !visible)
        .unwrap_or(false);
    let plan = completion_plan(work, success, hidden);
    // The tooltip is set first and unconditionally: if the OS refuses the
    // notification — disabled by the user, or the portable exe is not a
    // registered application — the tray's accessible name still carries the
    // result.
    set_tray_tooltip(app, &plan.tooltip);
    if let Some((title, body)) = plan.notification {
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Open BugSleuth", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &separator, &quit])?;

    TrayIconBuilder::with_id("main")
        // A distinct, simpler mark than the app icon. At 16-24px the app icon's
        // legs and highlight turn to noise; see tray-icon.svg.
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/tray.png"
        ))?)
        .icon_as_template(false)
        .tooltip("BugSleuth")
        .menu(&menu)
        // The menu belongs on right-click only, so a left click can do the
        // obvious thing instead.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => reveal(app),
            "quit" => quit_or_ask(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// Whether the window has been revealed yet this process. The maximize below
/// emulates a startup `maximized` config, so it must run once — re-running it
/// on every reveal discards the size the user has since chosen.
static FIRST_REVEAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Show the window, wherever the request came from.
///
/// One implementation. There were two — this and a near-copy in `lib.rs` that
/// did not unminimize, so revealing from the tray restored a minimised window
/// and revealing from the frontend left it minimised and focused, which looks
/// exactly like the app failing to start.
pub(crate) fn reveal<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        // Maximised on first reveal. The window is configured hidden so the
        // user never sees an unstyled flash, and `maximized` in the config is
        // not honoured for a window that starts invisible — so it is asked for
        // here, where the window is actually being shown. The report is a wide
        // document and the default size made it a column.
        if FIRST_REVEAL.swap(false, std::sync::atomic::Ordering::Relaxed) {
            let _ = window.maximize();
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Left click shows the window, or hides it if it is already in front.
fn toggle<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    if visible && focused {
        let _ = window.hide();
    } else {
        reveal(app);
    }
}

/// Ask the frontend to flush settings, then quit or confirm what would be lost.
///
/// The tray's Quit exited immediately while the window's Quit asked for
/// confirmation, for the identical action. The README calls the tray item the
/// only real exit, so it is the *more* likely of the two to be used, and it was
/// the one that could throw away tens of minutes of paid sweeping in silence.
/// Every tray quit goes through the frontend now: it owns pending settings and
/// adds the busy-work confirmation when needed.
pub(crate) fn quit_or_ask<R: Runtime>(app: &AppHandle<R>) {
    reveal(app);
    // Best effort. If the window cannot be told, the safe outcome is that
    // nothing is thrown away and the in-window Quit still works.
    let _ = app.emit("confirm-quit", ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_completion_requests_a_notification_with_distinct_text() {
        let ok = completion_plan(BackgroundWork::Review, true, true);
        let failed = completion_plan(BackgroundWork::Review, false, true);
        assert!(
            ok.notification.is_some() && failed.notification.is_some(),
            "a hidden window got no completion notification"
        );
        assert_ne!(
            ok.notification, failed.notification,
            "success and failure sent the same notification text"
        );
        assert_ne!(
            ok.tooltip, failed.tooltip,
            "success and failure share a tooltip"
        );
        // Apply is distinguished from review, so the notification says which.
        let apply = completion_plan(BackgroundWork::Apply, true, true);
        assert_ne!(ok.notification, apply.notification);
    }

    #[test]
    fn a_visible_window_updates_the_tooltip_without_a_notification() {
        let plan = completion_plan(BackgroundWork::Apply, true, false);
        assert!(
            plan.notification.is_none(),
            "a visible window fired a duplicate notification"
        );
        assert!(
            !plan.tooltip.is_empty(),
            "a finished job left the tray silent"
        );
    }
}
