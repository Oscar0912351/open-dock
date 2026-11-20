use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager, WebviewWindow, Window};
use tauri_plugin_notification::NotificationExt;

use crate::{
    config::schema::{ControlTheme, Language},
    desktop::glass,
    error::AppError,
    state::AppState,
};

pub fn apply_theme(window: &WebviewWindow, theme: ControlTheme) {
    let dark = theme == ControlTheme::Dark;
    let fallback = if dark {
        tauri::window::Color(0, 0, 0, 255)
    } else {
        tauri::window::Color(246, 246, 248, 255)
    };
    if glass::is_windows_11() {
        if let Err(error) = window_vibrancy::apply_mica(window, Some(dark)) {
            log::warn!("Kunne ikke aktivere Mica på kontrolcentret: {error}");
            let _ = window.set_background_color(Some(fallback));
        }
    } else if let Err(error) = window.set_background_color(Some(fallback)) {
        log::warn!("Kunne ikke sætte baggrundsfarve på kontrolcentret: {error}");
    }
}

pub fn configure(window: &WebviewWindow, theme: ControlTheme) {
    apply_theme(window, theme);
}

pub fn open(app: &AppHandle, route: Option<String>) -> Result<(), AppError> {
    let control = app
        .get_webview_window("control")
        .ok_or_else(|| AppError::Other("Kontrolvinduet findes ikke".into()))?;
    let _ = control.unminimize();
    control.show()?;
    control.set_focus()?;
    if let Some(target_route) = route {
        let trimmed = target_route.trim();
        if !trimmed.is_empty() {
            let _ = app.emit_to("control", "app://navigate", trimmed.to_string());
        }
    }
    Ok(())
}

pub fn hide_to_background(app: &AppHandle, window: &Window) {
    if let Err(error) = window.hide() {
        log::error!("Kunne ikke skjule kontrolcentret: {error}");
    }
    let state = app.state::<AppState>();
    if !state.tray_notice_shown.swap(true, Ordering::AcqRel) {
        let msg = match state.config.get().general.language {
            Language::Da => {
                "Open Dock kører videre i baggrunden. Find den i proceslinjens hjørne."
            }
            Language::En => {
                "Open Dock keeps running in the background. Find it in the system tray."
            }
        };
        let _ = app.emit_to("control", "app://tray-notice", msg);
        if let Err(error) = app.notification().builder().title("Open Dock").body(msg).show() {
            log::debug!("Kunne ikke vise baggrunds-notifikation: {error}");
        }
    }
}
