//! Discord-aktivitet via lokal lyd-detektion (WASAPI).
//!
//! Ingen Discord-API/login: vi observerer om Discord-processen findes, og om den
//! har aktive lyd-sessioner (mikrofon-capture eller render med signal). Et voice-call
//! holder mikrofon-sessionen åben (også ved mute), så dette er en ærlig heuristik
//! for "i call" — ikke en officiel status. UI skal mærke det som lydbaseret.
//! Debounce: 2 positive samples (4 s) før "i call", 3 negative (6 s) før fra.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use ts_rs::TS;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct DiscordSnapshot {
    /// Discord-processen kører.
    pub running: bool,
    /// Heuristik: aktiv mikrofon/call-lyd fra Discord (se moduldokumentation).
    pub in_call: bool,
    /// Epoch ms for hvornår "i call" sidst slog til (null når ikke i call).
    pub call_started_at: Option<i64>,
    pub updated_at: i64,
}

/// Ren debounce-tilstand for call-heuristikken (unit-testet uden COM).
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
}

pub struct DiscordProvider {
    running: AtomicBool,
    worker: Mutex<Option<Worker>>,
    latest: Arc<Mutex<Option<DiscordSnapshot>>>,
}

struct Worker {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Default for DiscordProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordProvider {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            worker: Mutex::new(None),
            latest: Arc::new(Mutex::new(None)),
        }
    }

    pub fn snapshot(&self) -> Option<DiscordSnapshot> {
        self.latest.lock().clone()
    }

    pub fn start(&self, app: &AppHandle) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        log::info!(target: "provider:discord", "Starter discord-provider (WASAPI-lyddetektion)");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let latest = Arc::clone(&self.latest);
        let app_handle = app.clone();
        let join = std::thread::Builder::new()
            .name("od-discord".into())
            .spawn(move || run_discord_thread(app_handle, stop_thread, latest))
            .ok();
        if join.is_none() {
            log::warn!(target: "provider:discord", "Kunne ikke starte od-discord-tråd");
            self.running.store(false, Ordering::Release);
            return;
        }
        *self.worker.lock() = Some(Worker { stop, join });
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::AcqRel) {
            return;
        }
        log::info!(target: "provider:discord", "Stopper discord-provider");
        if let Some(mut worker) = self.worker.lock().take() {
            worker.stop.store(true, Ordering::Release);
            if let Some(join) = worker.join.take() {
                let _ = join.join();
            }
        }
    }
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn run_discord_thread(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<Option<DiscordSnapshot>>>,
) {
    let com_ok = init_com();
    let mut detector = CallDetector::default();
    let mut last_key: Option<(bool, bool, Option<i64>)> = None;

    while !stop.load(Ordering::Acquire) {
        let pids = discord_process_ids();
        let running = !pids.is_empty();
        let raw_audio = running && discord_audio_active(&pids);
        let now = unix_now_ms();
        let in_call = detector.tick(raw_audio, now);
        let snapshot = DiscordSnapshot {
            running,
            in_call,
            call_started_at: if in_call { detector.started_at() } else { None },
            updated_at: now,
        };
        *latest.lock() = Some(snapshot.clone());
        let key = (snapshot.running, snapshot.in_call, snapshot.call_started_at);
        if last_key != Some(key) {
            last_key = Some(key);
            if let Err(err) = app.emit("provider://discord", &snapshot) {
                log::warn!(target: "provider:discord", "Kunne ikke emitte provider://discord: {err}");
            }
        }
        // Sov i små trin så stop reagerer hurtigt.
        let mut slept = Duration::ZERO;
        while slept < POLL_INTERVAL && !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(100));
            slept += Duration::from_millis(100);
        }
    }
    if com_ok {
        uninit_com();
    }
}

#[cfg(windows)]
fn init_com() -> bool {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok()
}

#[cfg(windows)]
fn uninit_com() {
    use windows::Win32::System::Com::CoUninitialize;
    unsafe { CoUninitialize() };
}

#[cfg(not(windows))]
fn init_com() -> bool {
    false
}

#[cfg(not(windows))]
fn uninit_com() {}

const DISCORD_EXES: [&str; 4] = [
    "discord.exe",
    "discordcanary.exe",
    "discordptb.exe",
    "discorddevelopment.exe",
];

#[cfg(windows)]
fn discord_process_ids() -> Vec<u32> {
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
fn discord_process_ids() -> Vec<u32> {
    Vec::new()
}

/// Sand hvis Discord har en aktiv capture-session (mikrofon åben = i call)
/// eller en render-session med målbart signal (call-lyd, ikke bare stilhed).
#[cfg(windows)]
fn discord_audio_active(pids: &[u32]) -> bool {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eConsole, eRender, AudioSessionStateActive,
        IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    let enumerator: IMMDeviceEnumerator =
        match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
            Ok(e) => e,
            Err(err) => {
                log::warn!(target: "provider:discord", "MMDeviceEnumerator fejlede: {err}");
                return false;
            }
        };

    // Discord bruger typisk communications-enhederne; console med som fallback.
    let endpoints = [
        (eCapture, eCommunications),
        (eRender, eCommunications),
        (eRender, eConsole),
    ];
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
            if flow == eCapture {
                // Aktiv mikrofon-session fra Discord = call i gang.
                return true;
            }
            if let Ok(meter) = control.cast::<IAudioMeterInformation>() {
                if let Ok(peak) = unsafe { meter.GetPeakValue() } {
                    if peak > 0.0005 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(not(windows))]
fn discord_audio_active(_pids: &[u32]) -> bool {
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
        assert!(d.tick(false, 5000)); // grace: stadig i call
        assert!(d.tick(false, 7000)); // grace: stadig i call
        assert!(d.tick(true, 9000)); // tilbage før grace udløb
        assert_eq!(d.started_at(), Some(3000));
        assert!(d.tick(false, 11000));
        assert!(d.tick(false, 13000));
        assert!(!d.tick(false, 15000)); // tredje negative → forladt
        assert_eq!(d.started_at(), None);
    }
}
