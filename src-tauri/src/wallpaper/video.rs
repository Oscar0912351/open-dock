use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use image::{imageops::FilterType, DynamicImage, RgbImage};
use windows::{
    core::{implement, Interface, BSTR},
    Win32::{
        Foundation::{HMODULE, RECT},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
                ID3D11Resource, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
            },
            Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
        },
        Media::MediaFoundation::{
            CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFDXGIDeviceManager, IMFMediaEngine,
            IMFMediaEngineClassFactory, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl,
            MFCreateAttributes, MFCreateDXGIDeviceManager, MFShutdown, MFStartup,
            MFVideoNormalizedRect, MFARGB, MFSTARTUP_NOSOCKET, MF_MEDIA_ENGINE_CALLBACK,
            MF_MEDIA_ENGINE_DXGI_MANAGER, MF_MEDIA_ENGINE_EVENT_CANPLAY,
            MF_MEDIA_ENGINE_EVENT_ERROR, MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY,
            MF_MEDIA_ENGINE_EVENT_LOADEDDATA, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_VERSION,
        },
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
    },
};

use crate::{config::schema::WallpaperFit, error::AppError};

use super::shader_compiler::{read_bgra_staging_to_rgb, save_png_atomic};

pub struct MfSessionGuard(bool);

impl MfSessionGuard {
    pub fn start() -> Result<Self, AppError> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)? };
        Ok(Self(true))
    }
}

impl Drop for MfSessionGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                let _ = MFShutdown();
            }
        }
    }
}

#[implement(IMFMediaEngineNotify)]
struct MediaNotify {
    ready: Arc<AtomicBool>,
    first_frame: Arc<AtomicBool>,
    error_code: Arc<AtomicU32>,
}

impl IMFMediaEngineNotify_Impl for MediaNotify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, _param2: u32) -> windows_core::Result<()> {
        if event == MF_MEDIA_ENGINE_EVENT_CANPLAY.0 as u32
            || event == MF_MEDIA_ENGINE_EVENT_LOADEDDATA.0 as u32
        {
            self.ready.store(true, Ordering::Release);
        } else if event == MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0 as u32 {
            self.ready.store(true, Ordering::Release);
            self.first_frame.store(true, Ordering::Release);
        } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            self.error_code.store((param1 as u32).max(1), Ordering::Release);
        }
        Ok(())
    }
}

pub fn compute_video_rects(
    video_w: u32,
    video_h: u32,
    target_w: u32,
    target_h: u32,
    fit: WallpaperFit,
) -> (MFVideoNormalizedRect, RECT) {
    let vw = video_w.max(1) as f32;
    let vh = video_h.max(1) as f32;
    let tw = target_w.max(1) as f32;
    let th = target_h.max(1) as f32;

    let video_ar = vw / vh;
    let target_ar = tw / th;

    match fit {
        WallpaperFit::Fill | WallpaperFit::Span => {
            let src = if video_ar > target_ar {
                let norm_w = (target_ar / video_ar).clamp(0.01, 1.0);
                let pad = (1.0 - norm_w) * 0.5;
                MFVideoNormalizedRect {
                    left: pad,
                    top: 0.0,
                    right: 1.0 - pad,
                    bottom: 1.0,
                }
            } else {
                let norm_h = (video_ar / target_ar).clamp(0.01, 1.0);
                let pad = (1.0 - norm_h) * 0.5;
                MFVideoNormalizedRect {
                    left: 0.0,
                    top: pad,
                    right: 1.0,
                    bottom: 1.0 - pad,
                }
            };
            (
                src,
                RECT {
                    left: 0,
                    top: 0,
                    right: target_w.max(1) as i32,
                    bottom: target_h.max(1) as i32,
                },
            )
        }
        WallpaperFit::Fit => {
            let scale = (tw / vw).min(th / vh);
            let dw = (vw * scale).round().max(1.0) as i32;
            let dh = (vh * scale).round().max(1.0) as i32;
            let dx = ((target_w as i32 - dw) / 2).max(0);
            let dy = ((target_h as i32 - dh) / 2).max(0);
            (
                MFVideoNormalizedRect {
                    left: 0.0,
                    top: 0.0,
                    right: 1.0,
                    bottom: 1.0,
                },
                RECT {
                    left: dx,
                    top: dy,
                    right: dx + dw,
                    bottom: dy + dh,
                },
            )
        }
        WallpaperFit::Stretch => (
            MFVideoNormalizedRect {
                left: 0.0,
                top: 0.0,
                right: 1.0,
                bottom: 1.0,
            },
            RECT {
                left: 0,
                top: 0,
                right: target_w.max(1) as i32,
                bottom: target_h.max(1) as i32,
            },
        ),
        WallpaperFit::Center => {
            if video_w <= target_w && video_h <= target_h {
                let dx = (target_w as i32 - video_w as i32) / 2;
                let dy = (target_h as i32 - video_h as i32) / 2;
                (
                    MFVideoNormalizedRect {
                        left: 0.0,
                        top: 0.0,
                        right: 1.0,
                        bottom: 1.0,
                    },
                    RECT {
                        left: dx,
                        top: dy,
                        right: dx + video_w as i32,
                        bottom: dy + video_h as i32,
                    },
                )
            } else {
                let norm_w = (tw / vw).min(1.0);
                let norm_h = (th / vh).min(1.0);
                let pad_x = (1.0 - norm_w) * 0.5;
                let pad_y = (1.0 - norm_h) * 0.5;
                let dw = vw.min(tw).round() as i32;
                let dh = vh.min(th).round() as i32;
                let dx = (target_w as i32 - dw) / 2;
                let dy = (target_h as i32 - dh) / 2;
                (
                    MFVideoNormalizedRect {
                        left: pad_x,
                        top: pad_y,
                        right: 1.0 - pad_x,
                        bottom: 1.0 - pad_y,
                    },
                    RECT {
                        left: dx,
                        top: dy,
                        right: dx + dw,
                        bottom: dy + dh,
                    },
                )
            }
        }
    }
}

