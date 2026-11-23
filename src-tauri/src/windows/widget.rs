use std::{collections::HashMap, sync::atomic::Ordering};

use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};
use uuid::Uuid;
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{IsWindow, ShowWindow, SW_HIDE},
};

use crate::{
    config::schema::{Appearance, GlassEngine, Layer, VerticalAnchor, WidgetInstance, WidgetKind},
    desktop::{
        backdrop::{self, BackdropSpec},
        glass,
        monitors::{self, MonitorInfo},
        styles, zorder,
    },
    error::AppError,
    state::{AppState, GameOverlayState, PauseReason, WidgetBoot, WidgetGlassState},
};

pub fn spawn_all(app: &AppHandle) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    let config = state.config.get();
    let monitors: Vec<MonitorInfo> = state
        .monitors
        .read()
        .iter()
        .map(|entry| entry.info.clone())
        .collect();

    let mut order: Vec<(usize, i32, i32, Uuid)> = config
        .widgets
        .iter()
        .enumerate()
        .map(|(idx, widget)| {
            let mon_rank = monitors
                .iter()
                .position(|m| m.name == widget.placement.monitor.name)
                .map(|pos| if monitors[pos].is_primary { 0 } else { pos + 1 })
                .unwrap_or(99);
            let y_ord = (widget.placement.y * 10.0).round() as i32;
            let x_ord = (widget.placement.x * 10.0).round() as i32;
            (mon_rank, y_ord, x_ord, widget.id.max(Uuid::nil()))
                .tap_debug_order(idx)
        })
        .collect();

    order.sort_by_key(|&(mon_rank, y_ord, x_ord, _)| (mon_rank, y_ord, x_ord));

    let delays: HashMap<Uuid, u32> = order
        .into_iter()
        .enumerate()
        .map(|(idx, (_, _, _, id))| {
            let delay = ((idx as u32) * 90).min(540);
            (id, delay)
        })
        .collect();

    *state.start_delays.write() = delays;

    for widget in &config.widgets {
        if let Err(error) = spawn_one(app, widget) {
            log::error!("Kunne ikke oprette widgetvindue {}: {error:?}", widget.id);
        }
    }
    Ok(())
}

trait TupleOrderExt {
    fn tap_debug_order(self, _idx: usize) -> Self;
}

impl TupleOrderExt for (usize, i32, i32, Uuid) {
    fn tap_debug_order(self, _idx: usize) -> Self {
        self
    }
}

pub fn spawn_one(app: &AppHandle, instance: &WidgetInstance) -> Result<(), AppError> {
    let label = instance.window_label();
    if app.get_webview_window(&label).is_some() {
        return Ok(());
    }

    let id_str = instance.id.to_string();
    let url = WebviewUrl::App(format!("widget.html?id={id_str}").into());
    let (base_w, base_h) = instance.base_size();
    let logical_w = (base_w * instance.scale as f64).round().max(32.0);
    let logical_h = (base_h * instance.scale as f64).round().max(32.0);

    let window = crate::windows::apply_webview_defaults(
        WebviewWindowBuilder::new(app, &label, url),
    )
        .title("Open Dock")
        .inner_size(logical_w, logical_h)
        .position(48.0, 48.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .always_on_top(false)
        .drag_and_drop(false)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .build()?;

    crate::windows::webview_opt::configure_widget_webview(&window);

    let raw_hwnd = window.hwnd()?.0 as isize;
    let layer = instance.layer;
    let edit_mode = app.state::<AppState>().edit_mode.load(Ordering::Acquire);
    let label_for_ui = label.clone();
    let attach_id = id_str;
    let click_through = instance.click_through;
    crate::desktop::run_on_ui_thread_sync(app, move || -> Result<(), AppError> {
        let hwnd = HWND(raw_hwnd as _);
        unsafe {
            styles::apply_widget_styles(hwnd)?;
            zorder::attach(hwnd, layer, attach_id, label_for_ui.clone())?;
        }
        if edit_mode {
            if let Err(error) = zorder::set_widget_activatable(hwnd, true, false) {
                log::warn!("Kunne ikke sætte redigerings-aktivering på {label_for_ui}: {error}");
            }
        }
        if let Err(error) = zorder::set_click_through(hwnd, click_through && !edit_mode) {
            log::warn!("Kunne ikke sætte click-through på {label_for_ui}: {error}");
        }
        Ok(())
    })??;

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        match window.is_visible() {
            Ok(true) => {}
            Ok(false) => {
                let visible = window
                    .app_handle()
                    .state::<AppState>()
                    .widgets_visible
                    .load(Ordering::Acquire);
                if !visible {
                    return;
                }
                log::warn!("widget_ready blev ikke kaldt inden for 4 sekunder; viser {label}");
                if let Err(error) = show_without_activation(&window) {
                    log::error!("Kunne ikke vise widget efter timeout: {error:?}");
                }
            }
            Err(error) => log::warn!("Kunne ikke kontrollere widget-synlighed: {error}"),
        }
    });

    Ok(())
}

