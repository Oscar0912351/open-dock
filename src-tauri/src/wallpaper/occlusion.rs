use std::mem::size_of;

use windows::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS},
    System::StationsAndDesktops::{
        CloseDesktop, OpenInputDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS,
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetLayeredWindowAttributes, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, GWL_EXSTYLE, LWA_ALPHA,
        WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    },
};

use crate::config::schema::RectI;

const DESKTOP_READOBJECTS_FLAG: u32 = 0x0001;

pub fn is_workstation_locked() -> bool {
    match unsafe {
        OpenInputDesktop(
            DESKTOP_CONTROL_FLAGS(0),
            false,
            DESKTOP_ACCESS_FLAGS(DESKTOP_READOBJECTS_FLAG),
        )
    } {
        Ok(hdesk) => {
            unsafe {
                let _ = CloseDesktop(hdesk);
            }
            false
        }
        Err(_) => true,
    }
}

pub fn rect_coverage_ratio(target: RectI, win: RectI) -> f64 {
    let target_w = target.width.max(0) as i64;
    let target_h = target.height.max(0) as i64;
    let target_area = target_w * target_h;
    if target_area <= 0 || win.width <= 0 || win.height <= 0 {
        return 0.0;
    }

    let ix0 = target.x.max(win.x);
    let iy0 = target.y.max(win.y);
    let ix1 = (target.x + target.width).min(win.x + win.width);
    let iy1 = (target.y + target.height).min(win.y + win.height);

    let iw = (ix1 - ix0).max(0) as i64;
    let ih = (iy1 - iy0).max(0) as i64;
    ((iw * ih) as f64) / (target_area as f64)
}

struct OcclusionQuery {
    monitor_rect: RectI,
    work_area: RectI,
    own_pid: u32,
    covered: bool,
}

pub fn is_monitor_covered(monitor_rect: RectI, work_area: RectI) -> bool {
    let mut query = OcclusionQuery {
        monitor_rect,
        work_area,
        own_pid: std::process::id(),
        covered: false,
    };

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let query = unsafe { &mut *(lparam.0 as *mut OcclusionQuery) };
        if !unsafe { IsWindowVisible(hwnd).as_bool() } || unsafe { IsIconic(hwnd).as_bool() } {
            return true.into();
        }

        let mut pid = 0_u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid == query.own_pid {
            return true.into();
        }

        let mut cloaked = 0_u32;
        if unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut _,
                size_of::<u32>() as u32,
            )
        }
        .is_ok()
            && cloaked != 0
        {
            return true.into();
        }

        let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 };
        if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
            return true.into();
        }

        if (ex_style & WS_EX_LAYERED.0) != 0 {
            let mut alpha = 255_u8;
            let mut flags = windows::Win32::UI::WindowsAndMessaging::LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
            if unsafe { GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)) }
                .is_ok()
                && (flags.0 & LWA_ALPHA.0) != 0
                && alpha < 250
            {
                return true.into();
            }
        }

        let cls = window_class(hwnd);
        if matches!(
            cls.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return true.into();
        }

        let Some(win_rect) = extended_window_rect(hwnd) else {
            return true.into();
        };

        let work_cov = rect_coverage_ratio(query.work_area, win_rect);
        let mon_cov = rect_coverage_ratio(query.monitor_rect, win_rect);
        let zoomed = unsafe { IsZoomed(hwnd).as_bool() };

        if (zoomed && work_cov >= 0.88) || work_cov >= 0.94 || mon_cov >= 0.94 {
            query.covered = true;
            return false.into();
        }

        true.into()
    }

    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut query as *mut OcclusionQuery as isize));
    }
    query.covered
}

fn extended_window_rect(hwnd: HWND) -> Option<RectI> {
    let mut rect = RECT::default();
    let dwm_ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    }
    .is_ok();
    if !dwm_ok && unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return None;
    }
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 {
        return None;
    }
    Some(RectI {
        x: rect.left,
        y: rect.top,
        width: w,
        height: h,
    })
}

fn window_class(hwnd: HWND) -> String {
    let mut buf = [0_u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_coverage_ratio_computes_exact_overlaps() {
        let monitor = RectI {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let work = RectI {
            x: 0,
            y: 0,
            width: 1920,
            height: 1032,
        };

        // Fullscreen cover
        assert!((rect_coverage_ratio(monitor, monitor) - 1.0).abs() < 1e-6);

        // Maximized window matches work area
        assert!((rect_coverage_ratio(work, work) - 1.0).abs() < 1e-6);
        assert!(rect_coverage_ratio(monitor, work) > 0.95);

        // Half-screen snapped window -> 0.5
        let half = RectI {
            x: 0,
            y: 0,
            width: 960,
            height: 1032,
        };
        assert!((rect_coverage_ratio(work, half) - 0.5).abs() < 1e-6);

        // Window on second monitor -> 0.0
        let other = RectI {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert!((rect_coverage_ratio(monitor, other) - 0.0).abs() < 1e-6);
    }
}
