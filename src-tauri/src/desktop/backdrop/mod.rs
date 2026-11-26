mod capture;
mod duplication;
mod effects;
mod hdr;
mod squircle;
mod window;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem::size_of,
    sync::LazyLock,
};

use parking_lot::Mutex;
use serde::Serialize;
use ts_rs::TS;
use windows::{
    core::{h, w, Interface},
    Foundation::TimeSpan,
    System::{DispatcherQueue, DispatcherQueueController},
    Win32::{
        Foundation::{ERROR_SUCCESS, HWND, RECT},
        Graphics::Direct2D::{D2D1CreateFactory, ID2D1Factory, D2D1_FACTORY_TYPE_MULTI_THREADED},
        System::{
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
            WinRT::{
                Composition::ICompositorDesktopInterop, CreateDispatcherQueueController,
                DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
            },
        },
        UI::WindowsAndMessaging::{
            DestroyWindow, GetWindowRect, IsWindow, IsWindowVisible, ShowWindow, SW_HIDE,
        },
    },
    UI::Composition::{
        CompositionGeometry, CompositionRoundedRectangleGeometry, Compositor,
        Desktop::DesktopWindowTarget, SpriteVisual,
    },
};
use windows_numerics::Vector2;

use crate::config::schema::{Layer, RadiusPreset, ThemeId, VerticalAnchor, WidgetSize};

use super::glass::{self, LiveBackdropMode};

use capture::{CaptureTargetSpec, SendSwapChain};
use effects::{build_effect_brush, build_surface_brush};
use squircle::{build_clip, spec_clip_key};
use window::{
    align_backdrop_to_widget, create_backdrop_hwnd, ensure_class_registered,
    set_exclude_from_capture, widget_is_topmost,
};

pub use capture::{notify_color_change, notify_display_change};
pub use squircle::{build_squircle_points, compute_blur_and_saturation, compute_corner_geometry};

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WidgetLiveLuma {
    pub seq: u32,
    pub luma: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackdropSpec {
    pub theme: ThemeId,
    pub glass_strength: f32,
    pub radius: RadiusPreset,
    pub size: WidgetSize,
    pub is_cluster: bool,
    pub base_width: f64,
    pub widget_scale: f32,
    pub dpi_scale: f64,
    pub layer: Layer,
}

static BACKDROP_MAP: LazyLock<Mutex<HashMap<isize, isize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn backdrop_hwnd_for(widget_hwnd: HWND) -> Option<HWND> {
    let raw = BACKDROP_MAP
        .lock()
        .get(&(widget_hwnd.0 as isize))
        .copied()?;
    if raw == 0 {
        return None;
    }
    let hwnd = HWND(raw as _);
    if unsafe { IsWindow(Some(hwnd)).as_bool() } {
        Some(hwnd)
    } else {
        None
    }
}

pub fn is_live(widget_hwnd: HWND) -> bool {
    backdrop_hwnd_for(widget_hwnd).is_some()
}

pub fn live_luma(widget_hwnd: HWND) -> WidgetLiveLuma {
    let (seq, luma) = capture::live_luma_snapshot(widget_hwnd.0 as isize);
    WidgetLiveLuma { seq, luma }
}

pub fn system_transparency_enabled() -> bool {
    let mut val: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    if status == ERROR_SUCCESS {
        val != 0
    } else {
        true
    }
}

struct BackdropEntry {
    backdrop_hwnd: HWND,
    target: DesktopWindowTarget,
    visual: SpriteVisual,
    spec: BackdropSpec,
    mode: LiveBackdropMode,
    window_rect: RECT,
    phys_size: (i32, i32),
    sigma_sat: (i32, i32),
    clip_key: (i32, i32, u8, u8, u8, bool, i32),
    window_visible: bool,
    hidden_idle: bool,
    animating_clip: Option<CompositionRoundedRectangleGeometry>,
}

struct BackdropRuntime {
    _dispatcher_controller: Option<DispatcherQueueController>,
    compositor: Compositor,
    d2d_factory: ID2D1Factory,
    entries: HashMap<isize, BackdropEntry>,
}

thread_local! {
    static RUNTIME: RefCell<Option<BackdropRuntime>> = const { RefCell::new(None) };
    static SYNCING: Cell<bool> = const { Cell::new(false) };
}

fn init_runtime() -> windows_core::Result<BackdropRuntime> {
    let controller = if DispatcherQueue::GetForCurrentThread().is_err() {
        let options = DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        };
        Some(unsafe { CreateDispatcherQueueController(options)? })
    } else {
        None
    };

    let compositor = Compositor::new()?;
    let d2d_factory: ID2D1Factory =
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)? };
    ensure_class_registered()?;

    Ok(BackdropRuntime {
        _dispatcher_controller: controller,
        compositor,
        d2d_factory,
        entries: HashMap::new(),
    })
}