pub fn close_one(app: &AppHandle, id: Uuid) {
    let label = format!("w-{id}");
    if let Some(window) = app.get_webview_window(&label) {
        if let Err(error) = window.close() {
            log::warn!("Kunne ikke lukke widgetvindue {label}: {error}");
        }
    }
}

pub fn close_all(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("w-") {
            if let Err(error) = window.close() {
                log::warn!("Kunne ikke lukke widgetvindue {label}: {error}");
            }
        }
    }
}

fn widget_game_overlay_active(app: &AppHandle, hwnd: HWND) -> bool {
    let state = app.state::<AppState>();
    if !state.game_overlay_active.load(Ordering::Acquire) {
        return false;
    }
    let target = *state.fullscreen_target.read();
    crate::desktop::fullscreen::hwnd_covered_by_fullscreen(hwnd, target)
}

fn sync_widget_click_through(
    hwnd: HWND,
    instance_click_through: bool,
    edit_mode: bool,
    label: &str,
) {
    let overlay = zorder::is_widget_game_overlay(hwnd);
    let idle_hidden = zorder::is_idle_hidden(hwnd);
    let ignore = (overlay || instance_click_through || idle_hidden) && !edit_mode;
    if let Err(error) = zorder::set_click_through(hwnd, ignore) {
        log::warn!("Kunne ikke opdatere click-through på {label}: {error}");
    }
}

pub fn set_all_visible(app: &AppHandle, visible: bool) {
    let state = app.state::<AppState>();
    let config = state.config.get();
    let fullscreen_target = if *state.auto_pause_reason.read() == Some(PauseReason::Fullscreen) {
        *state.fullscreen_target.read()
    } else {
        None
    };

    for widget in &config.widgets {
        let label = widget.window_label();
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        let Ok(raw_hwnd) = window.hwnd() else {
            continue;
        };
        let hwnd = HWND(raw_hwnd.0 as _);
        let hidden_by_fs = widget.layer == Layer::Top
            && crate::desktop::fullscreen::hwnd_covered_by_fullscreen(hwnd, fullscreen_target);
        zorder::set_fullscreen_hidden(hwnd, hidden_by_fs);
        if !visible || hidden_by_fs {
            // Tauris `hide()` er en no-op på de subklassede widget-vinduer;
            // skjul direkte på HWND'et i stedet.
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            crate::windows::webview_opt::set_webview_suspended(&window, true);
            continue;
        }
        crate::windows::webview_opt::set_webview_suspended(&window, false);
        if let Err(error) = zorder::set_layer(hwnd, widget.layer) {
            log::warn!("Kunne ikke vise widget {label}: {error}");
        }
    }
}

