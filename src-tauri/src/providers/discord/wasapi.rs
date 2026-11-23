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
}

impl CallDetector {
    pub fn tick(&mut self, raw: bool) -> bool {
        if raw {
            self.raw_true = self.raw_true.saturating_add(1);
            self.raw_false = 0;
        } else {
            self.raw_false = self.raw_false.saturating_add(1);
            self.raw_true = 0;
        }
        if !self.on && self.raw_true >= 2 {
            self.on = true;
        }
        if self.on && self.raw_false >= 3 {
            self.on = false;
        }
        self.on
    }

    pub fn reset(&mut self) {
        self.on = false;
        self.raw_true = 0;
        self.raw_false = 0;
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

/// Genbrugelig COM-wrapper omkring `IMMDeviceEnumerator` på worker-tråden så vi undgår
/// `CoCreateInstance` ved hvert poll.
#[cfg(windows)]
#[derive(Default)]
pub struct AudioSessionMonitor {
    enumerator: Option<windows::Win32::Media::Audio::IMMDeviceEnumerator>,
}

#[cfg(windows)]
impl AudioSessionMonitor {
    /// Finder en aktiv Discord capture-session og returnerer dens
    /// session-instance-id — et stabilt id for den konkrete audio-session,
    /// brugt som nøgle til at genkende samme call efter app-genstart.
    /// `None` = ingen aktiv session (ikke i call).
    pub fn discord_mic_session(&mut self, pids: &[u32]) -> Option<String> {
        use windows::core::Interface;
        use windows::Win32::Media::Audio::{
            eCapture, eCommunications, eConsole, AudioSessionStateActive, IAudioSessionControl2,
            IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

        if pids.is_empty() {
            return None;
        }

        if self.enumerator.is_none() {
            match unsafe { CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
                Ok(e) => self.enumerator = Some(e),
                Err(err) => {
                    log::warn!(target: "provider:discord", "MMDeviceEnumerator fejlede: {err}");
                    return None;
                }
            }
        }
        let enumerator = self.enumerator.as_ref()?;

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
                if state != AudioSessionStateActive {
                    continue;
                }
                let id = unsafe { control2.GetSessionInstanceIdentifier() }
                    .ok()
                    .map(|pw| {
                        let s = unsafe { pw.to_string() }.unwrap_or_default();
                        unsafe { CoTaskMemFree(Some(pw.as_ptr().cast())) };
                        s
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("pid:{pid}"));
                return Some(id);
            }
        }
        None
    }
}

#[cfg(not(windows))]
#[derive(Default)]
pub struct AudioSessionMonitor;

#[cfg(not(windows))]
impl AudioSessionMonitor {
    pub fn discord_mic_session(&mut self, _pids: &[u32]) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::CallDetector;

    #[test]
    fn requires_two_positive_samples_before_call() {
        let mut d = CallDetector::default();
        assert!(!d.tick(true));
        assert!(d.tick(true));
    }

    #[test]
    fn single_blip_does_not_trigger_call() {
        let mut d = CallDetector::default();
        assert!(!d.tick(true));
        assert!(!d.tick(false));
        assert!(!d.tick(true));
        assert!(d.tick(true));
    }

    #[test]
    fn requires_three_negative_samples_before_leave() {
        let mut d = CallDetector::default();
        d.tick(true);
        d.tick(true);
        assert!(d.tick(false));
        assert!(d.tick(false));
        assert!(d.tick(true));
        assert!(d.tick(false));
        assert!(d.tick(false));
        assert!(!d.tick(false));
    }
}
