use std::{
    sync::atomic::Ordering,
    time::Duration,
};

use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;
use windows::Win32::{
    Foundation::{HWND, POINT},
    UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON, VK_MENU},
        WindowsAndMessaging::{
            BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, GetCursorPos, SetWindowPos,
            SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
        },
    },
};

use crate::{
    config::schema::{MonitorKey, Placement, PointF, RectF, RectI, VerticalAnchor},
    error::AppError,
    state::{AppState, EditDragging, EditModeState, EditSelected},
    windows::{overlay, widget},
};

use super::{
    monitors::{self, MonitorEntry, MonitorInfo},
    snap::{self, SnapCtx},
    zorder,
};

struct DragOtherItem {
    monitors: Vec<(String, RectF)>,
}

pub fn set_edit_mode(app: &AppHandle, on: bool) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    state.edit_mode.store(on, Ordering::Release);

    if !on {
        *state.selected_widget.write() = None;
        let _ = app.emit("edit://selected", EditSelected { id: None });
        let _ = app.emit("edit://dragging", EditDragging { id: None });
        overlay::clear_guides(app);
        overlay::close_all(app);
    }

    widget::apply_edit_mode(app, on);

    if on {
        overlay::open_all(app)?;
    }

    if let Err(error) = app.emit("edit://mode", EditModeState { on }) {
        log::warn!("Kunne ikke sende edit://mode: {error}");
    }
    super::fullscreen::trigger_check();
    crate::tray::sync(app);

    Ok(())
}

pub fn enter_and_begin_drag(app: &AppHandle, id: Uuid) -> Result<(), AppError> {
    if !is_vk_down(VK_LBUTTON.0 as i32) {
        return Ok(());
    }
    let state = app.state::<AppState>();
    if !state.edit_mode.load(Ordering::Acquire) {
        set_edit_mode(app, true)?;
    }
    begin_drag(app, id)
}

pub fn nudge(app: &AppHandle, id: Uuid, dx: f64, dy: f64) -> Result<(), AppError> {
    if !dx.is_finite() || !dy.is_finite() {
        return Ok(());
    }

    let state = app.state::<AppState>();
    let monitor_infos: Vec<MonitorInfo> = state
        .monitors
        .read()
        .iter()
        .map(|entry| entry.info.clone())
        .collect();

    let updated = state.config.mutate(|cfg| {
        let Some(target) = cfg.widgets.iter_mut().find(|w| w.id == id) else {
            return Err(AppError::WidgetNotFound);
        };

        let base_size = widget::effective_base_size(app, target);
        if let Some(mon) = monitors::find_target_monitor(&target.placement.monitor, &monitor_infos)
        {
            let scale = mon.scale.max(0.5);
            let work_w = mon.work_area.width as f64 / scale;
            let work_h = mon.work_area.height as f64 / scale;
            let w = base_size.0 * target.scale as f64;
            let h = base_size.1 * target.scale as f64;
            target.placement = Placement {
                monitor: MonitorKey {
                    name: mon.name.clone(),
                    rect: mon.rect,
                },
                x: (target.placement.x + dx).clamp(0.0, (work_w - w).max(0.0)),
                y: (target.placement.y + dy).clamp(0.0, (work_h - h).max(0.0)),
            };
        } else {
            target.placement.x += dx;
            target.placement.y += dy;
        }
        Ok(target.clone())
    })?;

    widget::apply_instance_live(app, &updated);
    Ok(())
}

