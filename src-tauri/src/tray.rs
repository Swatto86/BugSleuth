//! The system tray icon and its menu.
//!
//! BugSleuth is resident because a sweep takes tens of minutes: you start one,
//! close the window, and want to be told when it lands. That is the only reason
//! for a tray icon, and it is why closing the window hides rather than exits.
//!
//! **Quit here is the one real exit path.** The window's close handler always
//! hides, so without this menu item there would be no way out but the task
//! manager.

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
            "quit" => app.exit(0),
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

fn reveal<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
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
