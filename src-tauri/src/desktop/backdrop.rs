use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem::size_of,
    sync::{LazyLock, OnceLock},
};

use parking_lot::{Mutex, RwLock};
use windows::{
    core::{h, w, Interface, BOOL, HSTRING, PCWSTR, PWSTR},
    Foundation::{IPropertyValue, PropertyValue, TimeSpan},
    Graphics::{IGeometrySource2D, IGeometrySource2D_Impl},
    Graphics::Effects::{IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl},
    System::{DispatcherQueue, DispatcherQueueController},
    UI::Composition::{
        CompositionBackdropBrush, CompositionBitmapInterpolationMode, CompositionEffectBrush,
        CompositionGeometry, CompositionPath, CompositionRoundedRectangleGeometry,
        CompositionStretch, CompositionSurfaceBrush, Compositor, SpriteVisual,
        Desktop::DesktopWindowTarget,
    },
    Win32::{
        Foundation::{E_INVALIDARG, E_NOTIMPL, ERROR_SUCCESS, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Direct2D::{
                CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation, D2D1CreateFactory,
                Common::{
                    D2D1_BORDER_MODE_HARD, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
                },
                ID2D1Factory, ID2D1Geometry, D2D1_FACTORY_TYPE_MULTI_THREADED,
                D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
            },
            Dwm::{
                DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
                DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_HOSTBACKDROPBRUSH,
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
            },
            Dxgi::IDXGISwapChain1,
            Gdi::HBRUSH,
        },
        System::{
            Com::CoTaskMemFree,
            LibraryLoader::GetModuleHandleW,
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
            WinRT::{
                CreateDispatcherQueueController,
                Composition::{ICompositorDesktopInterop, ICompositorInterop},
                DispatcherQueueOptions,
                Graphics::Direct2D::{
                    IGeometrySource2DInterop, IGeometrySource2DInterop_Impl,
                    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl,
                    GRAPHICS_EFFECT_PROPERTY_MAPPING,
                    GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT,
                },
                DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
            },
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindow, GetWindowLongPtrW,
            GetWindowRect, IsWindow, IsWindowVisible, RegisterClassExW, SetWindowDisplayAffinity,
            SetWindowPos, ShowWindow, GWL_EXSTYLE, GW_HWNDNEXT, HWND_NOTOPMOST, HWND_TOPMOST,
            SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
            SW_HIDE, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WNDCLASSEXW, WS_EX_NOACTIVATE,
            WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
            WS_POPUP,
        },
    },
};
use windows_core::implement;
use windows_numerics::Vector2;

use crate::config::{
    geometry::{
        cluster_radius_base, laerk_radius_multiplier, squircle_multiplier, widget_radius_base,
    },
    schema::{Layer, RadiusPreset, ThemeId, VerticalAnchor, WidgetSize},
};

use super::{
    backdrop_capture::{self, CaptureTargetSpec, SendSwapChain},
    glass::{self, LiveBackdropMode},
    plate,
};

const BACKDROP_CLASS_NAME: PCWSTR = w!("OpenDockGlassBackdrop");

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
    let raw = BACKDROP_MAP.lock().get(&(widget_hwnd.0 as isize)).copied()?;
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

fn supports_css_squircle() -> bool {
    static SQUIRCLE_SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SQUIRCLE_SUPPORTED.get_or_init(|| {
        let mut raw_ver = PWSTR::null();
        let res = unsafe {
            webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString(
                PCWSTR::null(),
                &mut raw_ver,
            )
        };
        if res.is_err() || raw_ver.0.is_null() {
            return true;
        }
        let ver_str = unsafe { raw_ver.to_string() }.unwrap_or_default();
        unsafe {
            CoTaskMemFree(Some(raw_ver.0 as *const _));
        }
        let major = ver_str
            .split(|c: char| c == '.' || c.is_whitespace())
            .next()
            .and_then(|part| part.parse::<u32>().ok())
            .unwrap_or(139);
        major >= 139
    })
}