pub fn begin_drag(app: &AppHandle, id: Uuid) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    if !state.edit_mode.load(Ordering::Acquire) {
        return Ok(());
    }

    let config = state.config.get();
    let instance = config
        .widgets
        .iter()
        .find(|w| w.id == id)
        .cloned()
        .ok_or(AppError::WidgetNotFound)?;

    {
        let mut selected = state.selected_widget.write();
        if *selected != Some(id) {
            *selected = Some(id);
            let _ = app.emit("edit://selected", EditSelected { id: Some(id) });
        }
    }

    if !is_vk_down(VK_LBUTTON.0 as i32) {
        return Ok(());
    }

    if state.dragging.swap(true, Ordering::AcqRel) {
        return Ok(());
    }

    let label = instance.window_label();
    let Some(window) = app.get_webview_window(&label) else {
        state.dragging.store(false, Ordering::Release);
        return Err(AppError::WidgetNotFound);
    };

    let setup_res = (|| -> Result<(isize, i32, i32, u32, u32, POINT), AppError> {
        let hwnd = window.hwnd()?.0 as isize;
        let orig_pos = window.outer_position()?;
        let orig_size = window.outer_size()?;
        let mut start_cursor = POINT::default();
        unsafe {
            GetCursorPos(&mut start_cursor)?;
        }
        Ok((
            hwnd,
            orig_pos.x,
            orig_pos.y,
            orig_size.width,
            orig_size.height,
            start_cursor,
        ))
    })();

    let (hwnd_raw, orig_x, orig_y, orig_w, orig_h, start_cursor) = match setup_res {
        Ok(vals) => vals,
        Err(err) => {
            state.dragging.store(false, Ordering::Release);
            return Err(err);
        }
    };

    let monitor_entries = state.monitors.read().clone();
    if monitor_entries.is_empty() {
        state.dragging.store(false, Ordering::Release);
        return Ok(());
    }
    let monitor_infos: Vec<MonitorInfo> = monitor_entries
        .iter()
        .map(|entry| entry.info.clone())
        .collect();

    let base_size = widget::effective_base_size(app, &instance);
    let target_log_w = base_size.0 * instance.scale as f64;
    let target_log_h = base_size.1 * instance.scale as f64;

    // Forudberegn logiske rektangler (relativt til hver skærms work area) for alle andre widgets
    let others_pool: Vec<DragOtherItem> = config
        .widgets
        .iter()
        .filter(|w| w.id != id)
        .map(|other| {
            let obase = widget::effective_base_size(app, other);
            let mut per_mon = Vec::new();
            if let Some(geom) = monitors::resolve_placement_with_size(other, &monitor_infos, obase) {
                if let Some(m) = monitor_infos.iter().find(|m| m.name == geom.monitor_name) {
                    let s = m.scale.max(0.5);
                    per_mon.push((
                        m.name.clone(),
                        RectF {
                            x: (geom.phys_x - m.work_area.x) as f64 / s,
                            y: (geom.phys_y - m.work_area.y) as f64 / s,
                            width: geom.phys_w as f64 / s,
                            height: geom.phys_h as f64 / s,
                        },
                    ));
                }
            }
            DragOtherItem { monitors: per_mon }
        })
        .collect();

    let app_handle = app.clone();
    let spawn_res = std::thread::Builder::new()
        .name("od-drag".into())
        .spawn(move || {
            run_drag_loop(DragSession {
                app: app_handle,
                id,
                id_str: id.to_string(),
                label,
                hwnd: HWND(hwnd_raw as _),
                orig_x,
                orig_y,
                orig_w,
                orig_h,
                offset_x: start_cursor.x - orig_x,
                offset_y: start_cursor.y - orig_y,
                start_cursor,
                target_log_w,
                target_log_h,
                monitor_entries,
                others_pool,
            });
        });

    if let Err(error) = spawn_res {
        state.dragging.store(false, Ordering::Release);
        return Err(AppError::Other(format!(
            "Kunne ikke starte træk-tråd: {error}"
        )));
    }

    Ok(())
}

struct DragSession {
    app: AppHandle,
    id: Uuid,
    id_str: String,
    label: String,
    hwnd: HWND,
    orig_x: i32,
    orig_y: i32,
    orig_w: u32,
    orig_h: u32,
    offset_x: i32,
    offset_y: i32,
    start_cursor: POINT,
    target_log_w: f64,
    target_log_h: f64,
    monitor_entries: Vec<MonitorEntry>,
    others_pool: Vec<DragOtherItem>,
}

