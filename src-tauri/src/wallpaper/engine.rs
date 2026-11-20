use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;
use windows::{
    core::Interface,
    Win32::{
        Foundation::{CloseHandle, HANDLE, HMODULE, WAIT_OBJECT_0},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext,
                ID3D11PixelShader, ID3D11RenderTargetView, ID3D11Resource, ID3D11Texture2D,
                ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC,
                D3D11_CPU_ACCESS_WRITE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11_USAGE_DYNAMIC,
                D3D11_VIEWPORT,
            },
            DirectComposition::{
                DCompositionCreateDevice, IDCompositionDevice, IDCompositionScaleTransform,
                IDCompositionTarget, IDCompositionTransform, IDCompositionVisual,
                DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR,
            },
            Dxgi::{
                Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory2,
                IDXGISwapChain1, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
                DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_DISCARD,
                DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
        System::{
            Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
            Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
            Threading::{
                CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CREATE_WAITABLE_TIMER_MANUAL_RESET,
                TIMER_ALL_ACCESS,
            },
        },
        UI::WindowsAndMessaging::{
            DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG,
            PM_REMOVE, QS_ALLINPUT,
        },
    },
};

use crate::{
    config::schema::{
        clamp_wallpaper_brightness, clamp_wallpaper_speed, normalize_wallpaper_fps,
        MonitorWallpaper, RectI, WallpaperConfig, WallpaperKind, WallpaperResolution,
    },
    desktop::monitors::MonitorInfo,
    error::AppError,
    state::{AppState, PauseReason},
};