pub fn compute_corner_geometry(
    spec: &BackdropSpec,
    phys_w: f32,
    phys_h: f32,
    squircle_supported: bool,
) -> (f32, bool) {
    let r_base = if spec.is_cluster {
        cluster_radius_base(spec.radius)
    } else {
        widget_radius_base(spec.radius, spec.size)
    };

    let r_mult = match spec.theme {
        ThemeId::Laerk => laerk_radius_multiplier(),
        ThemeId::Aurora | ThemeId::Grafit => 1.0_f32,
    };

    let u_phys = phys_w.max(1.0) / (spec.base_width.max(1.0) as f32);
    let r_widget = r_base * r_mult * u_phys;

    let is_round_pill =
        !spec.is_cluster && spec.size == WidgetSize::Pill && spec.radius == RadiusPreset::Extra;
    let use_squircle = squircle_supported && !is_round_pill;
    let raw_r = if use_squircle {
        r_widget * squircle_multiplier()
    } else {
        r_widget
    };
    let max_r = (phys_w * 0.5).min(phys_h * 0.5).max(0.0);
    (raw_r.clamp(0.0, max_r), use_squircle)
}

pub fn compute_blur_and_saturation(
    theme: ThemeId,
    glass_strength: f32,
    dpi_scale: f64,
) -> (f32, f32) {
    let (blur_logical, sat) = plate::theme_glass_params(theme, glass_strength);
    let scale = if dpi_scale.is_finite() {
        dpi_scale.max(0.5) as f32
    } else {
        1.0
    };
    let sigma = (blur_logical * scale).clamp(0.0, 200.0);
    (sigma, sat)
}

fn push_unique_point(pts: &mut Vec<Vector2>, p: Vector2) {
    if let Some(last) = pts.last() {
        if (last.X - p.X).abs() < 1e-4 && (last.Y - p.Y).abs() < 1e-4 {
            return;
        }
    }
    pts.push(p);
}

