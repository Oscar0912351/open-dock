use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self},
        Arc, LazyLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};
use windows::Win32::{
    Foundation::RECT,
    Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory2, IDXGISwapChain1},
};

use super::duplication::{monitor_worker_loop, TargetRuntime};

#[cfg(test)]
use super::duplication::{acquire_min_interval, redraw_wait_ms, target_min_interval};
#[cfg(test)]
use super::hdr::{sdr_white_scale_from_raw, HdrToSdrConverter};

#[cfg(test)]
use windows::core::Interface;
#[cfg(test)]
use windows::Win32::{
    Foundation::HMODULE,
    Graphics::{
        Direct3D::D3D_FEATURE_LEVEL_11_0,
        Direct3D11::{
            D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
            ID3D11Resource, ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
            D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
            D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
        },
        Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
        },
    },
};

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
pub(super) struct MonitorKey {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
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

pub(super) struct WorkerSharedState {
    pub(super) targets: HashMap<isize, TargetRuntime>,
    pub(super) display_dirty: bool,
    pub(super) color_dirty: bool,
    pub(super) shutdown: bool,
    pub(super) last_drag_at: Option<Instant>,
}

pub(super) struct WorkerControl {
    pub(super) mon_key: MonitorKey,
    pub(super) state: Mutex<WorkerSharedState>,
    pub(super) cv: Condvar,
    pub(super) thread: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
pub(super) struct CaptureCoordinator {
    pub(super) monitors: Vec<MonitorKey>,
    pub(super) workers: HashMap<MonitorKey, Arc<WorkerControl>>,
    pub(super) widget_monitor: HashMap<isize, MonitorKey>,
}

pub(super) static COORDINATOR: LazyLock<Mutex<CaptureCoordinator>> =
    LazyLock::new(|| Mutex::new(CaptureCoordinator::default()));
static LATEST_GEN: LazyLock<Mutex<HashMap<isize, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_GEN: AtomicU64 = AtomicU64::new(1);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static LIVE_LUMA: LazyLock<Mutex<HashMap<isize, f32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static LUMA_SEQ: AtomicU32 = AtomicU32::new(0);
static SRGB_LINEAR_LUT: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut lut = [0.0_f32; 256];
    for (i, slot) in lut.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *slot = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
    }
    lut
});

pub(super) fn mean_luma_bgra(
    data: &[u8],
    width: u32,
    height: u32,
    row_pitch: u32,
    step: u32,
) -> Option<f32> {
    if width == 0 || height == 0 {
        return None;
    }
    let row_pitch = row_pitch as usize;
    let width = width as usize;
    let needed = (height as usize - 1)
        .checked_mul(row_pitch)?
        .checked_add(width.checked_mul(4)?)?;
    if data.len() < needed {
        return None;
    }

    let step = step.max(1) as usize;
    let lut = &*SRGB_LINEAR_LUT;
    let mut sum = 0.0_f64;
    let mut count = 0_u64;
    let mut y = 0_usize;
    while y < height as usize {
        let row_base = y * row_pitch;
        let mut x = 0_usize;
        while x < width {
            let idx = row_base + x * 4;
            let b = data[idx] as usize;
            let g = data[idx + 1] as usize;
            let r = data[idx + 2] as usize;
            sum += (0.2126 * lut[r] + 0.7152 * lut[g] + 0.0722 * lut[b]) as f64;
            count += 1;
            x += step;
        }
        y += step;
    }
    if count == 0 {
        return None;
    }
    Some(((sum / count as f64) as f32).clamp(0.0, 1.0))
}

pub(super) fn store_live_luma(widget_hwnd: isize, luma: f32) -> u32 {
    let mut live = LIVE_LUMA.lock();
    live.insert(widget_hwnd, luma);
    LUMA_SEQ.fetch_add(1, Ordering::AcqRel) + 1
}

pub fn live_luma_snapshot(widget_hwnd: isize) -> (u32, Option<f32>) {
    let live = LIVE_LUMA.lock();
    let luma = live.get(&widget_hwnd).copied();
    let seq = LUMA_SEQ.fetch_add(1, Ordering::AcqRel) + 1;
    (seq, luma)
}

