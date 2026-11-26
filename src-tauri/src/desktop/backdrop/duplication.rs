use std::{
    mem::{size_of, ManuallyDrop},
    sync::{mpsc::SyncSender, Arc},
    time::{Duration, Instant},
};

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
                D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_BORDER_EDGE_MODE_CLAMP,
                D2D1_BORDER_PROP_EDGE_MODE_X, D2D1_BORDER_PROP_EDGE_MODE_Y,
                D2D1_COLORMATRIX_PROP_ALPHA_MODE, D2D1_COLORMATRIX_PROP_COLOR_MATRIX,
                D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_MULTI_THREADED,
                D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED, D2D1_GAUSSIANBLUR_PROP_BORDER_MODE,
                D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION,
                D2D1_INTERPOLATION_MODE_LINEAR, D2D1_PROPERTY_TYPE_UNKNOWN,
                D2D1_SATURATION_PROP_SATURATION,
            },
            Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
                ID3D11Resource, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
                D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
            },
            Dxgi::{
                Common::{
                    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory2,
                IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource, IDXGISurface,
                IDXGISwapChain1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_DEVICE_REMOVED,
                DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_NOT_CURRENTLY_AVAILABLE,
                DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
                DXGI_OUTDUPL_MOVE_RECT, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
                DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
    },
};
use windows_numerics::Vector2;

use super::capture::{
    bump_generation, fail_pending_replies, influence_rect, is_generation_current, rect_dims,
    rects_intersect, regions_to_copy, remove_worker_if_same, CaptureTargetSpec, MonitorKey,
    SendSwapChain, WorkerControl,
};
use super::hdr::{
    device_name_display, ensure_hdr_input_cache, query_monitor_color_info, HdrInputCache,
    HdrToSdrConverter, MonitorColorInfo,
};

pub(super) struct TargetRuntime {
    pub(super) spec: CaptureTargetSpec,
    pub(super) phys_w: u32,
    pub(super) phys_h: u32,
    pub(super) swapchain_gen: u64,
    pub(super) needs_swapchain: bool,
    pub(super) needs_redraw: bool,
    pub(super) first_drawn: bool,
    pub(super) logged_first_present: bool,
    pub(super) last_draw_err: Option<(HRESULT, Instant)>,
    pub(super) last_redraw_at: Option<Instant>,
    pub(super) rect_changed_at: Instant,
    pub(super) pending_reply: Option<SyncSender<Result<SendSwapChain, String>>>,
    pub(super) swap_chain: Option<IDXGISwapChain1>,
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
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 1.0,
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
            return Err(format!(
                "Ikke-understøttet capture-format: {:?}",
                desc.Format
            ));
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
        let back_surface: IDXGISurface =
            unsafe { swap_chain.GetBuffer(0) }.map_err(|e| e.code())?;
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
        let target_bitmap = unsafe {
            self.dc
                .CreateBitmapFromDxgiSurface(&back_surface, Some(&bmp_props))
        }
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
    hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET || hr == D2DERR_RECREATE_TARGET
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
    if let Some(app) = crate::desktop::app_handle() {
        let sc = SendSwapChain(swap_chain);
        let _ = app.run_on_main_thread(move || {
            super::attach_capture_swapchain(HWND(widget_hwnd as _), gen_id, sc);
        });
    }
}

pub(super) fn target_min_interval(rect_changed_at: Instant, now: Instant) -> Duration {
    if now.saturating_duration_since(rect_changed_at) < Duration::from_millis(120) {
        Duration::from_millis(8)
    } else {
        Duration::from_millis(16)
    }
}

pub(super) fn acquire_min_interval(
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

pub(super) fn redraw_wait_ms(
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

pub(super) fn monitor_worker_loop(ctrl: Arc<WorkerControl>) {
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
                            && !s
                                .targets
                                .values()
                                .any(|t| !t.spec.paused || t.needs_swapchain)
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
                                all_dirty_mon
                                    .extend_from_slice(&dirty_buf[..cnt.min(dirty_buf.len())]);
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
                                        log::warn!(
                                            "Capture desktop-format opdatering fejlede: {err}"
                                        );
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