/// Bygger lukket polygon-approksimation af Chromium's `corner-shape: squircle`
/// (`superellipse(2)` => |x|^4 + |y|^4 = 1) med 24 segmenter pr. hjørne (45°-split).
pub fn build_squircle_points(x0: f32, y0: f32, w: f32, h: f32, r: f32) -> Vec<Vector2> {
    let x1 = x0 + w;
    let y1 = y0 + h;
    let r = r.clamp(0.0, (w * 0.5).min(h * 0.5));
    if r <= 0.25 {
        return vec![
            Vector2::new(x0, y0),
            Vector2::new(x1, y0),
            Vector2::new(x1, y1),
            Vector2::new(x0, y1),
        ];
    }

    const HALF_STEPS: usize = 12;
    let t_mid = 2.0_f32.powf(-0.25); // 2^(-1/4) ≈ 0.8408964
    let mut pts = Vec::with_capacity(4 * (HALF_STEPS * 2 + 1));

    // Øverste venstre hjørne: (x0, y0 + r) -> (x0 + r, y0)
    let cx = x0 + r;
    let cy = y0 + r;
    for i in 0..=HALF_STEPS {
        let v = t_mid * (i as f32 / HALF_STEPS as f32);
        let u = (1.0 - v.powi(4)).max(0.0).powf(0.25);
        push_unique_point(&mut pts, Vector2::new(cx - u * r, cy - v * r));
    }
    for i in 1..=HALF_STEPS {
        let u = t_mid * (1.0 - i as f32 / HALF_STEPS as f32);
        let v = (1.0 - u.powi(4)).max(0.0).powf(0.25);
        push_unique_point(&mut pts, Vector2::new(cx - u * r, cy - v * r));
    }

    // Øverste højre hjørne: (x1 - r, y0) -> (x1, y0 + r)
    let cx = x1 - r;
    let cy = y0 + r;
    for i in 0..=HALF_STEPS {
        let u = t_mid * (i as f32 / HALF_STEPS as f32);
        let v = (1.0 - u.powi(4)).max(0.0).powf(0.25);
        push_unique_point(&mut pts, Vector2::new(cx + u * r, cy - v * r));
    }
    for i in 1..=HALF_STEPS {
        let v = t_mid * (1.0 - i as f32 / HALF_STEPS as f32);
        let u = (1.0 - v.powi(4)).max(0.0).powf(0.25);
        push_unique_point(&mut pts, Vector2::new(cx + u * r, cy - v * r));
    }

    // Nederste højre hjørne: (x1, y1 - r) -> (x1 - r, y1)
    let cx = x1 - r;
    let cy = y1 - r;
    for i in 0..=HALF_STEPS {
        let v = t_mid * (i as f32 / HALF_STEPS as f32);
        let u = (1.0 - v.powi(4)).max(0.0).powf(0.25);
        push_unique_point(&mut pts, Vector2::new(cx + u * r, cy + v * r));
    }
    for i in 1..=HALF_STEPS {
        let u = t_mid * (1.0 - i as f32 / HALF_STEPS as f32);
        let v = (1.0 - u.powi(4)).max(0.0).powf(0.25);
        push_unique_point(&mut pts, Vector2::new(cx + u * r, cy + v * r));
    }

    // Nederste venstre hjørne: (x0 + r, y1) -> (x0, y1 - r)
    let cx = x0 + r;
    let cy = y1 - r;
    for i in 0..=HALF_STEPS {
        let u = t_mid * (i as f32 / HALF_STEPS as f32);
        let v = (1.0 - u.powi(4)).max(0.0).powf(0.25);
        push_unique_point(&mut pts, Vector2::new(cx - u * r, cy + v * r));
    }
    for i in 1..=HALF_STEPS {
        let v = t_mid * (1.0 - i as f32 / HALF_STEPS as f32);
        let u = (1.0 - v.powi(4)).max(0.0).powf(0.25);
        push_unique_point(&mut pts, Vector2::new(cx - u * r, cy + v * r));
    }

    if pts.len() > 1 {
        let first = pts[0];
        if let Some(last) = pts.last() {
            if (last.X - first.X).abs() < 1e-4 && (last.Y - first.Y).abs() < 1e-4 {
                pts.pop();
            }
        }
    }
    pts
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct D2dGaussianBlurEffect {
    name: RwLock<HSTRING>,
    std_dev: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for D2dGaussianBlurEffect_Impl {}

impl IGraphicsEffect_Impl for D2dGaussianBlurEffect_Impl {
    fn Name(&self) -> windows_core::Result<HSTRING> {
        Ok(self.name.read().clone())
    }

    fn SetName(&self, name: &HSTRING) -> windows_core::Result<()> {
        *self.name.write() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for D2dGaussianBlurEffect_Impl {
    fn GetEffectId(&self) -> windows_core::Result<windows_core::GUID> {
        Ok(CLSID_D2D1GaussianBlur)
    }

    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> windows_core::Result<()> {
        if index.is_null() || mapping.is_null() {
            return Err(windows_core::Error::from(E_INVALIDARG));
        }
        let prop = unsafe { name.to_string() }.unwrap_or_default();
        if prop.eq_ignore_ascii_case("BlurAmount") {
            unsafe {
                *index = 0;
                *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
            }
            return Ok(());
        }
        Err(windows_core::Error::from(E_INVALIDARG))
    }

    fn GetPropertyCount(&self) -> windows_core::Result<u32> {
        Ok(3)
    }

    fn GetProperty(&self, index: u32) -> windows_core::Result<IPropertyValue> {
        match index {
            // D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION = 0
            0 => PropertyValue::CreateSingle(self.std_dev)?.cast(),
            // D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION = 1
            1 => PropertyValue::CreateUInt32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)?.cast(),
            // D2D1_GAUSSIANBLUR_PROP_BORDER_MODE = 2 (HARD hindrer kant-fade)
            2 => PropertyValue::CreateUInt32(D2D1_BORDER_MODE_HARD.0 as u32)?.cast(),
            _ => Err(windows_core::Error::from(E_INVALIDARG)),
        }
    }

    fn GetSource(&self, index: u32) -> windows_core::Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(windows_core::Error::from(E_INVALIDARG))
        }
    }

    fn GetSourceCount(&self) -> windows_core::Result<u32> {
        Ok(1)
    }
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct D2dSaturationEffect {
    name: RwLock<HSTRING>,
    saturation: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for D2dSaturationEffect_Impl {}

impl IGraphicsEffect_Impl for D2dSaturationEffect_Impl {
    fn Name(&self) -> windows_core::Result<HSTRING> {
        Ok(self.name.read().clone())
    }

    fn SetName(&self, name: &HSTRING) -> windows_core::Result<()> {
        *self.name.write() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for D2dSaturationEffect_Impl {
    fn GetEffectId(&self) -> windows_core::Result<windows_core::GUID> {
        Ok(CLSID_D2D1Saturation)
    }

    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> windows_core::Result<()> {
        if index.is_null() || mapping.is_null() {
            return Err(windows_core::Error::from(E_INVALIDARG));
        }
        let prop = unsafe { name.to_string() }.unwrap_or_default();
        if prop.eq_ignore_ascii_case("Saturation") {
            unsafe {
                *index = 0;
                *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
            }
            return Ok(());
        }
        Err(windows_core::Error::from(E_INVALIDARG))
    }

    fn GetPropertyCount(&self) -> windows_core::Result<u32> {
        Ok(1)
    }

    fn GetProperty(&self, index: u32) -> windows_core::Result<IPropertyValue> {
        match index {
            // D2D1_SATURATION_PROP_SATURATION = 0
            0 => PropertyValue::CreateSingle(self.saturation)?.cast(),
            _ => Err(windows_core::Error::from(E_INVALIDARG)),
        }
    }

    fn GetSource(&self, index: u32) -> windows_core::Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(windows_core::Error::from(E_INVALIDARG))
        }
    }

    fn GetSourceCount(&self) -> windows_core::Result<u32> {
        Ok(1)
    }
}

fn build_d2d_path_geometry(
    factory: &ID2D1Factory,
    pts: &[Vector2],
) -> windows_core::Result<ID2D1Geometry> {
    let path = unsafe { factory.CreatePathGeometry()? };
    let sink = unsafe { path.Open()? };
    unsafe {
        sink.BeginFigure(pts[0], D2D1_FIGURE_BEGIN_FILLED);
        for &pt in &pts[1..] {
            sink.AddLine(pt);
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
    }
    path.cast()
}

#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct PathGeometrySource {
    geometry: ID2D1Geometry,
    pts: Vec<Vector2>,
}

impl IGeometrySource2D_Impl for PathGeometrySource_Impl {}

impl IGeometrySource2DInterop_Impl for PathGeometrySource_Impl {
    fn GetGeometry(&self) -> windows_core::Result<ID2D1Geometry> {
        Ok(self.geometry.clone())
    }

    fn TryGetGeometryUsingFactory(
        &self,
        factory: windows_core::Ref<ID2D1Factory>,
    ) -> windows_core::Result<ID2D1Geometry> {
        if let Some(fac) = factory.as_ref() {
            if self.pts.len() >= 3 {
                return build_d2d_path_geometry(fac, &self.pts);
            }
        }
        Err(windows_core::Error::from(E_NOTIMPL))
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

static CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

thread_local! {
    static RUNTIME: RefCell<Option<BackdropRuntime>> = const { RefCell::new(None) };
    static SYNCING: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "system" fn backdrop_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn ensure_class_registered() -> windows_core::Result<()> {
    if CLASS_REGISTERED.get().is_some() {
        return Ok(());
    }
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(backdrop_wnd_proc),
            hInstance: hinstance.into(),
            hbrBackground: HBRUSH::default(),
            lpszClassName: BACKDROP_CLASS_NAME,
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
    }
    let _ = CLASS_REGISTERED.set(());
    Ok(())
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

fn set_exclude_from_capture(hwnd: HWND, exclude: bool) -> windows_core::Result<()> {
    let affinity = if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    unsafe { SetWindowDisplayAffinity(hwnd, affinity) }
}

fn build_surface_brush(
    compositor: &Compositor,
    swap_chain: &IDXGISwapChain1,
) -> windows_core::Result<CompositionSurfaceBrush> {
    let interop: ICompositorInterop = compositor.cast()?;
    let surface = unsafe { interop.CreateCompositionSurfaceForSwapChain(swap_chain)? };
    let brush = compositor.CreateSurfaceBrushWithSurface(&surface)?;
    brush.SetStretch(CompositionStretch::Fill)?;
    brush.SetBitmapInterpolationMode(CompositionBitmapInterpolationMode::Linear)?;
    Ok(brush)
}

fn create_backdrop_hwnd(
    rect: RECT,
    topmost: bool,
    use_host_backdrop: bool,
) -> windows_core::Result<HWND> {
    let hinstance = unsafe { GetModuleHandleW(None)? };
    let w = (rect.right - rect.left).max(1);
    let h = (rect.bottom - rect.top).max(1);
    let mut ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
    if topmost {
        ex |= WS_EX_TOPMOST;
    }

    let hwnd = unsafe {
        CreateWindowExW(
            ex,
            BACKDROP_CLASS_NAME,
            w!(""),
            WS_POPUP,
            rect.left,
            rect.top,
            w,
            h,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?
    };

    if use_host_backdrop {
        let enable = BOOL(1);
        if let Err(err) = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                &enable as *const BOOL as *const _,
                size_of::<BOOL>() as u32,
            )
        } {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(err);
        }
    }

    let corner = DWMWCP_DONOTROUND;
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            size_of::<windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    };
    let border_none = DWMWA_COLOR_NONE;
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border_none as *const u32 as *const _,
            size_of::<u32>() as u32,
        )
    };
    let disable_transitions = BOOL(1);
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &disable_transitions as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        )
    };

    Ok(hwnd)
}

