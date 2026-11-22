use std::{mem::size_of, sync::OnceLock};

use tauri::WebviewWindow;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
        DWM_WINDOW_CORNER_PREFERENCE,
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

static IS_WIN11: OnceLock<bool> = OnceLock::new();

pub fn is_windows_11() -> bool {
    *IS_WIN11.get_or_init(|| {
        let mut info = OsVersionInfoW {
            dw_os_version_info_size: size_of::<OsVersionInfoW>() as u32,
            dw_major_version: 0,
            dw_minor_version: 0,
            dw_build_number: 0,
            dw_platform_id: 0,
            sz_csd_version: [0; 128],
        };
        let status = unsafe { RtlGetVersion(&mut info) };
        status == 0 && info.dw_major_version >= 10 && info.dw_build_number >= 22000
    })
}

pub fn effective_engine(
    preferred: GlassEngine,
    widget_override: Option<GlassEngine>,
    layer: Layer,
    size: WidgetSize,
    windows_11: bool,
) -> GlassEngine {
    let mut chosen = widget_override.unwrap_or(preferred);
    if layer != Layer::Desktop && chosen == GlassEngine::Wallpaper {
        chosen = GlassEngine::Acrylic;
    }
    if chosen == GlassEngine::Acrylic && (!windows_11 || size == WidgetSize::Pill) {
        chosen = GlassEngine::Tint;
    }
    chosen
}

pub fn effective_engine_in_context(
    preferred: GlassEngine,
    widget_override: Option<GlassEngine>,
    layer: Layer,
    size: WidgetSize,
    windows_11: bool,
    game_overlay: bool,
) -> GlassEngine {
    if game_overlay {
        return GlassEngine::Tint;
    }
    effective_engine(preferred, widget_override, layer, size, windows_11)
}

pub fn acrylic_tint_for_theme(theme: ThemeId) -> (u8, u8, u8, u8) {
    match theme {
        ThemeId::Aurora => (18, 18, 22, 70),
        ThemeId::Laerk => (244, 240, 233, 170),
        ThemeId::Grafit => (18, 20, 24, 130),
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
            if let Err(error) = window_vibrancy::apply_acrylic(window, Some(tint)) {
                log::warn!("Kunne ikke aktivere Windows Acrylic: {error}");
            }
            set_corner_preference(hwnd, DWMWCP_ROUND);
        }
        GlassEngine::Wallpaper | GlassEngine::Tint => {
            let _ = window_vibrancy::clear_acrylic(window);
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
    fn effective_engine_follows_plan_02_section_7_4() {
        // Desktop keeps wallpaper
        assert_eq!(
            effective_engine(GlassEngine::Wallpaper, None, Layer::Desktop, WidgetSize::M, true),
            GlassEngine::Wallpaper
        );
        // Top/Normal falls back from wallpaper -> acrylic on Win11
        assert_eq!(
            effective_engine(GlassEngine::Wallpaper, None, Layer::Top, WidgetSize::M, true),
            GlassEngine::Acrylic
        );
        // Top/Normal + Pill falls back wallpaper -> acrylic -> tint
        assert_eq!(
            effective_engine(GlassEngine::Wallpaper, None, Layer::Top, WidgetSize::Pill, true),
            GlassEngine::Tint
        );
        // Win10 falls back acrylic -> tint
        assert_eq!(
            effective_engine(GlassEngine::Wallpaper, None, Layer::Normal, WidgetSize::L, false),
            GlassEngine::Tint
        );
        // Pill cannot use acrylic even on Desktop
        assert_eq!(
            effective_engine(GlassEngine::Acrylic, None, Layer::Desktop, WidgetSize::Pill, true),
            GlassEngine::Tint
        );
        // Widget override wins
        assert_eq!(
            effective_engine(
                GlassEngine::Wallpaper,
                Some(GlassEngine::Tint),
                Layer::Desktop,
                WidgetSize::M,
                true
            ),
            GlassEngine::Tint
        );
    }
}
