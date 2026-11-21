use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio::time::{interval, MissedTickBehavior};
use ts_rs::TS;

use crate::{
    config::schema::{MonitorKey, Placement, RectF, RectI, SizeF, VerticalAnchor, WidgetInstance},
    state::AppState,
};

use super::{plate::LumaGrid, wallpaper, zorder};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct MonitorInfo {
    pub name: String,
    pub rect: RectI,
    pub work_area: RectI,
    pub scale: f64,
    pub logical_size: SizeF,
    pub plate_path: String,
    pub avg_luma: f32,
    pub is_primary: bool,
}

#[derive(Debug, Clone)]
pub struct MonitorEntry {
    pub info: MonitorInfo,
    pub luma: LumaGrid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TauriMonitorSnapshot {
    pub name: String,
    pub rect: RectI,
    pub work_area: RectI,
    pub scale: f64,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WidgetPlacement {
    pub id: String,
    pub monitor: String,
    pub rect: RectF,
    pub luma: f32,
}

pub fn list_tauri_monitors(app: &AppHandle) -> Vec<TauriMonitorSnapshot> {
    let primary_name = app
        .primary_monitor()
        .ok()
        .flatten()
        .and_then(|m| m.name().map(|n| n.to_string()));
    let primary_pos = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| (m.position().x, m.position().y));

    let Ok(monitors) = app.available_monitors() else {
        return Vec::new();
    };

    monitors
        .into_iter()
        .enumerate()
        .map(|(index, m)| {
            let pos = m.position();
            let size = m.size();
            let work = m.work_area();
            let name = m
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("DISPLAY-{}", index + 1));
            let is_primary = primary_name
                .as_deref()
                .map(|p| p == name)
                .unwrap_or_else(|| primary_pos == Some((pos.x, pos.y)));
            TauriMonitorSnapshot {
                name,
                rect: RectI {
                    x: pos.x,
                    y: pos.y,
                    width: size.width as i32,
                    height: size.height as i32,
                },
                work_area: RectI {
                    x: work.position.x,
                    y: work.position.y,
                    width: work.size.width as i32,
                    height: work.size.height as i32,
                },
                scale: m.scale_factor(),
                is_primary,
            }
        })
        .collect()
}

pub fn compute_widget_placement(
    id: &str,
    window: &WebviewWindow,
    monitors: &[MonitorEntry],
) -> Option<WidgetPlacement> {
    if monitors.is_empty() {
        return None;
    }
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let wx = pos.x;
    let wy = pos.y;
    let ww = size.width as i32;
    let wh = size.height as i32;
    let center_x = wx + ww / 2;
    let center_y = wy + wh / 2;

    let entry = monitors
        .iter()
        .find(|entry| rect_contains(entry.info.rect, center_x, center_y))
        .or_else(|| {
            monitors
                .iter()
                .max_by_key(|entry| overlap_area(entry.info.rect, wx, wy, ww, wh))
        })
        .or_else(|| monitors.iter().find(|entry| entry.info.is_primary))
        .unwrap_or(&monitors[0]);

    let scale = entry.info.scale.max(0.5);
    let rect = RectF {
        x: (wx - entry.info.rect.x) as f64 / scale,
        y: (wy - entry.info.rect.y) as f64 / scale,
        width: ww as f64 / scale,
        height: wh as f64 / scale,
    };
    let luma = entry.luma.average_for_rect(
        entry.info.logical_size.width,
        entry.info.logical_size.height,
        rect,
    );

    Some(WidgetPlacement {
        id: id.to_string(),
        monitor: entry.info.name.clone(),
        rect,
        luma,
    })
}

pub fn emit_all_placements(app: &AppHandle, monitors: &[MonitorEntry]) {
    for (widget_id, label) in zorder::widget_labels() {
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        if let Some(placement) = compute_widget_placement(&widget_id, &window, monitors) {
            if let Err(error) = app.emit_to(&label, "widget://placement", &placement) {
                log::warn!("Kunne ikke sende widget://placement til {label}: {error}");
            }
            let _ = app.emit_to("control", "widget://placement", &placement);
        }
    }
}

