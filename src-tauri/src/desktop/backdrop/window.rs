use std::{mem::size_of, sync::OnceLock};

use windows::{
    core::{w, BOOL, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
                DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_HOSTBACKDROPBRUSH,
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
            },
            Gdi::HBRUSH,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindow, GetWindowLongPtrW,
            GetWindowRect, IsWindowVisible, RegisterClassExW, SetWindowDisplayAffinity,
            SetWindowPos, GWL_EXSTYLE, GW_HWNDNEXT, HWND_NOTOPMOST, HWND_TOPMOST, SWP_HIDEWINDOW,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
            WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WNDCLASSEXW, WS_EX_NOACTIVATE,
            WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
            WS_POPUP,
        },
    },
};

const BACKDROP_CLASS_NAME: PCWSTR = w!("OpenDockGlassBackdrop");

static CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

unsafe extern "system" fn backdrop_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

pub(super) fn ensure_class_registered() -> windows_core::Result<()> {
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

pub(super) fn set_exclude_from_capture(hwnd: HWND, exclude: bool) -> windows_core::Result<()> {
    let affinity = if exclude {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    unsafe { SetWindowDisplayAffinity(hwnd, affinity) }
}

pub(super) fn create_backdrop_hwnd(
    rect: RECT,
    topmost: bool,
    use_host_backdrop: bool,
) -> windows_core::Result<HWND> {
    let hinstance = unsafe { GetModuleHandleW(None)? };
    let w = (rect.right - rect.left).max(1);
    let h = (rect.bottom - rect.top).max(1);
    let mut ex =
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
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

pub(super) fn widget_is_topmost(widget_hwnd: HWND) -> bool {
    let ex = unsafe { GetWindowLongPtrW(widget_hwnd, GWL_EXSTYLE) as u32 };
    (ex & WS_EX_TOPMOST.0) != 0
}

pub(super) fn backdrop_is_topmost(backdrop_hwnd: HWND) -> bool {
    let ex = unsafe { GetWindowLongPtrW(backdrop_hwnd, GWL_EXSTYLE) as u32 };
    (ex & WS_EX_TOPMOST.0) != 0
}

pub(super) fn align_backdrop_to_widget(widget_hwnd: HWND, backdrop_hwnd: HWND, hidden_idle: bool) {
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
