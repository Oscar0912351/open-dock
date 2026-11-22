use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    time::Duration,
};

use serde_json::Value;
use sysinfo::{CpuRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::{
    config::{
        migrate,
        schema::{Config, General, WallpaperConfig, WeatherConfig},
    },
    desktop::{drag, fullscreen},
    error::AppError,
    providers::scheduler::{ProviderKind, ProviderSnapshot, Scheduler},
    state::{AppState, AppStats, AppStatus, AppVisible},
    windows::{control, widget},
};

#[tauri::command]
pub fn provider_subscribe(
    window: tauri::Window,
    kind: ProviderKind,
    key: Option<String>,
) -> Result<Option<ProviderSnapshot>, AppError> {
    let scheduler = window.state::<Scheduler>();
    Ok(scheduler.subscribe(window.label().to_string(), kind, key))
}

#[tauri::command]
pub fn provider_unsubscribe(
    window: tauri::Window,
    kind: ProviderKind,
    key: Option<String>,
) -> Result<(), AppError> {
    let scheduler = window.state::<Scheduler>();
    scheduler.unsubscribe(window.label(), kind, key);
    Ok(())
}

#[tauri::command]
pub fn app_open_control(app: AppHandle, route: Option<String>) -> Result<(), AppError> {
    control::open(&app, route)
}

#[tauri::command]
pub fn app_toggle_devtools(window: tauri::WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
}

#[tauri::command]
pub fn app_quit(app: AppHandle) -> Result<(), AppError> {
    quit_app(&app);
    Ok(())
}

pub fn quit_app(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state
            .quitting
            .store(true, std::sync::atomic::Ordering::Release);
        if let Err(error) = state.config.flush() {
            log::warn!(target: "cmd", "Kunne ikke gemme konfiguration før afslutning: {error:?}");
        }
    }
    crate::desktop::shutdown();
    app.exit(0);
}

#[tauri::command]
pub fn app_set_visible(app: AppHandle, visible: bool) -> Result<(), AppError> {
    set_widgets_visible(&app, visible)
}

pub fn set_widgets_visible(app: &AppHandle, visible: bool) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    if !visible && state.edit_mode.load(std::sync::atomic::Ordering::Acquire) {
        drag::set_edit_mode(app, false)?;
    }
    state
        .widgets_visible
        .store(visible, std::sync::atomic::Ordering::Release);
    widget::set_all_visible(app, visible);
    fullscreen::recompute_pause(app);
    if let Err(error) = app.emit("app://visible", AppVisible { visible }) {
        log::warn!(target: "cmd", "Kunne ikke sende app://visible: {error}");
    }
    crate::tray::sync(app);
    Ok(())
}

#[tauri::command]
pub fn app_set_paused(app: AppHandle, paused: bool) -> Result<(), AppError> {
    set_manual_paused(&app, paused);
    Ok(())
}

pub fn set_manual_paused(app: &AppHandle, paused: bool) {
    let state = app.state::<AppState>();
    state
        .manual_paused
        .store(paused, std::sync::atomic::Ordering::Release);
    fullscreen::recompute_pause(app);
}