fn build_effect_brush(
    compositor: &Compositor,
    sigma: f32,
    sat: f32,
) -> windows_core::Result<CompositionEffectBrush> {
    let source_param = windows::UI::Composition::CompositionEffectSourceParameter::Create(h!("source"))?;
    let param_source: IGraphicsEffectSource = source_param.cast()?;

    let blur = D2dGaussianBlurEffect {
        name: RwLock::new(HSTRING::from("Blur")),
        std_dev: sigma,
        source: param_source,
    };
    let blur_source: IGraphicsEffectSource = blur.into();

    let root_effect: IGraphicsEffect = if (sat - 1.0).abs() > 0.005 {
        let sat_fx = D2dSaturationEffect {
            name: RwLock::new(HSTRING::from("Saturation")),
            saturation: sat,
            source: blur_source,
        };
        let sat_source: IGraphicsEffectSource = sat_fx.into();
        sat_source.cast()?
    } else {
        blur_source.cast()?
    };

    let factory = compositor.CreateEffectFactory(&root_effect)?;
    let brush = factory.CreateBrush()?;
    let host_backdrop: CompositionBackdropBrush = compositor.CreateHostBackdropBrush()?;
    brush.SetSourceParameter(h!("source"), &host_backdrop)?;
    Ok(brush)
}

