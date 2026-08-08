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
mod update;

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build());

    // One instance, because the settings are one file. Every instance loads the
    // whole settings object at startup and writes the whole of it back on any
    // change, so two running at once silently overwrite each other and
    // whichever quits last wins — a portable copy and an installed copy were
    // open together, and the repository the user had chosen to review was
    // replaced by the other's stale idea of it, with nothing said.
    //
    // The acceptance suite runs against this, guard and all, rather than
    // against a specially-built binary: it kills any stray instance first. A
    // test build that differs from the shipped one is a test of something
    // nobody ships.
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        reveal(app);
    }));

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
            commands::check_signin,
            commands::load_settings,
            commands::save_settings,
            commands::run::start_run,
            commands::run::cancel_run,
            commands::apply::apply_fixes,
            commands::saved::clear_saved,
            commands::pick_directory,
            commands::frontend_ready,
            commands::quit,
            catalogue::available_models,
            update::check_for_update,
            update::install_update,
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
    tray::reveal(app);
}
