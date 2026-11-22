//! Fallback-detektion af Discord-calls via Windows WASAPI capture-sessioner (mikrofon).
//!
//! Vi tjekker udelukkende `eCapture`-enheder (ikke video/musik i render-lyd), så
//! afspillede klip i Discord ikke længere giver falske call-udslag.

const DISCORD_EXES: [&str; 4] = [
    "discord.exe",
    "discordcanary.exe",
    "discordptb.exe",
    "discorddevelopment.exe",
];

/// Ren debounce-tilstand for WASAPI-heuristikken (2 positive ind, 3 negative ud).
#[derive(Default)]
pub struct CallDetector {
    on: bool,
    raw_true: u32,
    raw_false: u32,
    started_at: Option<i64>,
}

impl CallDetector {
    pub fn tick(&mut self, raw: bool, now_ms: i64) -> bool {
        if raw {
            self.raw_true = self.raw_true.saturating_add(1);
            self.raw_false = 0;
        } else {
            self.raw_false = self.raw_false.saturating_add(1);
            self.raw_true = 0;
        }
        if !self.on && self.raw_true >= 2 {
            self.on = true;
            self.started_at = Some(now_ms);
        }
        if self.on && self.raw_false >= 3 {
            self.on = false;
            self.started_at = None;
        }
        self.on
    }

    pub fn started_at(&self) -> Option<i64> {
        self.started_at
    }

    pub fn reset(&mut self) {
        self.on = false;
        self.raw_true = 0;
        self.raw_false = 0;
        self.started_at = None;
    }
}

#[cfg(windows)]
pub fn discord_process_ids() -> Vec<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut pids = Vec::new();
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(handle) => handle,
        Err(err) => {
            log::warn!(target: "provider:discord", "CreateToolhelp32Snapshot fejlede: {err}");
            return pids;
        }
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let wide = String::from_utf16_lossy(&entry.szExeFile);
            let name = wide.split('\0').next().unwrap_or("").to_ascii_lowercase();
            if DISCORD_EXES.contains(&name.as_str()) {
                pids.push(entry.th32ProcessID);
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    pids
}

#[cfg(not(windows))]
pub fn discord_process_ids() -> Vec<u32> {
    Vec::new()
}

/// Sand når Discord har en aktiv mikrofon-session (`eCapture`), hvilket kendetegner et voice call.
#[cfg(windows)]
pub fn discord_mic_active(pids: &[u32]) -> bool {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eConsole, AudioSessionStateActive, IAudioSessionControl2,
        IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    if pids.is_empty() {
        return false;
    }

    let enumerator: IMMDeviceEnumerator =
        match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
            Ok(e) => e,
            Err(err) => {
                log::warn!(target: "provider:discord", "MMDeviceEnumerator fejlede: {err}");
                return false;
            }
        };

    let endpoints = [(eCapture, eCommunications), (eCapture, eConsole)];
    for (flow, role) in endpoints {
        let Ok(device) = (unsafe { enumerator.GetDefaultAudioEndpoint(flow, role) }) else {
            continue;
        };
        let Ok(manager) = (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) })
        else {
            continue;
        };
        let Ok(sessions) = (unsafe { manager.GetSessionEnumerator() }) else {
            continue;
        };
        let Ok(count) = (unsafe { sessions.GetCount() }) else {
            continue;
        };
        for i in 0..count {
            let Ok(control) = (unsafe { sessions.GetSession(i) }) else {
                continue;
            };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            let Ok(pid) = (unsafe { control2.GetProcessId() }) else {
                continue;
            };
            if !pids.contains(&pid) {
                continue;
            }
            let Ok(state) = (unsafe { control2.GetState() }) else {
                continue;
            };
            if state == AudioSessionStateActive {
                return true;
            }
        }
    }
    false
}

#[cfg(not(windows))]
pub fn discord_mic_active(_pids: &[u32]) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::CallDetector;

    #[test]
    fn requires_two_positive_samples_before_call() {
        let mut d = CallDetector::default();
        assert!(!d.tick(true, 1000));
        assert!(d.tick(true, 3000));
        assert_eq!(d.started_at(), Some(3000));
    }

    #[test]
    fn single_blip_does_not_trigger_call() {
        let mut d = CallDetector::default();
        assert!(!d.tick(true, 1000));
        assert!(!d.tick(false, 3000));
        assert!(!d.tick(true, 5000));
        assert!(d.tick(true, 7000));
    }

    #[test]
    fn requires_three_negative_samples_before_leave() {
        let mut d = CallDetector::default();
        d.tick(true, 1000);
        d.tick(true, 3000);
        assert!(d.tick(false, 5000));
        assert!(d.tick(false, 7000));
        assert!(d.tick(true, 9000));
        assert_eq!(d.started_at(), Some(3000));
        assert!(d.tick(false, 11000));
        assert!(d.tick(false, 13000));
        assert!(!d.tick(false, 15000));
        assert_eq!(d.started_at(), None);
    }
}
