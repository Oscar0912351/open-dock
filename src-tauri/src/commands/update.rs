use tauri::{AppHandle, Manager};

use crate::{
    error::AppError,
    update::{UpdateStatus, UpdateTracker},
};

/// Henter manifestet fra GitHub, sammenligner remote build-id med det indlejrede
/// og downloader automatisk i baggrunden hvis et nyt build findes. Status og
/// download-fremdrift sendes som update://status- og update://progress-events.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateStatus, AppError> {
    let result = crate::update::check_and_stage(&app).await;
    if let Err(error) = &result {
        log::error!(target: "cmd", "update_check fejlede: {error:?}");
    }
    result
}

/// Sidst kendte opdateringsstatus uden netværkskald (null indtil første tjek).
#[tauri::command]
pub fn update_status(app: AppHandle) -> Option<UpdateStatus> {
    app.try_state::<UpdateTracker>()
        .and_then(|tracker| tracker.last.lock().clone())
}

/// Lukker appen og lader en frakoblet kæde køre den verificerede installer
/// silent (/S) og genstarte appen bagefter. Bruger den stagede download hvis
/// den findes, ellers hentes og verificeres den først.
#[tauri::command]
pub async fn update_install(window: tauri::Window, app: AppHandle) -> Result<(), AppError> {
    crate::commands::ensure_control_window(&window)?;
    let result = crate::update::install(&app).await;
    if let Err(error) = &result {
        log::error!(target: "cmd", "update_install fejlede: {error:?}");
    }
    result
}
