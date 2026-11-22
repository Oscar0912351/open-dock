use std::{
    mem::size_of,
    sync::atomic::{AtomicIsize, AtomicU64, Ordering},
    thread,
    time::Duration,
};

use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS},
            Gdi::{
                GetMonitorInfoW, MonitorFromRect, MonitorFromWindow, MONITORINFO,
                MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
            },
        },
        UI::{
            Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
            WindowsAndMessaging::{
                FindWindowExW, GetClassNameW, GetForegroundWindow, GetTopWindow, GetWindow,
                GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
                IsWindowVisible, IsZoomed, EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE, GWL_STYLE,
                GW_HWNDNEXT, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_CAPTION,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
            },
        },
    },
};

use super::zorder;

static FOREGROUND_GENERATION: AtomicU64 = AtomicU64::new(0);
static HOOK: AtomicIsize = AtomicIsize::new(0);

pub fn install() -> windows::core::Result<()> {
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if hook.0.is_null() {
        return Err(windows::core::Error::from_thread());
    }
    HOOK.store(hook.0 as isize, Ordering::Release);
    Ok(())
}

pub fn shutdown() {
    let raw = HOOK.swap(0, Ordering::AcqRel);
    if raw == 0 {
        return;
    }
    unsafe {
        if let Err(error) = UnhookWinEvent(HWINEVENTHOOK(raw as _)).ok() {
            log::warn!("Kunne ikke frigive Windows' foreground-hook: {error}");
        }
    }
}

pub fn desktop_is_front() -> bool {
    let mut hwnd = match unsafe { GetTopWindow(None) } {
        Ok(hwnd) => hwnd,
        Err(error) => {
            log::warn!("Kunne ikke læse Windows' z-orden: {error}");
            return false;
        }
    };

    while !hwnd.0.is_null() {
        if is_desktop_host(hwnd) {
            return true;
        }
        if is_blocking_app_window(hwnd) {
            return false;
        }
        hwnd = match unsafe { GetWindow(hwnd, GW_HWNDNEXT) } {
            Ok(next) => next,
            Err(_) => break,
        };
    }
    false
}

pub fn foreground_is_desktop() -> bool {
    is_desktop_host(unsafe { GetForegroundWindow() })
}

pub fn foreground_is_shell_or_own() -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return true;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id == std::process::id() {
        return true;
    }
    is_shell_or_system_window(hwnd)
}

pub fn foreground_is_borderless_fullscreen() -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() || is_desktop_host(hwnd) || is_shell_or_system_window(hwnd) {
        return false;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id == std::process::id() {
        return false;
    }
    if unsafe { IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() } {
        return false;
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
        return false;
    }
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 };
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
        return false;
    }
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 };
    if unsafe { IsZoomed(hwnd).as_bool() } && (style & WS_CAPTION.0) == WS_CAPTION.0 {
        return false;
    }
    let mut rect = RECT::default();
    let got_rect = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    }
    .is_ok()
        || unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok();
    if !got_rect {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info).as_bool() } {
        return false;
    }
    let mon = info.rcMonitor;
    let covers_monitor = rect.left <= mon.left + 1
        && rect.top <= mon.top + 1
        && rect.right >= mon.right - 1
        && rect.bottom >= mon.bottom - 1;
    if !covers_monitor {
        return false;
    }
    let work = info.rcWork;
    if work != mon && rect == work {
        return false;
    }
    true
}

fn is_shell_or_system_window(hwnd: HWND) -> bool {
    matches!(
        window_class(hwnd).as_str(),
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "TopLevelWindowForOverflowXamlIsland"
            | "Windows.UI.Core.CoreWindow"
            | "XamlExplorerHostIslandWindow"
            | "Xaml_WindowedPopupClass"
            | "MultitaskingViewFrame"
            | "ForegroundStaging"
            | "ApplicationManager_DesktopShellWindow"
            | "TaskListThumbnailWnd"
            | "Shell_InputSwitchTopLevelWindow"
            | "#32768"
    )
}

pub(crate) fn is_desktop_host(hwnd: HWND) -> bool {
    let class = window_class(hwnd);
    if class != "Progman" && class != "WorkerW" {
        return false;
    }
    unsafe { FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None).is_ok() }
}

fn is_blocking_app_window(hwnd: HWND) -> bool {
    if !unsafe { IsWindowVisible(hwnd).as_bool() } || unsafe { IsIconic(hwnd).as_bool() } {
        return false;
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
        return false;
    }
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 };
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 || ex_style & WS_EX_TOPMOST.0 != 0 {
        return false;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id == std::process::id() {
        return false;
    }
    if matches!(
        window_class(hwnd).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    ) {
        return false;
    }
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err()
        || rect.right <= rect.left
        || rect.bottom <= rect.top
    {
        return false;
    }
    !unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }
        .0
        .is_null()
}

fn window_class(hwnd: HWND) -> String {
    let mut buffer = [0_u16; 256];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) } as usize;
    String::from_utf16_lossy(&buffer[..length])
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object_id: i32,
    _child_id: i32,
    _thread_id: u32,
    _time: u32,
) {
    if !is_desktop_host(hwnd) {
        FOREGROUND_GENERATION.fetch_add(1, Ordering::AcqRel);
        zorder::set_show_desktop(false);
        super::fullscreen::trigger_check();
        if zorder::game_overlay() {
            let _ = thread::Builder::new()
                .name("opendock-overlay-z".into())
                .spawn(|| {
                    thread::sleep(Duration::from_millis(60));
                    if zorder::game_overlay() {
                        zorder::reassert_game_overlay();
                    }
                });
        }
        return;
    }

    super::fullscreen::trigger_check();

    let generation = FOREGROUND_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    match thread::Builder::new()
        .name("opendock-win-d".into())
        .spawn(move || {
            thread::sleep(Duration::from_millis(150));
            if generation == FOREGROUND_GENERATION.load(Ordering::Acquire) && desktop_is_front() {
                zorder::set_show_desktop(true);
                log::debug!("Vis skrivebord registreret; desktop-widgets er midlertidigt løftet");
            }
        })
    {
        Ok(_) => {}
        Err(error) => log::error!("Kunne ikke starte Vis skrivebord-detektion: {error}"),
    }
}
