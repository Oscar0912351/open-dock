use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    error::AppError,
    providers::timer::{TimerListSnapshot, TimerManager},
};

fn manager(app: &AppHandle) -> tauri::State<'_, TimerManager> {
    app.state::<TimerManager>()
}

/// Henter alle timere + recents for én widget (eller ét klynge-modul).
#[tauri::command]
pub fn timer_get(app: AppHandle, id: Uuid) -> TimerListSnapshot {
    manager(&app).snapshot(id)
}

/// Tilføjer en timer: `label` (evt. tom) og `duration_ms` fra frontendens
/// parser. Fejler på ugyldig varighed eller mere end 8 timere.
#[tauri::command]
pub fn timer_add(
    app: AppHandle,
    id: Uuid,
    label: String,
    duration_ms: i64,
) -> Result<(), AppError> {
    manager(&app).add(id, &label, duration_ms)
}

#[tauri::command]
pub fn timer_pause(app: AppHandle, id: Uuid, item: Uuid) -> Result<(), AppError> {
    manager(&app).pause(id, item);
    Ok(())
}

#[tauri::command]
pub fn timer_resume(app: AppHandle, id: Uuid, item: Uuid) -> Result<(), AppError> {
    manager(&app).resume(id, item);
    Ok(())
}

/// `delta_ms` er relativ justering (fx +60_000 / −60_000), ikke absolut tid.
#[tauri::command]
pub fn timer_adjust(
    app: AppHandle,
    id: Uuid,
    item: Uuid,
    delta_ms: i64,
) -> Result<(), AppError> {
    manager(&app).adjust(id, item, delta_ms);
    Ok(())
}

/// Fjerner én timer-række fra widgetten.
#[tauri::command]
pub fn timer_remove(app: AppHandle, id: Uuid, item: Uuid) -> Result<(), AppError> {
    manager(&app).remove_item(id, item);
    Ok(())
}

/// Afspiller en testlyd i widget-indstillinger-panelet (volume 0-100).
#[tauri::command]
pub fn timer_preview_sound(app: AppHandle, sound: String, volume: u8) -> Result<(), AppError> {
    crate::providers::sound::play(&app, &sound, volume);
    Ok(())
}
