use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    config::schema::{WidgetInstance, WidgetKind},
    desktop::drag,
    error::AppError,
    state::AppState,
    windows::widget,
};

#[tauri::command]
pub async fn edit_set(app: AppHandle, on: bool) -> Result<(), AppError> {
    let result = drag::set_edit_mode(&app, on);
    if let Err(error) = &result {
        log::error!(target: "cmd", "edit_set({on}) fejlede: {error:?}");
    }
    result
}

#[tauri::command]
pub fn edit_begin_drag(app: AppHandle, id: Uuid) -> Result<(), AppError> {
    let result = drag::begin_drag(&app, id);
    if let Err(error) = &result {
        log::error!(target: "cmd", "edit_begin_drag fejlede for {id}: {error:?}");
    }
    result
}

#[tauri::command]
pub fn edit_nudge(app: AppHandle, id: Uuid, dx: f64, dy: f64) -> Result<(), AppError> {
    let result = drag::nudge(&app, id, dx, dy);
    if let Err(error) = &result {
        log::error!(target: "cmd", "edit_nudge fejlede for {id}: {error:?}");
    }
    result
}

#[tauri::command]
pub fn edit_cycle_size(app: AppHandle, id: Uuid) -> Result<WidgetInstance, AppError> {
    let state = app.state::<AppState>();
    let updated = state.config.mutate(|cfg| {
        let Some(target) = cfg.widgets.iter_mut().find(|w| w.id == id) else {
            return Err(AppError::WidgetNotFound);
        };

        if target.kind == WidgetKind::Cluster {
            if let Some(cluster) = target.cluster.as_mut() {
                cluster.width = match cluster.width {
                    232 => 264,
                    264 => 288,
                    _ => 232,
                };
            }
        } else {
            let allowed = target.kind.allowed_sizes();
            let pos = allowed
                .iter()
                .position(|&s| s == target.size)
                .unwrap_or(0);
            target.size = allowed[(pos + 1) % allowed.len()];
        }

        Ok(target.clone())
    })?;

    if updated.kind == WidgetKind::Cluster {
        if let Some(cluster) = &updated.cluster {
            let mut sizes = state.cluster_sizes.write();
            if let Some(entry) = sizes.get_mut(&id) {
                entry.0 = cluster.width as f64;
            }
        }
    }

    widget::apply_instance_live(&app, &updated);
    Ok(updated)
}