pub fn ready(app: &AppHandle, id: &str) -> Result<WidgetBoot, AppError> {
    let uuid = Uuid::parse_str(id).map_err(|_| AppError::WidgetNotFound)?;
    let state = app.state::<AppState>();
    let config = state.config.get();
    let instance = config
        .widgets
        .into_iter()
        .find(|w| w.id == uuid)
        .ok_or(AppError::WidgetNotFound)?;

    let label = instance.window_label();
    let window = app
        .get_webview_window(&label)
        .ok_or(AppError::WidgetNotFound)?;

    let monitor_entries = state.monitors.read().clone();
    let monitor_infos: Vec<MonitorInfo> = monitor_entries.iter().map(|e| e.info.clone()).collect();
    apply_window_geometry(app, &window, &instance, &monitor_infos)?;

    let hwnd = HWND(window.hwnd()?.0 as _);
    let appearance = config.appearance;
    let game_overlay = widget_game_overlay_active(app, hwnd);
    zorder::set_widget_game_overlay(hwnd, game_overlay);

    let effective_engine = glass::effective_engine_in_context(
        appearance.glass_engine,
        instance.glass_override_engine,
        instance.size,
        glass::is_windows_11(),
        game_overlay,
    );
    let live_backdrop = apply_widget_glass_and_backdrop(
        app,
        &window,
        &instance,
        &appearance,
        effective_engine,
        &monitor_infos,
    );

    let edit_mode = state.edit_mode.load(Ordering::Acquire);
    sync_widget_click_through(hwnd, instance.click_through, edit_mode, &label);

    let hidden_by_fs = instance.layer == Layer::Top
        && *state.auto_pause_reason.read() == Some(PauseReason::Fullscreen)
        && crate::desktop::fullscreen::hwnd_covered_by_fullscreen(
            hwnd,
            *state.fullscreen_target.read(),
        );
    zorder::set_fullscreen_hidden(hwnd, hidden_by_fs);

    if state.widgets_visible.load(Ordering::Acquire) && !hidden_by_fs {
        show_without_activation(&window)?;
    }

    let placement = monitors::compute_widget_placement(id, &window, &monitor_entries);
    let monitor = placement
        .as_ref()
        .and_then(|p| monitor_entries.iter().find(|m| m.info.name == p.monitor))
        .or_else(|| monitor_entries.iter().find(|m| m.info.is_primary))
        .or_else(|| monitor_entries.first())
        .map(|m| m.info.clone());

    if let Some(ref p) = placement {
        let _ = app.emit_to(&label, "widget://placement", p);
        let _ = app.emit_to("control", "widget://placement", p);
    }

    let start_delay_ms = state.start_delays.write().remove(&uuid).unwrap_or(0);

    Ok(WidgetBoot {
        instance,
        appearance,
        effective_engine,
        live_backdrop,
        monitor,
        placement,
        start_delay_ms,
        edit_mode,
        paused: state.paused.load(Ordering::Acquire),
        game_overlay_active: game_overlay,
    })
}

pub fn apply_game_overlay(
    app: &AppHandle,
    target: Option<crate::desktop::fullscreen::FullscreenTarget>,
) {
    let state = app.state::<AppState>();
    let config = state.config.get();
    let edit_mode = state.edit_mode.load(Ordering::Acquire);
    let mut any_active = false;

    for widget in &config.widgets {
        let label = widget.window_label();
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        let Ok(raw_hwnd) = window.hwnd() else {
            continue;
        };
        let hwnd = HWND(raw_hwnd.0 as _);
        let active = crate::desktop::fullscreen::hwnd_covered_by_fullscreen(hwnd, target);
        any_active |= active;
        zorder::set_widget_game_overlay(hwnd, active);
        sync_widget_click_through(hwnd, widget.click_through, edit_mode, &label);
        if let Err(error) = app.emit_to(&label, "app://game-overlay", GameOverlayState { active }) {
            log::warn!("Kunne ikke sende app://game-overlay til {label}: {error}");
        }
    }

    reapply_glass_all(app);

    log::info!(
        "Spil-overlay {}",
        if any_active { "aktiveret" } else { "deaktiveret" }
    );
}