fn build_clip(
    compositor: &Compositor,
    d2d_factory: &ID2D1Factory,
    spec: &BackdropSpec,
    phys_w: f32,
    phys_h: f32,
) -> windows_core::Result<windows::UI::Composition::CompositionGeometricClip> {
    let (r, use_squircle) =
        compute_corner_geometry(spec, phys_w, phys_h, supports_css_squircle());

    let x0 = 0.5_f32;
    let y0 = 0.5_f32;
    let w = (phys_w - 1.0).max(1.0);
    let h = (phys_h - 1.0).max(1.0);
    let r_inset = (r - 0.5).clamp(0.0, (w * 0.5).min(h * 0.5));

    if use_squircle && r_inset > 0.25 {
        let pts = build_squircle_points(x0, y0, w, h, r_inset);
        if pts.len() >= 3 {
            let d2d_geom = build_d2d_path_geometry(d2d_factory, &pts)?;
            let geom_source: IGeometrySource2D = PathGeometrySource {
                geometry: d2d_geom,
                pts,
            }
            .into();
            let comp_path = CompositionPath::Create(&geom_source)?;
            let path_geom = compositor.CreatePathGeometryWithPath(&comp_path)?;
            let comp_geom: CompositionGeometry = path_geom.cast()?;
            return compositor.CreateGeometricClipWithGeometry(&comp_geom);
        }
    }

    let rr = compositor.CreateRoundedRectangleGeometry()?;
    rr.SetOffset(Vector2::new(x0, y0))?;
    rr.SetSize(Vector2::new(w, h))?;
    rr.SetCornerRadius(Vector2::new(r_inset, r_inset))?;
    let comp_geom: CompositionGeometry = rr.cast()?;
    compositor.CreateGeometricClipWithGeometry(&comp_geom)
}