fn remove_entry_locked(rt: &mut BackdropRuntime, key: isize, widget_hwnd: HWND) {
    BACKDROP_MAP.lock().remove(&key);
    capture::unregister(key);
    unsafe {
        if IsWindow(Some(widget_hwnd)).as_bool() {
            let _ = set_exclude_from_capture(widget_hwnd, false);
        }
    }
    if let Some(old) = rt.entries.remove(&key) {
        drop(old.visual);
        drop(old.target);
        unsafe {
            if IsWindow(Some(old.backdrop_hwnd)).as_bool() {
                let _ = DestroyWindow(old.backdrop_hwnd);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create_entry_for_mode(
    rt: &BackdropRuntime,
    widget_hwnd: HWND,
    mode: LiveBackdropMode,
    spec: BackdropSpec,
    rect: RECT,
    phys_w: i32,
    phys_h: i32,
    sigma: f32,
    sat: f32,
    sigma_sat: (i32, i32),
    ckey: (i32, i32, u8, u8, u8, bool, i32),
    w_visible: bool,
) -> Option<BackdropEntry> {
    let key = widget_hwnd.0 as isize;
    let topmost = widget_is_topmost(widget_hwnd);

    match mode {
        LiveBackdropMode::Off => None,
        LiveBackdropMode::NativeHostBackdrop => {
            unsafe {
                if IsWindow(Some(widget_hwnd)).as_bool() {
                    let _ = set_exclude_from_capture(widget_hwnd, false);
                }
            }
            let backdrop_hwnd = match create_backdrop_hwnd(rect, topmost, true) {
                Ok(h) => h,
                Err(err) => {
                    log::warn!("DWM HostBackdrop kunne ikke aktiveres på backdrop-vindue: {err}");
                    return None;
                }
            };
            let target =
                match rt
                    .compositor
                    .cast::<ICompositorDesktopInterop>()
                    .and_then(|interop| unsafe {
                        interop.CreateDesktopWindowTarget(backdrop_hwnd, true)
                    }) {
                    Ok(t) => t,
                    Err(err) => {
                        log::warn!("CreateDesktopWindowTarget fejlede: {err}");
                        unsafe {
                            let _ = DestroyWindow(backdrop_hwnd);
                        }
                        return None;
                    }
                };
            let visual = match rt.compositor.CreateSpriteVisual() {
                Ok(v) => v,
                Err(err) => {
                    log::warn!("CreateSpriteVisual fejlede: {err}");
                    drop(target);
                    unsafe {
                        let _ = DestroyWindow(backdrop_hwnd);
                    }
                    return None;
                }
            };

            let res: windows_core::Result<()> = (|| {
                visual.SetSize(Vector2::new(phys_w as f32, phys_h as f32))?;
                let brush = build_effect_brush(&rt.compositor, sigma, sat)?;
                visual.SetBrush(&brush)?;
                let clip = build_clip(
                    &rt.compositor,
                    &rt.d2d_factory,
                    &spec,
                    phys_w as f32,
                    phys_h as f32,
                )?;
                visual.SetClip(&clip)?;
                target.SetRoot(&visual)?;
                Ok(())
            })();

            if let Err(err) = res {
                log::warn!("Opsætning af live liquid glass-graf fejlede: {err}");
                drop(visual);
                drop(target);
                unsafe {
                    let _ = DestroyWindow(backdrop_hwnd);
                }
                return None;
            }

            BACKDROP_MAP.lock().insert(key, backdrop_hwnd.0 as isize);
            align_backdrop_to_widget(widget_hwnd, backdrop_hwnd, false);
            log::info!("Levende glas aktivt (NativeHostBackdrop) for widget {key}");

            Some(BackdropEntry {
                backdrop_hwnd,
                target,
                visual,
                spec,
                mode: LiveBackdropMode::NativeHostBackdrop,
                window_rect: rect,
                phys_size: (phys_w, phys_h),
                sigma_sat,
                clip_key: ckey,
                window_visible: w_visible,
                hidden_idle: false,
                animating_clip: None,
            })
        }
        LiveBackdropMode::CaptureD2d => {
            let backdrop_hwnd = match create_backdrop_hwnd(rect, topmost, false) {
                Ok(h) => h,
                Err(err) => {
                    log::warn!("Capture backdrop-vindue kunne ikke oprettes: {err}");
                    return None;
                }
            };
            if let Err(err) = set_exclude_from_capture(widget_hwnd, true)
                .and_then(|_| set_exclude_from_capture(backdrop_hwnd, true))
            {
                log::warn!("SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE) fejlede: {err}");
                unsafe {
                    let _ = set_exclude_from_capture(widget_hwnd, false);
                    let _ = DestroyWindow(backdrop_hwnd);
                }
                return None;
            }

            let sc = match capture::register_sync(CaptureTargetSpec {
                widget_hwnd: key,
                rect,
                sigma,
                sat,
                paused: false,
            }) {
                Ok(Some(sc)) => sc,
                Ok(None) => {
                    log::warn!("Capture register_sync returnerede ingen swapchain for ny widget");
                    unsafe {
                        let _ = set_exclude_from_capture(widget_hwnd, false);
                        let _ = DestroyWindow(backdrop_hwnd);
                    }
                    return None;
                }
                Err(reason) => {
                    log::warn!("Capture live glass kunne ikke startes: {reason}");
                    unsafe {
                        let _ = set_exclude_from_capture(widget_hwnd, false);
                        let _ = DestroyWindow(backdrop_hwnd);
                    }
                    return None;
                }
            };

            let target =
                match rt
                    .compositor
                    .cast::<ICompositorDesktopInterop>()
                    .and_then(|interop| unsafe {
                        interop.CreateDesktopWindowTarget(backdrop_hwnd, true)
                    }) {
                    Ok(t) => t,
                    Err(err) => {
                        log::warn!("CreateDesktopWindowTarget (capture) fejlede: {err}");
                        capture::unregister(key);
                        unsafe {
                            let _ = set_exclude_from_capture(widget_hwnd, false);
                            let _ = DestroyWindow(backdrop_hwnd);
                        }
                        return None;
                    }
                };

            let visual = match rt.compositor.CreateSpriteVisual() {
                Ok(v) => v,
                Err(err) => {
                    log::warn!("CreateSpriteVisual (capture) fejlede: {err}");
                    drop(target);
                    capture::unregister(key);
                    unsafe {
                        let _ = set_exclude_from_capture(widget_hwnd, false);
                        let _ = DestroyWindow(backdrop_hwnd);
                    }
                    return None;
                }
            };

            let res: windows_core::Result<()> = (|| {
                visual.SetSize(Vector2::new(phys_w as f32, phys_h as f32))?;
                let brush = build_surface_brush(&rt.compositor, &sc.0)?;
                visual.SetBrush(&brush)?;
                let clip = build_clip(
                    &rt.compositor,
                    &rt.d2d_factory,
                    &spec,
                    phys_w as f32,
                    phys_h as f32,
                )?;
                visual.SetClip(&clip)?;
                target.SetRoot(&visual)?;
                Ok(())
            })();

            if let Err(err) = res {
                log::warn!("Opsætning af CaptureD2d liquid glass-graf fejlede: {err}");
                drop(visual);
                drop(target);
                capture::unregister(key);
                unsafe {
                    let _ = set_exclude_from_capture(widget_hwnd, false);
                    let _ = DestroyWindow(backdrop_hwnd);
                }
                return None;
            }

            if !w_visible {
                capture::set_paused(key, true);
            }

            BACKDROP_MAP.lock().insert(key, backdrop_hwnd.0 as isize);
            align_backdrop_to_widget(widget_hwnd, backdrop_hwnd, false);
            if w_visible {
                capture::request_redraw(key);
            }
            log::info!(
                "Levende glas aktivt (CaptureD2d) for widget {key} (backdrop={}, vis={w_visible}, rect={}x{})",
                backdrop_hwnd.0 as isize,
                phys_w,
                phys_h
            );

            Some(BackdropEntry {
                backdrop_hwnd,
                target,
                visual,
                spec,
                mode: LiveBackdropMode::CaptureD2d,
                window_rect: rect,
                phys_size: (phys_w, phys_h),
                sigma_sat,
                clip_key: ckey,
                window_visible: w_visible,
                hidden_idle: false,
                animating_clip: None,
            })
        }
    }
}

pub fn apply(widget_hwnd: HWND, requested_mode: LiveBackdropMode, spec: BackdropSpec) -> bool {
    if requested_mode == LiveBackdropMode::Off || !unsafe { IsWindow(Some(widget_hwnd)).as_bool() }
    {
        detach(widget_hwnd);
        return false;
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(widget_hwnd, &mut rect) }.is_err() {
        detach(widget_hwnd);
        return false;
    }
    let phys_w = (rect.right - rect.left).max(1);
    let phys_h = (rect.bottom - rect.top).max(1);
    let (sigma, sat) = compute_blur_and_saturation(spec.theme, spec.glass_strength, spec.dpi_scale);
    let sigma_sat = ((sigma * 10.0).round() as i32, (sat * 100.0).round() as i32);
    let ckey = spec_clip_key(&spec, phys_w, phys_h);
    let w_visible = unsafe { IsWindowVisible(widget_hwnd).as_bool() };

    RUNTIME.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            match init_runtime() {
                Ok(rt) => *slot = Some(rt),
                Err(err) => {
                    log::warn!("Live liquid glass kunne ikke initialiseres: {err}");
                    return false;
                }
            }
        }
        let Some(rt) = slot.as_mut() else {
            return false;
        };

        let key = widget_hwnd.0 as isize;
        let existing_same_mode = rt
            .entries
            .get(&key)
            .map(|e| unsafe { IsWindow(Some(e.backdrop_hwnd)).as_bool() } && e.mode == requested_mode)
            .unwrap_or(false);

        if !existing_same_mode {
            remove_entry_locked(rt, key, widget_hwnd);

            let created = create_entry_for_mode(
                rt,
                widget_hwnd,
                requested_mode,
                spec,
                rect,
                phys_w,
                phys_h,
                sigma,
                sat,
                sigma_sat,
                ckey,
                w_visible,
            )
            .or_else(|| {
                if requested_mode == LiveBackdropMode::NativeHostBackdrop
                    && glass::is_windows_10_2004_or_newer()
                {
                    log::warn!("Native HostBackdrop fejlede; falder tilbage til CaptureD2d");
                    create_entry_for_mode(
                        rt,
                        widget_hwnd,
                        LiveBackdropMode::CaptureD2d,
                        spec,
                        rect,
                        phys_w,
                        phys_h,
                        sigma,
                        sat,
                        sigma_sat,
                        ckey,
                        w_visible,
                    )
                } else {
                    None
                }
            });

            if let Some(new_entry) = created {
                rt.entries.insert(key, new_entry);
                return true;
            }
            return false;
        }

        let Some(entry) = rt.entries.get_mut(&key) else {
            return false;
        };

        let size_changed = entry.phys_size != (phys_w, phys_h);
        let rect_changed = entry.window_rect != rect;
        let vis_changed = entry.window_visible != w_visible;

        if entry.mode == LiveBackdropMode::NativeHostBackdrop {
            if entry.sigma_sat != sigma_sat {
                match build_effect_brush(&rt.compositor, sigma, sat) {
                    Ok(brush) => {
                        if let Err(err) = entry.visual.SetBrush(&brush) {
                            log::warn!("Opdatering af backdrop-brush fejlede: {err}");
                        } else {
                            entry.sigma_sat = sigma_sat;
                        }
                    }
                    Err(err) => {
                        log::warn!("Kunne ikke bygge opdateret backdrop-brush: {err}");
                    }
                }
            }
        } else if entry.sigma_sat != sigma_sat && !size_changed && !rect_changed && !vis_changed {
            capture::update_params_nonblocking(key, sigma, sat);
            entry.sigma_sat = sigma_sat;
        } else if entry.sigma_sat != sigma_sat {
            entry.sigma_sat = sigma_sat;
        }

        if size_changed {
            let _ = entry.visual.SetSize(Vector2::new(phys_w as f32, phys_h as f32));
            entry.phys_size = (phys_w, phys_h);
        }

        if entry.clip_key != ckey {
            match build_clip(
                &rt.compositor,
                &rt.d2d_factory,
                &spec,
                phys_w as f32,
                phys_h as f32,
            ) {
                Ok(clip) => {
                    if entry.visual.SetClip(&clip).is_ok() {
                        entry.clip_key = ckey;
                        entry.animating_clip = None;
                    }
                }
                Err(err) => {
                    log::warn!("Kunne ikke opdatere backdrop-clip: {err}");
                }
            }
        }

        entry.spec = spec;
        entry.window_rect = rect;
        entry.window_visible = w_visible;
        if entry.mode == LiveBackdropMode::CaptureD2d && (size_changed || rect_changed || vis_changed)
        {
            capture::update_nonblocking(CaptureTargetSpec {
                widget_hwnd: key,
                rect,
                sigma,
                sat,
                paused: !(w_visible && !entry.hidden_idle),
            });
        }

        let b_hwnd = entry.backdrop_hwnd;
        let hidden_idle = entry.hidden_idle;
        align_backdrop_to_widget(widget_hwnd, b_hwnd, hidden_idle);
        true
    })
}

pub fn attach_capture_swapchain(widget_hwnd: HWND, generation: u64, swap_chain: SendSwapChain) {
    if !super::is_ui_thread() {
        return;
    }
    let key = widget_hwnd.0 as isize;
    if !capture::is_generation_current(key, generation) {
        return;
    }
    RUNTIME.with(|cell| {
        let slot = cell.borrow();
        let Some(rt) = slot.as_ref() else {
            return;
        };
        let Some(entry) = rt.entries.get(&key) else {
            return;
        };
        if entry.mode != LiveBackdropMode::CaptureD2d
            || !unsafe { IsWindow(Some(entry.backdrop_hwnd)).as_bool() }
        {
            return;
        }
        match build_surface_brush(&rt.compositor, &swap_chain.0) {
            Ok(brush) => {
                if let Err(err) = entry.visual.SetBrush(&brush) {
                    log::warn!("Kunne ikke binde opdateret capture-brush: {err}");
                } else if entry.window_visible && !entry.hidden_idle {
                    capture::request_redraw(key);
                }
            }
            Err(err) => {
                log::warn!("Kunne ikke bygge capture SurfaceBrush: {err}");
            }
        }
    });
}

pub fn sync_from_widget(widget_hwnd: HWND) {
    if !super::is_ui_thread() {
        if let Some(app) = super::app_handle() {
            let raw = widget_hwnd.0 as isize;
            let _ = app.run_on_main_thread(move || {
                sync_from_widget(HWND(raw as _));
            });
        }
        return;
    }

    if SYNCING.with(|flag| flag.replace(true)) {
        return;
    }

    RUNTIME.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(rt) = slot.as_mut() else {
            return;
        };
        let key = widget_hwnd.0 as isize;
        let Some(entry) = rt.entries.get_mut(&key) else {
            return;
        };
        if !unsafe { IsWindow(Some(entry.backdrop_hwnd)).as_bool() } {
            remove_entry_locked(rt, key, widget_hwnd);
            return;
        }

        let mut rect = RECT::default();
        if unsafe { GetWindowRect(widget_hwnd, &mut rect) }.is_err() {
            return;
        }
        let phys_w = (rect.right - rect.left).max(1);
        let phys_h = (rect.bottom - rect.top).max(1);
        let w_visible = unsafe { IsWindowVisible(widget_hwnd).as_bool() };
        let size_changed = entry.phys_size != (phys_w, phys_h);
        let rect_changed = entry.window_rect != rect;
        let vis_changed = entry.window_visible != w_visible;

        if size_changed {
            let _ = entry
                .visual
                .SetSize(Vector2::new(phys_w as f32, phys_h as f32));
            entry.phys_size = (phys_w, phys_h);
            entry.animating_clip = None;
            let ckey = spec_clip_key(&entry.spec, phys_w, phys_h);
            if let Ok(clip) = build_clip(
                &rt.compositor,
                &rt.d2d_factory,
                &entry.spec,
                phys_w as f32,
                phys_h as f32,
            ) {
                if entry.visual.SetClip(&clip).is_ok() {
                    entry.clip_key = ckey;
                }
            }
        }

        if entry.mode == LiveBackdropMode::CaptureD2d
            && (size_changed || rect_changed || vis_changed)
        {
            entry.window_rect = rect;
            entry.window_visible = w_visible;
            let (sigma, sat) = compute_blur_and_saturation(
                entry.spec.theme,
                entry.spec.glass_strength,
                entry.spec.dpi_scale,
            );
            capture::update_nonblocking(CaptureTargetSpec {
                widget_hwnd: key,
                rect,
                sigma,
                sat,
                paused: !(w_visible && !entry.hidden_idle),
            });
        } else {
            entry.window_rect = rect;
            entry.window_visible = w_visible;
        }

        align_backdrop_to_widget(widget_hwnd, entry.backdrop_hwnd, entry.hidden_idle);
    });

    SYNCING.with(|flag| flag.set(false));
}

