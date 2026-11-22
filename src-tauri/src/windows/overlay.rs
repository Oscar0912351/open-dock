use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            SetWindowPos, WINDOWPOS, HWND_BOTTOM, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, WM_MOUSEACTIVATE, WM_NCDESTROY,
            WM_WINDOWPOSCHANGING,
        },
    },
};

use crate::{
    config::schema::RectI,
    desktop::{
        monitors,
        snap::{EditGuides, Guide},
        styles,
    },
    error::AppError,
    state::AppState,
};

const GRID_SUBCLASS_ID: usize = 0xD61A57;
const BAR_LOGICAL_WIDTH: f64 = 560.0;
const BAR_LOGICAL_HEIGHT: f64 = 40.0;
const BAR_TOP_OFFSET: f64 = 12.0;

unsafe extern "system" fn grid_overlay_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_WINDOWPOSCHANGING if lp.0 != 0 => {
            let pos = &mut *(lp.0 as *mut WINDOWPOS);
            if !pos.flags.contains(SWP_NOZORDER) {
                pos.hwndInsertAfter = HWND_BOTTOM;
            }
        }
        WM_MOUSEACTIVATE => {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(hwnd, Some(grid_overlay_proc), id);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

#[derive(Clone)]
struct OverlayMonitorRect {
    name: String,
    work_area: RectI,
    scale: f64,
    is_primary: bool,
}

fn collect_monitor_rects(app: &AppHandle) -> Vec<OverlayMonitorRect> {
    let entries = app.state::<AppState>().monitors.read().clone();
    if !entries.is_empty() {
        return entries
            .into_iter()
            .map(|e| OverlayMonitorRect {
                name: e.info.name,
                work_area: e.info.work_area,
                scale: e.info.scale.max(0.5),
                is_primary: e.info.is_primary,
            })
            .collect();
    }

    monitors::list_tauri_monitors(app)
        .into_iter()
        .map(|m| OverlayMonitorRect {
            name: m.name,
            work_area: m.work_area,
            scale: m.scale.max(0.5),
            is_primary: m.is_primary,
        })
        .collect()
}

pub fn open_all(app: &AppHandle) -> Result<(), AppError> {
    close_all(app);

    let monitors = collect_monitor_rects(app);
    if monitors.is_empty() {
        return Ok(());
    }

    for (index, mon) in monitors.iter().enumerate() {
        let label = format!("overlay-{index}");
        let encoded_name = encode_query_component(&mon.name);
        let url = WebviewUrl::App(format!("overlay.html?mode=grid&monitor={encoded_name}").into());

        let log_x = mon.work_area.x as f64 / mon.scale;
        let log_y = mon.work_area.y as f64 / mon.scale;
        let log_w = (mon.work_area.width as f64 / mon.scale).max(64.0);
        let log_h = (mon.work_area.height as f64 / mon.scale).max(64.0);

        let window = WebviewWindowBuilder::new(app, &label, url)
            .title("Open Dock Overlay")
            .position(log_x, log_y)
            .inner_size(log_w, log_h)
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

        window.set_position(PhysicalPosition::new(mon.work_area.x, mon.work_area.y))?;
        window.set_size(PhysicalSize::new(
            mon.work_area.width.max(1) as u32,
            mon.work_area.height.max(1) as u32,
        ))?;
        window.set_ignore_cursor_events(false)?;

        let raw_hwnd = window.hwnd()?.0 as isize;
        let wx = mon.work_area.x;
        let wy = mon.work_area.y;
        let ww = mon.work_area.width.max(1);
        let wh = mon.work_area.height.max(1);
        crate::desktop::run_on_ui_thread_sync(app, move || -> Result<(), AppError> {
            let hwnd = HWND(raw_hwnd as _);
            unsafe {
                styles::apply_widget_styles(hwnd)?;
                SetWindowSubclass(hwnd, Some(grid_overlay_proc), GRID_SUBCLASS_ID, 0).ok()?;
                SetWindowPos(
                    hwnd,
                    Some(HWND_BOTTOM),
                    wx,
                    wy,
                    ww,
                    wh,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )?;
            }
            Ok(())
        })??;
    }

    let primary = monitors
        .iter()
        .find(|m| m.is_primary)
        .unwrap_or(&monitors[0]);
    let phys_w = (BAR_LOGICAL_WIDTH * primary.scale)
        .round()
        .clamp(200.0, primary.work_area.width.max(200) as f64) as i32;
    let phys_h = (BAR_LOGICAL_HEIGHT * primary.scale).round().max(32.0) as i32;
    let phys_x = primary.work_area.x + (primary.work_area.width - phys_w) / 2;
    let phys_y = primary.work_area.y + (BAR_TOP_OFFSET * primary.scale).round() as i32;

    let bar_window =
        WebviewWindowBuilder::new(app, "overlay-bar", WebviewUrl::App("overlay.html?mode=bar".into()))
            .title("Open Dock Redigering")
            .inner_size(BAR_LOGICAL_WIDTH, BAR_LOGICAL_HEIGHT)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .skip_taskbar(true)
            .focused(false)
            .visible(false)
            .always_on_top(true)
            .drag_and_drop(false)
            .background_color(tauri::window::Color(0, 0, 0, 0))
            .build()?;

    bar_window.set_position(PhysicalPosition::new(phys_x, phys_y))?;
    bar_window.set_size(PhysicalSize::new(phys_w as u32, phys_h as u32))?;
    bar_window.set_ignore_cursor_events(false)?;

    let raw_bar_hwnd = bar_window.hwnd()?.0 as isize;
    crate::desktop::run_on_ui_thread_sync(app, move || -> Result<(), AppError> {
        let bar_hwnd = HWND(raw_bar_hwnd as _);
        unsafe {
            styles::apply_widget_styles(bar_hwnd)?;
            SetWindowPos(
                bar_hwnd,
                Some(HWND_TOPMOST),
                phys_x,
                phys_y,
                phys_w,
                phys_h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
        }
        Ok(())
    })??;

    Ok(())
}

pub fn close_all(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("overlay-") {
            if let Err(error) = window.close() {
                log::warn!("Kunne ikke lukke overlay-vindue {label}: {error}");
            }
        }
    }
}

pub fn send_grids_to_bottom(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("overlay-") && label != "overlay-bar" {
            if let Ok(raw_hwnd) = window.hwnd() {
                let hwnd = HWND(raw_hwnd.0 as _);
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        Some(HWND_BOTTOM),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }
}

pub fn emit_guides(app: &AppHandle, monitor: &str, lines: Vec<Guide>) {
    let payload = EditGuides {
        monitor: monitor.to_string(),
        lines,
    };
    if let Err(error) = app.emit("edit://guides", &payload) {
        log::warn!("Kunne ikke sende edit://guides: {error}");
    }
}

pub fn clear_guides(app: &AppHandle) {
    emit_guides(app, "", Vec::new());
}

fn encode_query_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