pub fn apply_instance_live(app: &AppHandle, instance: &WidgetInstance) {
    let label = instance.window_label();
    let Some(window) = app.get_webview_window(&label) else {
        return;
    };

    let state = app.state::<AppState>();
    let config = state.config.get();
    let monitor_entries = state.monitors.read().clone();
    let monitor_infos: Vec<MonitorInfo> = monitor_entries.iter().map(|e| e.info.clone()).collect();

    if let Err(error) = apply_window_geometry(app, &window, instance, &monitor_infos) {
        log::warn!("Kunne ikke opdatere størrelse/placering på {label}: {error:?}");
    }

    let edit_mode = state.edit_mode.load(Ordering::Acquire);
    let mut game_overlay = false;
    if let Ok(raw_hwnd) = window.hwnd() {
        let hwnd = HWND(raw_hwnd.0 as _);
        let hidden_by_fs = instance.layer == Layer::Top
            && *state.auto_pause_reason.read() == Some(PauseReason::Fullscreen)
            && crate::desktop::fullscreen::hwnd_covered_by_fullscreen(
                hwnd,
                *state.fullscreen_target.read(),
            );
        zorder::set_fullscreen_hidden(hwnd, hidden_by_fs);
        game_overlay = widget_game_overlay_active(app, hwnd);
        zorder::set_widget_game_overlay(hwnd, game_overlay);
        if let Err(error) = zorder::set_layer(hwnd, instance.layer) {
            log::warn!("Kunne ikke opdatere lag på {label}: {error}");
        }
        if hidden_by_fs {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
        sync_widget_click_through(hwnd, instance.click_through, edit_mode, &label);
    }

    let effective_engine = glass::effective_engine_in_context(
        config.appearance.glass_engine,
        instance.glass_override_engine,
        instance.size,
        glass::is_windows_11(),
        game_overlay,
    );
    let live_backdrop = apply_widget_glass_and_backdrop(
        app,
        &window,
        instance,
        &config.appearance,
        effective_engine,
        &monitor_infos,
    );

    let glass_payload = WidgetGlassState {
        appearance: config.appearance,
        effective_engine,
        live_backdrop,
    };
    let _ = app.emit_to(&label, "widget://glass", &glass_payload);

    let id_str = instance.id.to_string();
    if let Some(placement) = monitors::compute_widget_placement(&id_str, &window, &monitor_entries) {
        let _ = app.emit_to(&label, "widget://placement", &placement);
        let _ = app.emit_to("control", "widget://placement", &placement);
    }
}

pub fn reapply_glass_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let config = state.config.get();
    let win11 = glass::is_windows_11();
    let monitor_infos: Vec<MonitorInfo> = state
        .monitors
        .read()
        .iter()
        .map(|entry| entry.info.clone())
        .collect();

    for widget in &config.widgets {
        let label = widget.window_label();
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        let game_overlay = window
            .hwnd()
            .ok()
            .map(|raw| widget_game_overlay_active(app, HWND(raw.0 as _)))
            .unwrap_or(false);
        let effective_engine = glass::effective_engine_in_context(
            config.appearance.glass_engine,
            widget.glass_override_engine,
            widget.size,
            win11,
            game_overlay,
        );
        let live_backdrop = apply_widget_glass_and_backdrop(
            app,
            &window,
            widget,
            &config.appearance,
            effective_engine,
            &monitor_infos,
        );
        let payload = WidgetGlassState {
            appearance: config.appearance.clone(),
            effective_engine,
            live_backdrop,
        };
        if let Err(error) = app.emit_to(&label, "widget://glass", &payload) {
            log::warn!("Kunne ikke sende widget://glass til {label}: {error}");
        }
    }
}

