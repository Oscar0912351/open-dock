#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevatedRunError {
    Cancelled,
    Failed(String),
}

#[cfg(target_os = "windows")]
pub fn process_is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let process = GetCurrentProcess();
        let mut token = HANDLE::default();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut return_length = 0u32;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        );
        let _ = CloseHandle(token);
        result.is_ok() && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(target_os = "windows"))]
pub fn process_is_elevated() -> bool {
    false
}

#[cfg(any(target_os = "windows", test))]
pub fn quote_windows_args<S: AsRef<str>>(args: &[S]) -> String {
    let mut parts = Vec::with_capacity(args.len());
    for raw in args {
        let arg = raw.as_ref();
        if !arg.is_empty() && !arg.chars().any(|c| c.is_whitespace() || c == '"') {
            parts.push(arg.to_string());
            continue;
        }
        let mut out = String::with_capacity(arg.len() + 2);
        out.push('"');
        let mut backslashes = 0usize;
        for ch in arg.chars() {
            if ch == '\\' {
                backslashes += 1;
            } else if ch == '"' {
                for _ in 0..(backslashes * 2 + 1) {
                    out.push('\\');
                }
                out.push('"');
                backslashes = 0;
            } else {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push(ch);
            }
        }
        for _ in 0..(backslashes * 2) {
            out.push('\\');
        }
        out.push('"');
        parts.push(out);
    }
    parts.join(" ")
}

/// Launches the current executable once with UAC (`runas`) hidden in the background,
/// waits for completion (up to `timeout_ms`), and returns its exit code.
#[cfg(target_os = "windows")]
pub fn run_elevated_self_and_wait(
    args: &[String],
    timeout_ms: u32,
) -> Result<u32, ElevatedRunError> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let exe = std::env::current_exe()
        .map_err(|e| ElevatedRunError::Failed(format!("Kunne ikke finde programstien: {e}")))?;
    let args_line = quote_windows_args(args);

    let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
    let file: Vec<u16> = exe
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let parameters: Vec<u16> = args_line.encode_utf16().chain(std::iter::once(0)).collect();

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    if let Err(err) = unsafe { ShellExecuteExW(&mut info) } {
        if err.code() == ERROR_CANCELLED.to_hresult() {
            return Err(ElevatedRunError::Cancelled);
        }
        return Err(ElevatedRunError::Failed(format!(
            "Kunne ikke bede om administratoradgang: {err}"
        )));
    }

    if info.hProcess.is_invalid() {
        return Err(ElevatedRunError::Failed(
            "Kunne ikke følge administrator-processen".to_string(),
        ));
    }

    let wait = unsafe { WaitForSingleObject(info.hProcess, timeout_ms) };
    let mut exit_code = 1u32;
    let got_code = if wait == WAIT_OBJECT_0 {
        unsafe { GetExitCodeProcess(info.hProcess, &mut exit_code) }.is_ok()
    } else {
        false
    };
    let _ = unsafe { CloseHandle(info.hProcess) };

    if wait != WAIT_OBJECT_0 {
        return Err(ElevatedRunError::Failed(
            "Handling med administratoradgang tog for lang tid".to_string(),
        ));
    }
    if !got_code {
        return Err(ElevatedRunError::Failed(
            "Kunne ikke aflæse resultat fra administrator-processen".to_string(),
        ));
    }
    Ok(exit_code)
}

#[cfg(not(target_os = "windows"))]
pub fn run_elevated_self_and_wait(
    _args: &[String],
    _timeout_ms: u32,
) -> Result<u32, ElevatedRunError> {
    Err(ElevatedRunError::Failed(
        "Kun understøttet på Windows".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::quote_windows_args;

    #[test]
    fn quotes_windows_args_safely() {
        assert_eq!(
            quote_windows_args(&[
                "--install-cpu-sensor-service",
                "--source-dir",
                "C:\\Program Files\\Open Dock\\cpu-sensor\\",
                "has\"quote",
            ]),
            "--install-cpu-sensor-service --source-dir \"C:\\Program Files\\Open Dock\\cpu-sensor\\\\\" \"has\\\"quote\""
        );
    }
}
