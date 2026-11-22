pub mod drag;
pub mod fullscreen;
pub mod glass;
pub mod monitors;
pub mod plate;
pub mod showdesktop;
pub mod snap;
pub mod styles;
pub mod wallpaper;
pub mod zorder;

use std::sync::{
    atomic::{AtomicIsize, AtomicU32, Ordering},
    OnceLock,
};

use tauri::AppHandle;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, IsWindow, PostMessageW,
            RegisterClassExW, RegisterWindowMessageW, SPI_SETDESKWALLPAPER, SPI_SETWORKAREA,
            WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE, WNDCLASSEXW,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
        },
    },
};

use crate::error::AppError;

const WATCHER_CLASS_NAME: PCWSTR = w!("OpenDockDesktopWatcher");
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static WATCHER_HWND: AtomicIsize = AtomicIsize::new(0);
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static UI_THREAD_ID: OnceLock<std::thread::ThreadId> = OnceLock::new();

pub fn run_on_ui_thread_sync<R, F>(app: &AppHandle, f: F) -> Result<R, AppError>
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    if UI_THREAD_ID.get().copied() == Some(std::thread::current().id()) {
        return Ok(f());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel::<R>(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })?;
    rx.recv()
        .map_err(|_| AppError::Other("UI-tråd svarede ikke ved vinduesopsætning".into()))
}

pub fn install(app: &AppHandle) -> Result<(), AppError> {
    let _ = UI_THREAD_ID.set(std::thread::current().id());
    if APP_HANDLE.set(app.clone()).is_err() {
        log::warn!("Desktop-overvågning blev installeret mere end én gang");
    }
    unsafe {
        let message = RegisterWindowMessageW(w!("TaskbarCreated"));
        if message == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        TASKBAR_CREATED.store(message, Ordering::Release);

        let hinstance = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(watcher_proc),
            hInstance: hinstance.into(),
            lpszClassName: WATCHER_CLASS_NAME,
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            WATCHER_CLASS_NAME,
            w!("OpenDockDesktopWatcher"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;
        WATCHER_HWND.store(hwnd.0 as isize, Ordering::Release);
        showdesktop::install()?;
    }
    wallpaper::install(app.clone());
    monitors::start_poll(app.clone());
    fullscreen::start(app.clone());
    Ok(())
}

pub fn shutdown() {
    crate::providers::cpu_sensor::shutdown();
    crate::wallpaper::shutdown();
    wallpaper::shutdown();
    showdesktop::shutdown();
    let raw = WATCHER_HWND.swap(0, Ordering::AcqRel);
    if raw != 0 {
        let hwnd = HWND(raw as _);
        unsafe {
            if IsWindow(Some(hwnd)).as_bool() {
                if UI_THREAD_ID.get().copied() == Some(std::thread::current().id()) {
                    let _ = DestroyWindow(hwnd);
                } else {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
    }
}

pub(crate) fn app_handle() -> Option<&'static AppHandle> {
    APP_HANDLE.get()
}

unsafe extern "system" fn watcher_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Acquire) {
        log::info!("Explorer genstartet; genanvender widgets på skrivebordet");
        zorder::reapply_all();
        wallpaper::trigger_refresh_now();
        if let Some(app) = app_handle() {
            crate::wallpaper::notify_explorer_restarted(app);
        }
    } else if msg == WM_SETTINGCHANGE {
        let is_wallpaper = wp.0 == SPI_SETDESKWALLPAPER.0 as usize
            || (wp.0 == 0
                && lp.0 != 0
                && PCWSTR(lp.0 as *const u16)
                    .to_string()
                    .map(|s| s.eq_ignore_ascii_case("Wallpaper"))
                    .unwrap_or(false));
        if is_wallpaper {
            log::debug!("Tapetændring modtaget; opdaterer tapetplader");
            wallpaper::trigger_refresh_debounced();
            if let Some(app) = app_handle() {
                crate::wallpaper::notify_wallpaper_changed(app);
            }
        } else if wp.0 == SPI_SETWORKAREA.0 as usize {
            log::debug!("Arbejdsområde ændret; opdaterer skærme og placering");
            wallpaper::trigger_refresh_debounced();
        }
    } else if msg == WM_DISPLAYCHANGE || msg == WM_DPICHANGED {
        log::debug!("Skærm- eller DPI-ændring modtaget; opdaterer plader og placering");
        wallpaper::trigger_refresh_now();
        if let Some(app) = app_handle() {
            crate::wallpaper::notify_monitors_changed(app);
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
