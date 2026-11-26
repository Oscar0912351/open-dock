use std::sync::OnceLock;

use windows::{
    core::{Interface, PCWSTR, PWSTR},
    Graphics::{IGeometrySource2D, IGeometrySource2D_Impl},
    Win32::{
        Foundation::E_NOTIMPL,
        Graphics::Direct2D::{
            Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED},
            ID2D1Factory, ID2D1Geometry,
        },
        System::{
            Com::CoTaskMemFree,
            WinRT::Graphics::Direct2D::{IGeometrySource2DInterop, IGeometrySource2DInterop_Impl},
        },
    },
    UI::Composition::{CompositionGeometry, CompositionPath, Compositor},
};
use windows_core::implement;
use windows_numerics::Vector2;

use crate::config::{
    geometry::{
        cluster_radius_base, laerk_radius_multiplier, squircle_multiplier, widget_radius_base,
    },
    schema::{RadiusPreset, ThemeId, WidgetSize},
};

use crate::desktop::plate;

use super::BackdropSpec;

pub(super) fn supports_css_squircle() -> bool {
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

pub(super) fn build_d2d_path_geometry(
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
pub(super) struct PathGeometrySource {
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

pub(super) fn build_clip(
    compositor: &Compositor,
    d2d_factory: &ID2D1Factory,
    spec: &BackdropSpec,
    phys_w: f32,
    phys_h: f32,
) -> windows_core::Result<windows::UI::Composition::CompositionGeometricClip> {
    let (r, use_squircle) = compute_corner_geometry(spec, phys_w, phys_h, supports_css_squircle());

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

pub(super) fn spec_clip_key(
    spec: &BackdropSpec,
    w: i32,
    h: i32,
) -> (i32, i32, u8, u8, u8, bool, i32) {
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
