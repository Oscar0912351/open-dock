use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, LazyLock,
    },
};

use parking_lot::Mutex;
use tauri::Manager;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            SetForegroundWindow, SetWindowPos, WINDOWPOS, HWND_BOTTOM, HWND_NOTOPMOST,
            HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SWP_NOZORDER,
            SWP_SHOWWINDOW, WM_MOUSEACTIVATE, WM_NCDESTROY, WM_WINDOWPOSCHANGING,
        },
    },
};

use super::styles;
pub use crate::config::schema::Layer;

const SUBCLASS_ID: usize = 0xD61A55;
static SHOW_DESKTOP: AtomicBool = AtomicBool::new(false);
static GAME_OVERLAY: AtomicBool = AtomicBool::new(false);
static REGISTRY: LazyLock<Mutex<HashMap<isize, Arc<HwndState>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct HwndState {
    id: String,
    label: String,
    layer: AtomicU8,
    activatable: AtomicBool,
}

impl HwndState {
    fn new(id: String, label: String, layer: Layer) -> Self {
        Self {
            id,
            label,
            layer: AtomicU8::new(layer as u8),
            activatable: AtomicBool::new(false),
        }
    }

    fn layer(&self) -> Layer {
        Layer::from_u8(self.layer.load(Ordering::Acquire))
    }
}

/// # Safety
///
/// `hwnd` skal være et gyldigt Win32-vindueshandle oprettet på UI-tråden og ikke allerede subclasset med `SUBCLASS_ID`.
pub unsafe fn attach(
    hwnd: HWND,
    layer: Layer,
    id: String,
    label: String,
) -> windows::core::Result<()> {
    let state = Arc::new(HwndState::new(id, label, layer));
    let callback_state = Arc::into_raw(state.clone());
    if let Err(error) = SetWindowSubclass(hwnd, Some(widget_proc), SUBCLASS_ID, callback_state as usize).ok() {
        drop(Arc::from_raw(callback_state));
        return Err(error);
    }
    REGISTRY.lock().insert(hwnd.0 as isize, state);
    Ok(())
}