fn spec_clip_key(spec: &BackdropSpec, w: i32, h: i32) -> (i32, i32, u8, u8, u8, bool, i32) {
    let radius_k = match spec.radius {
        RadiusPreset::Soft => 0,
        RadiusPreset::Round => 1,
        RadiusPreset::Extra => 2,
    };
    let theme_k = match spec.theme {
        ThemeId::Laerk => 0,
        ThemeId::Aurora => 1,
        ThemeId::Grafit => 2,
    };
    let size_k = match spec.size {
        WidgetSize::Pill => 0,
        WidgetSize::S => 1,
        WidgetSize::M => 2,
        WidgetSize::L => 3,
    };
    (
        w,
        h,
        radius_k,
        theme_k,
        size_k,
        spec.is_cluster,
        spec.base_width.round() as i32,
    )
}

fn widget_is_topmost(widget_hwnd: HWND) -> bool {
    let ex = unsafe { GetWindowLongPtrW(widget_hwnd, GWL_EXSTYLE) as u32 };
    (ex & WS_EX_TOPMOST.0) != 0
}

fn backdrop_is_topmost(backdrop_hwnd: HWND) -> bool {
    let ex = unsafe { GetWindowLongPtrW(backdrop_hwnd, GWL_EXSTYLE) as u32 };
    (ex & WS_EX_TOPMOST.0) != 0
}

