use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::{
    config::{
        defaults,
        schema::{
            Language, Layer, MonitorKey, PatchValue, Placement, RectF, VerticalAnchor,
            WidgetInstance, WidgetKind, WidgetPatch, WidgetSize,
        },
    },
    desktop::{
        monitors::{self, MonitorInfo},
        snap::{self, SnapCtx},
    },
    error::AppError,
    state::{AppState, EditSelected, WidgetBoot},
    windows::{menu, widget},
};

#[tauri::command]
pub async fn widget_add(
    app: AppHandle,
    kind: WidgetKind,
    size: WidgetSize,
    monitor: Option<String>,
) -> Result<WidgetInstance, AppError> {
    add_widget_internal(&app, kind, size, monitor)
}

pub fn add_widget_internal(
    app: &AppHandle,
    kind: WidgetKind,
    size: WidgetSize,
    monitor: Option<String>,
) -> Result<WidgetInstance, AppError> {
    if !kind.allows_size(size) {
        return Err(AppError::SizeNotAllowed);
    }

    let state = app.state::<AppState>();
    let monitor_infos: Vec<MonitorInfo> = state
        .monitors
        .read()
        .iter()
        .map(|entry| entry.info.clone())
        .collect();

    let instance = state.config.mutate(|cfg| {
        let placement = monitors::first_free_slot(
            &monitor_infos,
            monitor.as_deref(),
            &cfg.widgets,
            size.base_size(),
        );
        let inst = WidgetInstance {
            id: Uuid::new_v4(),
            kind,
            size,
            scale: 1.0,
            placement,
            layer: Layer::Desktop,
            click_through: false,
            theme_override: None,
            glass_override: None,
            glass_override_engine: None,
            settings: defaults::default_settings(kind),
            cluster: None,
            created_at: defaults::unix_now(),
        };
        cfg.widgets.push(inst.clone());
        Ok(inst)
    })?;

    widget::spawn_one(app, &instance)?;
    Ok(instance)
}

#[tauri::command]
pub fn widget_update(
    app: AppHandle,
    id: Uuid,
    patch: WidgetPatch,
) -> Result<WidgetInstance, AppError> {
    let result = apply_widget_update(&app, id, patch);
    if let Err(error) = &result {
        log::error!(target: "cmd", "widget_update fejlede for {id}: {error:?}");
    }
    result
}

pub fn apply_widget_update(
    app: &AppHandle,
    id: Uuid,
    patch: WidgetPatch,
) -> Result<WidgetInstance, AppError> {
    let reset_cluster_size = matches!(patch.cluster, PatchValue::Set(_));
    let updated = app.state::<AppState>().config.mutate(|cfg| {
        let Some(target) = cfg.widgets.iter_mut().find(|w| w.id == id) else {
            return Err(AppError::WidgetNotFound);
        };
        target.apply_patch(patch)?;
        Ok(target.clone())
    })?;

    if reset_cluster_size {
        app.state::<AppState>().cluster_sizes.write().remove(&id);
    }

    widget::apply_instance_live(app, &updated);
    Ok(updated)
}

#[tauri::command]
pub fn widget_remove(app: AppHandle, id: Uuid) -> Result<(), AppError> {
    remove_widget_by_id(&app, id).map(|_| ())
}

