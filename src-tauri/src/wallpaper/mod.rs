pub mod catalog;
pub mod engine;
pub mod host;
pub mod library;
pub mod occlusion;
pub mod quality;
pub mod shader_compiler;
pub mod static_apply;
pub mod video;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc::Sender, OnceLock},
};

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};

use crate::{
    config::schema::{
        Config, MonitorWallpaper, MonitorWallpaperPatch, WallpaperConfig,
        WallpaperEngineSettingsPatch, WallpaperFit, WallpaperKind,
    },
    desktop::monitors::{list_tauri_monitors, MonitorInfo},
    error::AppError,
    state::{AppState, PauseReason},
};

use self::{
    catalog::{find_photo, find_shader, WallpaperItem},
    engine::{EngineCmd, ResolvedSurfaceSpec, WallpaperEngineStatus},
};

static ENGINE_TX: OnceLock<Mutex<Option<Sender<EngineCmd>>>> = OnceLock::new();

fn tx_lock() -> &'static Mutex<Option<Sender<EngineCmd>>> {
    ENGINE_TX.get_or_init(|| Mutex::new(None))
}

fn send_cmd(cmd: EngineCmd) {
    if let Some(tx) = tx_lock().lock().as_ref() {
        let _ = tx.send(cmd);
    }
}

pub fn install(app: &AppHandle) {
    let mut guard = tx_lock().lock();
    if guard.is_none() {
        *guard = engine::spawn_engine_thread(app.clone());
    }
    drop(guard);

    let cfg = app.state::<AppState>().config.get();
    if !cfg.wallpaper.assignments.is_empty() {
        sync_runtime(app, &cfg);
    }
}

pub fn shutdown() {
    if let Some(tx) = tx_lock().lock().take() {
        let _ = tx.send(EngineCmd::Shutdown);
    }
}

pub fn notify_pause_changed(app: &AppHandle) {
    let reason = app.state::<AppState>().effective_pause.read().reason.clone();
    send_cmd(EngineCmd::SetGlobalPause(reason));
}

pub fn notify_explorer_restarted(_app: &AppHandle) {
    send_cmd(EngineCmd::ExplorerRestarted);
}

pub fn notify_monitors_changed(app: &AppHandle) {
    let cfg = app.state::<AppState>().config.get();
    sync_runtime(app, &cfg);
}

pub fn notify_wallpaper_changed(_app: &AppHandle) {
    send_cmd(EngineCmd::VerifyZOrder);
}

pub fn stop_all(app: &AppHandle) {
    send_cmd(EngineCmd::StopAll { ack: None });
    app.state::<AppState>().live_plate_overrides.write().clear();
    crate::desktop::wallpaper::trigger_refresh_now();
}

pub fn engine_status(app: &AppHandle) -> WallpaperEngineStatus {
    app.state::<AppState>().wallpaper_status.read().clone()
}

fn collect_monitors(app: &AppHandle) -> Vec<MonitorInfo> {
    let existing: Vec<MonitorInfo> = app
        .state::<AppState>()
        .monitors
        .read()
        .iter()
        .map(|e| e.info.clone())
        .collect();
    if !existing.is_empty() {
        return existing;
    }
    list_tauri_monitors(app)
        .into_iter()
        .map(|m| MonitorInfo::from_snapshot(&m))
        .collect()
}

async fn sync_specs_and_wait(
    wp_cfg: WallpaperConfig,
    specs: Vec<ResolvedSurfaceSpec>,
    global_pause: Option<PauseReason>,
) -> Result<(), AppError> {
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    let sent = {
        let guard = tx_lock().lock();
        if let Some(tx) = guard.as_ref() {
            tx.send(EngineCmd::Sync {
                config: wp_cfg,
                specs,
                global_pause,
                ack: Some(ack_tx),
            })
            .is_ok()
        } else {
            false
        }
    };
    if !sent {
        return Err(AppError::Other("Baggrundsmotoren kører ikke".into()));
    }

    let res = tokio::task::spawn_blocking(move || {
        ack_rx.recv_timeout(std::time::Duration::from_secs(6))
    })
    .await
    .map_err(|e| AppError::Other(format!("Venten på baggrundsmotor afbrudt: {e}")))?;

    match res {
        Ok(Ok(())) => Ok(()),
        Ok(Err(msg)) => Err(AppError::Other(msg)),
        Err(_) => Err(AppError::Other(
            "Baggrundsmotoren svarede ikke inden for tidsgrænsen".into(),
        )),
    }
}

