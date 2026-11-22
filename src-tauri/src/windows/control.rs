use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

use crate::{
    config::schema::{ControlTheme, Language},
    desktop::glass,
    error::AppError,
    state::AppState,
};

fn encode_query_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

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
    let trimmed_route = route
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Some(control) = app.get_webview_window("control") {
        let _ = control.unminimize();
        control.show()?;
        control.set_focus()?;
        if let Some(target_route) = trimmed_route {
            let _ = app.emit_to("control", "app://navigate", target_route);
        }
        return Ok(());
    }

    let path = match trimmed_route {
        Some(target_route) => format!("control.html?route={}", encode_query_component(&target_route)),
        None => "control.html".to_string(),
    };
    let theme = app
        .try_state::<AppState>()
        .map(|state| state.config.get().general.control_theme)
        .unwrap_or(ControlTheme::Dark);

    let control = WebviewWindowBuilder::new(app, "control", WebviewUrl::App(path.into()))
        .title("Open Dock")
        .inner_size(1120.0, 720.0)
        .min_inner_size(960.0, 620.0)
        .center()
        .visible(false)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(true)
        .build()?;

    configure(&control, theme);
    control.show()?;
    control.set_focus()?;
    Ok(())
}

pub fn notify_closed_to_tray(app: &AppHandle) {
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
        if let Err(error) = app.notification().builder().title("Open Dock").body(msg).show() {
            log::debug!("Kunne ikke vise baggrunds-notifikation: {error}");
        }
    }
}
