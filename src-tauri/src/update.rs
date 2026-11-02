//! At launch, asks GitHub for the latest release and offers to install it.
//!
//! The endpoint (`tauri.conf.json` > `plugins.updater`) is the release's `latest.json` asset
//! behind `github.com/…/releases/latest/download/`: a plain download redirect, not the REST API,
//! so no token and no rate limit. Drafts are skipped until published.

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

pub fn check_in_background(app: AppHandle) {
    // A dev build would offer to replace itself with the installed release.
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        // Offline or GitHub down: the app works as before, nothing to tell the user.
        if let Err(e) = offer(&app).await {
            eprintln!("update check: {e}");
        }
    });
}

async fn offer(app: &AppHandle) -> tauri_plugin_updater::Result<()> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(());
    };
    let install = app
        .dialog()
        .message(format!(
            "Soundboard {} est disponible (installée : {}).\n\nL'installer maintenant ? L'application redémarrera.",
            update.version, update.current_version
        ))
        .title("Mise à jour disponible")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Installer".into(),
            "Plus tard".into(),
        ))
        .blocking_show();
    if install {
        // Signature checked against `pubkey` before anything runs. Windows hands over to the
        // installer and exits here; macOS swaps the .app and needs the restart.
        update.download_and_install(|_, _| {}, || {}).await?;
        app.restart();
    }
    Ok(())
}
