use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::{
    config::schema::{Appearance, AppearancePatch, Config, GeneralPatch},
    error::AppError,
    state::AppState,
    windows::widget,
};

#[tauri::command]
pub fn config_get(app: AppHandle) -> Result<Config, AppError> {
    Ok(app.state::<AppState>().config.get())
}

#[tauri::command]
pub fn config_take_restore_notice(app: AppHandle) -> Result<Option<String>, AppError> {
    Ok(app.state::<AppState>().config.take_restore_notice())
}

#[tauri::command]
pub fn appearance_get(app: AppHandle) -> Result<Appearance, AppError> {
    Ok(app.state::<AppState>().config.get().appearance)
}

#[tauri::command]
pub fn appearance_patch(app: AppHandle, patch: AppearancePatch) -> Result<Config, AppError> {
    let glass_plate_affected = patch.theme_id.is_some() || patch.glass_strength.is_some();
    let config = app.state::<AppState>().config.mutate(|cfg| {
        cfg.appearance.apply_patch(patch);
        Ok(cfg.clone())
    })?;
    widget::reapply_glass_all(&app);
    if glass_plate_affected {
        crate::desktop::wallpaper::trigger_refresh_debounced();
    }
    Ok(config)
}

pub fn sync_autostart(app: &AppHandle, enabled: bool) {
    let manager = app.autolaunch();
    if manager.is_enabled().ok() == Some(enabled) {
        return;
    }
    let res = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(error) = res {
        log::warn!("Kunne ikke opdatere autostart ({enabled}): {error}");
    }
}

#[tauri::command]
pub fn general_patch(app: AppHandle, patch: GeneralPatch) -> Result<Config, AppError> {
    let autostart_change = patch.autostart;
    let control_theme_change = patch.control_theme;
    let state = app.state::<AppState>();

    let config = state.config.mutate(|cfg| {
        cfg.general.apply_patch(patch);
        Ok(cfg.clone())
    })?;

    if let Some(enabled) = autostart_change {
        sync_autostart(&app, enabled);
    }

    if let Some(theme) = control_theme_change {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(control) = handle.get_webview_window("control") {
                crate::windows::control::apply_theme(&control, theme);
            }
        });
    }

    Ok(config)
}

#[tauri::command]
pub fn onboarding_complete(app: AppHandle) -> Result<Config, AppError> {
    let state = app.state::<AppState>();
    let config = state.config.mutate(|cfg| {
        cfg.onboarded = true;
        Ok(cfg.clone())
    })?;
    state.config.flush()?;
    sync_autostart(&app, config.general.autostart);
    Ok(config)
}
