use std::{ffi::c_void, mem::size_of, sync::OnceLock};

use tauri::WebviewWindow;
use windows::{
    core::{w, PCSTR},
    Win32::{
        Foundation::HWND,
        Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
            DWMWCP_ROUND, DWM_WINDOW_CORNER_PREFERENCE,
        },
        System::LibraryLoader::{GetModuleHandleW, GetProcAddress},
    },
};

use crate::config::schema::{GlassEngine, Layer, ThemeId, WidgetSize};

#[repr(C)]
struct OsVersionInfoW {
    dw_os_version_info_size: u32,
    dw_major_version: u32,
    dw_minor_version: u32,
    dw_build_number: u32,
    dw_platform_id: u32,
    sz_csd_version: [u16; 128],
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(lp_version_information: *mut OsVersionInfoW) -> i32;
}

static OS_BUILD: OnceLock<u32> = OnceLock::new();

pub fn os_build_number() -> u32 {
    *OS_BUILD.get_or_init(|| {
        let mut info = OsVersionInfoW {
            dw_os_version_info_size: size_of::<OsVersionInfoW>() as u32,
            dw_major_version: 0,
            dw_minor_version: 0,
            dw_build_number: 0,
            dw_platform_id: 0,
            sz_csd_version: [0; 128],
        };
        let status = unsafe { RtlGetVersion(&mut info) };
        if status == 0 && info.dw_major_version >= 10 {
            info.dw_build_number
        } else {
            0
        }
    })
}

pub fn is_windows_11() -> bool {
    os_build_number() >= 22000
}

pub fn is_windows_10_2004_or_newer() -> bool {
    os_build_number() >= 19041
}

pub fn effective_engine(
    preferred: GlassEngine,
    widget_override: Option<GlassEngine>,
    size: WidgetSize,
    windows_11: bool,
) -> GlassEngine {
    let mut chosen = widget_override.unwrap_or(preferred);
    if chosen == GlassEngine::Acrylic && (!windows_11 || size == WidgetSize::Pill) {
        chosen = GlassEngine::Tint;
    }
    chosen
}

pub fn effective_engine_in_context(
    preferred: GlassEngine,
    widget_override: Option<GlassEngine>,
    size: WidgetSize,
    windows_11: bool,
    game_overlay: bool,
) -> GlassEngine {
    if game_overlay {
        return GlassEngine::Tint;
    }
    effective_engine(preferred, widget_override, size, windows_11)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveBackdropMode {
    Off,
    NativeHostBackdrop,
    CaptureD2d,
}

pub fn resolve_live_backdrop_mode(
    engine: GlassEngine,
    _layer: Layer,
    windows_11: bool,
    windows_10_2004: bool,
    transparency_enabled: bool,
) -> LiveBackdropMode {
    if engine != GlassEngine::Wallpaper {
        return LiveBackdropMode::Off;
    }
    if windows_11 && transparency_enabled {
        LiveBackdropMode::NativeHostBackdrop
    } else if windows_10_2004 {
        LiveBackdropMode::CaptureD2d
    } else {
        LiveBackdropMode::Off
    }
}

/// SWCA-acrylic-tint per tema. Lav alpha med vilje: det rigtige indhold under
/// widgetten (tapet eller vinduer) skal dominere ligesom på wallpaper-motoren;
/// tinten samler kun udtrykket en smule.
pub fn acrylic_tint_for_theme(theme: ThemeId) -> (u8, u8, u8, u8) {
    match theme {
        ThemeId::Aurora => (18, 18, 22, 28),
        ThemeId::Laerk => (244, 240, 233, 40),
        ThemeId::Grafit => (18, 20, 24, 36),
    }
}

const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
const WCA_ACCENT_POLICY: u32 = 0x13;

#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttribData {
    attrib: u32,
    pv_data: *mut c_void,
    cb_data: usize,
}

type SetWindowCompositionAttributeFn =
    unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> i32;

/// Sætter ægte SWCA-acrylic med valgfri tint. `window_vibrancy::apply_acrylic`
/// bruger på Windows 11 `DWMSBT_TRANSIENTWINDOW` — et fladt, gråt
/// systemmateriale der hverken respekterer tinten eller viser det underliggende
/// indhold tydeligt. SWCA-acrylic slør det DWM-komponerede indhold under
/// vinduet (tapet + vinduer) med en tint vi selv styrer.
fn set_accent(hwnd: HWND, acrylic: bool, tint: Option<(u8, u8, u8, u8)>) {
    // Undokumenteret API — findes ikke i user32.lib og resolves derfor
    // dynamisk, på samme måde som window_vibrancy gør internt.
    let func = {
        static FUNC: OnceLock<Option<SetWindowCompositionAttributeFn>> = OnceLock::new();
        *FUNC.get_or_init(|| unsafe {
            let module = GetModuleHandleW(w!("user32.dll")).ok()?;
            let proc =
                GetProcAddress(module, PCSTR(b"SetWindowCompositionAttribute\0".as_ptr()))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                SetWindowCompositionAttributeFn,
            >(proc))
        })
    };
    let Some(set) = func else {
        log::warn!("SetWindowCompositionAttribute kunne ikke resolves; acrylic sprunget over");
        return;
    };
    let (r, g, b, a) = tint.unwrap_or_default();
    // Acrylic tolererer ikke alpha=0 (slår effekten helt fra).
    let alpha = if acrylic && a == 0 { 1 } else { a };
    let mut policy = AccentPolicy {
        accent_state: if acrylic {
            ACCENT_ENABLE_ACRYLICBLURBEHIND
        } else {
            ACCENT_DISABLED
        },
        accent_flags: 0,
        gradient_color: (r as u32)
            | ((g as u32) << 8)
            | ((b as u32) << 16)
            | ((alpha as u32) << 24),
        animation_id: 0,
    };
    let mut data = WindowCompositionAttribData {
        attrib: WCA_ACCENT_POLICY,
        pv_data: &mut policy as *mut _ as *mut c_void,
        cb_data: size_of::<AccentPolicy>(),
    };
    unsafe {
        set(hwnd, &mut data);
    }
}