fn align_backdrop_to_widget(widget_hwnd: HWND, backdrop_hwnd: HWND, hidden_idle: bool) {
    let mut w_rect = RECT::default();
    if unsafe { GetWindowRect(widget_hwnd, &mut w_rect) }.is_err() {
        return;
    }
    let w = (w_rect.right - w_rect.left).max(1);
    let h = (w_rect.bottom - w_rect.top).max(1);
    let want_visible = unsafe { IsWindowVisible(widget_hwnd).as_bool() } && !hidden_idle;

    let mut b_rect = RECT::default();
    let got_b_rect = unsafe { GetWindowRect(backdrop_hwnd, &mut b_rect) }.is_ok();
    let b_visible = unsafe { IsWindowVisible(backdrop_hwnd).as_bool() };
    let b_directly_behind = unsafe { GetWindow(widget_hwnd, GW_HWNDNEXT) }
        .map(|next| next == backdrop_hwnd)
        .unwrap_or(false);
    let w_topmost = widget_is_topmost(widget_hwnd);
    let b_topmost = backdrop_is_topmost(backdrop_hwnd);

    if got_b_rect
        && b_rect == w_rect
        && b_visible == want_visible
        && b_directly_behind
        && w_topmost == b_topmost
    {
        return;
    }

    if w_topmost != b_topmost {
        let band = if w_topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        if let Err(err) = unsafe {
            SetWindowPos(
                backdrop_hwnd,
                Some(band),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        } {
            log::warn!("align_backdrop_to_widget SetWindowPos fejlede: {err}");
        }
    }

    let mut flags = SWP_NOACTIVATE;
    if want_visible && !b_visible {
        flags |= SWP_SHOWWINDOW;
    } else if !want_visible && b_visible {
        flags |= SWP_HIDEWINDOW;
    }
    if got_b_rect && b_rect == w_rect {
        flags |= SWP_NOMOVE | SWP_NOSIZE;
    }
    if b_directly_behind && w_topmost == b_topmost {
        flags |= SWP_NOZORDER;
    }

    if let Err(err) = unsafe {
        SetWindowPos(
            backdrop_hwnd,
            Some(widget_hwnd),
            w_rect.left,
            w_rect.top,
            w,
            h,
            flags,
        )
    } {
        log::warn!("align_backdrop_to_widget SetWindowPos fejlede: {err}");
    }
}

fn remove_entry_locked(rt: &mut BackdropRuntime, key: isize, widget_hwnd: HWND) {
    BACKDROP_MAP.lock().remove(&key);
    backdrop_capture::unregister(key);
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
            let target = match rt.compositor.cast::<ICompositorDesktopInterop>().and_then(|interop| {
                unsafe { interop.CreateDesktopWindowTarget(backdrop_hwnd, true) }
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

            let sc = match backdrop_capture::register_sync(CaptureTargetSpec {
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

            let target = match rt.compositor.cast::<ICompositorDesktopInterop>().and_then(|interop| {
                unsafe { interop.CreateDesktopWindowTarget(backdrop_hwnd, true) }
            }) {
                Ok(t) => t,
                Err(err) => {
                    log::warn!("CreateDesktopWindowTarget (capture) fejlede: {err}");
                    backdrop_capture::unregister(key);
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
                    backdrop_capture::unregister(key);
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
                backdrop_capture::unregister(key);
                unsafe {
                    let _ = set_exclude_from_capture(widget_hwnd, false);
                    let _ = DestroyWindow(backdrop_hwnd);
                }
                return None;
            }

            if !w_visible {
                backdrop_capture::set_paused(key, true);
            }

            BACKDROP_MAP.lock().insert(key, backdrop_hwnd.0 as isize);
            align_backdrop_to_widget(widget_hwnd, backdrop_hwnd, false);
            if w_visible {
                backdrop_capture::request_redraw(key);
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
            backdrop_capture::update_params_nonblocking(key, sigma, sat);
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
            backdrop_capture::update_nonblocking(CaptureTargetSpec {
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

pub fn attach_capture_swapchain(
    widget_hwnd: HWND,
    generation: u64,
    swap_chain: SendSwapChain,
) {
    if !super::is_ui_thread() {
        return;
    }
    let key = widget_hwnd.0 as isize;
    if !backdrop_capture::is_generation_current(key, generation) {
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
                    backdrop_capture::request_redraw(key);
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
            let _ = entry.visual.SetSize(Vector2::new(phys_w as f32, phys_h as f32));
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
            backdrop_capture::update_nonblocking(CaptureTargetSpec {
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
            backdrop_capture::set_paused(key, !(entry.window_visible && !entry.hidden_idle));
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
                backdrop_capture::set_paused(key, !(w_visible && !entry.hidden_idle));
            }
            return;
        }
        entry.hidden_idle = !visible;
        if entry.mode == LiveBackdropMode::CaptureD2d {
            backdrop_capture::set_paused(key, !(w_visible && !entry.hidden_idle));
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
                    if let Ok(ease) = rt
                        .compositor
                        .CreateCubicBezierEasingFunction(Vector2::new(0.22, 0.61), Vector2::new(0.36, 1.0))
                    {
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
    backdrop_capture::unregister(key);
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
    backdrop_capture::shutdown_all();
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
