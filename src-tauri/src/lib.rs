mod commands;
pub mod config;
pub mod desktop;
pub mod elevation;
pub mod error;
pub mod providers;
mod shortcuts;
mod state;
mod tray;
pub mod wallpaper;
mod windows;

use config::{
    schema::SizeF,
    store::ConfigStore,
};
use desktop::monitors::{list_tauri_monitors, MonitorInfo};
use log::{error, warn, LevelFilter};
use state::AppState;
use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_log::{Target, TargetKind};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    elevation::try_auto_elevate();

    let log_level = if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    let mut log_targets = vec![Target::new(TargetKind::LogDir {
        file_name: Some("open-dock".into()),
    })];
    if cfg!(debug_assertions) {
        log_targets.push(Target::new(TargetKind::Stdout));
    }

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Err(error) = windows::control::open(app, None) {
                warn!("Kunne ikke åbne kontrolcentret fra single-instance: {error:?}");
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log_level)
                .targets(log_targets)
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::app::provider_subscribe,
            commands::app::provider_unsubscribe,
            commands::app::app_open_control,
            commands::app::app_toggle_devtools,
            commands::app::app_quit,
            commands::app::app_set_visible,
            commands::app::app_set_paused,
            commands::app::app_status,
            commands::app::app_stats,
            commands::app::app_open_folder,
            commands::app::layout_export,
            commands::app::layout_import,
            commands::app::app_reset,
            commands::config::config_get,
            commands::config::config_take_restore_notice,
            commands::config::appearance_get,
            commands::config::appearance_patch,
            commands::config::general_patch,
            commands::config::onboarding_complete,
            commands::widgets::widget_add,
            commands::widgets::widget_update,
            commands::widgets::widget_remove,
            commands::widgets::widget_duplicate,
            commands::widgets::widget_restore,
            commands::widgets::widget_move,
            commands::widgets::widget_ready,
            commands::widgets::widget_set_focusable,
            commands::widgets::widget_context_menu,
            commands::widgets::cluster_add_preset,
            commands::widgets::cluster_set_window_size,
            commands::widgets::monitors_list,
            commands::edit::edit_set,
            commands::edit::edit_begin_drag,
            commands::edit::edit_nudge,
            commands::edit::edit_cycle_size,
            commands::media::media_control,
            commands::media::media_select_source,
            commands::system::system_history,
            commands::system::system_caps,
            commands::weather::weather_search,
            commands::weather::weather_location_add,
            commands::weather::weather_location_remove,
            commands::weather::weather_refresh,
            commands::note::note_load,
            commands::note::note_save,
            commands::wallpaper::wallpaper_catalog,
            commands::wallpaper::wallpaper_apply,
            commands::wallpaper::wallpaper_update_assignment,
            commands::wallpaper::wallpaper_engine_patch,
            commands::wallpaper::wallpaper_stop_live,
            commands::wallpaper::wallpaper_import,
            commands::wallpaper::wallpaper_delete_custom,
            commands::wallpaper::wallpaper_engine_status,
            commands::wallpaper::wallpaper_windows_quality_get,
            commands::wallpaper::wallpaper_windows_quality_set,
        ])
        .setup(|app| {
            if let Ok(resource_dir) = app.path().resource_dir() {
                providers::cpu_sensor::configure(resource_dir);
            }
            log::info!(
                "Open Dock starter (elevated={})",
                elevation::process_is_elevated()
            );

            let snapshots = list_tauri_monitors(app.handle());
            let initial_primary = snapshots
                .iter()
                .find(|m| m.is_primary)
                .or_else(|| snapshots.first())
                .map(|m| {
                    let scale = m.scale.max(0.5);
                    MonitorInfo {
                        name: m.name.clone(),
                        rect: m.rect,
                        work_area: m.work_area,
                        scale: m.scale,
                        logical_size: SizeF {
                            width: m.rect.width as f64 / scale,
                            height: m.rect.height as f64 / scale,
                        },
                        plate_path: String::new(),
                        avg_luma: 0.15,
                        is_primary: m.is_primary,
                    }
                });

            let config_store = ConfigStore::load(app.handle(), initial_primary.as_ref());
            let initial_cfg = config_store.get();
            let onboarded = initial_cfg.onboarded;
            let initial_autostart = initial_cfg.general.autostart;
            let initial_control_theme = initial_cfg.general.control_theme;
            app.manage(AppState::new(config_store, Vec::new()));
            app.manage(providers::scheduler::Scheduler::new(app.handle().clone()));

            desktop::install(app.handle())?;
            wallpaper::install(app.handle());
            tray::install(app.handle())?;
            shortcuts::install(app.handle());
            windows::widget::spawn_all(app.handle())?;

            let launched_via_autostart = std::env::args().any(|arg| arg == "--autostart");
            if onboarded && (launched_via_autostart || !cfg!(debug_assertions)) {
                commands::config::sync_autostart(app.handle(), initial_autostart);
            }
            if let Some(control) = app.get_webview_window("control") {
                windows::control::configure(&control, initial_control_theme);
                if !launched_via_autostart || !onboarded {
                    control.show()?;
                    control.set_focus()?;
                }
            } else if !launched_via_autostart || !onboarded {
                warn!("Kontrolvinduet mangler under opstart");
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id.starts_with("ctx:") {
                windows::menu::handle_event(app, id);
            } else if id.starts_with("tray:") {
                tray::handle_menu_event(app, id);
            }
        })
        .on_window_event(|window, event| {
            if window.label() == "control" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    windows::control::hide_to_background(window.app_handle(), window);
                }
            }
            if matches!(event, WindowEvent::Destroyed) {
                if let Some(scheduler) = window.app_handle().try_state::<providers::scheduler::Scheduler>() {
                    scheduler.unsubscribe_window(window.label());
                }
            }
        })
        .build(tauri::generate_context!());

    match result {
        Ok(app) => app.run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    if let Err(error) = state.config.flush() {
                        error!("Kunne ikke gemme konfiguration ved lukning: {error:?}");
                    }
                }
                desktop::shutdown();
            }
        }),
        Err(error) => error!("Open Dock stoppede med en fejl: {error}"),
    }
}