fn apply_widget_glass_and_backdrop(
    app: &AppHandle,
    window: &WebviewWindow,
    instance: &WidgetInstance,
    appearance: &Appearance,
    effective_engine: GlassEngine,
    monitors: &[MonitorInfo],
) -> bool {
    let effective_theme = instance.theme_override.unwrap_or(appearance.theme_id);
    glass::apply_window_glass(window, effective_engine, effective_theme);

    let mode = glass::resolve_live_backdrop_mode(
        effective_engine,
        instance.layer,
        glass::is_windows_11(),
        glass::is_windows_10_2004_or_newer(),
        backdrop::system_transparency_enabled(),
    );
    let Ok(raw_hwnd) = window.hwnd() else {
        return false;
    };
    let raw = raw_hwnd.0 as isize;
    let dpi_scale = monitors::find_target_monitor(&instance.placement.monitor, monitors)
        .map(|m| m.scale)
        .unwrap_or(1.0);
    let base_width = effective_base_size(app, instance).0;
    let spec = BackdropSpec {
        theme: effective_theme,
        glass_strength: instance.glass_override.unwrap_or(appearance.glass_strength),
        radius: appearance.radius,
        size: instance.size,
        is_cluster: instance.kind == WidgetKind::Cluster,
        base_width,
        widget_scale: instance.scale,
        dpi_scale,
        layer: instance.layer,
    };

    match crate::desktop::run_on_ui_thread_sync(app, move || {
        backdrop::apply(HWND(raw as _), mode, spec)
    }) {
        Ok(live) => live,
        Err(error) => {
            log::warn!("Kunne ikke opdatere live backdrop på UI-tråden: {error}");
            false
        }
    }
}

pub fn set_backdrop_visible(
    app: &AppHandle,
    id: &str,
    visible: bool,
    duration_ms: u32,
) -> Result<(), AppError> {
    let uuid = Uuid::parse_str(id).map_err(|_| AppError::WidgetNotFound)?;
    let label = format!("w-{uuid}");
    let Some(window) = app.get_webview_window(&label) else {
        return Ok(());
    };
    let raw = window.hwnd()?.0 as isize;
    let _ = app.run_on_main_thread(move || {
        let hwnd = HWND(raw as _);
        if unsafe { IsWindow(Some(hwnd)).as_bool() } {
            backdrop::set_idle_visible(hwnd, visible, duration_ms);
        }
    });
    Ok(())
}

pub fn animate_backdrop_box(
    app: &AppHandle,
    id: &str,
    from_h_u: f64,
    to_h_u: f64,
    anchor: VerticalAnchor,
    duration_ms: u32,
) -> Result<(), AppError> {
    let uuid = Uuid::parse_str(id).map_err(|_| AppError::WidgetNotFound)?;
    let label = format!("w-{uuid}");
    let Some(window) = app.get_webview_window(&label) else {
        return Ok(());
    };
    let raw = window.hwnd()?.0 as isize;
    let _ = app.run_on_main_thread(move || {
        let hwnd = HWND(raw as _);
        if unsafe { IsWindow(Some(hwnd)).as_bool() } {
            backdrop::animate_cluster_box(hwnd, from_h_u, to_h_u, anchor, duration_ms);
        }
    });
    Ok(())
}

pub fn reposition_all(app: AppHandle) {
    let app_for_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        let state = app_for_main.state::<AppState>();
        let config = state.config.get();
        let monitor_entries = state.monitors.read().clone();
        let monitor_infos: Vec<MonitorInfo> =
            monitor_entries.iter().map(|e| e.info.clone()).collect();

        for widget in &config.widgets {
            let label = widget.window_label();
            let Some(window) = app_for_main.get_webview_window(&label) else {
                continue;
            };
            if let Err(error) = apply_window_geometry(&app_for_main, &window, widget, &monitor_infos) {
                log::warn!("Kunne ikke genplacere {label}: {error:?}");
            }
            let id_str = widget.id.to_string();
            if let Some(placement) =
                monitors::compute_widget_placement(&id_str, &window, &monitor_entries)
            {
                let _ = app_for_main.emit_to(&label, "widget://placement", &placement);
                let _ = app_for_main.emit_to("control", "widget://placement", &placement);
            }
        }
    });
}