pub fn start_poll(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_snapshot = list_tauri_monitors(&app);
        let mut poll = interval(Duration::from_secs(5));
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            poll.tick().await;
            let current = list_tauri_monitors(&app);
            if current != last_snapshot {
                last_snapshot = current;
                log::info!("Skærmopsætning ændret; genkomponerer tapetplader");
                wallpaper::trigger_refresh_now();
                let monitors = app.state::<AppState>().monitors.read().clone();
                emit_all_placements(&app, &monitors);
                crate::wallpaper::notify_monitors_changed(&app);
            }
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWindowGeometry {
    pub monitor_name: String,
    pub phys_x: i32,
    pub phys_y: i32,
    pub phys_w: u32,
    pub phys_h: u32,
}

pub fn find_target_monitor<'a>(
    key: &MonitorKey,
    monitors: &'a [MonitorInfo],
) -> Option<&'a MonitorInfo> {
    monitors
        .iter()
        .find(|m| {
            m.name == key.name && m.rect.width == key.rect.width && m.rect.height == key.rect.height
        })
        .or_else(|| monitors.iter().find(|m| m.name == key.name))
        .or_else(|| monitors.iter().find(|m| m.rect == key.rect))
        .or_else(|| monitors.iter().find(|m| m.is_primary))
        .or_else(|| monitors.first())
}

pub fn resolve_placement(
    instance: &WidgetInstance,
    monitors: &[MonitorInfo],
) -> Option<ResolvedWindowGeometry> {
    resolve_placement_with_size(instance, monitors, instance.base_size())
}

pub fn resolve_placement_with_size(
    instance: &WidgetInstance,
    monitors: &[MonitorInfo],
    base_size: (f64, f64),
) -> Option<ResolvedWindowGeometry> {
    let mon = find_target_monitor(&instance.placement.monitor, monitors)?;
    let scale = mon.scale.max(0.5);
    let (base_w, base_h) = base_size;

    let phys_w = (base_w * instance.scale as f64 * scale)
        .round()
        .clamp(1.0, mon.work_area.width.max(1) as f64) as i32;
    let phys_h = (base_h * instance.scale as f64 * scale)
        .round()
        .clamp(1.0, mon.work_area.height.max(1) as f64) as i32;

    let phys_x = mon.work_area.x + (instance.placement.x * scale).round() as i32;
    // For anchor-bottom klynger er placement.y "top ved idle-højde"; den
    // nuværende kasses top forskydes så bundkanten holdes konstant.
    let anchor_offset = instance
        .cluster
        .as_ref()
        .filter(|c| c.anchor == VerticalAnchor::Bottom)
        .map(|c| {
            (c.initial_height() * instance.scale as f64 * scale).round() as i32 - phys_h
        })
        .unwrap_or(0);
    let phys_y = mon.work_area.y + (instance.placement.y * scale).round() as i32 + anchor_offset;

    let max_x = mon.work_area.x + (mon.work_area.width - phys_w).max(0);
    let max_y = mon.work_area.y + (mon.work_area.height - phys_h).max(0);

    Some(ResolvedWindowGeometry {
        monitor_name: mon.name.clone(),
        phys_x: phys_x.clamp(mon.work_area.x, max_x),
        phys_y: phys_y.clamp(mon.work_area.y, max_y),
        phys_w: phys_w as u32,
        phys_h: phys_h as u32,
    })
}