pub async fn apply_wallpaper(
    app: &AppHandle,
    wallpaper_id: String,
    monitor: Option<String>,
    fit: Option<WallpaperFit>,
) -> Result<Config, AppError> {
    let target_key = monitor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("*")
        .to_string();

    let state = app.state::<AppState>();
    let current_cfg = state.config.get();
    let chosen_fit = fit.unwrap_or_else(|| {
        current_cfg
            .wallpaper
            .assignments
            .get(&target_key)
            .or_else(|| current_cfg.wallpaper.assignments.get("*"))
            .map(|a| a.fit)
            .unwrap_or_default()
    });

    enum ResolvedTarget {
        Shader,
        Photo(PathBuf),
        Video,
    }

    let resolved = if find_shader(&wallpaper_id).is_some() {
        ResolvedTarget::Shader
    } else if find_photo(&wallpaper_id).is_some() {
        let path = library::ensure_photo_downloaded(app, &wallpaper_id).await?;
        ResolvedTarget::Photo(path)
    } else if let Some(custom) = current_cfg
        .wallpaper
        .custom_items
        .iter()
        .find(|c| c.id == wallpaper_id)
    {
        match custom.kind {
            WallpaperKind::Photo => ResolvedTarget::Photo(PathBuf::from(&custom.file_path)),
            WallpaperKind::Video => {
                if !Path::new(&custom.file_path).is_file() {
                    return Err(AppError::Other(format!(
                        "Videofilen findes ikke: {}",
                        custom.file_path
                    )));
                }
                ResolvedTarget::Video
            }
            WallpaperKind::Shader => ResolvedTarget::Shader,
        }
    } else {
        return Err(AppError::Other(format!(
            "Ukendt baggrund valgt: {wallpaper_id}"
        )));
    };

    let mut candidate_wp = current_cfg.wallpaper.clone();
    let mut next_assignment = candidate_wp
        .assignments
        .get(&target_key)
        .cloned()
        .unwrap_or_else(|| MonitorWallpaper::new(wallpaper_id.clone(), chosen_fit));
    next_assignment.wallpaper_id = wallpaper_id.clone();
    if let Some(f) = fit {
        next_assignment.fit = f;
    }
    if target_key == "*" {
        candidate_wp.assignments.clear();
    }
    candidate_wp
        .assignments
        .insert(target_key.clone(), next_assignment);

    let monitors = collect_monitors(app);
    let global_pause = state.effective_pause.read().reason.clone();
    let spec_map = engine::build_surface_specs(app, &candidate_wp, &monitors);
    let specs: Vec<ResolvedSurfaceSpec> = monitors
        .iter()
        .filter_map(|m| spec_map.get(&m.name).cloned())
        .collect();

    match resolved {
        ResolvedTarget::Photo(ref img_path) => {
            // Stop any conflicting live surface on the target monitor(s) before applying static wallpaper.
            let _ = sync_specs_and_wait(candidate_wp.clone(), specs.clone(), global_pause).await;

            let img_clone = img_path.clone();
            let target_opt = if target_key == "*" {
                None
            } else {
                Some(target_key.clone())
            };
            let lossless = current_cfg.wallpaper.engine.lossless_png_apply;
            let app_clone = app.clone();
            let static_res = tokio::task::spawn_blocking(move || {
                static_apply::apply_static_wallpaper(
                    &app_clone,
                    target_opt.as_deref(),
                    &img_clone,
                    chosen_fit,
                    lossless,
                )
            })
            .await
            .map_err(|e| AppError::Other(format!("Kunne ikke anvende baggrund: {e}")))?;

            if let Err(err) = static_res {
                sync_runtime(app, &current_cfg);
                return Err(err);
            }
        }
        ResolvedTarget::Shader | ResolvedTarget::Video => {
            if specs.is_empty() {
                return Err(AppError::Other(
                    "Ingen aktive skærme fundet til animeret baggrund".into(),
                ));
            }
            if let Err(err) = sync_specs_and_wait(candidate_wp, specs.clone(), global_pause).await {
                sync_runtime(app, &current_cfg);
                return Err(err);
            }
        }
    }

    let updated = state.config.mutate(|cfg| {
        let mut assignment = cfg
            .wallpaper
            .assignments
            .get(&target_key)
            .cloned()
            .unwrap_or_else(|| MonitorWallpaper::new(wallpaper_id.clone(), chosen_fit));
        assignment.wallpaper_id = wallpaper_id.clone();
        if let Some(f) = fit {
            assignment.fit = f;
        }
        if target_key == "*" {
            cfg.wallpaper.assignments.clear();
        }
        cfg.wallpaper.assignments.insert(target_key.clone(), assignment);
        Ok(cfg.clone())
    })?;

    sync_glass_plate_overrides(app, &updated, &specs);
    Ok(updated)
}