pub fn remove_widget_by_id(app: &AppHandle, id: Uuid) -> Result<WidgetInstance, AppError> {
    let state = app.state::<AppState>();
    let removed = state.config.mutate(|cfg| {
        let Some(pos) = cfg.widgets.iter().position(|w| w.id == id) else {
            return Err(AppError::WidgetNotFound);
        };
        Ok(cfg.widgets.remove(pos))
    })?;

    widget::close_one(app, removed.id);
    state.cluster_sizes.write().remove(&removed.id);

    let mut note_ids: Vec<Uuid> = Vec::new();
    if removed.kind == WidgetKind::Note {
        note_ids.push(removed.id);
    } else if let Some(cluster) = &removed.cluster {
        for module in &cluster.modules {
            if module.kind == WidgetKind::Note {
                note_ids.push(module.id);
            }
        }
    }
    if !note_ids.is_empty() {
        let app_handle = app.clone();
        let removed_id = removed.id;
        let notes_dir = state.config.notes_dir();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let still_removed = !app_handle
                .state::<AppState>()
                .config
                .get()
                .widgets
                .iter()
                .any(|w| w.id == removed_id);
            if still_removed {
                for note_id in note_ids {
                    let note_file = notes_dir.join(format!("{note_id}.txt"));
                    if note_file.exists() {
                        let _ = std::fs::remove_file(note_file);
                    }
                }
            }
        });
    }
    state.push_removed(removed.clone());
    {
        let mut sel = state.selected_widget.write();
        if *sel == Some(removed.id) {
            *sel = None;
            let _ = app.emit("edit://selected", EditSelected { id: None });
        }
    }

    let _ = app.emit_to("control", "widget://removed", &removed);
    let control_visible = app
        .get_webview_window("control")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if !control_visible {
        use tauri_plugin_notification::NotificationExt;
        let body = match state.config.get().general.language {
            Language::En => "Widget removed",
            Language::Da => "Widget fjernet",
        };
        if let Err(err) = app.notification().builder().title("Open Dock").body(body).show() {
            log::warn!(target: "cmd", "Kunne ikke vise widget-fjernet notifikation: {err}");
        }
    }

    Ok(removed)
}

#[tauri::command]
pub async fn widget_duplicate(app: AppHandle, id: Uuid) -> Result<WidgetInstance, AppError> {
    let state = app.state::<AppState>();
    let monitor_infos: Vec<MonitorInfo> = state
        .monitors
        .read()
        .iter()
        .map(|entry| entry.info.clone())
        .collect();

    let copy = state.config.mutate(|cfg| {
        let Some(source) = cfg.widgets.iter().find(|w| w.id == id).cloned() else {
            return Err(AppError::WidgetNotFound);
        };

        let mut dup = WidgetInstance {
            id: Uuid::new_v4(),
            created_at: defaults::unix_now(),
            ..source
        };
        if let Some(ref mut cluster) = dup.cluster {
            for module in &mut cluster.modules {
                module.id = Uuid::new_v4();
            }
        }
        dup.placement.x += 24.0;
        dup.placement.y += 24.0;

        if let Some(mon) = monitors::find_target_monitor(&dup.placement.monitor, &monitor_infos) {
            let scale = mon.scale.max(0.5);
            let work_w = mon.work_area.width as f64 / scale;
            let work_h = mon.work_area.height as f64 / scale;
            let (bw, bh) = widget::effective_base_size(&app, &dup);
            let w = bw * dup.scale as f64;
            let h = bh * dup.scale as f64;
            dup.placement.x = dup.placement.x.clamp(0.0, (work_w - w).max(0.0));
            dup.placement.y = dup.placement.y.clamp(0.0, (work_h - h).max(0.0));
        }

        cfg.widgets.push(dup.clone());
        Ok(dup)
    })?;

    widget::spawn_one(&app, &copy)?;
    Ok(copy)
}

#[tauri::command]
pub async fn widget_restore(
    app: AppHandle,
    instance: Option<WidgetInstance>,
) -> Result<WidgetInstance, AppError> {
    let state = app.state::<AppState>();
    let candidate = match instance {
        Some(inst) => {
            let _ = state.pop_removed(Some(inst.id));
            inst
        }
        None => state.pop_removed(None).ok_or(AppError::WidgetNotFound)?,
    };

    let restored = state.config.mutate(|cfg| {
        if let Some(existing) = cfg.widgets.iter().find(|w| w.id == candidate.id) {
            return Ok(existing.clone());
        }
        cfg.widgets.push(candidate.clone());
        Ok(candidate)
    })?;

    widget::spawn_one(&app, &restored)?;
    Ok(restored)
}

