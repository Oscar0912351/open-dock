use tauri::{AppHandle, Manager};

use crate::{
    error::AppError,
    providers::{
        gpu::SystemCaps,
        scheduler::Scheduler,
        system::SystemSample,
    },
};

#[tauri::command]
pub fn system_history(app: AppHandle) -> Result<Vec<SystemSample>, AppError> {
    Ok(app.state::<Scheduler>().system().history())
}

#[tauri::command]
pub fn system_caps(app: AppHandle) -> Result<SystemCaps, AppError> {
    Ok(app.state::<Scheduler>().system().caps())
}
