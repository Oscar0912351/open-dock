use tauri::{AppHandle, Manager};

use crate::{
    error::AppError,
    providers::{media::MediaAction, scheduler::Scheduler},
};

#[tauri::command]
pub fn media_control(
    app: AppHandle,
    action: MediaAction,
    source: Option<String>,
) -> Result<(), AppError> {
    app.state::<Scheduler>()
        .media()
        .control(&app, action, source);
    Ok(())
}

#[tauri::command]
pub fn media_select_source(app: AppHandle, source: Option<String>) -> Result<(), AppError> {
    app.state::<Scheduler>().media().set_preferred(source);
    Ok(())
}
