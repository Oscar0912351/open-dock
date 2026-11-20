use std::sync::OnceLock;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::{InvalidateRect, ScreenToClient},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, FindWindowExW,
            FindWindowW, GetWindowLongPtrW, IsWindow, RegisterClassExW, SendMessageTimeoutW,
            SetLayeredWindowAttributes, SetParent, SetWindowLongPtrW, SetWindowPos, CS_DBLCLKS,
            GWL_EXSTYLE, GWL_STYLE, HWND_TOP, LWA_ALPHA, SMTO_NORMAL, SWP_FRAMECHANGED,
            SWP_NOACTIVATE, SWP_SHOWWINDOW, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN,
            WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOPARENTNOTIFY,
            WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
};

use crate::{config::schema::RectI, error::AppError};

const HOST_CLASS_NAME: PCWSTR = w!("OpenDockWallpaperHost");
const WS_EX_NOREDIRECTIONBITMAP_FLAG: u32 = 0x0020_0000;

static CLASS_REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopTopology {
    RaisedDesktop24H2 {
        progman: HWND,
        shell_def_view: HWND,
    },
    ClassicWorkerW {
        worker_w: HWND,
    },
}

pub struct WallpaperHostWindow {
    hwnd: HWND,
    parent: HWND,
    topology: DesktopTopology,
    rect: RectI,
}

impl WallpaperHostWindow {
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn verify_zorder(&self) -> bool {
        if !unsafe { IsWindow(Some(self.hwnd)).as_bool() }
            || !unsafe { IsWindow(Some(self.parent)).as_bool() }
        {
            return false;
        }
        let mut pt = POINT {
            x: self.rect.x,
            y: self.rect.y,
        };
        unsafe {
            let _ = ScreenToClient(self.parent, &mut pt);
        }
        let insert_after = match self.topology {
            DesktopTopology::RaisedDesktop24H2 { shell_def_view, .. } => Some(shell_def_view),
            DesktopTopology::ClassicWorkerW { .. } => Some(HWND_TOP),
        };
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                insert_after,
                pt.x,
                pt.y,
                self.rect.width.max(1),
                self.rect.height.max(1),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        true
    }
}

impl Drop for WallpaperHostWindow {
    fn drop(&mut self) {
        if !self.hwnd.0.is_null() && unsafe { IsWindow(Some(self.hwnd)).as_bool() } {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
        if !self.parent.0.is_null() && unsafe { IsWindow(Some(self.parent)).as_bool() } {
            unsafe {
                let _ = InvalidateRect(Some(self.parent), None, true);
            }
        }
    }
}

pub fn create_and_attach_host_window(monitor_rect: RectI) -> Result<WallpaperHostWindow, AppError> {
    ensure_window_class()?;
    let topology = resolve_desktop_topology()?;

    let (parent, insert_after) = match topology {
        DesktopTopology::RaisedDesktop24H2 {
            progman,
            shell_def_view,
        } => (progman, shell_def_view),
        DesktopTopology::ClassicWorkerW { worker_w } => (worker_w, HWND_TOP),
    };

    let mut pt = POINT {
        x: monitor_rect.x,
        y: monitor_rect.y,
    };
    unsafe {
        let _ = ScreenToClient(parent, &mut pt);
    }

    let hinstance = unsafe { GetModuleHandleW(None)? };
    let ex_style = WS_EX_TOOLWINDOW
        | WS_EX_NOACTIVATE
        | WS_EX_TRANSPARENT
        | WS_EX_LAYERED
        | WS_EX_NOPARENTNOTIFY;
    let popup_style = WS_POPUP | WS_CLIPSIBLINGS | WS_CLIPCHILDREN;
    let child_style = WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN;

    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            HOST_CLASS_NAME,
            w!("OpenDockWallpaper"),
            popup_style,
            monitor_rect.x,
            monitor_rect.y,
            monitor_rect.width.max(1),
            monitor_rect.height.max(1),
            None,
            None,
            Some(hinstance.into()),
            None,
        )?
    };

    unsafe {
        let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, child_style.0 as isize);
        let _ = SetParent(hwnd, Some(parent));
        if let Err(error) = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) {
            log::warn!("SetLayeredWindowAttributes fejlede på baggrundsvært: {error}");
        }
        SetWindowPos(
            hwnd,
            Some(insert_after),
            pt.x,
            pt.y,
            monitor_rect.width.max(1),
            monitor_rect.height.max(1),
            SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
        )?;
    }

    Ok(WallpaperHostWindow {
        hwnd,
        parent,
        topology,
        rect: monitor_rect,
    })
}

pub fn resolve_desktop_topology() -> Result<DesktopTopology, AppError> {
    let progman = unsafe { FindWindowW(w!("Progman"), None)? };
    if progman.0.is_null() {
        return Err(AppError::Other("Kunne ikke finde Windows Progman-vinduet".into()));
    }

    let mut result = 0usize;
    unsafe {
        let _ = SendMessageTimeoutW(
            progman,
            0x052C,
            WPARAM(0x0000_000D),
            LPARAM(0x0000_0001),
            SMTO_NORMAL,
            1000,
            Some(&mut result),
        );
    }

    let progman_ex = unsafe { GetWindowLongPtrW(progman, GWL_EXSTYLE) as u32 };
    let progman_def_view =
        unsafe { FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), None) }.ok();
    let progman_worker =
        unsafe { FindWindowExW(Some(progman), None, w!("WorkerW"), None) }.ok();

    if let Some(shell_def_view) = progman_def_view.filter(|h| !h.0.is_null()) {
        let is_raised =
            (progman_ex & WS_EX_NOREDIRECTIONBITMAP_FLAG) != 0 || progman_worker.is_some();
        if is_raised {
            return Ok(DesktopTopology::RaisedDesktop24H2 {
                progman,
                shell_def_view,
            });
        }
    }

    let mut worker_w = HWND::default();
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let def_view = unsafe { FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None) };
        if let Ok(dv) = def_view {
            if !dv.0.is_null() {
                if let Ok(next_worker) =
                    unsafe { FindWindowExW(None, Some(hwnd), w!("WorkerW"), None) }
                {
                    if !next_worker.0.is_null() {
                        let slot = unsafe { &mut *(lparam.0 as *mut HWND) };
                        *slot = next_worker;
                        return false.into();
                    }
                }
            }
        }
        true.into()
    }

    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut worker_w as *mut HWND as isize));
    }

    if !worker_w.0.is_null() {
        return Ok(DesktopTopology::ClassicWorkerW { worker_w });
    }

    if let Some(shell_def_view) = progman_def_view.filter(|h| !h.0.is_null()) {
        return Ok(DesktopTopology::RaisedDesktop24H2 {
            progman,
            shell_def_view,
        });
    }

    Err(AppError::Other(
        "Kunne ikke finde skrivebordslaget (WorkerW/Progman) til animeret baggrund".into(),
    ))
}

fn ensure_window_class() -> Result<(), AppError> {
    let res = CLASS_REGISTERED.get_or_init(|| unsafe {
        let hinstance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(e) => return Err(format!("GetModuleHandleW fejlede: {e}")),
        };
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(host_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: HOST_CLASS_NAME,
            ..Default::default()
        };
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            let err = windows::core::Error::from_thread();
            return Err(format!("RegisterClassExW fejlede: {err}"));
        }
        Ok(())
    });

    res.as_ref()
        .map(|_| ())
        .map_err(|msg| AppError::Other(msg.clone()))
}

unsafe extern "system" fn host_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
