use std::{
    collections::HashMap,
    mem::{size_of, ManuallyDrop},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc, LazyLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};
use windows::{
    core::{Interface, HRESULT},
    Win32::{
        Devices::Display::{
            DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
            DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO,
            DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
            DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
        },
        Foundation::{
            D2DERR_RECREATE_TARGET, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, E_ACCESSDENIED,
            HMODULE, HWND, RECT,
        },
        Graphics::{
            Direct2D::{
                CLSID_D2D1Border, CLSID_D2D1ColorMatrix, CLSID_D2D1GaussianBlur,
                CLSID_D2D1Saturation,
                Common::{
                    D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BORDER_MODE_SOFT,
                    D2D1_COLORMATRIX_ALPHA_MODE_STRAIGHT, D2D1_COMPOSITE_MODE_SOURCE_OVER,
                    D2D1_PIXEL_FORMAT, D2D_RECT_F,
                },
                D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Effect,
                ID2D1Factory1, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE,
                D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
                D2D1_BORDER_EDGE_MODE_CLAMP, D2D1_BORDER_PROP_EDGE_MODE_X,
                D2D1_BORDER_PROP_EDGE_MODE_Y, D2D1_COLORMATRIX_PROP_ALPHA_MODE,
                D2D1_COLORMATRIX_PROP_COLOR_MATRIX, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
                D2D1_FACTORY_TYPE_MULTI_THREADED, D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
                D2D1_GAUSSIANBLUR_PROP_BORDER_MODE, D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION,
                D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_PROPERTY_TYPE_UNKNOWN, D2D1_SATURATION_PROP_SATURATION,
            },
            Direct3D::{
                D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0,
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            },
            Direct3D11::{
                D3D11CreateDevice, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext,
                ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11Resource,
                ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
                D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
                D3D11_BUFFER_DESC, D3D11_CPU_ACCESS_WRITE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_CULL_NONE, D3D11_FILL_SOLID, D3D11_MAPPED_SUBRESOURCE,
                D3D11_MAP_WRITE_DISCARD, D3D11_RASTERIZER_DESC, D3D11_SDK_VERSION,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
                D3D11_BOX,
            },
            Dxgi::{
                Common::{
                    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
                    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT,
                    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory2,
                IDXGIOutput1, IDXGIOutput5, IDXGIOutput6, IDXGIOutputDuplication, IDXGIResource,
                IDXGISurface, IDXGISwapChain1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_DEVICE_REMOVED,
                DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_NOT_CURRENTLY_AVAILABLE,
                DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_WAIT_TIMEOUT,
                DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_MOVE_RECT, DXGI_PRESENT,
                DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD,
                DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
    },
};
use windows_numerics::Vector2;

use crate::wallpaper::shader_compiler::compile_hlsl;

pub struct SendSwapChain(pub IDXGISwapChain1);
unsafe impl Send for SendSwapChain {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureTargetSpec {
    pub widget_hwnd: isize,
    pub rect: RECT,
    pub sigma: f32,
    pub sat: f32,
    pub paused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MonitorKey {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl From<RECT> for MonitorKey {
    fn from(r: RECT) -> Self {
        Self {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

impl From<MonitorKey> for RECT {
    fn from(k: MonitorKey) -> Self {
        Self {
            left: k.left,
            top: k.top,
            right: k.right,
            bottom: k.bottom,
        }
    }
}

struct TargetRuntime {
    spec: CaptureTargetSpec,
    phys_w: u32,
    phys_h: u32,
    swapchain_gen: u64,
    needs_swapchain: bool,
    needs_redraw: bool,
    first_drawn: bool,
    logged_first_present: bool,
    last_draw_err: Option<(HRESULT, Instant)>,
    last_redraw_at: Option<Instant>,
    rect_changed_at: Instant,
    pending_reply: Option<SyncSender<Result<SendSwapChain, String>>>,
    swap_chain: Option<IDXGISwapChain1>,
}

struct WorkerSharedState {
    targets: HashMap<isize, TargetRuntime>,
    display_dirty: bool,
    color_dirty: bool,
    shutdown: bool,
    last_drag_at: Option<Instant>,
}

struct WorkerControl {
    mon_key: MonitorKey,
    state: Mutex<WorkerSharedState>,
    cv: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct CaptureCoordinator {
    monitors: Vec<MonitorKey>,
    workers: HashMap<MonitorKey, Arc<WorkerControl>>,
    widget_monitor: HashMap<isize, MonitorKey>,
}

static COORDINATOR: LazyLock<Mutex<CaptureCoordinator>> =
    LazyLock::new(|| Mutex::new(CaptureCoordinator::default()));
static LATEST_GEN: LazyLock<Mutex<HashMap<isize, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_GEN: AtomicU64 = AtomicU64::new(1);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

fn bump_generation(widget_hwnd: isize) -> u64 {
    let gen_id = NEXT_GEN.fetch_add(1, Ordering::AcqRel);
    LATEST_GEN.lock().insert(widget_hwnd, gen_id);
    gen_id
}

pub fn is_generation_current(widget_hwnd: isize, generation: u64) -> bool {
    LATEST_GEN
        .lock()
        .get(&widget_hwnd)
        .copied()
        .map(|cur| cur == generation)
        .unwrap_or(false)
}

fn rect_dims(r: RECT) -> (u32, u32) {
    (
        (r.right - r.left).max(1) as u32,
        (r.bottom - r.top).max(1) as u32,
    )
}

pub fn rects_intersect(a: RECT, b: RECT) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

const MAX_REGION_COPIES: usize = 64;

fn clip_rect_to_tex(r: RECT, width: u32, height: u32) -> Option<RECT> {
    let w = width as i32;
    let h = height as i32;
    let c = RECT {
        left: r.left.clamp(0, w),
        top: r.top.clamp(0, h),
        right: r.right.clamp(0, w),
        bottom: r.bottom.clamp(0, h),
    };
    if c.right > c.left && c.bottom > c.top {
        Some(c)
    } else {
        None
    }
}

fn regions_to_copy(dirty: &[RECT], width: u32, height: u32) -> Vec<RECT> {
    let mut out: Vec<RECT> = dirty
        .iter()
        .filter_map(|&r| clip_rect_to_tex(r, width, height))
        .collect();
    if out.len() > MAX_REGION_COPIES {
        let mut bb = out[0];
        for r in &out[1..] {
            bb.left = bb.left.min(r.left);
            bb.top = bb.top.min(r.top);
            bb.right = bb.right.max(r.right);
            bb.bottom = bb.bottom.max(r.bottom);
        }
        out.truncate(0);
        out.push(bb);
    }
    out
}

pub fn influence_rect(widget_rect: RECT, mon_rect: RECT, sigma: f32) -> RECT {
    let pad = (sigma.max(0.0) * 3.0).ceil() as i32;
    let mon_w = (mon_rect.right - mon_rect.left).max(1);
    let mon_h = (mon_rect.bottom - mon_rect.top).max(1);
    let rel_l = (widget_rect.left - mon_rect.left - pad).clamp(0, mon_w);
    let rel_t = (widget_rect.top - mon_rect.top - pad).clamp(0, mon_h);
    let rel_r = (widget_rect.right - mon_rect.left + pad).clamp(0, mon_w);
    let rel_b = (widget_rect.bottom - mon_rect.top + pad).clamp(0, mon_h);
    RECT {
        left: rel_l,
        top: rel_t,
        right: rel_r.max(rel_l),
        bottom: rel_b.max(rel_t),
    }
}

pub fn pick_monitor_index(monitors: &[RECT], widget_rect: RECT) -> Option<usize> {
    if monitors.is_empty() {
        return None;
    }
    let cx = (widget_rect.left + widget_rect.right) / 2;
    let cy = (widget_rect.top + widget_rect.bottom) / 2;
    if let Some((idx, _)) = monitors.iter().enumerate().find(|(_, r)| {
        cx >= r.left && cx < r.right && cy >= r.top && cy < r.bottom
    }) {
        return Some(idx);
    }

    let mut best_idx = 0usize;
    let mut best_area = 0_i64;
    for (idx, r) in monitors.iter().enumerate() {
        let ix0 = widget_rect.left.max(r.left);
        let iy0 = widget_rect.top.max(r.top);
        let ix1 = widget_rect.right.min(r.right);
        let iy1 = widget_rect.bottom.min(r.bottom);
        let area = ((ix1 - ix0).max(0) as i64) * ((iy1 - iy0).max(0) as i64);
        if area > best_area {
            best_area = area;
            best_idx = idx;
        }
    }
    Some(best_idx)
}

fn enumerate_dxgi_monitor_keys() -> Vec<MonitorKey> {
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory2>() }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut a_idx = 0_u32;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(a_idx) } {
        a_idx += 1;
        let mut o_idx = 0_u32;
        while let Ok(output) = unsafe { adapter.EnumOutputs(o_idx) } {
            o_idx += 1;
            let Ok(desc) = (unsafe { output.GetDesc() }) else {
                continue;
            };
            if !desc.AttachedToDesktop.as_bool() {
                continue;
            }
            let r = desc.DesktopCoordinates;
            if r.right > r.left && r.bottom > r.top {
                let key = MonitorKey::from(r);
                if !out.contains(&key) {
                    out.push(key);
                }
            }
        }
    }
    out
}

fn ensure_monitors_cached(coord: &mut CaptureCoordinator) {
    if coord.monitors.is_empty() {
        coord.monitors = enumerate_dxgi_monitor_keys();
    }
}

fn select_monitor_for_rect(coord: &mut CaptureCoordinator, widget_rect: RECT) -> Option<MonitorKey> {
    ensure_monitors_cached(coord);
    let rects: Vec<RECT> = coord.monitors.iter().copied().map(RECT::from).collect();
    pick_monitor_index(&rects, widget_rect).map(|idx| coord.monitors[idx])
}

fn remove_widget_from_worker_locked(coord: &mut CaptureCoordinator, widget_hwnd: isize) {
    let Some(old_mon) = coord.widget_monitor.remove(&widget_hwnd) else {
        return;
    };
    if let Some(worker) = coord.workers.get(&old_mon).cloned() {
        let mut st = worker.state.lock();
        st.targets.remove(&widget_hwnd);
        let empty = st.targets.is_empty();
        worker.cv.notify_all();
        drop(st);
        if empty {
            coord.workers.remove(&old_mon);
        }
    }
}

fn get_or_spawn_worker_locked(
    coord: &mut CaptureCoordinator,
    mon_key: MonitorKey,
) -> Result<Arc<WorkerControl>, String> {
    if let Some(existing) = coord.workers.get(&mon_key) {
        return Ok(existing.clone());
    }
    let ctrl = Arc::new(WorkerControl {
        mon_key,
        state: Mutex::new(WorkerSharedState {
            targets: HashMap::new(),
            display_dirty: false,
            color_dirty: false,
            shutdown: false,
            last_drag_at: None,
        }),
        cv: Condvar::new(),
        thread: Mutex::new(None),
    });
    let worker_ctrl = ctrl.clone();
    let handle = thread::Builder::new()
        .name("opendock-glass-capture".into())
        .spawn(move || monitor_worker_loop(worker_ctrl))
        .map_err(|e| format!("Kunne ikke starte capture-tråd: {e}"))?;
    *ctrl.thread.lock() = Some(handle);
    coord.workers.insert(mon_key, ctrl.clone());
    Ok(ctrl)
}

pub fn register_sync(spec: CaptureTargetSpec) -> Result<Option<SendSwapChain>, String> {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return Err("Capture lukker ned".into());
    }

    let (phys_w, phys_h) = rect_dims(spec.rect);
    let (reply_tx, reply_rx) = mpsc::sync_channel::<Result<SendSwapChain, String>>(1);

    let (ctrl, need_wait) = {
        let mut coord = COORDINATOR.lock();
        coord.monitors = enumerate_dxgi_monitor_keys();
        let Some(mon_key) = select_monitor_for_rect(&mut coord, spec.rect) else {
            return Err("Ingen aktiv DXGI-skærm fundet".into());
        };

        let same_mon = coord.widget_monitor.get(&spec.widget_hwnd).copied() == Some(mon_key);
        if !same_mon {
            remove_widget_from_worker_locked(&mut coord, spec.widget_hwnd);
        }

        let ctrl = get_or_spawn_worker_locked(&mut coord, mon_key)?;
        coord.widget_monitor.insert(spec.widget_hwnd, mon_key);

        let now = Instant::now();
        let mut st = ctrl.state.lock();
        let mut wait_for_sc = true;

        if let Some(existing) = st.targets.get_mut(&spec.widget_hwnd) {
            existing.spec = spec;
            existing.rect_changed_at = now;
            existing.needs_redraw = true;
            if existing.phys_w == phys_w && existing.phys_h == phys_h && existing.swap_chain.is_some()
            {
                wait_for_sc = false;
            } else {
                existing.phys_w = phys_w;
                existing.phys_h = phys_h;
                existing.swapchain_gen = bump_generation(spec.widget_hwnd);
                existing.needs_swapchain = true;
                existing.pending_reply = Some(reply_tx);
            }
        } else {
            let gen_id = bump_generation(spec.widget_hwnd);
            st.targets.insert(
                spec.widget_hwnd,
                TargetRuntime {
                    spec,
                    phys_w,
                    phys_h,
                    swapchain_gen: gen_id,
                    needs_swapchain: true,
                    needs_redraw: true,
                    first_drawn: false,
                    logged_first_present: false,
                    last_draw_err: None,
                    last_redraw_at: None,
                    rect_changed_at: now,
                    pending_reply: Some(reply_tx),
                    swap_chain: None,
                },
            );
        }
        st.last_drag_at = Some(now);
        ctrl.cv.notify_all();
        drop(st);
        (ctrl, wait_for_sc)
    };

    if !need_wait {
        return Ok(None);
    }

    match reply_rx.recv_timeout(Duration::from_millis(1500)) {
        Ok(Ok(sc)) => Ok(Some(sc)),
        Ok(Err(reason)) => {
            unregister(spec.widget_hwnd);
            Err(reason)
        }
        Err(_) => {
            let mut st = ctrl.state.lock();
            if let Some(t) = st.targets.get_mut(&spec.widget_hwnd) {
                t.pending_reply = None;
            }
            drop(st);
            unregister(spec.widget_hwnd);
            Err("Timeout ved opstart af live capture-overflade".into())
        }
    }
}

pub fn update_nonblocking(spec: CaptureTargetSpec) {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let (phys_w, phys_h) = rect_dims(spec.rect);
    let now = Instant::now();

    let mut coord = COORDINATOR.lock();
    let Some(mon_key) = select_monitor_for_rect(&mut coord, spec.rect) else {
        return;
    };
    let prev_mon = coord.widget_monitor.get(&spec.widget_hwnd).copied();
    let same_mon = prev_mon == Some(mon_key);

    if !same_mon {
        remove_widget_from_worker_locked(&mut coord, spec.widget_hwnd);
        let Ok(ctrl) = get_or_spawn_worker_locked(&mut coord, mon_key) else {
            return;
        };
        coord.widget_monitor.insert(spec.widget_hwnd, mon_key);
        let gen_id = bump_generation(spec.widget_hwnd);
        let mut st = ctrl.state.lock();
        st.targets.insert(
            spec.widget_hwnd,
            TargetRuntime {
                spec,
                phys_w,
                phys_h,
                swapchain_gen: gen_id,
                needs_swapchain: true,
                needs_redraw: true,
                first_drawn: false,
                logged_first_present: false,
                last_draw_err: None,
                last_redraw_at: None,
                rect_changed_at: now,
                pending_reply: None,
                swap_chain: None,
            },
        );
        st.last_drag_at = Some(now);
        ctrl.cv.notify_all();
        return;
    }

    if let Some(ctrl) = coord.workers.get(&mon_key).cloned() {
        drop(coord);
        let mut st = ctrl.state.lock();
        if let Some(t) = st.targets.get_mut(&spec.widget_hwnd) {
            if t.phys_w != phys_w || t.phys_h != phys_h {
                t.phys_w = phys_w;
                t.phys_h = phys_h;
                t.swapchain_gen = bump_generation(spec.widget_hwnd);
                t.needs_swapchain = true;
                t.first_drawn = false;
            }
            t.spec = spec;
            t.needs_redraw = true;
            t.rect_changed_at = now;
            st.last_drag_at = Some(now);
            ctrl.cv.notify_all();
        }
    }
}

pub fn update_params_nonblocking(widget_hwnd: isize, sigma: f32, sat: f32) {
    let coord = COORDINATOR.lock();
    let Some(&mon_key) = coord.widget_monitor.get(&widget_hwnd) else {
        return;
    };
    let Some(ctrl) = coord.workers.get(&mon_key).cloned() else {
        return;
    };
    drop(coord);

    let mut st = ctrl.state.lock();
    if let Some(t) = st.targets.get_mut(&widget_hwnd) {
        t.spec.sigma = sigma;
        t.spec.sat = sat;
        t.needs_redraw = true;
        t.rect_changed_at = Instant::now();
        st.last_drag_at = Some(Instant::now());
        ctrl.cv.notify_all();
    }
}

pub fn set_paused(widget_hwnd: isize, paused: bool) {
    let coord = COORDINATOR.lock();
    let Some(&mon_key) = coord.widget_monitor.get(&widget_hwnd) else {
        return;
    };
    let Some(ctrl) = coord.workers.get(&mon_key).cloned() else {
        return;
    };
    drop(coord);

    let mut st = ctrl.state.lock();
    if let Some(t) = st.targets.get_mut(&widget_hwnd) {
        if t.spec.paused != paused {
            t.spec.paused = paused;
            if !paused {
                t.needs_redraw = true;
                t.rect_changed_at = Instant::now();
                st.last_drag_at = Some(Instant::now());
            }
            ctrl.cv.notify_all();
        }
    }
}

pub fn request_redraw(widget_hwnd: isize) {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let coord = COORDINATOR.lock();
    let Some(&mon_key) = coord.widget_monitor.get(&widget_hwnd) else {
        return;
    };
    let Some(ctrl) = coord.workers.get(&mon_key).cloned() else {
        return;
    };
    drop(coord);

    let now = Instant::now();
    let mut st = ctrl.state.lock();
    if let Some(t) = st.targets.get_mut(&widget_hwnd) {
        t.needs_redraw = true;
        t.first_drawn = false;
        t.rect_changed_at = now;
        st.last_drag_at = Some(now);
        ctrl.cv.notify_all();
    }
}

pub fn unregister(widget_hwnd: isize) {
    LATEST_GEN.lock().remove(&widget_hwnd);
    let mut coord = COORDINATOR.lock();
    remove_widget_from_worker_locked(&mut coord, widget_hwnd);
}

pub fn notify_display_change() {
    let workers: Vec<Arc<WorkerControl>> = {
        let mut coord = COORDINATOR.lock();
        coord.monitors = enumerate_dxgi_monitor_keys();
        coord.workers.values().cloned().collect()
    };
    for w in workers {
        let mut st = w.state.lock();
        st.display_dirty = true;
        st.color_dirty = true;
        w.cv.notify_all();
    }
}

pub fn notify_color_change() {
    let workers: Vec<Arc<WorkerControl>> = {
        let coord = COORDINATOR.lock();
        coord.workers.values().cloned().collect()
    };
    for w in workers {
        let mut st = w.state.lock();
        st.color_dirty = true;
        w.cv.notify_all();
    }
}

pub fn shutdown_all() {
    SHUTTING_DOWN.store(true, Ordering::Release);
    LATEST_GEN.lock().clear();

    let workers: Vec<Arc<WorkerControl>> = {
        let mut coord = COORDINATOR.lock();
        coord.widget_monitor.clear();
        coord.monitors.clear();
        coord.workers.drain().map(|(_, w)| w).collect()
    };

    for w in &workers {
        let mut st = w.state.lock();
        st.shutdown = true;
        for (_, mut target) in st.targets.drain() {
            if let Some(tx) = target.pending_reply.take() {
                let _ = tx.send(Err("Capture lukker ned".into()));
            }
        }
        w.cv.notify_all();
    }

    for w in workers {
        if let Some(handle) = w.thread.lock().take() {
            let _ = handle.join();
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq)]
struct MonitorColorInfo {
    hdr_enabled: bool,
    sdr_white_scale: f32,
}

fn utf16_z_slice(buf: &[u16]) -> &[u16] {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    &buf[..end]
}

fn device_name_display(buf: &[u16]) -> String {
    String::from_utf16_lossy(utf16_z_slice(buf))
}

pub fn sdr_white_scale_from_raw(raw: u32) -> f32 {
    if raw == 0 {
        1.0
    } else {
        (raw as f32 / 1000.0).clamp(1.0, 20.0)
    }
}

fn query_monitor_color_info(
    device_name: &[u16; 32],
    dxgi_output: &IDXGIOutput1,
) -> MonitorColorInfo {
    let dxgi_hdr = dxgi_output
        .cast::<IDXGIOutput6>()
        .ok()
        .and_then(|out6| unsafe { out6.GetDesc1() }.ok())
        .map(|d1| {
            matches!(
                d1.ColorSpace,
                DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020
                    | DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709
            )
        })
        .unwrap_or(false);

    let target_gdi = utf16_z_slice(device_name);
    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = Vec::new();
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = Vec::new();
    let mut queried = false;

    for _ in 0..3 {
        let mut path_count = 0_u32;
        let mut mode_count = 0_u32;
        let sz_rc = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        if sz_rc != ERROR_SUCCESS || path_count == 0 {
            break;
        }
        paths.resize(path_count as usize, DISPLAYCONFIG_PATH_INFO::default());
        modes.resize(mode_count as usize, DISPLAYCONFIG_MODE_INFO::default());
        let q_rc = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };
        if q_rc == ERROR_SUCCESS {
            paths.truncate(path_count as usize);
            queried = true;
            break;
        }
        if q_rc != ERROR_INSUFFICIENT_BUFFER {
            break;
        }
    }

    if queried && !target_gdi.is_empty() {
        for path in &paths {
            let mut sn = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            let rc_sn = unsafe { DisplayConfigGetDeviceInfo(&mut sn.header) };
            if rc_sn != ERROR_SUCCESS.0 as i32 {
                continue;
            }
            if utf16_z_slice(&sn.viewGdiDeviceName) != target_gdi {
                continue;
            }

            let mut ac = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                    size: size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            let ccd_hdr = if unsafe { DisplayConfigGetDeviceInfo(&mut ac.header) }
                == ERROR_SUCCESS.0 as i32
            {
                (unsafe { ac.Anonymous.value } & 0x2) != 0
            } else {
                false
            };

            let mut wl = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size: size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            let sdr_white_scale = if unsafe { DisplayConfigGetDeviceInfo(&mut wl.header) }
                == ERROR_SUCCESS.0 as i32
            {
                sdr_white_scale_from_raw(wl.SDRWhiteLevel)
            } else {
                1.0
            };

            return MonitorColorInfo {
                hdr_enabled: ccd_hdr || dxgi_hdr,
                sdr_white_scale,
            };
        }
    }

    MonitorColorInfo {
        hdr_enabled: dxgi_hdr,
        sdr_white_scale: 1.0,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HdrToSdrParams {
    inv_sdr_scale: f32,
    _pad: [f32; 3],
}

const HDR_TO_SDR_HLSL: &str = r#"
Texture2D<float4> srcTex : register(t0);

cbuffer Params : register(b0) {
    float invSdrScale;
    float _pad0;
    float _pad1;
    float _pad2;
};

float4 VSMain(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float linearToSrgb(float c) {
    c = saturate(c);
    return (c <= 0.0031308) ? (c * 12.92) : (1.055 * pow(max(c, 0.0031308), 1.0 / 2.4) - 0.055);
}

float4 PSMain(float4 pos : SV_Position) : SV_Target {
    float3 lin = srcTex.Load(int3(int2(pos.xy), 0)).rgb * invSdrScale;
    return float4(linearToSrgb(lin.r), linearToSrgb(lin.g), linearToSrgb(lin.b), 1.0);
}
"#;

struct HdrToSdrConverter {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cbuf: ID3D11Buffer,
    raster_state: ID3D11RasterizerState,
}

impl HdrToSdrConverter {
    fn new(device: &ID3D11Device) -> Result<Self, String> {
        let vs_bytes = compile_hlsl(
            HDR_TO_SDR_HLSL,
            b"VSMain\0",
            b"vs_5_0\0",
            b"hdr_to_sdr.hlsl\0",
        )
        .map_err(|e| format!("HDR VS kompilering: {e:?}"))?;
        let ps_bytes = compile_hlsl(
            HDR_TO_SDR_HLSL,
            b"PSMain\0",
            b"ps_5_0\0",
            b"hdr_to_sdr.hlsl\0",
        )
        .map_err(|e| format!("HDR PS kompilering: {e:?}"))?;

        let mut vs: Option<ID3D11VertexShader> = None;
        let mut ps: Option<ID3D11PixelShader> = None;
        unsafe {
            device
                .CreateVertexShader(&vs_bytes, None, Some(&mut vs))
                .map_err(|e| format!("HDR CreateVertexShader: {e}"))?;
            device
                .CreatePixelShader(&ps_bytes, None, Some(&mut ps))
                .map_err(|e| format!("HDR CreatePixelShader: {e}"))?;
        }
        let vs = vs.ok_or_else(|| "HDR vertex shader returnerede None".to_string())?;
        let ps = ps.ok_or_else(|| "HDR pixel shader returnerede None".to_string())?;

        let cbuf_desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of::<HdrToSdrParams>() as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut cbuf: Option<ID3D11Buffer> = None;
        unsafe {
            device
                .CreateBuffer(&cbuf_desc, None, Some(&mut cbuf))
                .map_err(|e| format!("HDR CreateBuffer: {e}"))?;
        }
        let cbuf = cbuf.ok_or_else(|| "HDR cbuf returnerede None".to_string())?;

        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: false.into(),
            ScissorEnable: false.into(),
            ..Default::default()
        };
        let mut raster_state: Option<ID3D11RasterizerState> = None;
        unsafe {
            device
                .CreateRasterizerState(&raster_desc, Some(&mut raster_state))
                .map_err(|e| format!("HDR CreateRasterizerState: {e}"))?;
        }
        let raster_state =
            raster_state.ok_or_else(|| "HDR raster_state returnerede None".to_string())?;

        Ok(Self {
            vs,
            ps,
            cbuf,
            raster_state,
        })
    }

    fn convert(
        &self,
        ctx: &ID3D11DeviceContext,
        srv: &ID3D11ShaderResourceView,
        rtv: &ID3D11RenderTargetView,
        width: u32,
        height: u32,
        inv_sdr_scale: f32,
    ) -> Result<(), String> {
        let params = HdrToSdrParams {
            inv_sdr_scale,
            _pad: [0.0; 3],
        };
        let cbuf_res: ID3D11Resource = self
            .cbuf
            .cast()
            .map_err(|e| format!("HDR cbuf ID3D11Resource cast: {e}"))?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            ctx.Map(&cbuf_res, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .map_err(|e| format!("HDR cbuf Map: {e}"))?;
            if !mapped.pData.is_null() {
                ptr::copy_nonoverlapping(&params, mapped.pData as *mut HdrToSdrParams, 1);
            }
            ctx.Unmap(&cbuf_res, 0);
        }

        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width.max(1) as f32,
            Height: height.max(1) as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };

        unsafe {
            // Direct2D deler immediate context med D3D11 og efterlader egen pipeline-state.
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.GSSetShader(None, None);
            ctx.HSSetShader(None, None);
            ctx.DSSetShader(None, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            ctx.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
            ctx.RSSetState(&self.raster_state);
            ctx.RSSetViewports(Some(&[vp]));
            ctx.OMSetBlendState(None, None, 0xFFFF_FFFF);
            ctx.OMSetDepthStencilState(None, 0);
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            ctx.Draw(3, 0);
            ctx.OMSetRenderTargets(None, None);
            ctx.PSSetShaderResources(0, Some(&[None]));
        }

        Ok(())
    }
}

struct HdrInputCache {
    tex: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
}

fn ensure_hdr_input_cache<'a>(
    d3d_device: &ID3D11Device,
    slot: &'a mut Option<HdrInputCache>,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
) -> Result<&'a HdrInputCache, String> {
    let matches = slot
        .as_ref()
        .map(|c| c.width == width && c.height == height && c.format == format)
        .unwrap_or(false);
    if !matches {
        let tex_desc = D3D11_TEXTURE2D_DESC {
            Width: width.max(1),
            Height: height.max(1),
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex: Option<ID3D11Texture2D> = None;
        unsafe {
            d3d_device
                .CreateTexture2D(&tex_desc, None, Some(&mut tex))
                .map_err(|e| format!("CreateTexture2D HDR cache: {e}"))?;
        }
        let tex = tex.ok_or_else(|| "CreateTexture2D HDR cache returnerede None".to_string())?;
        let res: ID3D11Resource = tex
            .cast()
            .map_err(|e| format!("HDR cache ID3D11Resource cast: {e}"))?;
        let mut srv: Option<ID3D11ShaderResourceView> = None;
        unsafe {
            d3d_device
                .CreateShaderResourceView(&res, None, Some(&mut srv))
                .map_err(|e| format!("CreateShaderResourceView HDR cache: {e}"))?;
        }
        let srv = srv.ok_or_else(|| "CreateShaderResourceView returnerede None".to_string())?;
        *slot = Some(HdrInputCache {
            tex,
            srv,
            width,
            height,
            format,
        });
    }
    slot.as_ref()
        .ok_or_else(|| "HDR input cache ikke initialiseret".to_string())
}

struct MonitorGpuPipeline {
    mon_rect: RECT,
    device_name: [u16; 32],
    color_info: MonitorColorInfo,
    d3d_device: ID3D11Device,
    d3d_context: ID3D11DeviceContext,
    dxgi_factory: IDXGIFactory2,
    dxgi_output: IDXGIOutput1,
    dc: ID2D1DeviceContext,
    desktop_tex: ID3D11Texture2D,
    desktop_tex_desc: D3D11_TEXTURE2D_DESC,
    desktop_rtv: ID3D11RenderTargetView,
    desktop_bitmap: ID2D1Bitmap1,
    hdr_converter: Option<HdrToSdrConverter>,
    hdr_input: Option<HdrInputCache>,
    last_frame_was_hdr: bool,
    logged_format: Option<DXGI_FORMAT>,
    content_dirty: bool,
    last_ingest_format: Option<DXGI_FORMAT>,
    color_matrix_fx: ID2D1Effect,
    _border_fx: ID2D1Effect,
    blur_fx: ID2D1Effect,
    sat_fx: ID2D1Effect,
}

fn find_adapter_and_output(
    target_key: MonitorKey,
) -> Result<(IDXGIAdapter1, IDXGIOutput1, RECT, [u16; 32]), String> {
    let factory: IDXGIFactory2 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
    let mut best: Option<(IDXGIAdapter1, IDXGIOutput1, RECT, [u16; 32], i64)> = None;
    let target_rect = RECT::from(target_key);

    let mut a_idx = 0_u32;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(a_idx) } {
        a_idx += 1;
        let mut o_idx = 0_u32;
        while let Ok(output) = unsafe { adapter.EnumOutputs(o_idx) } {
            o_idx += 1;
            let Ok(desc) = (unsafe { output.GetDesc() }) else {
                continue;
            };
            if !desc.AttachedToDesktop.as_bool() {
                continue;
            }
            let r = desc.DesktopCoordinates;
            if r.right <= r.left || r.bottom <= r.top {
                continue;
            }
            let Ok(output1) = output.cast::<IDXGIOutput1>() else {
                continue;
            };
            if r == target_rect {
                return Ok((adapter, output1, r, desc.DeviceName));
            }
            let ix0 = target_rect.left.max(r.left);
            let iy0 = target_rect.top.max(r.top);
            let ix1 = target_rect.right.min(r.right);
            let iy1 = target_rect.bottom.min(r.bottom);
            let area = ((ix1 - ix0).max(0) as i64) * ((iy1 - iy0).max(0) as i64);
            let better = best.as_ref().map(|b| area > b.4).unwrap_or(true);
            if better {
                best = Some((adapter.clone(), output1, r, desc.DeviceName, area));
            }
        }
    }

    best.map(|(a, o, r, name, _)| (a, o, r, name))
        .ok_or_else(|| "Kunne ikke finde DXGI-skærmoutput til capture".to_string())
}

fn create_desktop_texture_and_bitmap(
    d3d_device: &ID3D11Device,
    dc: &ID2D1DeviceContext,
    width: u32,
    height: u32,
) -> Result<
    (
        ID3D11Texture2D,
        D3D11_TEXTURE2D_DESC,
        ID3D11RenderTargetView,
        ID2D1Bitmap1,
    ),
    String,
> {
    let tex_desc = D3D11_TEXTURE2D_DESC {
        Width: width.max(1),
        Height: height.max(1),
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex: Option<ID3D11Texture2D> = None;
    unsafe {
        d3d_device
            .CreateTexture2D(&tex_desc, None, Some(&mut tex))
            .map_err(|e| format!("CreateTexture2D desktop cache: {e}"))?;
    }
    let tex = tex.ok_or_else(|| "CreateTexture2D returnerede None".to_string())?;
    let res: ID3D11Resource = tex
        .cast()
        .map_err(|e| format!("desktop ID3D11Resource cast: {e}"))?;
    let mut rtv: Option<ID3D11RenderTargetView> = None;
    unsafe {
        d3d_device
            .CreateRenderTargetView(&res, None, Some(&mut rtv))
            .map_err(|e| format!("CreateRenderTargetView desktop cache: {e}"))?;
    }
    let rtv = rtv.ok_or_else(|| "CreateRenderTargetView returnerede None".to_string())?;
    let surface: IDXGISurface = tex
        .cast()
        .map_err(|e| format!("desktop IDXGISurface cast: {e}"))?;
    let bmp_props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        colorContext: ManuallyDrop::new(None),
    };
    let bmp = unsafe { dc.CreateBitmapFromDxgiSurface(&surface, Some(&bmp_props)) }
        .map_err(|e| format!("CreateBitmapFromDxgiSurface desktop: {e}"))?;
    Ok((tex, tex_desc, rtv, bmp))
}

impl MonitorGpuPipeline {
    fn create(mon_key: MonitorKey) -> Result<Self, String> {
        let (adapter, dxgi_output, mon_rect, device_name) = find_adapter_and_output(mon_key)?;
        let color_info = query_monitor_color_info(&device_name, &dxgi_output);
        let base_adapter: IDXGIAdapter = adapter
            .cast()
            .map_err(|e| format!("IDXGIAdapter cast: {e}"))?;
        let levels = [D3D_FEATURE_LEVEL_11_0];
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        unsafe {
            D3D11CreateDevice(
                &base_adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
        }
        let d3d_device = device.ok_or_else(|| "D3D11Device var None".to_string())?;
        let d3d_context = context.ok_or_else(|| "D3D11DeviceContext var None".to_string())?;

        let dxgi_device: IDXGIDevice = d3d_device
            .cast()
            .map_err(|e| format!("IDXGIDevice cast: {e}"))?;
        let dev_adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter() }
            .map_err(|e| format!("dxgi_device.GetAdapter: {e}"))?;
        let dxgi_factory: IDXGIFactory2 = unsafe { dev_adapter.GetParent() }
            .map_err(|e| format!("adapter.GetParent(IDXGIFactory2): {e}"))?;

        let d2d_factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None) }
                .map_err(|e| format!("D2D1CreateFactory: {e}"))?;
        let d2d_device: ID2D1Device = unsafe { d2d_factory.CreateDevice(&dxgi_device) }
            .map_err(|e| format!("d2d_factory.CreateDevice: {e}"))?;
        let dc: ID2D1DeviceContext =
            unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }
                .map_err(|e| format!("CreateDeviceContext: {e}"))?;
        unsafe {
            dc.SetDpi(96.0, 96.0);
        }

        let (mon_w, mon_h) = rect_dims(mon_rect);
        let (desktop_tex, desktop_tex_desc, desktop_rtv, desktop_bitmap) =
            create_desktop_texture_and_bitmap(&d3d_device, &dc, mon_w, mon_h)?;

        const FORCE_OPAQUE_ALPHA_MATRIX: [f32; 20] = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        let mut matrix_bytes = [0u8; 80];
        for (i, f) in FORCE_OPAQUE_ALPHA_MATRIX.iter().enumerate() {
            matrix_bytes[i * 4..(i + 1) * 4].copy_from_slice(&f.to_ne_bytes());
        }

        let color_matrix_fx = unsafe { dc.CreateEffect(&CLSID_D2D1ColorMatrix) }
            .map_err(|e| format!("CreateEffect(ColorMatrix): {e}"))?;
        unsafe {
            color_matrix_fx
                .SetValue(
                    D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32,
                    D2D1_PROPERTY_TYPE_UNKNOWN,
                    &matrix_bytes,
                )
                .map_err(|e| format!("ColorMatrix matrix: {e}"))?;
            color_matrix_fx
                .SetValue(
                    D2D1_COLORMATRIX_PROP_ALPHA_MODE.0 as u32,
                    D2D1_PROPERTY_TYPE_UNKNOWN,
                    &(D2D1_COLORMATRIX_ALPHA_MODE_STRAIGHT.0 as u32).to_ne_bytes(),
                )
                .map_err(|e| format!("ColorMatrix alpha mode: {e}"))?;
            color_matrix_fx.SetInput(0, &desktop_bitmap, true);
        }

        let border_fx = unsafe { dc.CreateEffect(&CLSID_D2D1Border) }
            .map_err(|e| format!("CreateEffect(Border): {e}"))?;
        let clamp = (D2D1_BORDER_EDGE_MODE_CLAMP.0 as u32).to_ne_bytes();
        unsafe {
            border_fx
                .SetValue(
                    D2D1_BORDER_PROP_EDGE_MODE_X.0 as u32,
                    D2D1_PROPERTY_TYPE_UNKNOWN,
                    &clamp,
                )
                .map_err(|e| format!("Border EDGE_MODE_X: {e}"))?;
            border_fx
                .SetValue(
                    D2D1_BORDER_PROP_EDGE_MODE_Y.0 as u32,
                    D2D1_PROPERTY_TYPE_UNKNOWN,
                    &clamp,
                )
                .map_err(|e| format!("Border EDGE_MODE_Y: {e}"))?;
            let cm_out = color_matrix_fx
                .GetOutput()
                .map_err(|e| format!("ColorMatrix GetOutput: {e}"))?;
            border_fx.SetInput(0, &cm_out, true);
        }

        let blur_fx = unsafe { dc.CreateEffect(&CLSID_D2D1GaussianBlur) }
            .map_err(|e| format!("CreateEffect(GaussianBlur): {e}"))?;
        unsafe {
            blur_fx
                .SetValue(
                    D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION.0 as u32,
                    D2D1_PROPERTY_TYPE_UNKNOWN,
                    &(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32).to_ne_bytes(),
                )
                .map_err(|e| format!("Blur optimization: {e}"))?;
            blur_fx
                .SetValue(
                    D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32,
                    D2D1_PROPERTY_TYPE_UNKNOWN,
                    &(D2D1_BORDER_MODE_SOFT.0 as u32).to_ne_bytes(),
                )
                .map_err(|e| format!("Blur border mode: {e}"))?;
            let border_out = border_fx
                .GetOutput()
                .map_err(|e| format!("Border GetOutput: {e}"))?;
            blur_fx.SetInput(0, &border_out, true);
        }

        let sat_fx = unsafe { dc.CreateEffect(&CLSID_D2D1Saturation) }
            .map_err(|e| format!("CreateEffect(Saturation): {e}"))?;
        unsafe {
            let blur_out = blur_fx
                .GetOutput()
                .map_err(|e| format!("Blur GetOutput: {e}"))?;
            sat_fx.SetInput(0, &blur_out, true);
        }

        Ok(Self {
            mon_rect,
            device_name,
            color_info,
            d3d_device,
            d3d_context,
            dxgi_factory,
            dxgi_output,
            dc,
            desktop_tex,
            desktop_tex_desc,
            desktop_rtv,
            desktop_bitmap,
            hdr_converter: None,
            hdr_input: None,
            last_frame_was_hdr: false,
            logged_format: None,
            content_dirty: false,
            last_ingest_format: None,
            color_matrix_fx,
            _border_fx: border_fx,
            blur_fx,
            sat_fx,
        })
    }

    fn ensure_desktop_dims(&mut self, width: u32, height: u32) -> Result<(), String> {
        let w = width.max(1);
        let h = height.max(1);
        if w == self.desktop_tex_desc.Width && h == self.desktop_tex_desc.Height {
            return Ok(());
        }
        let (tex, desc, rtv, bmp) =
            create_desktop_texture_and_bitmap(&self.d3d_device, &self.dc, w, h)?;
        self.desktop_tex = tex;
        self.desktop_tex_desc = desc;
        self.desktop_rtv = rtv;
        self.desktop_bitmap = bmp;
        unsafe {
            self.color_matrix_fx.SetInput(0, &self.desktop_bitmap, true);
        }
        Ok(())
    }

    fn acquire_duplication(&mut self) -> Result<IDXGIOutputDuplication, windows_core::Error> {
        self.color_info = query_monitor_color_info(&self.device_name, &self.dxgi_output);
        log::info!(
            "CaptureD2d duplikering aktiv for {} (hdr={}, sdr_scale={:.3}x)",
            device_name_display(&self.device_name),
            self.color_info.hdr_enabled,
            self.color_info.sdr_white_scale
        );
        if let Ok(output5) = self.dxgi_output.cast::<IDXGIOutput5>() {
            let formats = if self.color_info.hdr_enabled {
                [DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_B8G8R8A8_UNORM]
            } else {
                [DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT]
            };
            unsafe { output5.DuplicateOutput1(&self.d3d_device, 0, &formats) }
        } else {
            unsafe { self.dxgi_output.DuplicateOutput(&self.d3d_device) }
        }
    }

    fn copy_frame_regions(
        &self,
        dst: &ID3D11Texture2D,
        src: &ID3D11Texture2D,
        dirty_rects: &[RECT],
        force_full: bool,
    ) -> bool {
        if force_full {
            unsafe {
                self.d3d_context.CopyResource(dst, src);
            }
            return true;
        }
        let mut copied = false;
        for r in regions_to_copy(
            dirty_rects,
            self.desktop_tex_desc.Width,
            self.desktop_tex_desc.Height,
        ) {
            let src_box = D3D11_BOX {
                left: r.left as u32,
                top: r.top as u32,
                front: 0,
                right: r.right as u32,
                bottom: r.bottom as u32,
                back: 1,
            };
            unsafe {
                self.d3d_context.CopySubresourceRegion(
                    dst,
                    0,
                    r.left as u32,
                    r.top as u32,
                    0,
                    src,
                    0,
                    Some(&src_box as *const D3D11_BOX),
                );
            }
            copied = true;
        }
        copied
    }

    fn ingest_desktop_frame(
        &mut self,
        tex: &ID3D11Texture2D,
        desc: &D3D11_TEXTURE2D_DESC,
        dirty_rects: &[RECT],
        force_full: bool,
    ) -> Result<(), String> {
        let dims_changed = desc.Width != self.desktop_tex_desc.Width
            || desc.Height != self.desktop_tex_desc.Height;
        self.ensure_desktop_dims(desc.Width, desc.Height)?;
        let format_changed = self.last_ingest_format != Some(desc.Format);
        let full = force_full || dims_changed || format_changed;

        if self.logged_format != Some(desc.Format) {
            self.logged_format = Some(desc.Format);
            log::info!(
                "CaptureD2d desktop-format: {:?} ({}x{}, hdr_convert={})",
                desc.Format,
                desc.Width,
                desc.Height,
                desc.Format == DXGI_FORMAT_R16G16B16A16_FLOAT
            );
        }

        let copied = if desc.Format == DXGI_FORMAT_R16G16B16A16_FLOAT {
            if self.hdr_converter.is_none() {
                self.hdr_converter = Some(HdrToSdrConverter::new(&self.d3d_device)?);
            }
            let cache_fresh = self
                .hdr_input
                .as_ref()
                .map(|c| {
                    c.width == desc.Width && c.height == desc.Height && c.format == desc.Format
                })
                .unwrap_or(false);
            let input_tex = ensure_hdr_input_cache(
                &self.d3d_device,
                &mut self.hdr_input,
                desc.Width,
                desc.Height,
                desc.Format,
            )?
            .tex
            .clone();
            self.last_frame_was_hdr = true;
            self.copy_frame_regions(&input_tex, tex, dirty_rects, full || !cache_fresh)
        } else if desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM {
            self.last_frame_was_hdr = false;
            self.copy_frame_regions(&self.desktop_tex, tex, dirty_rects, full)
        } else {
            return Err(format!("Ikke-understøttet capture-format: {:?}", desc.Format));
        };

        self.last_ingest_format = Some(desc.Format);
        if copied {
            self.content_dirty = true;
        }
        Ok(())
    }

    // Før hver draw: konverter HDR-cache -> desktop_tex (fuld frame) og
    // invalider D2D-grafens input — kun når nyt indhold er ankommet.
    fn prepare_for_draw(&mut self) {
        if !self.content_dirty {
            return;
        }
        if self.last_frame_was_hdr {
            if let (Some(ref conv), Some(ref cache)) = (&self.hdr_converter, &self.hdr_input) {
                let inv_scale = 1.0 / self.color_info.sdr_white_scale.max(1.0);
                if let Err(err) = conv.convert(
                    &self.d3d_context,
                    &cache.srv,
                    &self.desktop_rtv,
                    cache.width,
                    cache.height,
                    inv_scale,
                ) {
                    log::warn!("CaptureD2d HDR-konvertering før tegning fejlede: {err}");
                    return;
                }
            }
        }
        self.content_dirty = false;
        unsafe {
            self.color_matrix_fx.SetInput(0, &self.desktop_bitmap, true);
        }
    }

    fn reconvert_cached_hdr(&mut self) -> Result<bool, String> {
        if !self.last_frame_was_hdr || self.hdr_input.is_none() {
            return Ok(false);
        }
        self.content_dirty = true;
        Ok(true)
    }

    fn create_widget_swapchain(
        &self,
        phys_w: u32,
        phys_h: u32,
    ) -> Result<IDXGISwapChain1, windows_core::Error> {
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: phys_w.max(1),
            Height: phys_h.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        unsafe {
            self.dxgi_factory
                .CreateSwapChainForComposition(&self.d3d_device, &desc, None)
        }
    }

    fn draw_target(
        &self,
        spec: &CaptureTargetSpec,
        phys_w: u32,
        phys_h: u32,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<(), HRESULT> {
        let back_surface: IDXGISurface = unsafe { swap_chain.GetBuffer(0) }.map_err(|e| e.code())?;
        let bmp_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: ManuallyDrop::new(None),
        };
        let target_bitmap =
            unsafe { self.dc.CreateBitmapFromDxgiSurface(&back_surface, Some(&bmp_props)) }
                .map_err(|e| e.code())?;

        let sigma = spec.sigma.clamp(0.0, 200.0);
        unsafe {
            self.blur_fx
                .SetValue(
                    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                    D2D1_PROPERTY_TYPE_UNKNOWN,
                    &sigma.to_ne_bytes(),
                )
                .map_err(|e| e.code())?;
        }

        let output_image = if (spec.sat - 1.0).abs() > 0.005 {
            unsafe {
                self.sat_fx
                    .SetValue(
                        D2D1_SATURATION_PROP_SATURATION.0 as u32,
                        D2D1_PROPERTY_TYPE_UNKNOWN,
                        &spec.sat.to_ne_bytes(),
                    )
                    .map_err(|e| e.code())?;
                self.sat_fx.GetOutput().map_err(|e| e.code())?
            }
        } else {
            unsafe { self.blur_fx.GetOutput().map_err(|e| e.code())? }
        };

        let rel_x = (spec.rect.left - self.mon_rect.left) as f32;
        let rel_y = (spec.rect.top - self.mon_rect.top) as f32;
        let src_rect = D2D_RECT_F {
            left: rel_x,
            top: rel_y,
            right: rel_x + phys_w as f32,
            bottom: rel_y + phys_h as f32,
        };
        let origin = Vector2::new(0.0, 0.0);

        unsafe {
            self.dc.SetTarget(&target_bitmap);
            self.dc.BeginDraw();
            self.dc.Clear(None);
            self.dc.DrawImage(
                &output_image,
                Some(&origin),
                Some(&src_rect),
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
            let end_res = self.dc.EndDraw(None, None);
            self.dc.SetTarget(None);
            end_res.map_err(|e| e.code())?;
            swap_chain
                .Present(0, DXGI_PRESENT(0))
                .ok()
                .map_err(|e| e.code())?;
        }
        Ok(())
    }
}

fn should_log_draw_error(last: &mut Option<(HRESULT, Instant)>, hr: HRESULT) -> bool {
    let now = Instant::now();
    let log_it = match *last {
        None => true,
        Some((prev_hr, at)) => {
            prev_hr != hr || now.saturating_duration_since(at) > Duration::from_secs(5)
        }
    };
    if log_it {
        *last = Some((hr, now));
    }
    log_it
}

fn is_device_reset_hr(hr: HRESULT) -> bool {
    hr == DXGI_ERROR_DEVICE_REMOVED
        || hr == DXGI_ERROR_DEVICE_RESET
        || hr == D2DERR_RECREATE_TARGET
}

fn is_duplication_lost_hr(hr: HRESULT) -> bool {
    hr == DXGI_ERROR_ACCESS_LOST
        || hr == E_ACCESSDENIED
        || hr == DXGI_ERROR_SESSION_DISCONNECTED
        || hr == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE
        || hr == DXGI_ERROR_DEVICE_REMOVED
        || hr == DXGI_ERROR_DEVICE_RESET
}

fn post_swapchain_to_ui(widget_hwnd: isize, gen_id: u64, swap_chain: IDXGISwapChain1) {
    if !is_generation_current(widget_hwnd, gen_id) {
        return;
    }
    if let Some(app) = super::app_handle() {
        let sc = SendSwapChain(swap_chain);
        let _ = app.run_on_main_thread(move || {
            super::backdrop::attach_capture_swapchain(HWND(widget_hwnd as _), gen_id, sc);
        });
    }
}

fn fail_pending_replies(ctrl: &WorkerControl, msg: &str) {
    let mut st = ctrl.state.lock();
    for t in st.targets.values_mut() {
        if let Some(tx) = t.pending_reply.take() {
            let _ = tx.send(Err(msg.to_string()));
        }
    }
}

fn remove_worker_if_same(ctrl: &Arc<WorkerControl>) {
    let mut coord = COORDINATOR.lock();
    let matches = coord
        .workers
        .get(&ctrl.mon_key)
        .map(|cur| Arc::ptr_eq(cur, ctrl))
        .unwrap_or(false);
    if matches {
        coord.workers.remove(&ctrl.mon_key);
    }
}

fn target_min_interval(rect_changed_at: Instant, now: Instant) -> Duration {
    if now.saturating_duration_since(rect_changed_at) < Duration::from_millis(120) {
        Duration::from_millis(8)
    } else {
        Duration::from_millis(16)
    }
}

fn acquire_min_interval(
    drag_recent: bool,
    last_rect_change: Option<Instant>,
    now: Instant,
) -> Duration {
    if drag_recent {
        return Duration::from_millis(8);
    }
    match last_rect_change {
        Some(at) => target_min_interval(at, now),
        None => Duration::from_millis(16),
    }
}

fn redraw_wait_ms(
    first_drawn: bool,
    last_redraw_at: Option<Instant>,
    min_interval: Duration,
    now: Instant,
) -> u32 {
    if !first_drawn {
        return 0;
    }
    let Some(last) = last_redraw_at else {
        return 0;
    };
    let elapsed = now.saturating_duration_since(last);
    if elapsed >= min_interval {
        0
    } else {
        let rem = min_interval - elapsed;
        let ceil_ms = (rem.as_nanos() + 999_999) / 1_000_000;
        ceil_ms.clamp(1, 16) as u32
    }
}

fn monitor_worker_loop(ctrl: Arc<WorkerControl>) {
    let _com = crate::util::ComMtaGuard::init();

    let mut pipeline = match MonitorGpuPipeline::create(ctrl.mon_key) {
        Ok(p) => p,
        Err(err) => {
            log::warn!("Live capture pipeline kunne ikke oprettes: {err}");
            fail_pending_replies(&ctrl, &err);
            remove_worker_if_same(&ctrl);
            return;
        }
    };

    let mut duplication: Option<IDXGIOutputDuplication> = None;
    let mut has_desktop_frame = false;
    let mut move_buf: Vec<DXGI_OUTDUPL_MOVE_RECT> = Vec::with_capacity(16);
    let mut dirty_buf: Vec<RECT> = Vec::with_capacity(64);
    let mut last_desktop_activity = Instant::now() - Duration::from_secs(10);
    let mut last_color_poll = Instant::now();
    let mut initial_acquire_attempted = false;
    let mut last_acquire_at: Option<Instant> = None;

    'outer: loop {
        // 1. Check lifecycle, pause, and display/color changes.
        let (
            need_rebuild,
            color_dirty,
            any_need_swapchain,
            all_paused,
            min_redraw_wait_ms,
            drag_recent,
            latest_rect_change,
        ) = {
            let mut st = ctrl.state.lock();
            if st.shutdown || st.targets.is_empty() {
                drop(st);
                break 'outer;
            }
            if st.display_dirty {
                st.display_dirty = false;
                st.color_dirty = false;
                (true, false, false, false, None, false, None)
            } else {
                let any_active = st.targets.values().any(|t| !t.spec.paused);
                let any_sc = st.targets.values().any(|t| t.needs_swapchain);
                if !any_active && !any_sc {
                    drop(duplication.take());
                    let _res = ctrl.cv.wait_while(&mut st, |s| {
                        !s.shutdown
                            && !s.display_dirty
                            && !s.targets.is_empty()
                            && !s.targets.values().any(|t| !t.spec.paused || t.needs_swapchain)
                    });
                    continue 'outer;
                }
                let color_dirty = st.color_dirty;
                st.color_dirty = false;
                let now = Instant::now();
                let min_wait = st
                    .targets
                    .values()
                    .filter(|t| !t.spec.paused && t.needs_redraw && t.swap_chain.is_some())
                    .map(|t| {
                        let min_int = target_min_interval(t.rect_changed_at, now);
                        redraw_wait_ms(t.first_drawn, t.last_redraw_at, min_int, now)
                    })
                    .min();
                let drag_recent = st
                    .last_drag_at
                    .map(|at| at.elapsed() < Duration::from_millis(450))
                    .unwrap_or(false);
                let latest_rect_change = st
                    .targets
                    .values()
                    .filter(|t| !t.spec.paused)
                    .map(|t| t.rect_changed_at)
                    .max();
                (
                    false,
                    color_dirty,
                    any_sc,
                    !any_active && !any_sc,
                    min_wait,
                    drag_recent,
                    latest_rect_change,
                )
            }
        };
        let _ = all_paused;

        if need_rebuild {
            drop(duplication.take());
            has_desktop_frame = false;
            initial_acquire_attempted = false;
            last_color_poll = Instant::now();
            match MonitorGpuPipeline::create(ctrl.mon_key) {
                Ok(new_p) => {
                    pipeline = new_p;
                    let mut st = ctrl.state.lock();
                    for (&hwnd, t) in st.targets.iter_mut() {
                        t.swap_chain = None;
                        t.swapchain_gen = bump_generation(hwnd);
                        t.needs_swapchain = true;
                        t.needs_redraw = true;
                        t.first_drawn = false;
                        t.logged_first_present = false;
                    }
                }
                Err(err) => {
                    log::warn!("Genopbygning af capture pipeline fejlede: {err}");
                    let mut st = ctrl.state.lock();
                    ctrl.cv.wait_for(&mut st, Duration::from_millis(400));
                }
            }
            continue 'outer;
        }

        let need_color_check = duplication.is_some()
            && (color_dirty
                || ((pipeline.last_frame_was_hdr || pipeline.color_info.hdr_enabled)
                    && last_color_poll.elapsed() >= Duration::from_secs(2)));
        if need_color_check {
            last_color_poll = Instant::now();
            let next_info = query_monitor_color_info(&pipeline.device_name, &pipeline.dxgi_output);
            if next_info.hdr_enabled != pipeline.color_info.hdr_enabled {
                log::info!(
                    "CaptureD2d HDR-tilstand ændret for {}: {} -> {}; genopretter duplikering",
                    device_name_display(&pipeline.device_name),
                    pipeline.color_info.hdr_enabled,
                    next_info.hdr_enabled
                );
                pipeline.color_info = next_info;
                drop(duplication.take());
                has_desktop_frame = false;
                continue 'outer;
            }
            if (next_info.sdr_white_scale - pipeline.color_info.sdr_white_scale).abs() > 0.005 {
                log::info!(
                    "CaptureD2d SDR-hvidniveau ændret for {}: {:.3}x -> {:.3}x",
                    device_name_display(&pipeline.device_name),
                    pipeline.color_info.sdr_white_scale,
                    next_info.sdr_white_scale
                );
                pipeline.color_info = next_info;
                match pipeline.reconvert_cached_hdr() {
                    Ok(true) => {
                        let mut st = ctrl.state.lock();
                        for t in st.targets.values_mut() {
                            if !t.spec.paused {
                                t.needs_redraw = true;
                            }
                        }
                    }
                    Ok(false) => {}
                    Err(err) => {
                        log::warn!("CaptureD2d HDR-genkonvertering fejlede: {err}");
                    }
                }
            }
        }

        // 2. Ensure IDXGIOutputDuplication is active.
        if duplication.is_none() {
            match pipeline.acquire_duplication() {
                Ok(d) => {
                    duplication = Some(d);
                    has_desktop_frame = false;
                    last_color_poll = Instant::now();
                }
                Err(err) => {
                    let hr = err.code();
                    if !initial_acquire_attempted {
                        let msg = format!("DuplicateOutput fejlede: {err}");
                        log::warn!("{msg}");
                        fail_pending_replies(&ctrl, &msg);
                        break 'outer;
                    }
                    if is_device_reset_hr(hr) {
                        let mut st = ctrl.state.lock();
                        st.display_dirty = true;
                        drop(st);
                        continue 'outer;
                    }
                    let mut st = ctrl.state.lock();
                    ctrl.cv.wait_for(&mut st, Duration::from_millis(350));
                    continue 'outer;
                }
            }
        }

        // 3. Acquire a desktop frame before creating first swapchains if we don't have one yet.
        let timeout_ms = if !has_desktop_frame {
            150
        } else if any_need_swapchain {
            0
        } else if let Some(wait_ms) = min_redraw_wait_ms {
            wait_ms
        } else if drag_recent || last_desktop_activity.elapsed() < Duration::from_millis(450) {
            16
        } else {
            33
        };

        let mut frame_error: Option<HRESULT> = None;
        let mut got_new_desktop = false;
        let mut all_dirty_mon: Vec<RECT> = Vec::new();
        let mut dirty_full_monitor = false;

        // DXGI akkumulerer dirty-rects paa tvaers af u-afhentede frames, saa
        // at begraense afhentningshastigheden mister ikke noget indhold.
        if has_desktop_frame && !any_need_swapchain {
            let min_int = acquire_min_interval(drag_recent, latest_rect_change, Instant::now());
            if let Some(last) = last_acquire_at {
                let rem = min_int.saturating_sub(last.elapsed());
                if !rem.is_zero() {
                    let mut st = ctrl.state.lock();
                    let _ = ctrl.cv.wait_for(&mut st, rem);
                    drop(st);
                    continue 'outer;
                }
            }
        }

        if let Some(ref dup) = duplication {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut desktop_res: Option<IDXGIResource> = None;
            match unsafe { dup.AcquireNextFrame(timeout_ms, &mut frame_info, &mut desktop_res) } {
                Ok(()) => {
                    initial_acquire_attempted = true;
                    last_acquire_at = Some(Instant::now());
                    let has_update = frame_info.LastPresentTime != 0 || !has_desktop_frame;
                    if has_update {
                        if frame_info.TotalMetadataBufferSize > 0 {
                            let move_cap = (frame_info.TotalMetadataBufferSize as usize
                                / size_of::<DXGI_OUTDUPL_MOVE_RECT>().max(1))
                            .max(8);
                            move_buf.resize(move_cap, DXGI_OUTDUPL_MOVE_RECT::default());
                            let mut move_bytes = 0_u32;
                            let move_buf_bytes =
                                (move_buf.len() * size_of::<DXGI_OUTDUPL_MOVE_RECT>()) as u32;
                            if unsafe {
                                dup.GetFrameMoveRects(
                                    move_buf_bytes,
                                    move_buf.as_mut_ptr(),
                                    &mut move_bytes,
                                )
                            }
                            .is_ok()
                            {
                                let cnt = move_bytes as usize / size_of::<DXGI_OUTDUPL_MOVE_RECT>();
                                for m in &move_buf[..cnt.min(move_buf.len())] {
                                    all_dirty_mon.push(m.DestinationRect);
                                }
                            } else {
                                dirty_full_monitor = true;
                            }

                            let dirty_cap = (frame_info.TotalMetadataBufferSize as usize
                                / size_of::<RECT>().max(1))
                            .max(16);
                            dirty_buf.resize(dirty_cap, RECT::default());
                            let mut dirty_bytes = 0_u32;
                            let dirty_buf_bytes = (dirty_buf.len() * size_of::<RECT>()) as u32;
                            if unsafe {
                                dup.GetFrameDirtyRects(
                                    dirty_buf_bytes,
                                    dirty_buf.as_mut_ptr(),
                                    &mut dirty_bytes,
                                )
                            }
                            .is_ok()
                            {
                                let cnt = dirty_bytes as usize / size_of::<RECT>();
                                all_dirty_mon.extend_from_slice(&dirty_buf[..cnt.min(dirty_buf.len())]);
                            } else {
                                dirty_full_monitor = true;
                            }
                        } else {
                            dirty_full_monitor = true;
                        }

                        if let Some(res) = desktop_res.take() {
                            if let Ok(tex) = res.cast::<ID3D11Texture2D>() {
                                let mut desc = D3D11_TEXTURE2D_DESC::default();
                                unsafe { tex.GetDesc(&mut desc) };
                                match pipeline.ingest_desktop_frame(
                                    &tex,
                                    &desc,
                                    &all_dirty_mon,
                                    dirty_full_monitor || !has_desktop_frame,
                                ) {
                                    Ok(()) => {
                                        has_desktop_frame = true;
                                        got_new_desktop = true;
                                        last_desktop_activity = Instant::now();
                                    }
                                    Err(err) => {
                                        log::warn!("Capture desktop-format opdatering fejlede: {err}");
                                    }
                                }
                            }
                        }
                    }
                    let _ = unsafe { dup.ReleaseFrame() };
                }
                Err(err) => {
                    let hr = err.code();
                    if hr != DXGI_ERROR_WAIT_TIMEOUT {
                        frame_error = Some(hr);
                    } else if !initial_acquire_attempted {
                        initial_acquire_attempted = true;
                    }
                }
            }
        }

        if let Some(hr) = frame_error {
            if is_device_reset_hr(hr) {
                log::warn!("Capture D3D11/DXGI enhed nulstillet ({hr:?}); genopbygger");
                let mut st = ctrl.state.lock();
                st.display_dirty = true;
                drop(st);
                continue 'outer;
            }
            if is_duplication_lost_hr(hr) {
                log::debug!("DXGI desktop-duplication tabt ({hr:?}); generhverver");
                drop(duplication.take());
                let mut st = ctrl.state.lock();
                ctrl.cv.wait_for(&mut st, Duration::from_millis(350));
                continue 'outer;
            }
        }

        // 4. Mark intersecting active targets dirty when desktop updated.
        if got_new_desktop {
            let mut st = ctrl.state.lock();
            for t in st.targets.values_mut() {
                if t.spec.paused {
                    continue;
                }
                if !t.first_drawn || dirty_full_monitor || all_dirty_mon.is_empty() {
                    t.needs_redraw = true;
                } else {
                    let inf = influence_rect(t.spec.rect, pipeline.mon_rect, t.spec.sigma);
                    if all_dirty_mon.iter().any(|d| rects_intersect(inf, *d)) {
                        t.needs_redraw = true;
                    }
                }
            }
        }

        // 5. Build any requested per-widget swapchains without holding the worker mutex.
        let pending_sc_jobs: Vec<(isize, u32, u32, u64)> = {
            let mut st = ctrl.state.lock();
            st.targets
                .iter_mut()
                .filter(|(_, t)| t.needs_swapchain)
                .map(|(&hwnd, t)| {
                    t.needs_swapchain = false;
                    (hwnd, t.phys_w, t.phys_h, t.swapchain_gen)
                })
                .collect()
        };

        for (hwnd, pw, ph, gen_id) in pending_sc_jobs {
            if !is_generation_current(hwnd, gen_id) {
                continue;
            }
            match pipeline.create_widget_swapchain(pw, ph) {
                Ok(sc) => {
                    let spec_snapshot = {
                        let st = ctrl.state.lock();
                        st.targets.get(&hwnd).map(|t| t.spec)
                    };
                    let mut initial_drawn = false;
                    let mut draw_hr: Option<HRESULT> = None;
                    if let Some(spec) = spec_snapshot {
                        if has_desktop_frame {
                            pipeline.prepare_for_draw();
                            match pipeline.draw_target(&spec, pw, ph, &sc) {
                                Ok(()) => initial_drawn = true,
                                Err(hr) => draw_hr = Some(hr),
                            }
                        }
                    }

                    if let Some(hr) = draw_hr {
                        if is_device_reset_hr(hr) {
                            let mut st = ctrl.state.lock();
                            st.display_dirty = true;
                            drop(st);
                            continue 'outer;
                        }
                        let mut st = ctrl.state.lock();
                        if let Some(t) = st.targets.get_mut(&hwnd) {
                            if should_log_draw_error(&mut t.last_draw_err, hr) {
                                log::warn!("CaptureD2d draw fejlede for widget {hwnd}: {hr:?}");
                            }
                        }
                    }

                    let reply_tx = {
                        let mut st = ctrl.state.lock();
                        if let Some(t) = st.targets.get_mut(&hwnd) {
                            if t.swapchain_gen == gen_id {
                                t.swap_chain = Some(sc.clone());
                                if initial_drawn {
                                    t.needs_redraw = false;
                                    t.first_drawn = true;
                                    t.last_draw_err = None;
                                    t.last_redraw_at = Some(Instant::now());
                                    if !t.logged_first_present {
                                        t.logged_first_present = true;
                                        log::info!(
                                            "CaptureD2d første frame tegnet ({pw}x{ph}) for widget {hwnd}"
                                        );
                                    }
                                } else {
                                    t.needs_redraw = true;
                                }
                                t.pending_reply.take()
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };

                    if let Some(tx) = reply_tx {
                        let _ = tx.send(Ok(SendSwapChain(sc)));
                    } else {
                        post_swapchain_to_ui(hwnd, gen_id, sc);
                    }
                }
                Err(err) => {
                    let hr = err.code();
                    let msg = format!("CreateSwapChainForComposition fejlede: {err}");
                    log::warn!("{msg}");
                    let reply_tx = {
                        let mut st = ctrl.state.lock();
                        st.targets
                            .get_mut(&hwnd)
                            .and_then(|t| t.pending_reply.take())
                    };
                    if let Some(tx) = reply_tx {
                        let _ = tx.send(Err(msg));
                    }
                    if is_device_reset_hr(hr) {
                        let mut st = ctrl.state.lock();
                        st.display_dirty = true;
                        drop(st);
                        continue 'outer;
                    }
                }
            }
        }

        // 6. Render any active targets whose backdrop changed (throttled: 120fps drag, 60fps desktop).
        if has_desktop_frame {
            let now = Instant::now();
            let draw_jobs: Vec<(isize, CaptureTargetSpec, u32, u32, IDXGISwapChain1, u64)> = {
                let st = ctrl.state.lock();
                st.targets
                    .iter()
                    .filter_map(|(&hwnd, t)| {
                        if t.spec.paused || !t.needs_redraw {
                            return None;
                        }
                        let Some(sc) = t.swap_chain.as_ref() else {
                            return None;
                        };
                        let min_interval = target_min_interval(t.rect_changed_at, now);
                        if redraw_wait_ms(t.first_drawn, t.last_redraw_at, min_interval, now) > 0 {
                            return None;
                        }
                        Some((
                            hwnd,
                            t.spec,
                            t.phys_w,
                            t.phys_h,
                            sc.clone(),
                            t.swapchain_gen,
                        ))
                    })
                    .collect()
            };

            let mut need_device_reset = false;
            if !draw_jobs.is_empty() {
                pipeline.prepare_for_draw();
            }
            for (hwnd, spec, pw, ph, sc, gen_id) in draw_jobs {
                match pipeline.draw_target(&spec, pw, ph, &sc) {
                    Ok(()) => {
                        let mut st = ctrl.state.lock();
                        if let Some(t) = st.targets.get_mut(&hwnd) {
                            if t.swapchain_gen == gen_id {
                                t.needs_redraw = false;
                                t.first_drawn = true;
                                t.last_draw_err = None;
                                t.last_redraw_at = Some(Instant::now());
                                if !t.logged_first_present {
                                    t.logged_first_present = true;
                                    log::info!(
                                        "CaptureD2d første frame tegnet ({pw}x{ph}) for widget {hwnd}"
                                    );
                                }
                            }
                        }
                    }
                    Err(hr) => {
                        if is_device_reset_hr(hr) {
                            need_device_reset = true;
                            break;
                        }
                        let mut st = ctrl.state.lock();
                        if let Some(t) = st.targets.get_mut(&hwnd) {
                            if should_log_draw_error(&mut t.last_draw_err, hr) {
                                log::warn!("CaptureD2d draw fejlede for widget {hwnd}: {hr:?}");
                            }
                        }
                    }
                }
            }

            if need_device_reset {
                let mut st = ctrl.state.lock();
                st.display_dirty = true;
                drop(st);
                continue 'outer;
            }
        }
    }

    drop(duplication);
    remove_worker_if_same(&ctrl);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn rects_intersect_checks_overlap_and_touching_edges() {
        assert!(rects_intersect(r(0, 0, 100, 100), r(50, 50, 150, 150)));
        assert!(!rects_intersect(r(0, 0, 100, 100), r(100, 0, 200, 100)));
        assert!(!rects_intersect(r(0, 0, 100, 100), r(200, 200, 300, 300)));
    }

    #[test]
    fn influence_rect_expands_by_three_sigma_clamped_to_monitor() {
        let mon = r(1920, 0, 3840, 1080);
        let widget = r(1925, 10, 2201, 142);
        let inf = influence_rect(widget, mon, 26.0);
        // rel left = 5 - 78 => clamped to 0; rel top = 10 - 78 => clamped to 0
        // rel right = 281 + 78 = 359; rel bottom = 142 + 78 = 220
        assert_eq!(inf, r(0, 0, 359, 220));
    }

    #[test]
    fn pick_monitor_index_prefers_center_then_overlap() {
        let mons = [r(0, 0, 1920, 1080), r(1920, 0, 3840, 1080)];
        // Center on secondary monitor
        assert_eq!(pick_monitor_index(&mons, r(1910, 100, 2200, 240)), Some(1));
        // Mostly on primary
        assert_eq!(pick_monitor_index(&mons, r(1600, 100, 1930, 240)), Some(0));
        // Outside all -> falls back to largest overlap (0) -> index 0
        assert_eq!(pick_monitor_index(&mons, r(-500, -500, -100, -100)), Some(0));
        assert_eq!(pick_monitor_index(&[], r(0, 0, 100, 100)), None);
    }

    #[test]
    fn redraw_wait_ms_respects_drag_and_fps_throttles() {
        let now = Instant::now();
        let old_rect = now - Duration::from_secs(1);
        let recent_rect = now - Duration::from_millis(40);
        assert_eq!(target_min_interval(old_rect, now), Duration::from_millis(16));
        assert_eq!(target_min_interval(recent_rect, now), Duration::from_millis(8));

        let min_16 = Duration::from_millis(16);
        assert_eq!(redraw_wait_ms(false, Some(now), min_16, now), 0);
        assert_eq!(redraw_wait_ms(true, None, min_16, now), 0);
        assert_eq!(
            redraw_wait_ms(true, Some(now - Duration::from_millis(5)), min_16, now),
            11
        );
        assert_eq!(
            redraw_wait_ms(true, Some(now - Duration::from_micros(15_600)), min_16, now),
            1
        );
        assert_eq!(
            redraw_wait_ms(true, Some(now - Duration::from_millis(16)), min_16, now),
            0
        );
    }

    #[test]
    fn acquire_min_interval_uses_drag_and_rect_change() {
        let now = Instant::now();
        assert_eq!(
            acquire_min_interval(true, None, now),
            Duration::from_millis(8)
        );
        assert_eq!(
            acquire_min_interval(false, None, now),
            Duration::from_millis(16)
        );
        assert_eq!(
            acquire_min_interval(false, Some(now - Duration::from_millis(40)), now),
            Duration::from_millis(8)
        );
        assert_eq!(
            acquire_min_interval(false, Some(now - Duration::from_millis(500)), now),
            Duration::from_millis(16)
        );
        // Drag vinder altid over gammel rect-aendring
        assert_eq!(
            acquire_min_interval(true, Some(now - Duration::from_secs(2)), now),
            Duration::from_millis(8)
        );
    }

    #[test]
    fn regions_to_copy_clips_and_merges() {
        let w = 1920;
        let h = 1080;
        // Tom/out-of-bounds udelades, gyldige klippes til teksturkanter.
        let dirty = [
            r(-10, -5, 40, 30),
            r(3000, 0, 3100, 20),
            r(50, 2000, 60, 2100),
            r(1900, 1060, 1940, 1100),
            r(0, 0, 0, 0),
        ];
        let regs = regions_to_copy(&dirty, w, h);
        assert_eq!(regs.len(), 2);
        assert_eq!(regs[0], r(0, 0, 40, 30));
        assert_eq!(regs[1], r(1900, 1060, 1920, 1080));
        // >64 rects -> en enkelt bounding box
        let mut many = Vec::new();
        for i in 0..80 {
            many.push(r(i * 2, i, i * 2 + 4, i + 4));
        }
        let regs = regions_to_copy(&many, w, h);
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0], r(0, 0, 162, 83));
        // Ingen rects -> intet kopieres
        assert!(regions_to_copy(&[r(0, 0, 0, 0)], w, h).is_empty());
    }

    #[test]
    fn sdr_white_scale_from_raw_clamps_and_scales() {
        assert!((sdr_white_scale_from_raw(0) - 1.0).abs() < 1e-6);
        assert!((sdr_white_scale_from_raw(1000) - 1.0).abs() < 1e-6);
        assert!((sdr_white_scale_from_raw(2500) - 2.5).abs() < 1e-6);
        assert!((sdr_white_scale_from_raw(5500) - 5.5).abs() < 1e-6);
        assert!((sdr_white_scale_from_raw(100_000) - 20.0).abs() < 1e-6);
    }

    fn srgb_u8_to_linear(v: u8) -> f32 {
        let c = v as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn f32_to_f16_bits(val: f32) -> u16 {
        let bits = val.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exp = ((bits >> 23) & 0xFF) as i32;
        let frac = bits & 0x007F_FFFF;

        if exp == 0xFF {
            if frac == 0 {
                return sign | 0x7C00;
            }
            return sign | 0x7E00 | ((frac >> 13) as u16).max(1);
        }

        let exp16 = exp - 127 + 15;
        if exp16 >= 31 {
            return sign | 0x7C00;
        }
        if exp16 <= 0 {
            if exp16 < -10 {
                return sign;
            }
            let mant = (frac | 0x0080_0000) >> (14 - exp16);
            return sign | (mant as u16);
        }
        let round = (frac + 0x1000) >> 13;
        let exp_part = (exp16 as u16) << 10;
        sign | (exp_part + round as u16)
    }

    #[test]
    fn hdr_to_sdr_shader_inverts_dwm_sdr_white_level_1_to_1() {
        use windows::Win32::Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
            Direct3D11::{D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_STAGING},
        };

        let _com = crate::util::ComMtaGuard::init();
        let levels = [D3D_FEATURE_LEVEL_11_0];
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let hw = unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        };
        if hw.is_err() || device.is_none() || context.is_none() {
            device = None;
            context = None;
            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_WARP,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    Some(&levels),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .expect("D3D11 device (HW eller WARP)");
            }
        }
        let device = device.unwrap();
        let ctx = context.unwrap();
        let converter = HdrToSdrConverter::new(&device).expect("HdrToSdrConverter::new");

        let sdr_scale = sdr_white_scale_from_raw(5500);
        let encode_px = |r: u8, g: u8, b: u8, a: f32| -> [u16; 4] {
            [
                f32_to_f16_bits(srgb_u8_to_linear(r) * sdr_scale),
                f32_to_f16_bits(srgb_u8_to_linear(g) * sdr_scale),
                f32_to_f16_bits(srgb_u8_to_linear(b) * sdr_scale),
                f32_to_f16_bits(a),
            ]
        };

        let pixels: [[u16; 4]; 4] = [
            encode_px(148, 160, 168, 0.0),
            encode_px(255, 255, 255, 0.0),
            encode_px(24, 24, 28, 0.0),
            [
                f32_to_f16_bits(12.0),
                f32_to_f16_bits(12.0),
                f32_to_f16_bits(12.0),
                f32_to_f16_bits(1.0),
            ],
        ];

        let src_desc = D3D11_TEXTURE2D_DESC {
            Width: 4,
            Height: 1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let init_data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const _,
            SysMemPitch: 4 * 8,
            SysMemSlicePitch: 0,
        };
        let mut src_tex: Option<ID3D11Texture2D> = None;
        unsafe {
            device
                .CreateTexture2D(&src_desc, Some(&init_data), Some(&mut src_tex))
                .unwrap();
        }
        let src_tex = src_tex.unwrap();
        let src_res: ID3D11Resource = src_tex.cast().unwrap();
        let mut srv: Option<ID3D11ShaderResourceView> = None;
        unsafe {
            device
                .CreateShaderResourceView(&src_res, None, Some(&mut srv))
                .unwrap();
        }
        let srv = srv.unwrap();

        let dst_desc = D3D11_TEXTURE2D_DESC {
            Width: 4,
            Height: 1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut dst_tex: Option<ID3D11Texture2D> = None;
        unsafe {
            device
                .CreateTexture2D(&dst_desc, None, Some(&mut dst_tex))
                .unwrap();
        }
        let dst_tex = dst_tex.unwrap();
        let dst_res: ID3D11Resource = dst_tex.cast().unwrap();
        let mut rtv: Option<ID3D11RenderTargetView> = None;
        unsafe {
            device
                .CreateRenderTargetView(&dst_res, None, Some(&mut rtv))
                .unwrap();
        }
        let rtv = rtv.unwrap();

        converter
            .convert(&ctx, &srv, &rtv, 4, 1, 1.0 / sdr_scale)
            .expect("HDR->SDR convert");

        let staging_desc = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..dst_desc
        };
        let mut staging: Option<ID3D11Texture2D> = None;
        unsafe {
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .unwrap();
        }
        let staging = staging.unwrap();
        let staging_res: ID3D11Resource = staging.cast().unwrap();
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let got = unsafe {
            ctx.CopyResource(&staging_res, &dst_res);
            ctx.Map(&staging_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let slice = std::slice::from_raw_parts(mapped.pData as *const u8, 16);
            let copy: [u8; 16] = slice.try_into().unwrap();
            ctx.Unmap(&staging_res, 0);
            copy
        };

        // BGRA byteordre i DXGI_FORMAT_B8G8R8A8_UNORM
        let expected: [[u8; 4]; 4] = [
            [168, 160, 148, 255],
            [255, 255, 255, 255],
            [28, 24, 24, 255],
            [255, 255, 255, 255],
        ];
        for i in 0..4 {
            let px = &got[i * 4..(i + 1) * 4];
            assert_eq!(px[3], expected[i][3], "pixel {i} alpha");
            for c in 0..3 {
                let diff = (px[c] as i16 - expected[i][c] as i16).abs();
                assert!(
                    diff <= 1,
                    "pixel {i} kanal {c}: fik {}, forventede {}",
                    px[c],
                    expected[i][c]
                );
            }
        }
    }
}