pub(super) fn bump_generation(widget_hwnd: isize) -> u64 {
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

pub(super) fn rect_dims(r: RECT) -> (u32, u32) {
    (
        (r.right - r.left).max(1) as u32,
        (r.bottom - r.top).max(1) as u32,
    )
}

pub fn rects_intersect(a: RECT, b: RECT) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

const MAX_REGION_COPIES: usize = 64;

pub(super) fn clip_rect_to_tex(r: RECT, width: u32, height: u32) -> Option<RECT> {
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

pub(super) fn regions_to_copy(dirty: &[RECT], width: u32, height: u32) -> Vec<RECT> {
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
    if let Some((idx, _)) = monitors
        .iter()
        .enumerate()
        .find(|(_, r)| cx >= r.left && cx < r.right && cy >= r.top && cy < r.bottom)
    {
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

pub(super) fn enumerate_dxgi_monitor_keys() -> Vec<MonitorKey> {
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

pub(super) fn ensure_monitors_cached(coord: &mut CaptureCoordinator) {
    if coord.monitors.is_empty() {
        coord.monitors = enumerate_dxgi_monitor_keys();
    }
}

pub(super) fn select_monitor_for_rect(
    coord: &mut CaptureCoordinator,
    widget_rect: RECT,
) -> Option<MonitorKey> {
    ensure_monitors_cached(coord);
    let rects: Vec<RECT> = coord.monitors.iter().copied().map(RECT::from).collect();
    pick_monitor_index(&rects, widget_rect).map(|idx| coord.monitors[idx])
}

pub(super) fn remove_widget_from_worker_locked(coord: &mut CaptureCoordinator, widget_hwnd: isize) {
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

pub(super) fn get_or_spawn_worker_locked(
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
            if existing.phys_w == phys_w
                && existing.phys_h == phys_h
                && existing.swap_chain.is_some()
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
                    luma_dirty: false,
                    last_luma_at: None,
                    last_luma_sent: None,
                    logged_first_luma: false,
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
                luma_dirty: false,
                last_luma_at: None,
                last_luma_sent: None,
                logged_first_luma: false,
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
    LIVE_LUMA.lock().remove(&widget_hwnd);
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

pub(super) fn fail_pending_replies(ctrl: &WorkerControl, msg: &str) {
    let mut st = ctrl.state.lock();
    for t in st.targets.values_mut() {
        if let Some(tx) = t.pending_reply.take() {
            let _ = tx.send(Err(msg.to_string()));
        }
    }
}

pub(super) fn remove_worker_if_same(ctrl: &Arc<WorkerControl>) {
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

    fn bgra(width: usize, height: usize, pitch: usize, px: [u8; 4]) -> Vec<u8> {
        let mut data = vec![0_u8; pitch * height];
        for y in 0..height {
            for x in 0..width {
                let idx = y * pitch + x * 4;
                data[idx..idx + 4].copy_from_slice(&px);
            }
        }
        data
    }

    #[test]
    fn mean_luma_black_is_zero_and_white_is_one() {
        assert_eq!(mean_luma_bgra(&bgra(2, 2, 8, [0, 0, 0, 255]), 2, 2, 8, 1), Some(0.0));
        let white = mean_luma_bgra(&bgra(2, 2, 8, [255, 255, 255, 255]), 2, 2, 8, 1);
        assert!((white.unwrap_or(-1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mean_luma_half_black_half_white_is_midpoint() {
        let data = [
            0, 0, 0, 255,
            255, 255, 255, 255,
        ];
        let luma = mean_luma_bgra(&data, 2, 1, 8, 1).unwrap_or(-1.0);
        assert!((luma - 0.5).abs() < 1e-6);
    }

    #[test]
    fn mean_luma_uses_bgra_channel_order() {
        let blue = mean_luma_bgra(&bgra(1, 1, 4, [255, 0, 0, 255]), 1, 1, 4, 1).unwrap_or(-1.0);
        let red = mean_luma_bgra(&bgra(1, 1, 4, [0, 0, 255, 255]), 1, 1, 4, 1).unwrap_or(-1.0);
        assert!((blue - 0.0722).abs() < 1e-4);
        assert!((red - 0.2126).abs() < 1e-4);
    }

    #[test]
    fn mean_luma_ignores_row_padding() {
        let mut data = bgra(2, 2, 12, [0, 0, 0, 255]);
        for y in 0..2 {
            for x in 8..12 {
                data[y * 12 + x] = 0xFF;
            }
        }
        let luma = mean_luma_bgra(&data, 2, 2, 12, 1).unwrap_or(-1.0);
        assert_eq!(luma, 0.0);
    }

    #[test]
    fn mean_luma_rejects_short_buffer() {
        assert_eq!(mean_luma_bgra(&[0; 7], 2, 1, 8, 1), None);
        assert_eq!(mean_luma_bgra(&[0; 12], 2, 2, 8, 1), None);
    }

    #[test]
    fn mean_luma_step_matches_uniform_image() {
        let data = bgra(8, 8, 32, [32, 96, 160, 255]);
        let full = mean_luma_bgra(&data, 8, 8, 32, 1);
        let sparse = mean_luma_bgra(&data, 8, 8, 32, 4);
        assert_eq!(full, sparse);
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