pub fn set_window_visible(widget_hwnd: HWND, visible: bool) {
    if !super::is_ui_thread() {
        return;
    }

    RUNTIME.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(rt) = slot.as_mut() else {
            return;
        };
        let key = widget_hwnd.0 as isize;
        let Some(entry) = rt.entries.get_mut(&key) else {
            return;
        };
        entry.window_visible = visible;
        if entry.mode == LiveBackdropMode::CaptureD2d {
            capture::set_paused(key, !(entry.window_visible && !entry.hidden_idle));
        }
        let b_hwnd = entry.backdrop_hwnd;
        if !unsafe { IsWindow(Some(b_hwnd)).as_bool() } {
            return;
        }
        if !visible || entry.hidden_idle {
            unsafe {
                let _ = ShowWindow(b_hwnd, SW_HIDE);
            }
        } else {
            align_backdrop_to_widget(widget_hwnd, b_hwnd, false);
        }
    });
}

pub fn set_idle_visible(widget_hwnd: HWND, visible: bool, duration_ms: u32) {
    if !super::is_ui_thread() {
        return;
    }

    RUNTIME.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(rt) = slot.as_mut() else {
            return;
        };
        let key = widget_hwnd.0 as isize;
        let Some(entry) = rt.entries.get_mut(&key) else {
            return;
        };
        let w_visible = unsafe { IsWindowVisible(widget_hwnd).as_bool() };
        entry.window_visible = w_visible;
        if entry.hidden_idle == !visible {
            if entry.mode == LiveBackdropMode::CaptureD2d {
                capture::set_paused(key, !(w_visible && !entry.hidden_idle));
            }
            return;
        }
        entry.hidden_idle = !visible;
        if entry.mode == LiveBackdropMode::CaptureD2d {
            capture::set_paused(key, !(w_visible && !entry.hidden_idle));
        }
        let b_hwnd = entry.backdrop_hwnd;

        if visible {
            if duration_ms == 0 {
                let _ = entry.visual.StopAnimation(h!("Opacity"));
                let _ = entry.visual.SetOpacity(1.0);
                align_backdrop_to_widget(widget_hwnd, b_hwnd, false);
            } else {
                align_backdrop_to_widget(widget_hwnd, b_hwnd, false);
                if let Ok(anim) = rt.compositor.CreateScalarKeyFrameAnimation() {
                    let _ = anim.SetDuration(TimeSpan {
                        Duration: (duration_ms as i64) * 10_000,
                    });
                    if let Ok(ease) = rt.compositor.CreateCubicBezierEasingFunction(
                        Vector2::new(0.22, 0.61),
                        Vector2::new(0.36, 1.0),
                    ) {
                        let _ = anim.InsertKeyFrameWithEasingFunction(1.0, 1.0, &ease);
                    } else {
                        let _ = anim.InsertKeyFrame(1.0, 1.0);
                    }
                    let _ = entry.visual.StartAnimation(h!("Opacity"), &anim);
                } else {
                    let _ = entry.visual.SetOpacity(1.0);
                }
            }
            return;
        }

        if duration_ms == 0 {
            let _ = entry.visual.StopAnimation(h!("Opacity"));
            let _ = entry.visual.SetOpacity(0.0);
            unsafe {
                let _ = ShowWindow(b_hwnd, SW_HIDE);
            }
        } else if let Ok(anim) = rt.compositor.CreateScalarKeyFrameAnimation() {
            let _ = anim.SetDuration(TimeSpan {
                Duration: (duration_ms as i64) * 10_000,
            });
            if let Ok(ease) = rt
                .compositor
                .CreateCubicBezierEasingFunction(Vector2::new(0.22, 0.61), Vector2::new(0.36, 1.0))
            {
                let _ = anim.InsertKeyFrameWithEasingFunction(1.0, 0.0, &ease);
            } else {
                let _ = anim.InsertKeyFrame(1.0, 0.0);
            }
            let _ = entry.visual.StartAnimation(h!("Opacity"), &anim);
        } else {
            let _ = entry.visual.SetOpacity(0.0);
            unsafe {
                let _ = ShowWindow(b_hwnd, SW_HIDE);
            }
        }
    });
}

