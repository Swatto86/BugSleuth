//! The desktop shell.
//!
//! Deliberately thin. Every command here is a deserialize, a call into
//! `bugsleuth-engine`, and a serialize; anything longer belongs in the engine,
//! where it can be tested without launching a window.
//!
//! Nothing the frontend sends is trusted. Paths arrive as strings from a
//! webview, so they are canonicalized and checked before anything reads them,
//! and no `fs` or `shell` permission is granted to the frontend at all — the
//! only way to touch the disk is through a command that validates first.

mod catalogue;
mod commands;
mod outcome;
mod payload;
mod settings;
mod tray;

use tauri::Manager;

pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());

    // Test-only, compiled in solely for the acceptance suite. Without it the
    // app exposes no automation endpoint at all — WebDriver attaches to a blank
    // webview and every spec fails with an empty document. With it in a
    // published build the app would offer that surface to anything on the
    // machine, so it is a non-default feature and the release workflow asserts
    // the shipped binary does not carry it.
    #[cfg(feature = "wdio")]
    let builder = builder.plugin(tauri_plugin_wdio::init());

    builder
        .setup(|app| {
            tray::install(app.handle())?;

            // Safety net. The frontend reveals the window once it has painted,
            // which avoids an unstyled flash — but if it ever fails to boot,
            // that leaves a running process with no window and no way back.
            // Showing it anyway after a moment is far better than an app the
            // user can only end from the task manager.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                reveal(&handle);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing hides to the tray rather than exiting. `prevent_close`
            // runs before anything fallible, so a failure later cannot leave the
            // window half-closed — the one real exit path is the tray's Quit.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .manage(commands::RunControl::default())
        .invoke_handler(tauri::generate_handler![
            commands::preflight,
            commands::load_settings,
            commands::save_settings,
            commands::plan_run,
            commands::start_run,
            commands::cancel_run,
            commands::pick_directory,
            commands::frontend_ready,
            commands::quit,
            catalogue::available_models,
        ])
        .run(tauri::generate_context!())
        // The only failure here is the webview runtime refusing to start, at
        // which point there is no window to report it in and nothing left to
        // fall back to. Aborting with the reason is strictly better than
        // returning an error nobody can see.
        .unwrap_or_else(|error| panic!("BugSleuth could not start its window: {error}"));
}

/// Reveal the main window. Called once the frontend has mounted and applied its
/// theme, so the user never sees an unstyled flash — the window is configured
/// hidden with a matching background colour for the same reason.
pub(crate) fn reveal(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