pub fn update_assignment(
    app: &AppHandle,
    monitor: Option<String>,
    patch: MonitorWallpaperPatch,
) -> Result<Config, AppError> {
    let target_key = monitor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("*")
        .to_string();

    let state = app.state::<AppState>();
    let updated = state.config.mutate(|cfg| {
        if let Some(entry) = cfg.wallpaper.assignments.get_mut(&target_key) {
            entry.apply_patch(patch.clone());
        } else if target_key != "*" {
            if let Some(mut base) = cfg.wallpaper.assignments.get("*").cloned() {
                base.apply_patch(patch.clone());
                cfg.wallpaper.assignments.insert(target_key.clone(), base);
            }
        }
        Ok(cfg.clone())
    })?;

    // If active target is a static photo and fit changed, re-apply static wallpaper cleanly.
    if patch.fit.is_some() {
        if let Some(assignment) = updated
            .wallpaper
            .assignments
            .get(&target_key)
            .or_else(|| updated.wallpaper.assignments.get("*"))
        {
            if find_shader(&assignment.wallpaper_id).is_none()
                && updated
                    .wallpaper
                    .custom_items
                    .iter()
                    .find(|c| c.id == assignment.wallpaper_id)
                    .map(|c| c.kind != WallpaperKind::Video)
                    .unwrap_or(true)
            {
                let app_clone = app.clone();
                let wp_id = assignment.wallpaper_id.clone();
                let fit = assignment.fit;
                let lossless = updated.wallpaper.engine.lossless_png_apply;
                let target_opt = if target_key == "*" {
                    None
                } else {
                    Some(target_key.clone())
                };
                tauri::async_runtime::spawn(async move {
                    let path_res = if find_photo(&wp_id).is_some() {
                        library::ensure_photo_downloaded(&app_clone, &wp_id).await.ok()
                    } else {
                        app_clone
                            .state::<AppState>()
                            .config
                            .get()
                            .wallpaper
                            .custom_items
                            .into_iter()
                            .find(|c| c.id == wp_id)
                            .map(|c| PathBuf::from(c.file_path))
                    };
                    if let Some(p) = path_res {
                        let app_for_block = app_clone.clone();
                        match tokio::task::spawn_blocking(move || {
                            static_apply::apply_static_wallpaper(
                                &app_for_block,
                                target_opt.as_deref(),
                                &p,
                                fit,
                                lossless,
                            )
                        })
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(err)) => {
                                log::warn!("Kunne ikke opdatere statisk baggrundstilpasning: {err}");
                            }
                            Err(err) => {
                                log::warn!("Statisk baggrundsopgave afbrudt: {err}");
                            }
                        }
                    }
                });
            }
        }
    }

    sync_runtime(app, &updated);
    Ok(updated)
}

pub fn update_engine_settings(
    app: &AppHandle,
    patch: WallpaperEngineSettingsPatch,
) -> Result<Config, AppError> {
    let state = app.state::<AppState>();
    let updated = state.config.mutate(|cfg| {
        cfg.wallpaper.engine.apply_patch(patch);
        Ok(cfg.clone())
    })?;
    sync_runtime(app, &updated);
    Ok(updated)
}