pub struct ZeroCopyVideoPlayer {
    engine: IMFMediaEngine,
    _dxgi_mgr: IMFDXGIDeviceManager,
    _notify_keepalive: IMFMediaEngineNotify,
    _mf_guard: MfSessionGuard,
    ready: Arc<AtomicBool>,
    first_frame: Arc<AtomicBool>,
    error_code: Arc<AtomicU32>,
    native_size: (u32, u32),
    paused: bool,
    muted: bool,
    speed: f32,
}

impl ZeroCopyVideoPlayer {
    pub fn new(
        device: &ID3D11Device,
        video_path: &Path,
        muted: bool,
        speed: f32,
    ) -> Result<Self, AppError> {
        if !video_path.is_file() {
            return Err(AppError::Other(format!(
                "Videofil findes ikke: {}",
                video_path.display()
            )));
        }

        if let Ok(mt) = device.cast::<ID3D11Multithread>() {
            unsafe {
                let _ = mt.SetMultithreadProtected(true);
            }
        }

        let mf_guard = MfSessionGuard::start()?;

        let mut reset_token = 0_u32;
        let mut dxgi_mgr: Option<IMFDXGIDeviceManager> = None;
        unsafe { MFCreateDXGIDeviceManager(&mut reset_token, &mut dxgi_mgr)? };
        let dxgi_mgr = dxgi_mgr
            .ok_or_else(|| AppError::Other("Kunne ikke oprette DXGI video device manager".into()))?;
        let device_unknown: windows_core::IUnknown = device.cast()?;
        unsafe { dxgi_mgr.ResetDevice(&device_unknown, reset_token)? };

        let mut attrs: Option<IMFAttributes> = None;
        unsafe { MFCreateAttributes(&mut attrs, 3)? };
        let attrs =
            attrs.ok_or_else(|| AppError::Other("Kunne ikke oprette MediaFoundation-attributter".into()))?;

        let ready = Arc::new(AtomicBool::new(false));
        let first_frame = Arc::new(AtomicBool::new(false));
        let error_code = Arc::new(AtomicU32::new(0));
        let notify: IMFMediaEngineNotify = MediaNotify {
            ready: Arc::clone(&ready),
            first_frame: Arc::clone(&first_frame),
            error_code: Arc::clone(&error_code),
        }
        .into();

        let notify_unknown: windows_core::IUnknown = notify.cast()?;
        let mgr_unknown: windows_core::IUnknown = dxgi_mgr.cast()?;
        unsafe {
            attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify_unknown)?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &mgr_unknown)?;
            attrs.SetUINT32(
                &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
            )?;
        }

        let factory: IMFMediaEngineClassFactory = unsafe {
            CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?
        };
        let engine = unsafe { factory.CreateInstance(0, &attrs)? };

        let url = BSTR::from(video_path.to_string_lossy().as_ref());
        unsafe {
            engine.SetMuted(muted)?;
            let _ = engine.SetVolume(if muted { 0.0 } else { 1.0 });
            engine.SetLoop(true)?;
            engine.SetAutoPlay(true)?;
            let _ = engine.SetPlaybackRate(speed as f64);
            engine.SetSource(&url)?;
            let _ = engine.Play();
        }

        Ok(Self {
            engine,
            _dxgi_mgr: dxgi_mgr,
            _notify_keepalive: notify,
            _mf_guard: mf_guard,
            ready,
            first_frame,
            error_code,
            native_size: (1920, 1080),
            paused: false,
            muted,
            speed,
        })
    }

    pub fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        unsafe {
            let res = if paused {
                self.engine.Pause()
            } else {
                self.engine.Play()
            };
            if let Err(error) = res {
                log::debug!("Video pause/play ({paused}) ignoreret: {error}");
            }
        }
    }

    pub fn set_muted(&mut self, muted: bool) {
        if self.muted == muted {
            return;
        }
        self.muted = muted;
        unsafe {
            let _ = self.engine.SetMuted(muted);
            let _ = self.engine.SetVolume(if muted { 0.0 } else { 1.0 });
        }
    }

    pub fn set_speed(&mut self, speed: f32) {
        if (self.speed - speed).abs() < 0.01 {
            return;
        }
        self.speed = speed;
        unsafe {
            let _ = self.engine.SetPlaybackRate(speed as f64);
        }
    }

    pub fn has_error(&self) -> Option<u32> {
        let code = self.error_code.load(Ordering::Acquire);
        if code > 0 { Some(code) } else { None }
    }

    pub fn render_if_ready(
        &mut self,
        back_buffer: &ID3D11Texture2D,
        target_w: u32,
        target_h: u32,
        fit: WallpaperFit,
    ) -> Result<bool, AppError> {
        if !self.ready.load(Ordering::Acquire) {
            return Ok(false);
        }
        if !unsafe { self.engine.HasVideo().as_bool() } {
            return Ok(false);
        }
        let mut vw = 0_u32;
        let mut vh = 0_u32;
        if unsafe { self.engine.GetNativeVideoSize(Some(&mut vw), Some(&mut vh)) }.is_ok()
            && vw > 0
            && vh > 0
        {
            self.native_size = (vw, vh);
        }

        let tick_ok = unsafe { self.engine.OnVideoStreamTick() };
        let Ok(pts) = tick_ok else {
            return Ok(false);
        };
        if pts < 0 && !self.first_frame.load(Ordering::Acquire) {
            return Ok(false);
        }

        let (src, dst) = compute_video_rects(
            self.native_size.0,
            self.native_size.1,
            target_w,
            target_h,
            fit,
        );
        let border = MFARGB {
            rgbBlue: 0,
            rgbGreen: 0,
            rgbRed: 0,
            rgbAlpha: 255,
        };
        let target_unknown: windows_core::IUnknown = back_buffer.cast()?;
        unsafe {
            self.engine
                .TransferVideoFrame(&target_unknown, Some(&src), &dst, Some(&border))?;
        }
        Ok(true)
    }
}