fn run_drag_loop(session: DragSession) {
    let state = session.app.state::<AppState>();
    let Some(window) = session.app.get_webview_window(&session.label) else {
        state.dragging.store(false, Ordering::Release);
        return;
    };

    let mut has_moved = false;
    let mut tick: u64 = 0;
    let mut prev_cursor = session.start_cursor;
    let mut prev_alt = is_vk_down(VK_MENU.0 as i32);
    let mut prev_phys = (session.orig_x, session.orig_y);
    let mut last_monitor_idx = find_monitor_idx(
        &session.monitor_entries,
        session.orig_x + (session.orig_w as i32) / 2,
        session.orig_y + (session.orig_h as i32) / 2,
        session.orig_w as i32,
        session.orig_h as i32,
    );
    let mut last_log_pt = PointF {
        x: 0.0,
        y: 0.0,
    };
    let mut last_log_h = 0.0;

    loop {
        std::thread::sleep(Duration::from_millis(8));

        if is_vk_down(VK_ESCAPE.0 as i32) {
            let _ = zorder::set_window_rect(
                session.hwnd,
                session.orig_x,
                session.orig_y,
                session.orig_w as i32,
                session.orig_h as i32,
            );
            if let Some(placement) = monitors::compute_widget_placement(
                &session.id_str,
                &window,
                &session.monitor_entries,
            ) {
                let _ = session
                    .app
                    .emit_to(&session.label, "widget://placement", &placement);
                let _ = session
                    .app
                    .emit_to("control", "widget://placement", &placement);
            }
            overlay::clear_guides(&session.app);
            let _ = session
                .app
                .emit("edit://dragging", EditDragging { id: None });
            state.dragging.store(false, Ordering::Release);
            return;
        }

        if !is_vk_down(VK_LBUTTON.0 as i32) || !state.edit_mode.load(Ordering::Acquire) {
            break;
        }

        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) }.is_err() {
            continue;
        }

        if !has_moved {
            if (cursor.x - session.start_cursor.x).abs() < 3
                && (cursor.y - session.start_cursor.y).abs() < 3
            {
                continue;
            }
            has_moved = true;
            let _ = session
                .app
                .emit("edit://dragging", EditDragging { id: Some(session.id) });
        }

        let alt_down = is_vk_down(VK_MENU.0 as i32);
        if cursor.x == prev_cursor.x && cursor.y == prev_cursor.y && alt_down == prev_alt {
            continue;
        }
        prev_cursor = cursor;
        prev_alt = alt_down;

        let cur_size = window.outer_size().ok();
        let cur_w = cur_size.map(|s| s.width as i32).unwrap_or(session.orig_w as i32);
        let cur_h = cur_size
            .map(|s| s.height as i32)
            .unwrap_or(session.orig_h as i32);

        let cand_phys_x = cursor.x - session.offset_x;
        let cand_phys_y = cursor.y - session.offset_y;
        let mon_idx = find_monitor_idx(
            &session.monitor_entries,
            cand_phys_x + cur_w / 2,
            cand_phys_y + cur_h / 2,
            cur_w,
            cur_h,
        );
        last_monitor_idx = mon_idx;
        let mon = &session.monitor_entries[mon_idx].info;
        let scale = mon.scale.max(0.5);

        let work_log = RectF {
            x: 0.0,
            y: 0.0,
            width: mon.work_area.width as f64 / scale,
            height: mon.work_area.height as f64 / scale,
        };
        let cur_log_w = cur_w as f64 / scale;
        let cur_log_h = cur_h as f64 / scale;
        let cand_rect = RectF {
            x: (cand_phys_x - mon.work_area.x) as f64 / scale,
            y: (cand_phys_y - mon.work_area.y) as f64 / scale,
            width: cur_log_w,
            height: cur_log_h,
        };

        let others: Vec<RectF> = session
            .others_pool
            .iter()
            .filter_map(|item| {
                item.monitors
                    .iter()
                    .find(|(name, _)| *name == mon.name)
                    .map(|(_, r)| *r)
            })
            .collect();

        let ctx = SnapCtx::standard(work_log, others, true, !alt_down);
        let (snapped_pt, guides) = snap::resolve(cand_rect, &ctx);
        last_log_pt = snapped_pt;
        last_log_h = cur_log_h;

        let dest_phys_x = mon.work_area.x + (snapped_pt.x * scale).round() as i32;
        let dest_phys_y = mon.work_area.y + (snapped_pt.y * scale).round() as i32;

        if (dest_phys_x, dest_phys_y) != prev_phys {
            let mut moved = false;
            if let Some(backdrop_hwnd) = super::backdrop::backdrop_hwnd_for(session.hwnd) {
                let res = unsafe {
                    (|| -> windows::core::Result<()> {
                        let hdwp = BeginDeferWindowPos(2)?;
                        let hdwp = DeferWindowPos(
                            hdwp,
                            session.hwnd,
                            None,
                            dest_phys_x,
                            dest_phys_y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        )?;
                        let hdwp = DeferWindowPos(
                            hdwp,
                            backdrop_hwnd,
                            Some(session.hwnd),
                            dest_phys_x,
                            dest_phys_y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOACTIVATE,
                        )?;
                        EndDeferWindowPos(hdwp)
                    })()
                };
                moved = res.is_ok();
            }
            if !moved {
                unsafe {
                    let _ = SetWindowPos(
                        session.hwnd,
                        None,
                        dest_phys_x,
                        dest_phys_y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            prev_phys = (dest_phys_x, dest_phys_y);
        }

        tick += 1;
        if tick.is_multiple_of(2) {
            overlay::emit_guides(&session.app, &mon.name, guides);
            if let Some(placement) = monitors::compute_widget_placement(
                &session.id_str,
                &window,
                &session.monitor_entries,
            ) {
                let _ = session
                    .app
                    .emit_to(&session.label, "widget://placement", &placement);
                let _ = session
                    .app
                    .emit_to("control", "widget://placement", &placement);
            }
        }
    }

    overlay::clear_guides(&session.app);
    let _ = session
        .app
        .emit("edit://dragging", EditDragging { id: None });

    if has_moved {
        let mon = &session.monitor_entries[last_monitor_idx].info;
        let scale = mon.scale.max(0.5);
        let work_w = mon.work_area.width as f64 / scale;
        let work_h = mon.work_area.height as f64 / scale;

        let final_x = last_log_pt
            .x
            .clamp(0.0, (work_w - session.target_log_w).max(0.0));
        let final_y = last_log_pt
            .y
            .clamp(0.0, (work_h - session.target_log_h).max(0.0));

        let mut new_placement = Placement {
            monitor: MonitorKey {
                name: mon.name.clone(),
                rect: mon.rect,
            },
            x: final_x,
            y: final_y,
        };

        let result = state.config.mutate(|cfg| {
            let Some(target) = cfg.widgets.iter_mut().find(|w| w.id == session.id) else {
                return Err(AppError::WidgetNotFound);
            };
            // Anchor-bottom klynger: omsæt den aktuelle top til "top ved
            // idle-højde" så bundkanten genfindes efter genstart.
            if let Some(cluster) = target.cluster.as_ref() {
                if cluster.anchor == VerticalAnchor::Bottom && last_log_h > 0.0 {
                    let idle_log_h = cluster.initial_height() * target.scale as f64;
                    new_placement.y = (last_log_pt.y + last_log_h - idle_log_h)
                        .clamp(0.0, (work_h - idle_log_h).max(0.0));
                }
            }
            target.placement = new_placement;
            Ok(target.clone())
        });

        match result {
            Ok(updated) => widget::apply_instance_live(&session.app, &updated),
            Err(error) => log::warn!("Kunne ikke gemme placering efter træk: {error:?}"),
        }
    }

    state.dragging.store(false, Ordering::Release);
}

fn find_monitor_idx(
    entries: &[MonitorEntry],
    center_x: i32,
    center_y: i32,
    w: i32,
    h: i32,
) -> usize {
    if let Some(idx) = entries.iter().position(|e| {
        let r = e.info.rect;
        center_x >= r.x && center_x < r.x + r.width && center_y >= r.y && center_y < r.y + r.height
    }) {
        return idx;
    }

    let left = center_x - w / 2;
    let top = center_y - h / 2;
    if let Some((idx, _)) = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (i, overlap_area(e.info.rect, left, top, w, h)))
        .filter(|&(_, area)| area > 0)
        .max_by_key(|&(_, area)| area)
    {
        return idx;
    }

    entries
        .iter()
        .position(|e| e.info.is_primary)
        .unwrap_or(0)
}

fn overlap_area(rect: RectI, x: i32, y: i32, w: i32, h: i32) -> i64 {
    let ix0 = rect.x.max(x);
    let iy0 = rect.y.max(y);
    let ix1 = (rect.x + rect.width).min(x + w);
    let iy1 = (rect.y + rect.height).min(y + h);
    let iw = (ix1 - ix0).max(0) as i64;
    let ih = (iy1 - iy0).max(0) as i64;
    iw * ih
}

fn is_vk_down(vk: i32) -> bool {
    let state = unsafe { GetAsyncKeyState(vk) } as u16;
    (state & 0x8000) != 0
}