pub fn stop_live(app: &AppHandle, monitor: Option<String>) -> Result<Config, AppError> {
    let target_key = monitor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let state = app.state::<AppState>();
    let updated = state.config.mutate(|cfg| {
        let custom_items = cfg.wallpaper.custom_items.clone();
        match target_key.as_deref() {
            None | Some("*") => {
                cfg.wallpaper
                    .assignments
                    .retain(|_, a| !is_live_id_with_customs(&custom_items, &a.wallpaper_id));
            }
            Some(mon) => {
                if let Some(a) = cfg.wallpaper.assignments.get(mon) {
                    if is_live_id_with_customs(&custom_items, &a.wallpaper_id) {
                        cfg.wallpaper.assignments.remove(mon);
                    }
                } else if let Some(global) = cfg.wallpaper.assignments.get("*").cloned() {
                    if is_live_id_with_customs(&custom_items, &global.wallpaper_id) {
                        cfg.wallpaper.assignments.remove("*");
                    }
                }
            }
        }
        Ok(cfg.clone())
    })?;

    sync_runtime(app, &updated);
    Ok(updated)
}

pub async fn import_and_save(app: &AppHandle, source_path: &Path) -> Result<WallpaperItem, AppError> {
    let custom = library::import_local_file(app, source_path).await?;
    let id = custom.id.clone();
    let state = app.state::<AppState>();
    state.config.mutate(|cfg| {
        cfg.wallpaper.custom_items.insert(0, custom);
        Ok(())
    })?;

    let catalog = library::build_catalog(app)?;
    catalog
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| AppError::Other("Importeret baggrund blev ikke fundet".into()))
}

pub fn delete_custom(app: &AppHandle, id: &str) -> Result<Config, AppError> {
    let state = app.state::<AppState>();
    let mut removed_item = None;
    let updated = state.config.mutate(|cfg| {
        if let Some(idx) = cfg.wallpaper.custom_items.iter().position(|c| c.id == id) {
            removed_item = Some(cfg.wallpaper.custom_items.remove(idx));
        }
        cfg.wallpaper.assignments.retain(|_, a| a.wallpaper_id != id);
        Ok(cfg.clone())
    })?;

    if let Some(item) = removed_item {
        library::delete_custom_assets(app, &item);
    }
    sync_runtime(app, &updated);
    Ok(updated)
}

pub fn reapply_static_assignments(app: &AppHandle) {
    let cfg = app.state::<AppState>().config.get();
    for (target_key, assignment) in cfg.wallpaper.assignments.clone() {
        if is_live_id(&cfg, &assignment.wallpaper_id) {
            continue;
        }
        let app_clone = app.clone();
        let wp_id = assignment.wallpaper_id.clone();
        let fit = assignment.fit;
        let lossless = cfg.wallpaper.engine.lossless_png_apply;
        let target_opt = if target_key == "*" {
            None
        } else {
            Some(target_key)
        };
        tauri::async_runtime::spawn(async move {
            let path_opt = if find_photo(&wp_id).is_some() {
                library::ensure_photo_downloaded(&app_clone, &wp_id).await.ok()
            } else {
                app_clone
                    .state::<AppState>()
                    .config
                    .get()
                    .wallpaper
                    .custom_items
                    .into_iter()
                    .find(|c| c.id == wp_id)
                    .map(|c| PathBuf::from(c.file_path))
            };
            if let Some(path) = path_opt {
                let app_for_block = app_clone.clone();
                match tokio::task::spawn_blocking(move || {
                    static_apply::apply_static_wallpaper(
                        &app_for_block,
                        target_opt.as_deref(),
                        &path,
                        fit,
                        lossless,
                    )
                })
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        log::warn!("Kunne ikke genanvende statisk baggrund: {err}");
                    }
                    Err(err) => {
                        log::warn!("Statisk genanvendelsesopgave afbrudt: {err}");
                    }
                }
            }
        });
    }
}