#[tauri::command]
pub fn widget_move(
    app: AppHandle,
    id: Uuid,
    monitor: String,
    x: f64,
    y: f64,
) -> Result<WidgetInstance, AppError> {
    let state = app.state::<AppState>();
    let monitor_infos: Vec<MonitorInfo> = state
        .monitors
        .read()
        .iter()
        .map(|entry| entry.info.clone())
        .collect();
    let cluster_sizes = state.cluster_sizes.read().clone();

    let updated = state.config.mutate(|cfg| {
        let target_idx = cfg
            .widgets
            .iter()
            .position(|w| w.id == id)
            .ok_or(AppError::WidgetNotFound)?;

        let mon = monitor_infos
            .iter()
            .find(|m| m.name == monitor)
            .or_else(|| {
                monitors::find_target_monitor(&cfg.widgets[target_idx].placement.monitor, &monitor_infos)
            })
            .cloned();

        if let Some(m) = mon {
            let scale = m.scale.max(0.5);
            let work_w = m.work_area.width as f64 / scale;
            let work_h = m.work_area.height as f64 / scale;
            let target_base = base_size_for(&cfg.widgets[target_idx], &cluster_sizes);
            let target_scale = cfg.widgets[target_idx].scale as f64;
            let w = target_base.0 * target_scale;
            let h = target_base.1 * target_scale;

            let others: Vec<RectF> = cfg
                .widgets
                .iter()
                .filter(|other| other.id != id)
                .filter_map(|other| {
                    let obase = base_size_for(other, &cluster_sizes);
                    let geom = monitors::resolve_placement_with_size(other, &monitor_infos, obase)?;
                    if geom.monitor_name != m.name {
                        return None;
                    }
                    Some(RectF {
                        x: (geom.phys_x - m.work_area.x) as f64 / scale,
                        y: (geom.phys_y - m.work_area.y) as f64 / scale,
                        width: geom.phys_w as f64 / scale,
                        height: geom.phys_h as f64 / scale,
                    })
                })
                .collect();

            let work = RectF {
                x: 0.0,
                y: 0.0,
                width: work_w,
                height: work_h,
            };
            let ctx = SnapCtx::standard(work, others, cfg.general.grid_snap, true);
            let (snapped, _) = snap::resolve(
                RectF {
                    x,
                    y,
                    width: w,
                    height: h,
                },
                &ctx,
            );

            let target = &mut cfg.widgets[target_idx];
            target.placement = Placement {
                monitor: MonitorKey {
                    name: m.name,
                    rect: m.rect,
                },
                x: snapped.x,
                y: snapped.y,
            };
            Ok(target.clone())
        } else {
            let target = &mut cfg.widgets[target_idx];
            target.placement.monitor.name = monitor;
            target.placement.x = x;
            target.placement.y = y;
            Ok(target.clone())
        }
    })?;

    widget::apply_instance_live(&app, &updated);
    Ok(updated)
}

fn base_size_for(
    instance: &WidgetInstance,
    cluster_sizes: &std::collections::HashMap<Uuid, (f64, f64)>,
) -> (f64, f64) {
    if instance.kind == WidgetKind::Cluster {
        if let Some((w, h)) = cluster_sizes.get(&instance.id).copied() {
            let width = instance
                .cluster
                .as_ref()
                .map(|c| (c.width.max(132)) as f64)
                .unwrap_or(w);
            return (width, h);
        }
    }
    instance.base_size()
}

#[tauri::command]
pub fn widget_ready(app: AppHandle, id: String) -> Result<WidgetBoot, AppError> {
    let result = crate::windows::widget::ready(&app, &id);
    if let Err(error) = &result {
        log::error!(target: "cmd", "widget_ready fejlede for {id}: {error:?}");
    }
    result
}

#[tauri::command]
pub fn widget_set_focusable(app: AppHandle, id: String, focusable: bool) -> Result<(), AppError> {
    let result = crate::windows::widget::set_focusable(&app, &id, focusable);
    if let Err(error) = &result {
        log::error!(target: "cmd", "widget_set_focusable fejlede for {id}: {error:?}");
    }
    result
}

#[tauri::command]
pub fn widget_context_menu(app: AppHandle, id: String) -> Result<(), AppError> {
    let result = menu::show_widget_context_menu(&app, &id);
    if let Err(error) = &result {
        log::error!(target: "cmd", "widget_context_menu fejlede for {id}: {error:?}");
    }
    result
}

#[tauri::command]
pub async fn cluster_add_preset(
    app: AppHandle,
    preset_id: String,
    monitor: Option<String>,
) -> Result<Vec<WidgetInstance>, AppError> {
    add_cluster_preset_internal(&app, &preset_id, monitor)
}

