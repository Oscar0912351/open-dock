use std::{
    collections::HashMap,
    mem::{size_of, ManuallyDrop},
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
        Foundation::{D2DERR_RECREATE_TARGET, E_ACCESSDENIED, HMODULE, HWND, RECT},
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
            Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
                D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
            },
            Dxgi::{
                Common::{
                    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory2,
                IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, IDXGISurface,
                IDXGISwapChain1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_DEVICE_REMOVED,
                DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_NOT_CURRENTLY_AVAILABLE,
                DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_WAIT_TIMEOUT,
                DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_MOVE_RECT, DXGI_PRESENT,
                DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD,
                DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    },
};
use windows_numerics::Vector2;

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

struct ComMtaGuard(bool);

impl ComMtaGuard {
    fn init() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self(hr.is_ok())
    }
}

impl Drop for ComMtaGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct MonitorGpuPipeline {
    mon_rect: RECT,
    d3d_device: ID3D11Device,
    d3d_context: ID3D11DeviceContext,
    dxgi_factory: IDXGIFactory2,
    dxgi_output: IDXGIOutput1,
    dc: ID2D1DeviceContext,
    desktop_tex: ID3D11Texture2D,
    desktop_tex_desc: D3D11_TEXTURE2D_DESC,
    desktop_bitmap: ID2D1Bitmap1,
    color_matrix_fx: ID2D1Effect,
    _border_fx: ID2D1Effect,
    blur_fx: ID2D1Effect,
    sat_fx: ID2D1Effect,
}

fn find_adapter_and_output(
    target_key: MonitorKey,
) -> Result<(IDXGIAdapter1, IDXGIOutput1, RECT), String> {
    let factory: IDXGIFactory2 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
    let mut best: Option<(IDXGIAdapter1, IDXGIOutput1, RECT, i64)> = None;
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
                return Ok((adapter, output1, r));
            }
            let ix0 = target_rect.left.max(r.left);
            let iy0 = target_rect.top.max(r.top);
            let ix1 = target_rect.right.min(r.right);
            let iy1 = target_rect.bottom.min(r.bottom);
            let area = ((ix1 - ix0).max(0) as i64) * ((iy1 - iy0).max(0) as i64);
            let better = best.as_ref().map(|b| area > b.3).unwrap_or(true);
            if better {
                best = Some((adapter.clone(), output1, r, area));
            }
        }
    }

    best.map(|(a, o, r, _)| (a, o, r))
        .ok_or_else(|| "Kunne ikke finde DXGI-skærmoutput til capture".to_string())
}

fn create_desktop_texture_and_bitmap(
    d3d_device: &ID3D11Device,
    dc: &ID2D1DeviceContext,
    width: u32,
    height: u32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
) -> Result<(ID3D11Texture2D, D3D11_TEXTURE2D_DESC, ID2D1Bitmap1), String> {
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
            .map_err(|e| format!("CreateTexture2D desktop cache: {e}"))?;
    }
    let tex = tex.ok_or_else(|| "CreateTexture2D returnerede None".to_string())?;
    let surface: IDXGISurface = tex
        .cast()
        .map_err(|e| format!("desktop IDXGISurface cast: {e}"))?;
    let bmp_props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        colorContext: ManuallyDrop::new(None),
    };
    let bmp = unsafe { dc.CreateBitmapFromDxgiSurface(&surface, Some(&bmp_props)) }
        .map_err(|e| format!("CreateBitmapFromDxgiSurface desktop: {e}"))?;
    Ok((tex, tex_desc, bmp))
}

impl MonitorGpuPipeline {
    fn create(mon_key: MonitorKey) -> Result<Self, String> {
        let (adapter, dxgi_output, mon_rect) = find_adapter_and_output(mon_key)?;
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
        let (desktop_tex, desktop_tex_desc, desktop_bitmap) = create_desktop_texture_and_bitmap(
            &d3d_device,
            &dc,
            mon_w,
            mon_h,
            DXGI_FORMAT_B8G8R8A8_UNORM,
        )?;

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
            d3d_device,
            d3d_context,
            dxgi_factory,
            dxgi_output,
            dc,
            desktop_tex,
            desktop_tex_desc,
            desktop_bitmap,
            color_matrix_fx,
            _border_fx: border_fx,
            blur_fx,
            sat_fx,
        })
    }

    fn ensure_desktop_format(&mut self, incoming: &D3D11_TEXTURE2D_DESC) -> Result<(), String> {
        if incoming.Width == self.desktop_tex_desc.Width
            && incoming.Height == self.desktop_tex_desc.Height
            && incoming.Format == self.desktop_tex_desc.Format
        {
            return Ok(());
        }
        let (tex, desc, bmp) = create_desktop_texture_and_bitmap(
            &self.d3d_device,
            &self.dc,
            incoming.Width,
            incoming.Height,
            incoming.Format,
        )?;
        self.desktop_tex = tex;
        self.desktop_tex_desc = desc;
        self.desktop_bitmap = bmp;
        unsafe {
            self.color_matrix_fx.SetInput(0, &self.desktop_bitmap, true);
        }
        Ok(())
    }

    fn acquire_duplication(&self) -> Result<IDXGIOutputDuplication, windows_core::Error> {
        unsafe { self.dxgi_output.DuplicateOutput(&self.d3d_device) }
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
    let _com = ComMtaGuard::init();

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
    let mut initial_acquire_attempted = false;

    'outer: loop {
        // 1. Check lifecycle, pause, and display changes.
        let (need_rebuild, any_need_swapchain, all_paused, min_redraw_wait_ms, drag_recent) = {
            let mut st = ctrl.state.lock();
            if st.shutdown || st.targets.is_empty() {
                drop(st);
                break 'outer;
            }
            if st.display_dirty {
                st.display_dirty = false;
                (true, false, false, None, false)
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
                (false, any_sc, !any_active && !any_sc, min_wait, drag_recent)
            }
        };
        let _ = all_paused;

        if need_rebuild {
            drop(duplication.take());
            has_desktop_frame = false;
            initial_acquire_attempted = false;
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

        // 2. Ensure IDXGIOutputDuplication is active.
        if duplication.is_none() {
            match pipeline.acquire_duplication() {
                Ok(d) => {
                    duplication = Some(d);
                    has_desktop_frame = false;
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

        if let Some(ref dup) = duplication {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut desktop_res: Option<IDXGIResource> = None;
            match unsafe { dup.AcquireNextFrame(timeout_ms, &mut frame_info, &mut desktop_res) } {
                Ok(()) => {
                    initial_acquire_attempted = true;
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
                                match pipeline.ensure_desktop_format(&desc) {
                                    Ok(()) => {
                                        unsafe {
                                            pipeline
                                                .d3d_context
                                                .CopyResource(&pipeline.desktop_tex, &tex);
                                            pipeline
                                                .color_matrix_fx
                                                .SetInput(0, &pipeline.desktop_bitmap, true);
                                        }
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
}
