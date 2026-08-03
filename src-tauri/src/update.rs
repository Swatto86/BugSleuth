//! Updating the app from its own GitHub releases.
//!
//! Deliberately driven from Rust rather than from the webview. The updater
//! plugin's frontend permission would let the page download and execute a
//! binary, and this app grants the frontend no filesystem or shell access at
//! all — that is the whole reason every path arrives through a command that
//! validates it first. An update is the one thing that writes an executable to
//! disk, so it is the last thing that should be reachable from a page.
//!
//! What makes this safe is not the transport. The release is signed with a
//! minisign key held only in CI, and the public half is compiled into the app;
//! an update that does not verify against it is refused by the plugin before a
//! byte is executed. Serving `latest.json` over HTTPS from GitHub is not the
//! guarantee — the signature is.

use serde::Serialize;
use tauri_plugin_updater::UpdaterExt;

/// What the window shows when an update exists.
#[derive(Serialize)]
pub struct Available {
    pub version: String,
    pub current: String,
    /// Release notes, as published. May be empty.
    pub notes: String,
}

/// Is there a newer release? `None` means this is the latest.
///
/// Errors are returned rather than swallowed. A check that silently reports
/// "up to date" when it could not reach GitHub is the failure mode that leaves
/// someone on a build from eight releases ago believing they are current —
/// which is exactly what happened here, and why this exists at all.
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<Available>, String> {
    let current = app.package_info().version.to_string();
    let updater = app.updater().map_err(|e| e.to_string())?;

    match updater.check().await {
        Ok(Some(update)) => Ok(Some(Available {
            version: update.version.clone(),
            current,
            notes: update.body.clone().unwrap_or_default(),
        })),
        Ok(None) => Ok(None),
        Err(error) => Err(format!("could not check for updates: {error}")),
    }
}

/// Download and install the newer release, then restart into it.
///
/// Only ever called after `check_for_update` has offered one and the user has
/// agreed. The check runs again here rather than trusting a version string
/// from the window: the frontend is not a trusted source for what to install,
/// and the gap between offering and accepting is unbounded.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;

    let update = updater
        .check()
        .await
        .map_err(|e| format!("could not check for updates: {e}"))?
        .ok_or_else(|| "there is no update to install".to_string())?;

    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| format!("the update could not be installed: {e}"))?;

    // Nothing after this runs: the process is replaced by the new build.
    app.restart();
}

#[cfg(test)]
mod tests {
    /// The endpoint and key live in `tauri.conf.json`, where a typo is not a
    /// compile error and would only surface as an update that never arrives.
    /// Both are checked here because neither has any other test.
    #[test]
    fn the_updater_is_configured_against_this_repository_and_a_real_key() {
        let raw = include_str!("../tauri.conf.json");
        let config: serde_json::Value = serde_json::from_str(raw).expect("tauri.conf.json parses");
        let updater = &config["plugins"]["updater"];

        let pubkey = updater["pubkey"].as_str().unwrap_or_default();
        assert!(
            pubkey.starts_with("dW50cnVzdGVk"),
            "no minisign public key is configured, so every update would be refused"
        );

        let endpoint = updater["endpoints"][0].as_str().unwrap_or_default();
        assert!(
            endpoint.contains("Swatto86/BugSleuth") && endpoint.ends_with("latest.json"),
            "the updater points at {endpoint}, which is not this repository's latest.json"
        );
        assert!(
            endpoint.starts_with("https://"),
            "the update manifest must not be fetched over plain HTTP"
        );
    }
}
