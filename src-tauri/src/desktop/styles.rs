use windows::Win32::{
    Foundation::{GetLastError, HWND, SetLastError, WIN32_ERROR},
    UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    },
};

/// # Safety
///
/// `hwnd` skal være et gyldigt top-level Win32-vindueshandle ejet af den aktuelle proces.
pub unsafe fn apply_widget_styles(hwnd: HWND) -> windows::core::Result<()> {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    let ex = (ex | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) & !WS_EX_APPWINDOW.0;
    set_ex_style(hwnd, ex)?;
    SetWindowPos(
        hwnd,
        None,
        0,
        0,
        0,
        0,
        SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    )?;
    Ok(())
}

/// # Safety
///
/// `hwnd` skal være et gyldigt top-level Win32-vindueshandle ejet af den aktuelle proces.
pub unsafe fn set_activatable(hwnd: HWND, yes: bool) -> windows::core::Result<()> {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    let ex = if yes {
        ex & !WS_EX_NOACTIVATE.0
    } else {
        ex | WS_EX_NOACTIVATE.0
    };
    set_ex_style(hwnd, ex)
}

unsafe fn set_ex_style(hwnd: HWND, style: u32) -> windows::core::Result<()> {
    SetLastError(WIN32_ERROR(0));
    let previous = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
    if previous == 0 && GetLastError().0 != 0 {
        return Err(windows::core::Error::from_thread());
    }
    Ok(())
}
