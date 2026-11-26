use std::path::PathBuf;

use tauri::AppHandle;

use crate::{
    config::schema::{
        Config, MonitorWallpaperPatch, WallpaperEngineSettingsPatch, WallpaperFit,
    },
    error::AppError,
    wallpaper::{
        self,
        catalog::WallpaperItem,
        engine::WallpaperEngineStatus,
        library,
        quality::{self, WindowsQualityStatus},
    },
};

#[tauri::command]
pub async fn wallpaper_catalog(app: AppHandle) -> Result<Vec<WallpaperItem>, AppError> {
    crate::commands::blocking("wallpaper_catalog", move || library::build_catalog(&app)).await
}

#[tauri::command]
pub async fn wallpaper_apply(
    app: AppHandle,
    wallpaper_id: String,
    monitor: Option<String>,
    fit: Option<WallpaperFit>,
) -> Result<Config, AppError> {
    wallpaper::apply_wallpaper(&app, wallpaper_id, monitor, fit).await
}

#[tauri::command]
pub fn wallpaper_update_assignment(
    app: AppHandle,
    monitor: Option<String>,
    patch: MonitorWallpaperPatch,
) -> Result<Config, AppError> {
    wallpaper::update_assignment(&app, monitor, patch)
}

#[tauri::command]
pub fn wallpaper_engine_patch(
    app: AppHandle,
    patch: WallpaperEngineSettingsPatch,
) -> Result<Config, AppError> {
    wallpaper::update_engine_settings(&app, patch)
}

#[tauri::command]
pub fn wallpaper_stop_live(app: AppHandle, monitor: Option<String>) -> Result<Config, AppError> {
    wallpaper::stop_live(&app, monitor)
}

#[tauri::command]
pub async fn wallpaper_import(app: AppHandle, path: String) -> Result<WallpaperItem, AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::Other("Stien til filen må ikke være tom".into()));
    }
    wallpaper::import_and_save(&app, &PathBuf::from(trimmed)).await
}

#[tauri::command]
pub fn wallpaper_delete_custom(app: AppHandle, id: String) -> Result<Config, AppError> {
    wallpaper::delete_custom(&app, &id)
}

#[tauri::command]
pub fn wallpaper_engine_status(app: AppHandle) -> Result<WallpaperEngineStatus, AppError> {
    Ok(wallpaper::engine_status(&app))
}

#[tauri::command]
pub fn wallpaper_windows_quality_get() -> Result<WindowsQualityStatus, AppError> {
    Ok(quality::read_windows_quality())
}

#[tauri::command]
pub fn wallpaper_windows_quality_set(
    app: AppHandle,
    enabled: bool,
) -> Result<WindowsQualityStatus, AppError> {
    let status = quality::set_windows_lossless_quality(enabled)?;
    wallpaper::reapply_static_assignments(&app);
    Ok(status)
}