pub fn sync_runtime(app: &AppHandle, cfg: &Config) {
    let state = app.state::<AppState>();
    let monitors = collect_monitors(app);
    let global_pause = state.effective_pause.read().reason.clone();

    let spec_map = engine::build_surface_specs(app, &cfg.wallpaper, &monitors);
    let specs: Vec<ResolvedSurfaceSpec> = monitors
        .iter()
        .filter_map(|m| spec_map.get(&m.name).cloned())
        .collect();

    send_cmd(EngineCmd::Sync {
        config: cfg.wallpaper.clone(),
        specs: specs.clone(),
        global_pause,
        ack: None,
    });

    sync_glass_plate_overrides(app, cfg, &specs);
}

fn sync_glass_plate_overrides(
    app: &AppHandle,
    cfg: &Config,
    live_specs: &[ResolvedSurfaceSpec],
) {
    let state = app.state::<AppState>();
    if !cfg.wallpaper.engine.sync_glass_plates || live_specs.is_empty() {
        let had_any = {
            let mut guard = state.live_plate_overrides.write();
            let was_empty = guard.is_empty();
            guard.clear();
            !was_empty
        };
        if had_any {
            crate::desktop::wallpaper::trigger_refresh_now();
        }
        return;
    }

    let app_clone = app.clone();
    let specs_owned = live_specs.to_vec();
    tauri::async_runtime::spawn_blocking(move || {
        let mut next_overrides: HashMap<String, PathBuf> = HashMap::new();
        let dirs = library::wallpaper_dirs(&app_clone).ok();

        for spec in &specs_owned {
            match spec.kind {
                WallpaperKind::Shader => {
                    if let Ok(poster) = library::ensure_shader_poster(
                        &app_clone,
                        &spec.assignment.wallpaper_id,
                        spec.assignment.brightness,
                    ) {
                        next_overrides.insert(spec.monitor_name.clone(), poster);
                    }
                }
                WallpaperKind::Video => {
                    if let Some(ref d) = dirs {
                        let poster = d.posters.join(format!("{}.png", spec.assignment.wallpaper_id));
                        if poster.is_file() {
                            next_overrides.insert(spec.monitor_name.clone(), poster);
                        }
                    }
                }
                WallpaperKind::Photo => {}
            }
        }

        let state = app_clone.state::<AppState>();
        let changed = {
            let mut guard = state.live_plate_overrides.write();
            if *guard != next_overrides {
                *guard = next_overrides;
                true
            } else {
                false
            }
        };
        if changed {
            crate::desktop::wallpaper::trigger_refresh_now();
        }
    });
}

fn is_live_id(cfg: &Config, wallpaper_id: &str) -> bool {
    is_live_id_with_customs(&cfg.wallpaper.custom_items, wallpaper_id)
}

fn is_live_id_with_customs(
    custom_items: &[crate::config::schema::CustomWallpaper],
    wallpaper_id: &str,
) -> bool {
    if find_shader(wallpaper_id).is_some() {
        return true;
    }
    custom_items
        .iter()
        .any(|c| c.id == wallpaper_id && c.kind == WallpaperKind::Video)
}

pub fn resolve_assignment_for_monitor<'a>(
    wp: &'a WallpaperConfig,
    monitor_name: &str,
) -> Option<&'a MonitorWallpaper> {
    wp.assignments
        .get(monitor_name)
        .or_else(|| wp.assignments.get("*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_monitor_specific_before_wildcard() {
        let mut wp = WallpaperConfig::default();
        wp.assignments.insert(
            "*".into(),
            MonitorWallpaper::new("scene-amber-dunes".into(), WallpaperFit::Fill),
        );
        wp.assignments.insert(
            r"\\.\DISPLAY2".into(),
            MonitorWallpaper::new("photo-deep-space".into(), WallpaperFit::Fill),
        );

        assert_eq!(
            resolve_assignment_for_monitor(&wp, r"\\.\DISPLAY1")
                .unwrap()
                .wallpaper_id,
            "scene-amber-dunes"
        );
        assert_eq!(
            resolve_assignment_for_monitor(&wp, r"\\.\DISPLAY2")
                .unwrap()
                .wallpaper_id,
            "photo-deep-space"
        );
    }
}