impl Drop for ZeroCopyVideoPlayer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Pause();
            let _ = self.engine.Shutdown();
        }
    }
}



pub fn extract_video_poster(
    video_path: &Path,
    thumb_out: &Path,
    poster_out: Option<&Path>,
) -> Result<(u32, u32), AppError> {
    let _com = crate::util::ComMtaGuard::init();
    let (device, context) = create_video_offscreen_device()?;
    let player = ZeroCopyVideoPlayer::new(&device, video_path, true, 1.0)?;

    let deadline = Instant::now() + Duration::from_secs(4);
    let mut vw = 0_u32;
    let mut vh = 0_u32;

    while Instant::now() < deadline {
        if let Some(code) = player.has_error() {
            return Err(AppError::Other(format!(
                "Kunne ikke afkode videofilen (MediaFoundation-fejl {code})"
            )));
        }
        if player.ready.load(Ordering::Acquire) && unsafe { player.engine.HasVideo().as_bool() } {
            let _ = unsafe { player.engine.GetNativeVideoSize(Some(&mut vw), Some(&mut vh)) };
            if vw > 0 && vh > 0 {
                if let Ok(pts) = unsafe { player.engine.OnVideoStreamTick() } {
                    if pts >= 0 || player.first_frame.load(Ordering::Acquire) {
                        break;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    if vw == 0 || vh == 0 {
        return Err(AppError::Other(
            "Videofilen indeholder intet video-spor som Windows Media Foundation kan afkode".into(),
        ));
    }

    let cap_w = vw.min(1280);
    let cap_h = ((cap_w as f64 * vh as f64 / vw as f64).round() as u32).max(1);

    let tex_desc = D3D11_TEXTURE2D_DESC {
        Width: cap_w,
        Height: cap_h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut rt_tex: Option<ID3D11Texture2D> = None;
    unsafe { device.CreateTexture2D(&tex_desc, None, Some(&mut rt_tex))? };
    let rt_tex = rt_tex.ok_or_else(|| AppError::Other("Kunne ikke oprette video-poster-tekstur".into()))?;

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..tex_desc
    };
    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging))? };
    let staging = staging.ok_or_else(|| AppError::Other("Kunne ikke oprette staging-tekstur".into()))?;

    let src = MFVideoNormalizedRect {
        left: 0.0,
        top: 0.0,
        right: 1.0,
        bottom: 1.0,
    };
    let dst = RECT {
        left: 0,
        top: 0,
        right: cap_w as i32,
        bottom: cap_h as i32,
    };
    let border = MFARGB {
        rgbBlue: 0,
        rgbGreen: 0,
        rgbRed: 0,
        rgbAlpha: 255,
    };
    let rt_unknown: windows_core::IUnknown = rt_tex.cast()?;
    unsafe {
        player
            .engine
            .TransferVideoFrame(&rt_unknown, Some(&src), &dst, Some(&border))?;
        let rt_res: ID3D11Resource = rt_tex.cast()?;
        let st_res: ID3D11Resource = staging.cast()?;
        context.CopyResource(&st_res, &rt_res);
    }

    let st_res: ID3D11Resource = staging.cast()?;
    let rgb = read_bgra_staging_to_rgb(&context, &st_res, cap_w, cap_h)?;

    if let Some(poster) = poster_out {
        save_png_atomic(&rgb, poster)?;
    }

    let thumb_rgb = cover_resize_rgb(&rgb, 640, 360);
    save_png_atomic(&thumb_rgb, thumb_out)?;

    Ok((vw, vh))
}

pub fn cover_resize_rgb(img: &RgbImage, target_w: u32, target_h: u32) -> RgbImage {
    let (iw, ih) = img.dimensions();
    if iw == 0 || ih == 0 {
        return RgbImage::new(target_w, target_h);
    }
    let scale = (target_w as f64 / iw as f64).max(target_h as f64 / ih as f64);
    let crop_w = ((target_w as f64 / scale).round() as u32).clamp(1, iw);
    let crop_h = ((target_h as f64 / scale).round() as u32).clamp(1, ih);
    let cx = (iw - crop_w) / 2;
    let cy = (ih - crop_h) / 2;

    let dyn_img = DynamicImage::ImageRgb8(img.clone());
    let cropped = dyn_img.crop_imm(cx, cy, crop_w, crop_h);
    cropped
        .resize_exact(target_w, target_h, FilterType::Lanczos3)
        .to_rgb8()
}

fn create_video_offscreen_device() -> Result<(ID3D11Device, ID3D11DeviceContext), AppError> {
    let levels = [D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT;

    let hw = unsafe {
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
            )?;
        }
    }
    let device = device.ok_or_else(|| AppError::Other("Kunne ikke oprette D3D11 videoenhed".into()))?;
    let context = context.ok_or_else(|| AppError::Other("Kunne ikke oprette D3D11 videokontekst".into()))?;
    Ok((device, context))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_video_rects_handles_all_fits() {
        // 16:9 video on 21:9 monitor with Fill -> vertical center crop
        let (src, dst) = compute_video_rects(1920, 1080, 2560, 1080, WallpaperFit::Fill);
        assert_eq!(src.left, 0.0);
        assert_eq!(src.right, 1.0);
        assert!(src.top > 0.10 && src.bottom < 0.90);
        assert_eq!(dst.right, 2560);
        assert_eq!(dst.bottom, 1080);

        // 4:3 video on 16:9 monitor with Fit -> horizontal letterbox
        let (src_fit, dst_fit) = compute_video_rects(1600, 1200, 1920, 1080, WallpaperFit::Fit);
        assert_eq!(src_fit.left, 0.0);
        assert_eq!(src_fit.right, 1.0);
        assert_eq!(dst_fit.top, 0);
        assert_eq!(dst_fit.bottom, 1080);
        assert_eq!(dst_fit.left, 240);
        assert_eq!(dst_fit.right - dst_fit.left, 1440);

        // Stretch covers full dest with full src
        let (src_str, dst_str) = compute_video_rects(1280, 720, 1920, 1080, WallpaperFit::Stretch);
        assert_eq!((src_str.left, src_str.top, src_str.right, src_str.bottom), (0.0, 0.0, 1.0, 1.0));
        assert_eq!((dst_str.left, dst_str.top, dst_str.right, dst_str.bottom), (0, 0, 1920, 1080));

        // Center smaller video preserves 1:1 pixel dims
        let (src_ctr, dst_ctr) = compute_video_rects(800, 600, 1920, 1080, WallpaperFit::Center);
        assert_eq!((src_ctr.left, src_ctr.right), (0.0, 1.0));
        assert_eq!(dst_ctr.left, 560);
        assert_eq!(dst_ctr.top, 240);
        assert_eq!(dst_ctr.right - dst_ctr.left, 800);
        assert_eq!(dst_ctr.bottom - dst_ctr.top, 600);
    }
}