pub fn apply_edit_mode(app: &AppHandle, on: bool) {
    let state = app.state::<AppState>();
    let config = state.config.get();

    for widget in &config.widgets {
        let label = widget.window_label();
        if let Some(window) = app.get_webview_window(&label) {
            if let Ok(raw_hwnd) = window.hwnd() {
                let hwnd = HWND(raw_hwnd.0 as _);
                sync_widget_click_through(hwnd, widget.click_through, on, &label);
                if let Err(error) = zorder::set_widget_activatable(hwnd, on, false) {
                    log::warn!("Kunne ikke opdatere activatable for {label}: {error}");
                }
            }
        }
    }
}

pub fn set_idle_hidden(app: &AppHandle, id: &str, hidden: bool) -> Result<(), AppError> {
    let uuid = Uuid::parse_str(id).map_err(|_| AppError::WidgetNotFound)?;
    let state = app.state::<AppState>();
    let click_through = state
        .config
        .get()
        .widgets
        .iter()
        .find(|w| w.id == uuid)
        .map(|w| w.click_through)
        .unwrap_or(false);
    let label = format!("w-{uuid}");
    let Some(window) = app.get_webview_window(&label) else {
        return Ok(());
    };
    let hwnd = HWND(window.hwnd()?.0 as _);
    zorder::set_idle_hidden(hwnd, hidden);
    let edit_mode = state.edit_mode.load(Ordering::Acquire);
    sync_widget_click_through(hwnd, click_through, edit_mode, &label);
    Ok(())
}

pub fn set_focusable(app: &AppHandle, id: &str, focusable: bool) -> Result<(), AppError> {
    let uuid = Uuid::parse_str(id).map_err(|_| AppError::WidgetNotFound)?;
    let label = format!("w-{uuid}");
    let window = app
        .get_webview_window(&label)
        .ok_or(AppError::WidgetNotFound)?;
    let hwnd = HWND(window.hwnd()?.0 as _);
    zorder::set_activatable(hwnd, focusable)?;
    if focusable {
        let _ = window.set_focus();
    }
    Ok(())
}