pub fn add_cluster_preset_internal(
    app: &AppHandle,
    preset_id: &str,
    monitor: Option<String>,
) -> Result<Vec<WidgetInstance>, AppError> {
    let presets = defaults::cluster_preset_configs(preset_id)
        .ok_or_else(|| AppError::Other(format!("Ukendt klynge-skabelon: {preset_id}")))?;

    let state = app.state::<AppState>();
    let monitor_infos: Vec<MonitorInfo> = state
        .monitors
        .read()
        .iter()
        .map(|entry| entry.info.clone())
        .collect();

    let created = state.config.mutate(|cfg| {
        let mut added = Vec::with_capacity(presets.len());
        let mut prev_bottom_y: Option<f64> = None;
        let mut group_x: Option<f64> = None;

        for mut cluster in presets {
            let base_size = cluster.initial_base_size();
            let placement = if let (Some(x), Some(next_y)) = (group_x, prev_bottom_y) {
                let mon = monitor
                    .as_deref()
                    .and_then(|name| monitor_infos.iter().find(|m| m.name == name))
                    .or_else(|| monitor_infos.iter().find(|m| m.is_primary))
                    .or_else(|| monitor_infos.first());
                if let Some(m) = mon {
                    let scale = m.scale.max(0.5);
                    let work_h = m.work_area.height as f64 / scale;
                    let candidate_y = next_y + 12.0;
                    if candidate_y + base_size.1 <= work_h - 12.0 {
                        Placement {
                            monitor: MonitorKey {
                                name: m.name.clone(),
                                rect: m.rect,
                            },
                            x,
                            y: candidate_y,
                        }
                    } else {
                        monitors::first_free_slot(
                            &monitor_infos,
                            monitor.as_deref(),
                            &cfg.widgets,
                            base_size,
                        )
                    }
                } else {
                    monitors::first_free_slot(
                        &monitor_infos,
                        monitor.as_deref(),
                        &cfg.widgets,
                        base_size,
                    )
                }
            } else {
                monitors::first_free_slot(
                    &monitor_infos,
                    monitor.as_deref(),
                    &cfg.widgets,
                    base_size,
                )
            };

            if let Some(mon) = monitors::find_target_monitor(&placement.monitor, &monitor_infos) {
                let scale = mon.scale.max(0.5);
                let work_h = mon.work_area.height as f64 / scale;
                if placement.y + base_size.1 / 2.0 > work_h / 2.0 {
                    cluster.anchor = VerticalAnchor::Bottom;
                }
            }

            group_x.get_or_insert(placement.x);
            prev_bottom_y = Some(placement.y + base_size.1);

            let inst = WidgetInstance {
                id: Uuid::new_v4(),
                kind: WidgetKind::Cluster,
                size: WidgetSize::M,
                scale: 1.0,
                placement,
                layer: Layer::Desktop,
                click_through: false,
                theme_override: None,
                glass_override: None,
                glass_override_engine: None,
                settings: serde_json::json!({}),
                cluster: Some(cluster),
                created_at: defaults::unix_now(),
            };
            cfg.widgets.push(inst.clone());
            added.push(inst);
        }
        Ok(added)
    })?;

    if created.len() > 1 {
        let mut delays = state.start_delays.write();
        for (idx, inst) in created.iter().enumerate() {
            delays.insert(inst.id, (idx as u32) * 120);
        }
    }

    for inst in &created {
        if let Err(error) = widget::spawn_one(app, inst) {
            log::error!(target: "cmd", "Kunne ikke oprette klyngevindue {}: {error:?}", inst.id);
        }
    }

    Ok(created)
}

#[tauri::command]
pub fn cluster_set_window_size(
    app: AppHandle,
    id: Uuid,
    width: f64,
    height: f64,
    anchor: VerticalAnchor,
) -> Result<(), AppError> {
    let result = widget::cluster_set_window_size(&app, id, width, height, anchor);
    if let Err(error) = &result {
        log::error!(target: "cmd", "cluster_set_window_size fejlede for {id}: {error:?}");
    }
    result
}

#[tauri::command]
pub fn monitors_list(app: AppHandle) -> Result<Vec<MonitorInfo>, AppError> {
    let monitors = app
        .state::<AppState>()
        .monitors
        .read()
        .iter()
        .map(|entry| entry.info.clone())
        .collect();
    Ok(monitors)
}
