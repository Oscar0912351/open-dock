use std::sync::{atomic::Ordering, OnceLock};

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{
    commands,
    config::schema::Hotkeys,
    desktop::drag,
    error::AppError,
    state::AppState,
};

#[derive(Default)]
struct ActiveShortcuts {
    edit_mode: Option<String>,
    toggle_visible: Option<String>,
}

#[derive(Clone, Copy)]
enum ShortcutAction {
    EditMode,
    ToggleVisible,
}

static ACTIVE: OnceLock<Mutex<ActiveShortcuts>> = OnceLock::new();

fn active_lock() -> &'static Mutex<ActiveShortcuts> {
    ACTIVE.get_or_init(|| Mutex::new(ActiveShortcuts::default()))
}

pub fn install(app: &AppHandle) {
    let hotkeys = app.state::<AppState>().config.get().general.hotkeys;
    let mut guard = active_lock().lock();

    if let Err(error) = register_one(app, &hotkeys.edit_mode, ShortcutAction::EditMode) {
        log::warn!(
            target: "shortcuts",
            "Kunne ikke registrere genvej til redigering ({}): {error:?}",
            hotkeys.edit_mode
        );
        guard.edit_mode = None;
    } else {
        guard.edit_mode = Some(hotkeys.edit_mode.clone());
    }

    if let Err(error) = register_one(app, &hotkeys.toggle_visible, ShortcutAction::ToggleVisible) {
        log::warn!(
            target: "shortcuts",
            "Kunne ikke registrere genvej til skjul/vis ({}): {error:?}",
            hotkeys.toggle_visible
        );
        guard.toggle_visible = None;
    } else {
        guard.toggle_visible = Some(hotkeys.toggle_visible.clone());
    }
}

pub fn rebind(app: &AppHandle, next: &Hotkeys) -> Result<(), AppError> {
    let edit_sc = parse_shortcut(&next.edit_mode)?;
    let toggle_sc = parse_shortcut(&next.toggle_visible)?;
    if edit_sc == toggle_sc {
        return Err(AppError::Other(
            "Genvejen bruges allerede i Open Dock".into(),
        ));
    }

    let mut guard = active_lock().lock();
    if guard.edit_mode.as_deref() == Some(next.edit_mode.as_str())
        && guard.toggle_visible.as_deref() == Some(next.toggle_visible.as_str())
    {
        return Ok(());
    }

    let prev_edit = guard.edit_mode.clone();
    let prev_toggle = guard.toggle_visible.clone();

    unregister_best_effort(app, prev_edit.as_deref());
    unregister_best_effort(app, prev_toggle.as_deref());

    if let Err(err) = register_one(app, &next.edit_mode, ShortcutAction::EditMode) {
        let restored_edit = prev_edit
            .as_deref()
            .filter(|raw| register_one(app, raw, ShortcutAction::EditMode).is_ok())
            .map(str::to_string);
        let restored_toggle = prev_toggle
            .as_deref()
            .filter(|raw| register_one(app, raw, ShortcutAction::ToggleVisible).is_ok())
            .map(str::to_string);
        guard.edit_mode = restored_edit;
        guard.toggle_visible = restored_toggle;
        return Err(err);
    }

    if let Err(err) = register_one(app, &next.toggle_visible, ShortcutAction::ToggleVisible) {
        unregister_best_effort(app, Some(&next.edit_mode));
        let restored_edit = prev_edit
            .as_deref()
            .filter(|raw| register_one(app, raw, ShortcutAction::EditMode).is_ok())
            .map(str::to_string);
        let restored_toggle = prev_toggle
            .as_deref()
            .filter(|raw| register_one(app, raw, ShortcutAction::ToggleVisible).is_ok())
            .map(str::to_string);
        guard.edit_mode = restored_edit;
        guard.toggle_visible = restored_toggle;
        return Err(err);
    }

    guard.edit_mode = Some(next.edit_mode.clone());
    guard.toggle_visible = Some(next.toggle_visible.clone());
    Ok(())
}

fn parse_shortcut(raw: &str) -> Result<Shortcut, AppError> {
    raw.trim()
        .parse::<Shortcut>()
        .map_err(|_| AppError::Other("Ugyldig tastaturgenvej".into()))
}

fn unregister_best_effort(app: &AppHandle, raw: Option<&str>) {
    let Some(raw_str) = raw else {
        return;
    };
    if let Ok(sc) = parse_shortcut(raw_str) {
        let _ = app.global_shortcut().unregister(sc);
    }
}

fn register_one(
    app: &AppHandle,
    raw: &str,
    action: ShortcutAction,
) -> Result<(), AppError> {
    let sc = parse_shortcut(raw)?;
    let raw_label = raw.to_string();
    app.global_shortcut()
        .on_shortcut(sc, move |handle, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let app_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                match action {
                    ShortcutAction::EditMode => {
                        let state = app_handle.state::<AppState>();
                        let cur = state.edit_mode.load(Ordering::Acquire);
                        if !cur && !state.widgets_visible.load(Ordering::Acquire) {
                            if let Err(error) = commands::app::set_widgets_visible(&app_handle, true)
                            {
                                log::warn!(
                                    target: "shortcuts",
                                    "Kunne ikke vise widgets før redigering: {error:?}"
                                );
                            }
                        }
                        if let Err(error) = drag::set_edit_mode(&app_handle, !cur) {
                            log::warn!(
                                target: "shortcuts",
                                "Genvej til redigering fejlede: {error:?}"
                            );
                        }
                    }
                    ShortcutAction::ToggleVisible => {
                        let cur = app_handle
                            .state::<AppState>()
                            .widgets_visible
                            .load(Ordering::Acquire);
                        if let Err(error) = commands::app::set_widgets_visible(&app_handle, !cur) {
                            log::warn!(
                                target: "shortcuts",
                                "Genvej til skjul/vis fejlede: {error:?}"
                            );
                        }
                    }
                }
            });
        })
        .map_err(|error| {
            log::warn!(target: "shortcuts", "Kunne ikke registrere '{raw_label}': {error}");
            AppError::ShortcutTaken
        })
}
