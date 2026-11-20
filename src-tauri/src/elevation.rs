#[cfg(any(target_os = "windows", test))]
pub const ELEVATED_HANDOFF_FLAG: &str = "--open-dock-elevated-handoff";

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
fn build_elevated_args<I: IntoIterator<Item = String>>(args: I) -> String {
    let mut parts = vec![ELEVATED_HANDOFF_FLAG.to_string()];
    for arg in args {
        if arg == ELEVATED_HANDOFF_FLAG || arg.is_empty() {
            continue;
        }
        if arg.chars().any(|c| c.is_whitespace() || c == '"') {
            parts.push(format!("\"{}\"", arg.replace('"', "\\\"")));
        } else {
            parts.push(arg);
        }
    }
    parts.join(" ")
}

#[cfg(target_os = "windows")]
fn relaunch_windows() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{ShellExecuteW, SE_ERR_ACCESSDENIED};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;

    let exe = std::env::current_exe().map_err(|e| format!("Kunne ikke finde exe-sti: {e}"))?;
    let args_line = build_elevated_args(std::env::args().skip(1));

    let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
    let file: Vec<u16> = exe
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let parameters: Vec<u16> = args_line.encode_utf16().chain(std::iter::once(0)).collect();
    let directory: [u16; 1] = [0];

    let code = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR(directory.as_ptr()),
            SW_SHOW,
        )
    }
    .0 as usize;

    if code <= 32 {
        return Err(if code as u32 == SE_ERR_ACCESSDENIED {
            "Administratoradgang blev afvist af bruger eller UAC".to_string()
        } else {
            format!("Kunne ikke starte som administrator (ShellExecuteW kode {code})")
        });
    }
    Ok(())
}

/// Prompts for UAC elevation once on process startup before initializing Tauri or
/// single-instance locks. If declined, Open Dock continues unelevated (CPU temp unavailable).
pub fn try_auto_elevate() {
    #[cfg(target_os = "windows")]
    {
        if process_is_elevated() {
            return;
        }
        if std::env::args().any(|a| a == ELEVATED_HANDOFF_FLAG) {
            return;
        }

        match relaunch_windows() {
            Ok(()) => std::process::exit(0),
            Err(err) => {
                eprintln!("[elevation] fortsætter uden administratorrettigheder: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_elevated_args, ELEVATED_HANDOFF_FLAG};

    #[test]
    fn builds_elevated_args_with_handoff_flag_and_quoting() {
        assert_eq!(
            build_elevated_args(Vec::<String>::new()),
            ELEVATED_HANDOFF_FLAG
        );
        assert_eq!(
            build_elevated_args([
                ELEVATED_HANDOFF_FLAG.to_string(),
                "--autostart".to_string(),
                "C:\\Program Files\\Open Dock\\open-dock.exe".to_string(),
                "has\"quote".to_string(),
            ]),
            "--open-dock-elevated-handoff --autostart \"C:\\Program Files\\Open Dock\\open-dock.exe\" \"has\\\"quote\""
        );
    }
}