unsafe extern "system" fn widget_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let state = &*(data as *const HwndState);
    match msg {
        WM_WINDOWPOSCHANGING if lp.0 != 0 => {
            let pos = &mut *(lp.0 as *mut WINDOWPOS);
            if state.layer() == Layer::Desktop
                && !SHOW_DESKTOP.load(Ordering::Acquire)
                && !GAME_OVERLAY.load(Ordering::Acquire)
                && !pos.flags.contains(SWP_NOZORDER)
            {
                pos.hwndInsertAfter = HWND_BOTTOM;
            }
        }
        WM_MOUSEACTIVATE if !state.activatable.load(Ordering::Acquire) => {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(hwnd, Some(widget_proc), id);
            REGISTRY.lock().remove(&(hwnd.0 as isize));
            drop(Arc::from_raw(data as *const HwndState));
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

pub fn show_without_activation(hwnd: HWND) -> windows::core::Result<()> {
    let Some(state) = state_for(hwnd) else {
        return Err(windows::core::Error::from_thread());
    };
    place(hwnd, state.layer())
}

pub fn set_layer(hwnd: HWND, layer: Layer) -> windows::core::Result<()> {
    let Some(state) = state_for(hwnd) else {
        return Err(windows::core::Error::from_thread());
    };
    state.layer.store(layer as u8, Ordering::Release);
    place(hwnd, layer)
}

pub fn set_activatable(hwnd: HWND, yes: bool) -> windows::core::Result<()> {
    set_widget_activatable(hwnd, yes, yes)
}

pub fn set_widget_activatable(hwnd: HWND, yes: bool, focus: bool) -> windows::core::Result<()> {
    let Some(state) = state_for(hwnd) else {
        return Err(windows::core::Error::from_thread());
    };
    unsafe {
        styles::set_activatable(hwnd, yes)?;
        state.activatable.store(yes, Ordering::Release);
        if yes && focus && !SetForegroundWindow(hwnd).as_bool() {
            log::warn!("Windows tillod ikke aktivering af {}", state.label);
        }
    }
    Ok(())
}

pub fn set_window_rect(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) -> windows::core::Result<()> {
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    }
}

pub fn reapply_all() {
    let widgets = registry_snapshot();
    for (hwnd, state) in widgets {
        if let Err(error) = place(hwnd, state.layer()) {
            log::error!("Kunne ikke genanvende z-orden på {}: {error}", state.label);
        }
    }
    if let Some(app) = super::app_handle() {
        if app.state::<crate::state::AppState>().edit_mode.load(Ordering::Acquire) {
            crate::windows::overlay::send_grids_to_bottom(app);
        }
    }
    log::debug!("Windows-skrivebordslaget er genanvendt");
}

pub fn set_show_desktop(show: bool) {
    let previous = SHOW_DESKTOP.swap(show, Ordering::AcqRel);
    if previous == show {
        return;
    }
    reapply_all();
}

pub fn show_desktop() -> bool {
    SHOW_DESKTOP.load(Ordering::Acquire)
}

pub fn set_game_overlay(active: bool) -> bool {
    let previous = GAME_OVERLAY.swap(active, Ordering::AcqRel);
    if previous == active {
        return false;
    }
    reapply_all();
    true
}

pub fn game_overlay() -> bool {
    GAME_OVERLAY.load(Ordering::Acquire)
}

pub fn reassert_game_overlay() {
    if !GAME_OVERLAY.load(Ordering::Acquire) {
        return;
    }
    let visible = super::app_handle()
        .map(|app| {
            app.state::<crate::state::AppState>()
                .widgets_visible
                .load(Ordering::Acquire)
        })
        .unwrap_or(true);
    if !visible {
        return;
    }
    for (hwnd, _) in registry_snapshot() {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

pub fn top_widget_labels() -> Vec<String> {
    registry_snapshot()
        .into_iter()
        .filter(|(_, state)| state.layer() == Layer::Top)
        .map(|(_, state)| state.label.clone())
        .collect()
}

pub fn widget_labels() -> Vec<(String, String)> {
    registry_snapshot()
        .into_iter()
        .map(|(_, state)| (state.id.clone(), state.label.clone()))
        .collect()
}

pub fn layer_for_label(label: &str) -> Option<Layer> {
    REGISTRY
        .lock()
        .values()
        .find(|state| state.label == label)
        .map(|state| state.layer())
}

fn state_for(hwnd: HWND) -> Option<Arc<HwndState>> {
    REGISTRY.lock().get(&(hwnd.0 as isize)).cloned()
}

fn registry_snapshot() -> Vec<(HWND, Arc<HwndState>)> {
    REGISTRY
        .lock()
        .iter()
        .map(|(hwnd, state)| (HWND(*hwnd as _), state.clone()))
        .collect()
}

fn place(hwnd: HWND, layer: Layer) -> windows::core::Result<()> {
    // Tauri styrer også vinduets z-orden og ex-styles; synkronisér derfor i denne rækkefølge.
    let overlay = game_overlay();
    let always_on_top = overlay || layer == Layer::Top || (layer == Layer::Desktop && show_desktop());
    if let (Some(app), Some(state)) = (super::app_handle(), state_for(hwnd)) {
        if let Some(window) = app.get_webview_window(&state.label) {
            if let Err(error) = window.set_always_on_top(always_on_top) {
                log::error!("Kunne ikke opdatere Tauris lag for {}: {error}", state.label);
            }
        }
    }
    let insert_after = if overlay {
        Some(HWND_TOPMOST)
    } else {
        match layer {
            Layer::Desktop if show_desktop() => Some(HWND_TOPMOST),
            Layer::Desktop => Some(HWND_BOTTOM),
            Layer::Top => Some(HWND_TOPMOST),
            Layer::Normal => Some(HWND_NOTOPMOST),
        }
    };
    // A hidden widget (Bruger valgte "Skjul alle") must stay hidden when the
    // desktop layer is reapplied: only re-show it when widgets are meant visible.
    let show = super::app_handle()
        .map(|app| {
            app.state::<crate::state::AppState>()
                .widgets_visible
                .load(Ordering::Acquire)
        })
        .unwrap_or(true);
    unsafe {
        styles::apply_widget_styles(hwnd)?;
        if let Some(state) = state_for(hwnd) {
            styles::set_activatable(hwnd, state.activatable.load(Ordering::Acquire))?;
        }
        let mut flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        if show {
            flags |= SWP_SHOWWINDOW;
        }
        SetWindowPos(hwnd, insert_after, 0, 0, 0, 0, flags)?;
    }
    if layer == Layer::Desktop {
        if let Some(app) = super::app_handle() {
            if app.state::<crate::state::AppState>().edit_mode.load(Ordering::Acquire) {
                crate::windows::overlay::send_grids_to_bottom(app);
            }
        }
    }
    Ok(())
}