pub fn animate_cluster_box(
    widget_hwnd: HWND,
    from_h_u: f64,
    to_h_u: f64,
    anchor: VerticalAnchor,
    duration_ms: u32,
) {
    if !super::is_ui_thread() {
        return;
    }

    RUNTIME.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(rt) = slot.as_mut() else {
            return;
        };
        let key = widget_hwnd.0 as isize;
        let Some(entry) = rt.entries.get_mut(&key) else {
            return;
        };
        let (phys_w, phys_h) = entry.phys_size;
        let pw = phys_w.max(1) as f32;
        let ph = phys_h.max(1) as f32;

        if duration_ms == 0 || (from_h_u - to_h_u).abs() < 0.25 {
            entry.animating_clip = None;
            if let Ok(clip) = build_clip(&rt.compositor, &rt.d2d_factory, &entry.spec, pw, ph) {
                let _ = entry.visual.SetClip(&clip);
                entry.clip_key = spec_clip_key(&entry.spec, phys_w, phys_h);
            }
            return;
        }

        let u_phys = pw / (entry.spec.base_width.max(1.0) as f32);
        let from_h = (from_h_u as f32 * u_phys).clamp(1.0, ph);
        let to_h = (to_h_u as f32 * u_phys).clamp(1.0, ph);

        let w_inset = (pw - 1.0).max(1.0);
        let from_inset = (from_h - 1.0).max(1.0);
        let to_inset = (to_h - 1.0).max(1.0);
        let (r_base, _) = compute_corner_geometry(&entry.spec, pw, to_h, false);
        let r_inset = (r_base - 0.5).clamp(0.0, (w_inset * 0.5).min(to_inset * 0.5));

        let from_off_y = if anchor == VerticalAnchor::Bottom {
            (ph - from_h + 0.5).max(0.5)
        } else {
            0.5
        };
        let to_off_y = if anchor == VerticalAnchor::Bottom {
            (ph - to_h + 0.5).max(0.5)
        } else {
            0.5
        };

        let Ok(rr) = rt.compositor.CreateRoundedRectangleGeometry() else {
            return;
        };
        let _ = rr.SetCornerRadius(Vector2::new(r_inset, r_inset));
        let _ = rr.SetSize(Vector2::new(w_inset, from_inset));
        let _ = rr.SetOffset(Vector2::new(0.5, from_off_y));

        let Ok(comp_geom) = rr.cast::<CompositionGeometry>() else {
            return;
        };
        let Ok(clip) = rt.compositor.CreateGeometricClipWithGeometry(&comp_geom) else {
            return;
        };
        if entry.visual.SetClip(&clip).is_err() {
            return;
        }

        let dur = TimeSpan {
            Duration: (duration_ms as i64) * 10_000,
        };
        if let Ok(ease) = rt
            .compositor
            .CreateCubicBezierEasingFunction(Vector2::new(0.16, 1.0), Vector2::new(0.3, 1.0))
        {
            if let Ok(size_anim) = rt.compositor.CreateVector2KeyFrameAnimation() {
                let _ = size_anim.SetDuration(dur);
                let _ = size_anim.InsertKeyFrameWithEasingFunction(
                    0.0,
                    Vector2::new(w_inset, from_inset),
                    &ease,
                );
                let _ = size_anim.InsertKeyFrameWithEasingFunction(
                    1.0,
                    Vector2::new(w_inset, to_inset),
                    &ease,
                );
                let _ = rr.StartAnimation(h!("Size"), &size_anim);
            }
            if (from_off_y - to_off_y).abs() > 0.1 {
                if let Ok(off_anim) = rt.compositor.CreateVector2KeyFrameAnimation() {
                    let _ = off_anim.SetDuration(dur);
                    let _ = off_anim.InsertKeyFrameWithEasingFunction(
                        0.0,
                        Vector2::new(0.5, from_off_y),
                        &ease,
                    );
                    let _ = off_anim.InsertKeyFrameWithEasingFunction(
                        1.0,
                        Vector2::new(0.5, to_off_y),
                        &ease,
                    );
                    let _ = rr.StartAnimation(h!("Offset"), &off_anim);
                }
            }
        } else {
            let _ = rr.SetSize(Vector2::new(w_inset, to_inset));
            let _ = rr.SetOffset(Vector2::new(0.5, to_off_y));
        }

        entry.animating_clip = Some(rr);
    });
}