use super::{
    host::{create_and_attach_host_window, WallpaperHostWindow},
    occlusion::{is_monitor_covered, is_workstation_locked},
    shader_compiler::{
        compile_scene_pixel_shader, compile_vertex_shader, update_constant_buffer, SceneUniforms,
    },
    video::ZeroCopyVideoPlayer,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum EngineRunState {
    Idle,
    Running,
    PausedCovered,
    PausedFullscreen,
    PausedBattery,
    PausedLocked,
    PausedManual,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WallpaperEngineStatus {
    pub run_state: EngineRunState,
    pub active_surfaces: u32,
    pub actual_fps: f32,
    pub frame_time_ms: f32,
    pub backend: String,
    pub adapter_name: String,
    pub render_width: u32,
    pub render_height: u32,
    pub last_error: Option<String>,
}

impl Default for WallpaperEngineStatus {
    fn default() -> Self {
        Self {
            run_state: EngineRunState::Idle,
            active_surfaces: 0,
            actual_fps: 0.0,
            frame_time_ms: 0.0,
            backend: String::new(),
            adapter_name: String::new(),
            render_width: 0,
            render_height: 0,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSurfaceSpec {
    pub monitor_name: String,
    pub rect: RectI,
    pub work_area: RectI,
    pub assignment: MonitorWallpaper,
    pub kind: WallpaperKind,
    pub source_path: Option<PathBuf>,
}

pub enum EngineCmd {
    Sync {
        config: WallpaperConfig,
        specs: Vec<ResolvedSurfaceSpec>,
        global_pause: Option<PauseReason>,
    },
    SetGlobalPause(Option<PauseReason>),
    ExplorerRestarted,
    StopAll,
    Shutdown,
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

struct WaitableTimer {
    handle: HANDLE,
}

impl WaitableTimer {
    fn new() -> Option<Self> {
        let mut h = unsafe {
            CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
        };
        if h.is_err() {
            h = unsafe {
                CreateWaitableTimerExW(
                    None,
                    None,
                    CREATE_WAITABLE_TIMER_MANUAL_RESET,
                    TIMER_ALL_ACCESS.0,
                )
            };
        }
        h.ok().filter(|v| !v.is_invalid()).map(|handle| Self { handle })
    }

    fn wait_duration(&self, dur: Duration) {
        if dur.is_zero() {
            return;
        }
        let ticks_100ns = -((dur.as_nanos() / 100).max(1) as i64);
        if unsafe { SetWaitableTimer(self.handle, &ticks_100ns, 0, None, None, false) }.is_err() {
            thread::sleep(dur);
            return;
        }
        let handles = [self.handle];
        loop {
            let wait_res = unsafe {
                MsgWaitForMultipleObjects(
                    Some(&handles),
                    false,
                    dur.as_millis().clamp(1, 1000) as u32,
                    QS_ALLINPUT,
                )
            };
            if wait_res == WAIT_OBJECT_0 {
                break;
            }
            if wait_res.0 == WAIT_OBJECT_0.0 + 1 {
                pump_win32_messages();
                if unsafe { WaitForSingleObject(self.handle, 0) } == WAIT_OBJECT_0 {
                    break;
                }
            } else {
                break;
            }
        }
    }
}

impl Drop for WaitableTimer {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

enum Presenter {
    DirectComposition {
        _dcomp: IDCompositionDevice,
        _target: IDCompositionTarget,
        _visual: IDCompositionVisual,
        _scale: Option<IDCompositionScaleTransform>,
        swap_chain: IDXGISwapChain1,
    },
    DxgiHwnd {
        swap_chain: IDXGISwapChain1,
    },
}

impl Presenter {
    fn swap_chain(&self) -> &IDXGISwapChain1 {
        match self {
            Self::DirectComposition { swap_chain, .. } | Self::DxgiHwnd { swap_chain } => swap_chain,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::DirectComposition { .. } => "D3D11 · DirectComposition",
            Self::DxgiHwnd { .. } => "D3D11 · DXGI",
        }
    }

    fn present(&self) -> windows_core::Result<()> {
        unsafe { self.swap_chain().Present(0, DXGI_PRESENT(0)).ok() }
    }
}

struct ShaderRunner {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cbuf: ID3D11Buffer,
    rtv: ID3D11RenderTargetView,
}

impl ShaderRunner {
    fn new(
        device: &ID3D11Device,
        back_buffer: &ID3D11Texture2D,
        shader_id: &str,
        width: u32,
        height: u32,
        brightness: f32,
    ) -> Result<Self, AppError> {
        let vs_bytes = compile_vertex_shader()?;
        let ps_bytes = compile_scene_pixel_shader(shader_id)?;

        let mut vs: Option<ID3D11VertexShader> = None;
        let mut ps: Option<ID3D11PixelShader> = None;
        unsafe {
            device.CreateVertexShader(&vs_bytes, None, Some(&mut vs))?;
            device.CreatePixelShader(&ps_bytes, None, Some(&mut ps))?;
        }
        let vs = vs.ok_or_else(|| AppError::Other("Kunne ikke oprette vertex shader".into()))?;
        let ps = ps.ok_or_else(|| AppError::Other("Kunne ikke oprette pixel shader".into()))?;

        let init_uniforms = SceneUniforms {
            resolution: [width as f32, height as f32],
            time: 0.0,
            brightness,
            accent: [1.0, 1.0, 1.0, 1.0],
        };
        let cbuf_desc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<SceneUniforms>() as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let init_data = windows::Win32::Graphics::Direct3D11::D3D11_SUBRESOURCE_DATA {
            pSysMem: &init_uniforms as *const SceneUniforms as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut cbuf: Option<ID3D11Buffer> = None;
        unsafe { device.CreateBuffer(&cbuf_desc, Some(&init_data), Some(&mut cbuf))? };
        let cbuf = cbuf.ok_or_else(|| AppError::Other("Kunne ikke oprette constant buffer".into()))?;

        let rt_res: ID3D11Resource = back_buffer.cast()?;
        let mut rtv: Option<ID3D11RenderTargetView> = None;
        unsafe { device.CreateRenderTargetView(&rt_res, None, Some(&mut rtv))? };
        let rtv = rtv.ok_or_else(|| AppError::Other("Kunne ikke oprette RTV".into()))?;

        Ok(Self { vs, ps, cbuf, rtv })
    }

    fn render(
        &self,
        context: &ID3D11DeviceContext,
        width: u32,
        height: u32,
        time_s: f32,
        brightness: f32,
    ) -> Result<(), AppError> {
        let uniforms = SceneUniforms {
            resolution: [width as f32, height as f32],
            time: time_s,
            brightness,
            accent: [1.0, 1.0, 1.0, 1.0],
        };
        update_constant_buffer(context, &self.cbuf, &uniforms)?;
        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width as f32,
            Height: height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        unsafe {
            context.OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);
            context.RSSetViewports(Some(&[vp]));
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.VSSetShader(&self.vs, None);
            context.PSSetShader(&self.ps, None);
            context.PSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            context.Draw(3, 0);
        }
        Ok(())
    }
}

enum SurfaceContent {
    Shader(ShaderRunner),
    Video(ZeroCopyVideoPlayer),
}

struct MonitorSurface {
    spec: ResolvedSurfaceSpec,
    render_w: u32,
    render_h: u32,
    adapter_name: String,
    host: WallpaperHostWindow,
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    presenter: Presenter,
    back_buffer: ID3D11Texture2D,
    content: SurfaceContent,
    anim_time: f64,
    last_tick: Instant,
    covered: bool,
}

impl MonitorSurface {
    fn build(spec: ResolvedSurfaceSpec, resolution_mode: WallpaperResolution) -> Result<Self, AppError> {
        let mon_w = spec.rect.width.max(1) as u32;
        let mon_h = spec.rect.height.max(1) as u32;
        let (render_w, render_h) = compute_render_size(mon_w, mon_h, resolution_mode);

        let host = create_and_attach_host_window(spec.rect)?;
        let (device, context, adapter_name) = create_device_for_monitor(spec.rect)?;
        let presenter = create_presenter(&device, host.hwnd(), render_w, render_h, mon_w, mon_h)?;
        let back_buffer: ID3D11Texture2D = unsafe { presenter.swap_chain().GetBuffer(0)? };

        let content = match spec.kind {
            WallpaperKind::Shader => {
                let runner = ShaderRunner::new(
                    &device,
                    &back_buffer,
                    &spec.assignment.wallpaper_id,
                    render_w,
                    render_h,
                    clamp_wallpaper_brightness(spec.assignment.brightness),
                )?;
                SurfaceContent::Shader(runner)
            }
            WallpaperKind::Video => {
                let path = spec
                    .source_path
                    .as_deref()
                    .ok_or_else(|| AppError::Other("Videofil mangler sti".into()))?;
                let player = ZeroCopyVideoPlayer::new(
                    &device,
                    path,
                    spec.assignment.muted,
                    clamp_wallpaper_speed(spec.assignment.speed),
                )?;
                SurfaceContent::Video(player)
            }
            WallpaperKind::Photo => {
                return Err(AppError::Other("Statisk foto skal ikke startes i live-motor".into()));
            }
        };

        Ok(Self {
            spec,
            render_w,
            render_h,
            adapter_name,
            host,
            _device: device,
            context,
            presenter,
            back_buffer,
            content,
            anim_time: 12.0,
            last_tick: Instant::now(),
            covered: false,
        })
    }

    fn update_assignment_in_place(&mut self, next: &MonitorWallpaper) {
        self.spec.assignment = next.clone();
        if let SurfaceContent::Video(player) = &mut self.content {
            player.set_muted(next.muted);
            player.set_speed(clamp_wallpaper_speed(next.speed));
        }
    }

    fn step_and_present(&mut self, global_paused: bool) -> Result<bool, AppError> {
        let paused = global_paused || self.covered;
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f64().min(0.2);
        self.last_tick = now;

        match &mut self.content {
            SurfaceContent::Shader(runner) => {
                if paused {
                    return Ok(false);
                }
                let speed = clamp_wallpaper_speed(self.spec.assignment.speed) as f64;
                self.anim_time += dt * speed;
                runner.render(
                    &self.context,
                    self.render_w,
                    self.render_h,
                    self.anim_time as f32,
                    clamp_wallpaper_brightness(self.spec.assignment.brightness),
                )?;
                self.presenter.present().map_err(AppError::Win)?;
                Ok(true)
            }
            SurfaceContent::Video(player) => {
                if let Some(err_code) = player.has_error() {
                    return Err(AppError::Other(format!(
                        "Videoafspilning stoppede (fejlkode {err_code})"
                    )));
                }
                player.set_paused(paused);
                if paused {
                    return Ok(false);
                }
                let drew = player.render_if_ready(
                    &self.back_buffer,
                    self.render_w,
                    self.render_h,
                    self.spec.assignment.fit,
                )?;
                if drew {
                    self.presenter.present().map_err(AppError::Win)?;
                }
                Ok(drew)
            }
        }
    }
}

pub fn spawn_engine_thread(app: AppHandle) -> Option<Sender<EngineCmd>> {
    let (tx, rx) = mpsc::channel::<EngineCmd>();
    match thread::Builder::new()
        .name("opendock-wallpaper-engine".into())
        .spawn(move || engine_loop(app, rx))
    {
        Ok(_) => Some(tx),
        Err(error) => {
            log::error!("Kunne ikke starte baggrundsmotor-tråd: {error}");
            None
        }
    }
}

fn engine_loop(app: AppHandle, rx: mpsc::Receiver<EngineCmd>) {
    let _com = ComMtaGuard::init();
    let timer = WaitableTimer::new();

    let mut surfaces: Vec<MonitorSurface> = Vec::new();
    let mut specs_cache: Vec<ResolvedSurfaceSpec> = Vec::new();
    let mut engine_cfg = crate::config::schema::WallpaperEngineSettings::default();
    let mut global_pause: Option<PauseReason> = None;
    let mut last_error: Option<String> = None;

    let mut last_occlusion_check = Instant::now() - Duration::from_secs(1);
    let mut last_status_emit = Instant::now() - Duration::from_secs(2);
    let mut last_published: Option<WallpaperEngineStatus> = None;
    let mut ema_frame_ms = 0.0_f32;
    let mut ema_fps = 0.0_f32;
    let mut last_present_time = Instant::now();
    let mut battery_cached = false;
    let mut locked_cached = false;

    loop {
        let cmd = if surfaces.is_empty() && specs_cache.is_empty() {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => break,
            }
        } else {
            match rx.try_recv() {
                Ok(c) => Some(c),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        };

        if let Some(first_cmd) = cmd {
            let mut rebuild_needed = false;
            let mut should_exit = false;
            let mut explorer_restarted = false;

            let mut batch = vec![first_cmd];
            while let Ok(more) = rx.try_recv() {
                batch.push(more);
            }

            for item in batch {
                match item {
                    EngineCmd::Shutdown => {
                        should_exit = true;
                        break;
                    }
                    EngineCmd::StopAll => {
                        specs_cache.clear();
                        surfaces.clear();
                        last_error = None;
                    }
                    EngineCmd::ExplorerRestarted => {
                        explorer_restarted = true;
                    }
                    EngineCmd::SetGlobalPause(reason) => {
                        global_pause = reason;
                    }
                    EngineCmd::Sync {
                        config,
                        specs,
                        global_pause: gp,
                    } => {
                        global_pause = gp;
                        let res_changed = engine_cfg.resolution != config.engine.resolution;
                        engine_cfg = config.engine.clone();

                        if specs.is_empty() {
                            specs_cache.clear();
                            surfaces.clear();
                            last_error = None;
                            continue;
                        }

                        if !res_changed && specs_match_topology(&surfaces, &specs) {
                            for (surface, next_spec) in surfaces.iter_mut().zip(specs.iter()) {
                                surface.update_assignment_in_place(&next_spec.assignment);
                            }
                            specs_cache = specs;
                        } else {
                            specs_cache = specs;
                            rebuild_needed = true;
                        }
                    }
                }
            }

            if should_exit {
                surfaces.clear();
                break;
            }

            if explorer_restarted && !specs_cache.is_empty() {
                let mut z_ok = !surfaces.is_empty();
                for s in &surfaces {
                    if !s.host.verify_zorder() {
                        z_ok = false;
                        break;
                    }
                }
                if !z_ok {
                    rebuild_needed = true;
                }
            }

            if rebuild_needed {
                surfaces.clear();
                last_error = None;
                for spec in &specs_cache {
                    match MonitorSurface::build(spec.clone(), engine_cfg.resolution) {
                        Ok(s) => surfaces.push(s),
                        Err(error) => {
                            let msg = error.to_string();
                            log::error!(
                                "Kunne ikke starte live-baggrund på {}: {msg}",
                                spec.monitor_name
                            );
                            last_error = Some(msg);
                        }
                    }
                }
                last_occlusion_check = Instant::now() - Duration::from_secs(1);
                ema_frame_ms = 0.0;
                ema_fps = 0.0;
            }
        }

        pump_win32_messages();

        if surfaces.is_empty() {
            let status = WallpaperEngineStatus {
                run_state: if last_error.is_some() {
                    EngineRunState::Error
                } else {
                    EngineRunState::Idle
                },
                active_surfaces: 0,
                actual_fps: 0.0,
                frame_time_ms: 0.0,
                backend: String::new(),
                adapter_name: String::new(),
                render_width: 0,
                render_height: 0,
                last_error: last_error.clone(),
            };
            publish_status_if_needed(&app, &status, &mut last_published, &mut last_status_emit, true);
            continue;
        }

        let now = Instant::now();
        if now.duration_since(last_occlusion_check) >= Duration::from_millis(250) {
            last_occlusion_check = now;
            locked_cached = is_workstation_locked();
            battery_cached = engine_cfg.pause_on_battery && system_on_battery();
            for s in &mut surfaces {
                s.covered = engine_cfg.pause_when_covered
                    && is_monitor_covered(s.spec.rect, s.spec.work_area);
            }
        }

        let effective_pause = compute_effective_pause(
            &engine_cfg,
            &global_pause,
            locked_cached,
            battery_cached,
            &surfaces,
        );
        let all_paused = effective_pause.is_some();

        if all_paused {
            for s in &mut surfaces {
                let _ = s.step_and_present(true);
            }
            let status = build_running_status(
                &surfaces,
                effective_pause.unwrap_or(EngineRunState::PausedManual),
                0.0,
                0.0,
                last_error.clone(),
            );
            publish_status_if_needed(&app, &status, &mut last_published, &mut last_status_emit, false);

            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(next) => {
                    // Re-inject via self? Can't re-push to mpsc::Receiver; handle minimal state next loop
                    if matches!(next, EngineCmd::Shutdown) {
                        surfaces.clear();
                        break;
                    }
                    handle_single_idle_cmd(
                        next,
                        &mut surfaces,
                        &mut specs_cache,
                        &mut engine_cfg,
                        &mut global_pause,
                        &mut last_error,
                    );
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            continue;
        }

        let frame_start = Instant::now();
        let mut any_drawn = false;
        let mut lost_indices = Vec::new();

        for (idx, surface) in surfaces.iter_mut().enumerate() {
            match surface.step_and_present(false) {
                Ok(drew) => {
                    any_drawn |= drew;
                }
                Err(AppError::Win(ref win_err))
                    if win_err.code() == DXGI_ERROR_DEVICE_REMOVED
                        || win_err.code() == DXGI_ERROR_DEVICE_RESET =>
                {
                    log::warn!(
                        "DXGI-enhed nulstillet på {}; genskaber baggrund",
                        surface.spec.monitor_name
                    );
                    lost_indices.push(idx);
                }
                Err(error) => {
                    let msg = error.to_string();
                    log::warn!("Renderfejl på {}: {msg}", surface.spec.monitor_name);
                    last_error = Some(msg);
                }
            }
        }

        if !lost_indices.is_empty() {
            for idx in lost_indices.into_iter().rev() {
                let spec = surfaces[idx].spec.clone();
                surfaces.remove(idx);
                if let Ok(rebuilt) = MonitorSurface::build(spec, engine_cfg.resolution) {
                    surfaces.insert(idx, rebuilt);
                }
            }
        }

        let work_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        if any_drawn {
            let since_last = last_present_time.elapsed().as_secs_f32().max(0.001);
            last_present_time = Instant::now();
            let inst_fps = (1.0 / since_last).clamp(0.0, 240.0);
            ema_frame_ms = if ema_frame_ms == 0.0 {
                work_ms
            } else {
                ema_frame_ms * 0.85 + work_ms * 0.15
            };
            ema_fps = if ema_fps == 0.0 {
                inst_fps
            } else {
                ema_fps * 0.85 + inst_fps * 0.15
            };
        }

        let status = build_running_status(
            &surfaces,
            EngineRunState::Running,
            ema_fps,
            ema_frame_ms,
            last_error.clone(),
        );
        publish_status_if_needed(&app, &status, &mut last_published, &mut last_status_emit, false);

        let target_fps = normalize_wallpaper_fps(engine_cfg.fps_cap);
        let frame_budget = Duration::from_secs_f64(1.0 / target_fps as f64);
        let elapsed = frame_start.elapsed();
        if let Some(remaining) = frame_budget.checked_sub(elapsed) {
            if let Some(ref t) = timer {
                t.wait_duration(remaining);
            } else {
                thread::sleep(remaining);
            }
        }
    }
}

fn handle_single_idle_cmd(
    cmd: EngineCmd,
    surfaces: &mut Vec<MonitorSurface>,
    specs_cache: &mut Vec<ResolvedSurfaceSpec>,
    engine_cfg: &mut crate::config::schema::WallpaperEngineSettings,
    global_pause: &mut Option<PauseReason>,
    last_error: &mut Option<String>,
) {
    match cmd {
        EngineCmd::StopAll => {
            specs_cache.clear();
            surfaces.clear();
            *last_error = None;
        }
        EngineCmd::SetGlobalPause(r) => *global_pause = r,
        EngineCmd::ExplorerRestarted => {
            for s in surfaces.iter() {
                let _ = s.host.verify_zorder();
            }
        }
        EngineCmd::Sync {
            config,
            specs,
            global_pause: gp,
        } => {
            *global_pause = gp;
            let res_changed = engine_cfg.resolution != config.engine.resolution;
            *engine_cfg = config.engine;
            if specs.is_empty() {
                specs_cache.clear();
                surfaces.clear();
                *last_error = None;
            } else if !res_changed && specs_match_topology(surfaces, &specs) {
                for (surface, next_spec) in surfaces.iter_mut().zip(specs.iter()) {
                    surface.update_assignment_in_place(&next_spec.assignment);
                }
                *specs_cache = specs;
            } else {
                *specs_cache = specs;
                surfaces.clear();
                for spec in specs_cache.iter() {
                    if let Ok(s) = MonitorSurface::build(spec.clone(), engine_cfg.resolution) {
                        surfaces.push(s);
                    }
                }
            }
        }
        EngineCmd::Shutdown => {}
    }
}

fn specs_match_topology(surfaces: &[MonitorSurface], specs: &[ResolvedSurfaceSpec]) -> bool {
    if surfaces.len() != specs.len() {
        return false;
    }
    surfaces.iter().zip(specs.iter()).all(|(cur, nxt)| {
        cur.spec.monitor_name == nxt.monitor_name
            && cur.spec.rect == nxt.rect
            && cur.spec.kind == nxt.kind
            && cur.spec.assignment.wallpaper_id == nxt.assignment.wallpaper_id
            && cur.spec.source_path == nxt.source_path
    })
}

fn compute_effective_pause(
    cfg: &crate::config::schema::WallpaperEngineSettings,
    global_pause: &Option<PauseReason>,
    locked: bool,
    on_battery: bool,
    surfaces: &[MonitorSurface],
) -> Option<EngineRunState> {
    if locked {
        return Some(EngineRunState::PausedLocked);
    }
    match global_pause {
        Some(PauseReason::User) => return Some(EngineRunState::PausedManual),
        Some(PauseReason::Fullscreen) if cfg.pause_on_fullscreen => {
            return Some(EngineRunState::PausedFullscreen);
        }
        Some(PauseReason::Battery) if cfg.pause_on_battery => {
            return Some(EngineRunState::PausedBattery);
        }
        _ => {}
    }
    if on_battery {
        return Some(EngineRunState::PausedBattery);
    }
    if !surfaces.is_empty() && surfaces.iter().all(|s| s.covered) {
        return Some(EngineRunState::PausedCovered);
    }
    None
}

fn build_running_status(
    surfaces: &[MonitorSurface],
    run_state: EngineRunState,
    actual_fps: f32,
    frame_time_ms: f32,
    last_error: Option<String>,
) -> WallpaperEngineStatus {
    let first = surfaces.first();
    WallpaperEngineStatus {
        run_state,
        active_surfaces: surfaces.len() as u32,
        actual_fps: (actual_fps * 10.0).round() / 10.0,
        frame_time_ms: (frame_time_ms * 100.0).round() / 100.0,
        backend: first.map(|s| s.presenter.label().to_string()).unwrap_or_default(),
        adapter_name: first.map(|s| s.adapter_name.clone()).unwrap_or_default(),
        render_width: first.map(|s| s.render_w).unwrap_or(0),
        render_height: first.map(|s| s.render_h).unwrap_or(0),
        last_error,
    }
}

fn publish_status_if_needed(
    app: &AppHandle,
    status: &WallpaperEngineStatus,
    last_published: &mut Option<WallpaperEngineStatus>,
    last_emit: &mut Instant,
    force: bool,
) {
    let state_changed = last_published
        .as_ref()
        .map(|prev| {
            prev.run_state != status.run_state
                || prev.active_surfaces != status.active_surfaces
                || prev.backend != status.backend
                || prev.last_error != status.last_error
        })
        .unwrap_or(true);

    if !force && !state_changed && last_emit.elapsed() < Duration::from_secs(1) {
        return;
    }

    *last_published = Some(status.clone());
    *last_emit = Instant::now();

    if let Some(state) = app.try_state::<AppState>() {
        *state.wallpaper_status.write() = status.clone();
    }
    let _ = app.emit_to("control", "wallpaper://status", status);
}

fn compute_render_size(mon_w: u32, mon_h: u32, mode: WallpaperResolution) -> (u32, u32) {
    match mode {
        WallpaperResolution::Native => (mon_w.max(64), mon_h.max(64)),
        WallpaperResolution::Balanced => {
            let rw = ((mon_w as f64 * 0.75).round() as u32).max(640).min(mon_w);
            let rh = ((mon_h as f64 * 0.75).round() as u32).max(360).min(mon_h);
            (rw, rh)
        }
    }
}

fn create_device_for_monitor(
    rect: RectI,
) -> Result<(ID3D11Device, ID3D11DeviceContext, String), AppError> {
    let matched_adapter = find_adapter_for_rect(rect);
    let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
    let levels = [D3D_FEATURE_LEVEL_11_0];

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut adapter_name = String::from("GPU");

    if let Some((adapter, name)) = matched_adapter {
        let base_adapter: IDXGIAdapter = adapter.cast()?;
        let res = unsafe {
            D3D11CreateDevice(
                &base_adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                flags,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        };
        if res.is_ok() && device.is_some() && context.is_some() {
            adapter_name = name;
        } else {
            device = None;
            context = None;
        }
    }

    if device.is_none() || context.is_none() {
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                flags,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        adapter_name = "Hardware GPU".into();
    }

    let device = device.ok_or_else(|| AppError::Other("Kunne ikke oprette D3D11-enhed".into()))?;
    let context = context.ok_or_else(|| AppError::Other("Kunne ikke oprette D3D11-kontekst".into()))?;
    Ok((device, context, adapter_name))
}

fn find_adapter_for_rect(target: RectI) -> Option<(IDXGIAdapter1, String)> {
    let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory1() }.ok()?;
    let mut best_area = 0_i64;
    let mut best: Option<(IDXGIAdapter1, String)> = None;

    let mut a_idx = 0_u32;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(a_idx) } {
        a_idx += 1;
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..end]).trim().to_string();

        let mut o_idx = 0_u32;
        while let Ok(output) = unsafe { adapter.EnumOutputs(o_idx) } {
            o_idx += 1;
            let Ok(out_desc) = (unsafe { output.GetDesc() }) else {
                continue;
            };
            let r = out_desc.DesktopCoordinates;
            let ix0 = target.x.max(r.left);
            let iy0 = target.y.max(r.top);
            let ix1 = (target.x + target.width).min(r.right);
            let iy1 = (target.y + target.height).min(r.bottom);
            let area = ((ix1 - ix0).max(0) as i64) * ((iy1 - iy0).max(0) as i64);
            if area > best_area {
                best_area = area;
                best = Some((adapter.clone(), name.clone()));
            }
        }
    }
    best
}

fn create_presenter(
    device: &ID3D11Device,
    hwnd: windows::Win32::Foundation::HWND,
    render_w: u32,
    render_h: u32,
    mon_w: u32,
    mon_h: u32,
) -> Result<Presenter, AppError> {
    let dxgi_device: IDXGIDevice = device.cast()?;
    let adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter()? };
    let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };

    let comp_desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: render_w.max(1),
        Height: render_h.max(1),
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: 0,
    };

    if let Ok(dcomp) = unsafe { DCompositionCreateDevice::<_, IDCompositionDevice>(&dxgi_device) } {
        if let Ok(swap_chain) =
            unsafe { factory.CreateSwapChainForComposition(device, &comp_desc, None) }
        {
            if let (Ok(target), Ok(visual)) = (
                unsafe { dcomp.CreateTargetForHwnd(hwnd, true) },
                unsafe { dcomp.CreateVisual() },
            ) {
                let mut scale_opt = None;
                if render_w != mon_w || render_h != mon_h {
                    if let Ok(scale) = unsafe { dcomp.CreateScaleTransform() } {
                        let sx = mon_w as f32 / render_w.max(1) as f32;
                        let sy = mon_h as f32 / render_h.max(1) as f32;
                        unsafe {
                            let _ = scale.SetScaleX2(sx);
                            let _ = scale.SetScaleY2(sy);
                            let tf: IDCompositionTransform = scale.cast()?;
                            let _ = visual.SetTransform(&tf);
                            let _ = visual.SetBitmapInterpolationMode(
                                DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR,
                            );
                        }
                        scale_opt = Some(scale);
                    }
                }
                let content_unknown: windows_core::IUnknown = swap_chain.cast()?;
                unsafe {
                    visual.SetContent(&content_unknown)?;
                    target.SetRoot(&visual)?;
                    dcomp.Commit()?;
                }
                return Ok(Presenter::DirectComposition {
                    _dcomp: dcomp,
                    _target: target,
                    _visual: visual,
                    _scale: scale_opt,
                    swap_chain,
                });
            }
        }
    }

    // Fallback to HWND swap chain (Flip Discard, then Discard)
    if let Ok(sc) = unsafe { factory.CreateSwapChainForHwnd(device, hwnd, &comp_desc, None, None) } {
        return Ok(Presenter::DxgiHwnd { swap_chain: sc });
    }
    let fallback_desc = DXGI_SWAP_CHAIN_DESC1 {
        BufferCount: 1,
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        ..comp_desc
    };
    let sc = unsafe { factory.CreateSwapChainForHwnd(device, hwnd, &fallback_desc, None, None)? };
    Ok(Presenter::DxgiHwnd { swap_chain: sc })
}

fn pump_win32_messages() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn system_on_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.ACLineStatus == 0
}

pub fn build_surface_specs(
    app: &AppHandle,
    config: &WallpaperConfig,
    monitors: &[MonitorInfo],
) -> HashMap<String, ResolvedSurfaceSpec> {
    let mut map = HashMap::new();
    for mon in monitors {
        let Some(assignment) = config
            .assignments
            .get(&mon.name)
            .or_else(|| config.assignments.get("*"))
        else {
            continue;
        };

        if super::catalog::find_shader(&assignment.wallpaper_id).is_some() {
            map.insert(
                mon.name.clone(),
                ResolvedSurfaceSpec {
                    monitor_name: mon.name.clone(),
                    rect: mon.rect,
                    work_area: mon.work_area,
                    assignment: assignment.clone(),
                    kind: WallpaperKind::Shader,
                    source_path: None,
                },
            );
        } else if let Some(custom) = config
            .custom_items
            .iter()
            .find(|c| c.id == assignment.wallpaper_id && c.kind == WallpaperKind::Video)
        {
            let p = PathBuf::from(&custom.file_path);
            if p.is_file() {
                map.insert(
                    mon.name.clone(),
                    ResolvedSurfaceSpec {
                        monitor_name: mon.name.clone(),
                        rect: mon.rect,
                        work_area: mon.work_area,
                        assignment: assignment.clone(),
                        kind: WallpaperKind::Video,
                        source_path: Some(p),
                    },
                );
            }
        }
    }
    let _ = app;
    map
}
