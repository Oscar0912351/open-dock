use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    error::AppError,
    providers::timer::{TimerManager, TimerSnapshot},
};

fn manager(app: &AppHandle) -> tauri::State<'_, TimerManager> {
    app.state::<TimerManager>()
}

#[tauri::command]
pub fn timer_get(app: AppHandle, id: Uuid) -> TimerSnapshot {
    manager(&app).snapshot(id)
}

#[tauri::command]
pub fn timer_start(app: AppHandle, id: Uuid, duration_ms: Option<i64>) -> Result<(), AppError> {
    manager(&app).start(id, duration_ms);
    Ok(())
}

#[tauri::command]
pub fn timer_pause(app: AppHandle, id: Uuid) -> Result<(), AppError> {
    manager(&app).pause(id);
    Ok(())
}

#[tauri::command]
pub fn timer_resume(app: AppHandle, id: Uuid) -> Result<(), AppError> {
    manager(&app).resume(id);
    Ok(())
}

#[tauri::command]
pub fn timer_reset(app: AppHandle, id: Uuid) -> Result<(), AppError> {
    manager(&app).reset(id);
    Ok(())
}

#[tauri::command]
pub fn timer_dismiss(app: AppHandle, id: Uuid) -> Result<(), AppError> {
    manager(&app).dismiss(id);
    Ok(())
}

#[tauri::command]
pub fn timer_snooze(app: AppHandle, id: Uuid, minutes: i64) -> Result<(), AppError> {
    manager(&app).snooze(id, minutes);
    Ok(())
}

#[tauri::command]
pub fn timer_skip(app: AppHandle, id: Uuid) -> Result<(), AppError> {
    manager(&app).skip(id);
    Ok(())
}

#[tauri::command]
pub fn timer_preview_sound(app: AppHandle, sound: String, volume: u8) -> Result<(), AppError> {
    crate::providers::sound::play(&app, &sound, volume);
    Ok(())
}