pub fn detach(widget_hwnd: HWND) {
    let key = widget_hwnd.0 as isize;
    BACKDROP_MAP.lock().remove(&key);
    capture::unregister(key);
    unsafe {
        if IsWindow(Some(widget_hwnd)).as_bool() {
            let _ = set_exclude_from_capture(widget_hwnd, false);
        }
    }
    if !super::is_ui_thread() {
        return;
    }
    RUNTIME.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(rt) = slot.as_mut() else {
            return;
        };
        if let Some(entry) = rt.entries.remove(&key) {
            drop(entry.visual);
            drop(entry.target);
            unsafe {
                if IsWindow(Some(entry.backdrop_hwnd)).as_bool() {
                    let _ = DestroyWindow(entry.backdrop_hwnd);
                }
            }
        }
    });
}

pub fn shutdown_all() {
    capture::shutdown_all();
    let stale: Vec<(isize, isize)> = BACKDROP_MAP.lock().drain().collect();
    if super::is_ui_thread() {
        RUNTIME.with(|cell| {
            if let Some(mut rt) = cell.borrow_mut().take() {
                for (w_raw, entry) in rt.entries.drain() {
                    let w_hwnd = HWND(w_raw as _);
                    unsafe {
                        if IsWindow(Some(w_hwnd)).as_bool() {
                            let _ = set_exclude_from_capture(w_hwnd, false);
                        }
                    }
                    drop(entry.visual);
                    drop(entry.target);
                    unsafe {
                        if IsWindow(Some(entry.backdrop_hwnd)).as_bool() {
                            let _ = DestroyWindow(entry.backdrop_hwnd);
                        }
                    }
                }
            }
        });
    }
    for (w_raw, b_raw) in stale {
        if w_raw != 0 {
            let w_hwnd = HWND(w_raw as _);
            unsafe {
                if IsWindow(Some(w_hwnd)).as_bool() {
                    let _ = set_exclude_from_capture(w_hwnd, false);
                }
            }
        }
        if b_raw != 0 {
            let b_hwnd = HWND(b_raw as _);
            unsafe {
                if IsWindow(Some(b_hwnd)).as_bool() {
                    let _ = DestroyWindow(b_hwnd);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec(
        theme: ThemeId,
        radius: RadiusPreset,
        size: WidgetSize,
        is_cluster: bool,
    ) -> BackdropSpec {
        BackdropSpec {
            theme,
            glass_strength: 1.0,
            radius,
            size,
            is_cluster,
            base_width: crate::config::geometry::widget_base_size(size).0,
            widget_scale: 1.0,
            dpi_scale: 1.0,
            layer: Layer::Top,
        }
    }

    #[test]
    fn compute_corner_geometry_matches_css_tokens_and_squircle_rules() {
        // Aurora M Round at 100% DPI: r_base=22, mult=1.0, squircle=1.5*22=33
        let (r, sq) = compute_corner_geometry(
            &sample_spec(ThemeId::Aurora, RadiusPreset::Round, WidgetSize::M, false),
            276.0,
            132.0,
            true,
        );
        assert!(sq);
        assert!((r - 33.0).abs() < 1e-3);

        // Laerk uses 0.72 multiplier
        let (r_laerk, sq_laerk) = compute_corner_geometry(
            &sample_spec(ThemeId::Laerk, RadiusPreset::Round, WidgetSize::M, false),
            276.0,
            132.0,
            true,
        );
        assert!(sq_laerk);
        assert!((r_laerk - 22.0 * 0.72 * 1.5).abs() < 1e-3);

        // Pill + Extra forces round corner-shape without 1.5x squircle expansion
        let (r_pill, sq_pill) = compute_corner_geometry(
            &sample_spec(ThemeId::Aurora, RadiusPreset::Extra, WidgetSize::Pill, false),
            204.0,
            48.0,
            true,
        );
        assert!(!sq_pill);
        assert!((r_pill - 24.0).abs() < 1e-3);

        // Cluster uses CLUSTER_RADII (10/22/34) and respects squircle
        let (r_cl, sq_cl) = compute_corner_geometry(
            &sample_spec(ThemeId::Grafit, RadiusPreset::Extra, WidgetSize::M, true),
            264.0,
            180.0,
            true,
        );
        assert!(sq_cl);
        assert!((r_cl - 34.0 * 1.5 * (264.0 / 276.0)).abs() < 0.05);

        // Fallback when CSS squircle unsupported omits 1.5x factor
        let (r_fb, sq_fb) = compute_corner_geometry(
            &sample_spec(ThemeId::Aurora, RadiusPreset::Soft, WidgetSize::L, false),
            276.0,
            276.0,
            false,
        );
        assert!(!sq_fb);
        assert!((r_fb - 12.0).abs() < 1e-3);
    }

    #[test]
    fn squircle_points_form_closed_symmetric_inscribed_loop() {
        let w = 275.0_f32;
        let h = 131.0_f32;
        let pts = build_squircle_points(0.5, 0.5, w, h, 32.5);
        assert!(pts.len() >= 92);
        for pt in &pts {
            assert!(pt.X >= 0.5 - 1e-4 && pt.X <= 0.5 + w + 1e-4);
            assert!(pt.Y >= 0.5 - 1e-4 && pt.Y <= 0.5 + h + 1e-4);
        }
        // First point starts at top-left vertical tangent (x=0.5, y=0.5+r)
        assert!((pts[0].X - 0.5).abs() < 1e-4);
        assert!((pts[0].Y - 33.0).abs() < 1e-4);
    }
}