pub fn apply_window_glass(window: &WebviewWindow, engine: GlassEngine, theme: ThemeId) {
    let Ok(raw_hwnd) = window.hwnd() else {
        return;
    };
    let hwnd = HWND(raw_hwnd.0 as _);

    match engine {
        GlassEngine::Acrylic => {
            let tint = acrylic_tint_for_theme(theme);
            set_accent(hwnd, true, Some(tint));
            set_corner_preference(hwnd, DWMWCP_ROUND);
        }
        GlassEngine::Wallpaper | GlassEngine::Tint => {
            // clear_acrylic nulstiller også DWMWA_SYSTEMBACKDROP_TYPE hvis et
            // ældre build satte transient-materialet; set_accent rydder en
            // eventuel SWCA-acrylic.
            let _ = window_vibrancy::clear_acrylic(window);
            set_accent(hwnd, false, None);
            set_corner_preference(hwnd, DWMWCP_DONOTROUND);
        }
    }
}

fn set_corner_preference(hwnd: HWND, pref: DWM_WINDOW_CORNER_PREFERENCE) {
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const DWM_WINDOW_CORNER_PREFERENCE as *const _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    };
    if let Err(error) = result {
        log::debug!("DwmSetWindowAttribute(CORNER_PREFERENCE) ignoreret: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_engine_preserves_wallpaper_and_handles_acrylic_fallbacks() {
        // Wallpaper stays Wallpaper across sizes and Windows versions
        assert_eq!(
            effective_engine(GlassEngine::Wallpaper, None, WidgetSize::M, true),
            GlassEngine::Wallpaper
        );
        assert_eq!(
            effective_engine(GlassEngine::Wallpaper, None, WidgetSize::Pill, true),
            GlassEngine::Wallpaper
        );
        assert_eq!(
            effective_engine(GlassEngine::Wallpaper, None, WidgetSize::L, false),
            GlassEngine::Wallpaper
        );
        // Acrylic stays Acrylic on Win11 non-Pill
        assert_eq!(
            effective_engine(GlassEngine::Acrylic, None, WidgetSize::M, true),
            GlassEngine::Acrylic
        );
        // Pill and Win10 fall back Acrylic -> Tint
        assert_eq!(
            effective_engine(GlassEngine::Acrylic, None, WidgetSize::Pill, true),
            GlassEngine::Tint
        );
        assert_eq!(
            effective_engine(GlassEngine::Acrylic, None, WidgetSize::L, false),
            GlassEngine::Tint
        );
        // Widget override wins
        assert_eq!(
            effective_engine(
                GlassEngine::Wallpaper,
                Some(GlassEngine::Tint),
                WidgetSize::M,
                true
            ),
            GlassEngine::Tint
        );
        // Game overlay forces Tint
        assert_eq!(
            effective_engine_in_context(GlassEngine::Wallpaper, None, WidgetSize::M, true, true),
            GlassEngine::Tint
        );
    }

    #[test]
    fn live_backdrop_mode_selects_native_capture_or_off() {
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Top, true, true, true),
            LiveBackdropMode::NativeHostBackdrop
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Top, true, true, false),
            LiveBackdropMode::CaptureD2d
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Top, false, true, true),
            LiveBackdropMode::CaptureD2d
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Top, false, true, false),
            LiveBackdropMode::CaptureD2d
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Top, false, false, true),
            LiveBackdropMode::Off
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Desktop, true, true, true),
            LiveBackdropMode::NativeHostBackdrop
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Desktop, true, true, false),
            LiveBackdropMode::CaptureD2d
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Desktop, false, true, true),
            LiveBackdropMode::CaptureD2d
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Wallpaper, Layer::Desktop, false, false, true),
            LiveBackdropMode::Off
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Acrylic, Layer::Top, true, true, true),
            LiveBackdropMode::Off
        );
        assert_eq!(
            resolve_live_backdrop_mode(GlassEngine::Tint, Layer::Top, true, true, true),
            LiveBackdropMode::Off
        );
    }
}