#[tauri::command]
pub fn app_status(app: AppHandle) -> Result<AppStatus, AppError> {
    let state = app.state::<AppState>();
    let eff = state.effective_pause.read().clone();
    Ok(AppStatus {
        visible: state
            .widgets_visible
            .load(std::sync::atomic::Ordering::Acquire),
        paused: eff.paused,
        reason: eff.reason,
        edit_mode: state.edit_mode.load(std::sync::atomic::Ordering::Acquire),
        is_windows_11: crate::desktop::glass::is_windows_11(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[tauri::command]
pub async fn app_stats(app: AppHandle) -> Result<AppStats, AppError> {
    let state = app.state::<AppState>();
    let needs_warmup = {
        let mut guard = state.process_monitor.lock();
        if guard.is_none() {
            let mut sys = System::new_with_specifics(
                RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()),
            )
            .map_err(|err| AppError::Other(format!("Kunne ikke læse procesdata: {err}")))?;
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
            *guard = Some(sys);
            true
        } else {
            false
        }
    };

    if needs_warmup {
        tokio::time::sleep(Duration::from_millis(120)).await;
    }

    let mut guard = state.process_monitor.lock();
    let sys = guard
        .as_mut()
        .ok_or_else(|| AppError::Other("Procesovervågning ikke initialiseret".into()))?;
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );

    let root_pid = Pid::from_u32(std::process::id());
    let processes = sys.processes();
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (&pid, proc_info) in processes {
        if let Some(parent_pid) = proc_info.parent() {
            children.entry(parent_pid).or_default().push(pid);
        }
    }

    let mut tree: HashSet<Pid> = HashSet::new();
    let mut queue = VecDeque::new();
    tree.insert(root_pid);
    queue.push_back(root_pid);
    while let Some(cur) = queue.pop_front() {
        if let Some(kids) = children.get(&cur) {
            for &kid in kids {
                if tree.insert(kid) {
                    queue.push_back(kid);
                }
            }
        }
    }

    let mut total_mem: u64 = 0;
    let mut total_cpu: f32 = 0.0;
    for pid in tree {
        if let Some(proc_info) = processes.get(&pid) {
            total_mem = total_mem.saturating_add(proc_info.memory());
            total_cpu += proc_info.cpu_usage();
        }
    }

    let core_count = sys.cpus().len().max(1) as f32;
    let cpu_pct = (total_cpu / core_count).clamp(0.0, 100.0);

    Ok(AppStats {
        cpu_pct,
        mem_bytes: total_mem,
    })
}

#[tauri::command]
pub fn app_open_folder(app: AppHandle, kind: String) -> Result<(), AppError> {
    let dir = match kind.as_str() {
        "logs" => app.path().app_log_dir()?,
        "data" => app.path().app_config_dir()?,
        _ => return Err(AppError::Other("Ukendt mappetype".into())),
    };
    fs::create_dir_all(&dir).map_err(AppError::ConfigWrite)?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|err| AppError::Other(format!("Kunne ikke åbne mappen: {err}")))?;
    Ok(())
}

#[tauri::command]
pub fn layout_export(app: AppHandle, path: String) -> Result<(), AppError> {
    let cfg = app.state::<AppState>().config.get();
    let json = serde_json::to_string_pretty(&cfg)
        .map_err(|err| AppError::Other(format!("JSON-serialisering fejlede: {err}")))?;
    fs::write(&path, json).map_err(AppError::ConfigWrite)?;
    Ok(())
}

#[tauri::command]
pub async fn layout_import(app: AppHandle, path: String) -> Result<Config, AppError> {
    let raw = fs::read_to_string(&path).map_err(AppError::ConfigWrite)?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|_| AppError::Other("Filen indeholder ikke et gyldigt Open Dock-layout.".into()))?;
    let imported = migrate::migrate(value)
        .map_err(|_| AppError::Other("Filen passer ikke til Open Docks layoutformat.".into()))?;

    let state = app.state::<AppState>();
    if state.edit_mode.load(std::sync::atomic::Ordering::Acquire) {
        drag::set_edit_mode(&app, false)?;
    }

    widget::close_all(&app);
    wait_for_widget_windows_closed(&app).await;
    state.cluster_sizes.write().clear();

    let next = state.config.mutate(|cfg| {
        *cfg = imported;
        Ok(cfg.clone())
    })?;
    state.config.flush()?;

    if let Err(error) = crate::shortcuts::rebind(&app, &next.general.hotkeys) {
        log::warn!(target: "cmd", "Kunne ikke gendanne genveje efter import: {error:?}");
    }
    crate::commands::config::sync_autostart(&app, next.general.autostart);
    crate::tray::sync(&app);

    if let Err(error) = widget::spawn_all(&app) {
        log::error!(target: "cmd", "Kunne ikke genskabe widgets efter import: {error:?}");
    }
    widget::reapply_glass_all(&app);
    crate::wallpaper::sync_runtime(&app, &next);

    Ok(next)
}

#[tauri::command]
pub async fn app_reset(app: AppHandle) -> Result<Config, AppError> {
    let state = app.state::<AppState>();
    if state.edit_mode.load(std::sync::atomic::Ordering::Acquire) {
        drag::set_edit_mode(&app, false)?;
    }

    widget::close_all(&app);
    wait_for_widget_windows_closed(&app).await;

    let notes_dir = state.config.notes_dir();
    if notes_dir.exists() {
        if let Ok(entries) = fs::read_dir(&notes_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("txt") {
                    let _ = fs::remove_file(p);
                }
            }
        }
    }

    crate::wallpaper::stop_all(&app);
    state.cluster_sizes.write().clear();
    state.removed_history.lock().clear();
    state
        .widgets_visible
        .store(true, std::sync::atomic::Ordering::Release);
    state
        .manual_paused
        .store(false, std::sync::atomic::Ordering::Release);
    let _ = app.emit("app://visible", AppVisible { visible: true });

    let next = state.config.mutate(|cfg| {
        *cfg = Config {
            version: crate::config::schema::CONFIG_VERSION,
            general: General::default(),
            appearance: crate::config::schema::Appearance::default(),
            weather: WeatherConfig::default(),
            wallpaper: WallpaperConfig::default(),
            widgets: Vec::new(),
            onboarded: false,
        };
        Ok(cfg.clone())
    })?;
    state.config.flush()?;
    if let Err(error) = crate::shortcuts::rebind(&app, &next.general.hotkeys) {
        log::warn!(target: "cmd", "Kunne ikke nulstille genveje: {error:?}");
    }
    crate::commands::config::sync_autostart(&app, next.general.autostart);
    fullscreen::recompute_pause(&app);
    crate::tray::sync(&app);

    Ok(next)
}

async fn wait_for_widget_windows_closed(app: &AppHandle) {
    for _ in 0..30 {
        let any_open = app
            .webview_windows()
            .keys()
            .any(|label| label.starts_with("w-"));
        if !any_open {
            return;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}
