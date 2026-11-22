use tauri::{AppHandle, Manager};

use crate::{
    error::AppError,
    update::{UpdateStatus, UpdateTracker},
};

/// Henter manifestet fra GitHub og sammenligner remote build-id med det indlejrede.
/// Gemmer resultatet i UpdateTracker og sender det som update://status.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateStatus, AppError> {
    let result = crate::update::check().await;
    match &result {
        Ok(status) => {
            if let Some(tracker) = app.try_state::<UpdateTracker>() {
                *tracker.last.lock() = Some(status.clone());
            }
            use tauri::Emitter;
            if let Err(err) = app.emit("update://status", status) {
                log::warn!(target: "cmd", "Kunne ikke sende update://status: {err}");
            }
        }
        Err(error) => {
            log::error!(target: "cmd", "update_check fejlede: {error:?}");
        }
    }
    result
}

/// Sidst kendte opdateringsstatus uden netværkskald (null indtil første tjek).
#[tauri::command]
pub fn update_status(app: AppHandle) -> Option<UpdateStatus> {
    app.try_state::<UpdateTracker>()
        .and_then(|tracker| tracker.last.lock().clone())
}

/// Downloader nyeste installer, verificerer SHA-256, starter den og lukker appen.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), AppError> {
    let result = crate::update::install(&app).await;
    if let Err(error) = &result {
        log::error!(target: "cmd", "update_install fejlede: {error:?}");
    }
    result
}