pub fn first_free_slot(
    monitors: &[MonitorInfo],
    target_monitor: Option<&str>,
    existing: &[WidgetInstance],
    base_size: (f64, f64),
) -> Placement {
    let Some(mon) = target_monitor
        .and_then(|name| monitors.iter().find(|m| m.name == name))
        .or_else(|| monitors.iter().find(|m| m.is_primary))
        .or_else(|| monitors.first())
    else {
        return Placement {
            monitor: MonitorKey {
                name: r"\\.\DISPLAY1".into(),
                rect: RectI {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
            },
            x: 20.0,
            y: 20.0,
        };
    };

    let scale = mon.scale.max(0.5);
    let work_w = mon.work_area.width as f64 / scale;
    let work_h = mon.work_area.height as f64 / scale;
    let (bw, bh) = base_size;

    let occupied: Vec<RectF> = existing
        .iter()
        .filter_map(|w| {
            let geom = resolve_placement(w, monitors)?;
            if geom.monitor_name != mon.name {
                return None;
            }
            Some(RectF {
                x: (geom.phys_x - mon.work_area.x) as f64 / scale,
                y: (geom.phys_y - mon.work_area.y) as f64 / scale,
                width: geom.phys_w as f64 / scale,
                height: geom.phys_h as f64 / scale,
            })
        })
        .collect();

    const MARGIN: f64 = 20.0;
    const GAP: f64 = 12.0;
    const COL_STEP: f64 = 144.0;
    const ROW_STEP: f64 = 12.0;

    let start_x = (work_w - MARGIN - bw).max(0.0);
    let max_y = (work_h - MARGIN - bh).max(MARGIN);

    let mut col = 0;
    loop {
        let x = start_x - (col as f64) * COL_STEP;
        if col > 0 && x < MARGIN {
            break;
        }
        let mut row = 0;
        loop {
            let y = MARGIN + (row as f64) * ROW_STEP;
            if y > max_y && row > 0 {
                break;
            }
            if y + bh <= work_h && x + bw <= work_w {
                let collides = occupied.iter().any(|other| {
                    !(x + bw + GAP <= other.x
                        || other.x + other.width + GAP <= x
                        || y + bh + GAP <= other.y
                        || other.y + other.height + GAP <= y)
                });
                if !collides {
                    return Placement {
                        monitor: MonitorKey {
                            name: mon.name.clone(),
                            rect: mon.rect,
                        },
                        x,
                        y,
                    };
                }
            }
            if y >= max_y {
                break;
            }
            row += 1;
        }
        if x <= MARGIN {
            break;
        }
        col += 1;
    }

    Placement {
        monitor: MonitorKey {
            name: mon.name.clone(),
            rect: mon.rect,
        },
        x: ((work_w - bw) / 2.0).clamp(0.0, (work_w - bw).max(0.0)),
        y: ((work_h - bh) / 2.0).clamp(0.0, (work_h - bh).max(0.0)),
    }
}

fn rect_contains(rect: RectI, x: i32, y: i32) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::defaults;
    use crate::config::schema::WidgetKind;

    fn mon(name: &str, x: i32, w: i32, h: i32, scale: f64, is_primary: bool) -> MonitorInfo {
        MonitorInfo {
            name: name.into(),
            rect: RectI {
                x,
                y: 0,
                width: w,
                height: h,
            },
            work_area: RectI {
                x,
                y: 0,
                width: w,
                height: h - 48,
            },
            scale,
            logical_size: SizeF {
                width: w as f64 / scale,
                height: h as f64 / scale,
            },
            plate_path: String::new(),
            avg_luma: 0.15,
            is_primary,
        }
    }

    #[test]
    fn resolve_placement_clamps_on_disconnect_and_restores_with_dpi() {
        let m1 = mon(r"\\.\DISPLAY1", 0, 1920, 1080, 1.0, true);
        let m2 = mon(r"\\.\DISPLAY2", 1920, 2880, 1800, 1.5, false);

        let mut widget = defaults::default_widget(WidgetKind::Clock, Some(&m2));
        widget.placement.x = 1200.0;
        widget.placement.y = 100.0;

        // Connected with 150% DPI on Monitor 2
        let on_m2 = resolve_placement(&widget, &[m1.clone(), m2.clone()]).unwrap();
        assert_eq!(on_m2.monitor_name, m2.name);
        assert_eq!(on_m2.phys_x, 1920 + 1800);
        assert_eq!(on_m2.phys_y, 150);
        assert_eq!(on_m2.phys_w, (276.0 * 1.5_f64).round() as u32);
        assert_eq!(on_m2.phys_h, (132.0 * 1.5_f64).round() as u32);

        // Disconnected Monitor 2 -> clamped onto Monitor 1, saved placement untouched
        let disconnected = resolve_placement(&widget, &[m1.clone()]).unwrap();
        assert_eq!(disconnected.monitor_name, m1.name);
        assert!(disconnected.phys_x + disconnected.phys_w as i32 <= m1.work_area.width);
        assert_eq!(widget.placement.monitor.name, m2.name);
        assert_eq!(widget.placement.x, 1200.0);

        // Reconnected Monitor 2 -> jumps straight back
        let restored = resolve_placement(&widget, &[m1, m2]).unwrap();
        assert_eq!(restored, on_m2);
    }

    #[test]
    fn first_free_slot_avoids_overlaps() {
        let m1 = mon(r"\\.\DISPLAY1", 0, 1920, 1080, 1.0, true);
        let monitors = vec![m1.clone()];
        let mut placed: Vec<WidgetInstance> = Vec::new();

        for _ in 0..4 {
            let slot = first_free_slot(&monitors, None, &placed, (276.0, 132.0));
            for prev in &placed {
                let dx = (slot.x - prev.placement.x).abs();
                let dy = (slot.y - prev.placement.y).abs();
                assert!(dx >= 288.0 || dy >= 144.0);
            }
            let mut inst = defaults::default_widget(WidgetKind::Clock, Some(&m1));
            inst.placement = slot;
            placed.push(inst);
        }
    }
}
