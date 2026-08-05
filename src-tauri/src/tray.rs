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
        let _ = window.maximize();
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

/// Quit, unless work is in flight — in which case ask first.
///
/// The tray's Quit exited immediately while the window's Quit asked for
/// confirmation, for the identical action. The README calls the tray item the
/// only real exit, so it is the *more* likely of the two to be used, and it was
/// the one that could throw away tens of minutes of paid sweeping in silence.
/// Both go through the same question now: the window is revealed and asked to
/// put it, which also keeps the wording in one place.
pub(crate) fn quit_or_ask<R: Runtime>(app: &AppHandle<R>) {
    // Applying counts as well as sweeping. It is the worse of the two to kill —
    // a run loses sweeps that can be paid for again, an apply is killed
    // part-way through editing the repository — and it was not asked about at
    // all, so the tray's Quit could stop a model mid-edit in silence.
    let busy = app
        .try_state::<crate::commands::RunControl>()
        .is_some_and(|control| control.running() || control.applying());
    if !busy {
        app.exit(0);
        return;
    }
    reveal(app);
    // Best effort. If the window cannot be told, the safe outcome is that
    // nothing is thrown away and the in-window Quit still works.
    let _ = app.emit("confirm-quit", ());
}