pub fn effective_base_size(app: &AppHandle, instance: &WidgetInstance) -> (f64, f64) {
    if instance.kind == WidgetKind::Cluster {
        if let Some((w, h)) = app.state::<AppState>().cluster_sizes.read().get(&instance.id).copied() {
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

pub fn cluster_set_window_size(
    app: &AppHandle,
    id: Uuid,
    width: f64,
    height: f64,
    anchor: VerticalAnchor,
) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    let instance = state
        .config
        .get()
        .widgets
        .into_iter()
        .find(|w| w.id == id)
        .ok_or(AppError::WidgetNotFound)?;
    if instance.kind != WidgetKind::Cluster {
        return Err(AppError::SizeNotAllowed);
    }

    let label = instance.window_label();
    let window = app
        .get_webview_window(&label)
        .ok_or(AppError::WidgetNotFound)?;

    let monitor_entries = state.monitors.read().clone();
    let monitor_infos: Vec<MonitorInfo> = monitor_entries.iter().map(|e| e.info.clone()).collect();
    if monitor_infos.is_empty() {
        return Ok(());
    }

    let cur_pos = window.outer_position()?;
    let cur_size = window.outer_size()?;
    let cx = cur_pos.x + (cur_size.width as i32) / 2;
    let cy = cur_pos.y + (cur_size.height as i32) / 2;

    let mon = monitor_infos
        .iter()
        .find(|m| {
            cx >= m.rect.x
                && cx < m.rect.x + m.rect.width
                && cy >= m.rect.y
                && cy < m.rect.y + m.rect.height
        })
        .or_else(|| monitors::find_target_monitor(&instance.placement.monitor, &monitor_infos))
        .unwrap_or(&monitor_infos[0]);

    let scale = mon.scale.max(0.5);
    let clamped_w = if width.is_finite() { width.clamp(132.0, 480.0) } else { 264.0 };
    let clamped_h = if height.is_finite() { height.clamp(40.0, 640.0) } else { 48.0 };

    let phys_w = (clamped_w * instance.scale as f64 * scale)
        .round()
        .clamp(1.0, mon.work_area.width.max(1) as f64) as i32;
    let phys_h = (clamped_h * instance.scale as f64 * scale)
        .round()
        .clamp(1.0, mon.work_area.height.max(1) as f64) as i32;

    let max_x = mon.work_area.x + (mon.work_area.width - phys_w).max(0);
    let max_y = mon.work_area.y + (mon.work_area.height - phys_h).max(0);

    let phys_x = cur_pos.x.clamp(mon.work_area.x, max_x);
    let phys_y = if anchor == VerticalAnchor::Bottom {
        let bottom_phys_y = cur_pos.y + cur_size.height as i32;
        (bottom_phys_y - phys_h).clamp(mon.work_area.y, max_y)
    } else {
        cur_pos.y.clamp(mon.work_area.y, max_y)
    };

    let hwnd = HWND(window.hwnd()?.0 as _);
    zorder::set_window_rect(hwnd, phys_x, phys_y, phys_w, phys_h)?;
    if backdrop::is_live(hwnd) {
        backdrop::sync_from_widget(hwnd);
    }

    state.cluster_sizes.write().insert(id, (clamped_w, clamped_h));

    // For anchor-bottom klynger betyder placement.y "top ved idle-højde":
    // bundkanten holdes konstant mens kassen vokser/krymper opad.
    let persist_phys_y = instance
        .cluster
        .as_ref()
        .filter(|c| c.anchor == VerticalAnchor::Bottom)
        .map(|c| {
            let idle_phys = (c.initial_height() * instance.scale as f64 * scale)
                .round()
                .clamp(1.0, mon.work_area.height.max(1) as f64) as i32;
            (phys_y + phys_h - idle_phys).max(mon.work_area.y)
        })
        .unwrap_or(phys_y);

    let log_x = (phys_x - mon.work_area.x) as f64 / scale;
    let log_y = (persist_phys_y - mon.work_area.y) as f64 / scale;
    state.config.update_placement_quiet(id, log_x, log_y);

    let id_str = id.to_string();
    if let Some(placement) = monitors::compute_widget_placement(&id_str, &window, &monitor_entries) {
        let _ = app.emit_to(&label, "widget://placement", &placement);
        let _ = app.emit_to("control", "widget://placement", &placement);
    }

    Ok(())
}

fn apply_window_geometry(
    app: &AppHandle,
    window: &WebviewWindow,
    instance: &WidgetInstance,
    monitors: &[MonitorInfo],
) -> Result<(), AppError> {
    let base_size = effective_base_size(app, instance);
    if let Some(geom) = monitors::resolve_placement_with_size(instance, monitors, base_size) {
        // plan/02 §8 punkt 5: set_position(PhysicalPosition) før set_size(PhysicalSize)
        window.set_position(PhysicalPosition::new(geom.phys_x, geom.phys_y))?;
        window.set_size(PhysicalSize::new(geom.phys_w, geom.phys_h))?;
    } else {
        let (bw, bh) = base_size;
        let lw = (bw * instance.scale as f64).round().max(32.0);
        let lh = (bh * instance.scale as f64).round().max(32.0);
        window.set_size(LogicalSize::new(lw, lh))?;
    }
    Ok(())
}

fn show_without_activation(window: &WebviewWindow) -> Result<(), AppError> {
    let hwnd = HWND(window.hwnd()?.0 as _);
    zorder::show_without_activation(hwnd)?;
    Ok(())
}
