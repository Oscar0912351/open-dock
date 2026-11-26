use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, OnceLock},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;

use crate::util::unix_now_ms;

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();
static STATE: LazyLock<Mutex<SensorState>> = LazyLock::new(|| Mutex::new(SensorState::default()));

const SOURCE: &str = "lhm-direct-package";
pub const SERVICE_NAME: &str = "OpenDockCpuSensor";
const SERVICE_DISPLAY_NAME: &str = "Open Dock CPU-sensor";
const SERVICE_DESCRIPTION: &str =
    "Læser lokal CPU-temperatur til Open Dock uden administratorrettigheder i hovedappen.";
#[cfg(all(windows, target_arch = "x86_64"))]
const PIPE_PATH: &str = r"\\.\pipe\OpenDock.CpuSensor";

const INSTALL_FLAG: &str = "--install-cpu-sensor-service";
const UNINSTALL_FLAG: &str = "--uninstall-cpu-sensor-service";
const SOURCE_DIR_FLAG: &str = "--source-dir";

const EXIT_OK: i32 = 0;
const EXIT_NOT_ELEVATED: i32 = 10;
const EXIT_HELPER_MISSING: i32 = 11;
const EXIT_INTEGRITY_FAILED: i32 = 12;
const EXIT_STOP_FAILED: i32 = 13;
const EXIT_COPY_FAILED: i32 = 14;
const EXIT_REGISTER_FAILED: i32 = 15;
const EXIT_START_FAILED: i32 = 16;
const EXIT_REMOVE_FAILED: i32 = 17;
const EXIT_DRIVER_FAILED: i32 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum CpuSensorServiceState {
    Running,
    Stopped,
    NotInstalled,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct CpuSensorStatus {
    pub state: CpuSensorServiceState,
    pub driver_installed: bool,
    pub outdated: bool,
    pub elevated_fallback: bool,
    pub active: bool,
}

pub fn configure(resource_dir: PathBuf) {
    let _ = RESOURCE_DIR.set(resource_dir);
}

#[derive(Default)]
struct SensorState {
    #[cfg(all(windows, target_arch = "x86_64"))]
    pipe: Option<pipe_client::PipeClient>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    worker: Option<worker::Worker>,
    retry_at: Option<Instant>,
    last_status: Option<String>,
}


fn unavailable(status: &str) -> Value {
    json!({ "status": status, "celsius": null })
}

fn has_helper_files(dir: &Path) -> bool {
    dir.join("OpenDock.CpuSensors.exe").is_file() && dir.join("LibreHardwareMonitorLib.dll").is_file()
}

pub fn helper_directory(resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(root) = resource_dir {
        candidates.push(root.join("cpu-sensor"));
        candidates.push(root.join("resources").join("cpu-sensor"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("cpu-sensor"));
            candidates.push(parent.join("resources").join("cpu-sensor"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("cpu-sensor"));

    candidates
        .into_iter()
        .find(|dir| has_helper_files(dir))
        .ok_or_else(|| "CPU-sensor hjælpefiler ikke fundet".to_string())
}

pub fn parse_reply(raw: &[u8], expected_sequence: u64, now: i64) -> Result<Value, String> {
    if raw.len() > 4096 {
        return Err("oversized helper reply".into());
    }
    let reply: Value = serde_json::from_slice(raw).map_err(|_| "invalid helper JSON")?;
    if reply["schema"] != 1
        || reply["sequence"].as_u64() != Some(expected_sequence)
        || reply["source"].as_str() != Some(SOURCE)
    {
        return Err("invalid helper protocol".into());
    }
    let at = reply["at"].as_i64().ok_or("missing helper timestamp")?;
    if at < now - 8000 || at > now + 5000 {
        return Err("stale helper reply".into());
    }
    let status = reply["status"].as_str().ok_or("missing helper status")?;
    if status == "ready" {
        let value = reply["celsius"].as_f64().ok_or("missing CPU temperature")?;
        let sensor = reply["sensor"].as_str().unwrap_or("");
        let id = reply["sensorId"].as_str().unwrap_or("");
        let cpu = reply["hardware"].as_str().unwrap_or("");
        if !value.is_finite()
            || value <= 0.0
            || value >= 125.0
            || !["CPU Package", "Core (Tctl/Tdie)", "Core (Tdie)"].contains(&sensor)
            || !(id.starts_with("/amdcpu/") || id.starts_with("/intelcpu/"))
            || !id.contains("/temperature/")
            || id.len() > 200
            || cpu.is_empty()
            || cpu.len() > 200
        {
            return Err("invalid CPU package sensor".into());
        }
    } else if ![
        "driver-required",
        "admin-required",
        "sensor-unavailable",
        "sensor-error",
    ]
    .contains(&status)
        || !reply["celsius"].is_null()
    {
        return Err("invalid unavailable sensor state".into());
    }
    Ok(reply)
}

pub fn parse_sc_query_state(exit_code: Option<i32>, stdout: &str, stderr: &str) -> CpuSensorServiceState {
    if exit_code == Some(1060)
        || stdout.contains("1060")
        || stderr.contains("1060")
    {
        return CpuSensorServiceState::NotInstalled;
    }
    for line in stdout.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("STATE") {
            continue;
        }
        if let Some((_, rhs)) = trimmed.split_once(':') {
            if let Some(code_str) = rhs.split_whitespace().next() {
                return match code_str {
                    "4" => CpuSensorServiceState::Running,
                    _ => CpuSensorServiceState::Stopped,
                };
            }
        }
    }
    CpuSensorServiceState::NotInstalled
}

/// Checks for one-shot elevated CLI flags (`--install-cpu-sensor-service` /
/// `--uninstall-cpu-sensor-service`) before Tauri initializes and exits immediately.
pub fn maybe_handle_cli_action() {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|a| a == INSTALL_FLAG) {
            let source_dir = args
                .windows(2)
                .find(|w| w[0] == SOURCE_DIR_FLAG)
                .map(|w| PathBuf::from(&w[1]));
            let code = service_mgmt::elevated_install(source_dir.as_deref());
            std::process::exit(code);
        }
        if args.iter().any(|a| a == UNINSTALL_FLAG) {
            let code = service_mgmt::elevated_uninstall();
            std::process::exit(code);
        }
    }
}

pub fn service_status() -> CpuSensorStatus {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        service_mgmt::query_status()
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        CpuSensorStatus {
            state: CpuSensorServiceState::Unsupported,
            driver_installed: false,
            outdated: false,
            elevated_fallback: false,
            active: false,
        }
    }
}

pub fn is_available() -> bool {
    let status = service_status();
    status.active
}

pub fn install_service() -> Result<CpuSensorStatus, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        shutdown();
        let source_dir = helper_directory(RESOURCE_DIR.get().map(PathBuf::as_path))?;
        let code = if crate::elevation::process_is_elevated() {
            service_mgmt::elevated_install(Some(&source_dir))
        } else {
            let args = vec![
                INSTALL_FLAG.to_string(),
                SOURCE_DIR_FLAG.to_string(),
                source_dir.to_string_lossy().to_string(),
            ];
            match crate::elevation::run_elevated_self_and_wait(&args, 60_000) {
                Ok(c) => c as i32,
                Err(crate::elevation::ElevatedRunError::Cancelled) => {
                    return Err("Administratoradgang blev annulleret".to_string());
                }
                Err(crate::elevation::ElevatedRunError::Failed(msg)) => return Err(msg),
            }
        };
        if code != EXIT_OK {
            return Err(exit_code_message(code).to_string());
        }
        reset_backoff();
        Ok(service_mgmt::query_status())
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        Err("CPU-sensortjenesten understøttes kun på Windows (x64)".to_string())
    }
}

pub fn uninstall_service() -> Result<CpuSensorStatus, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        shutdown();
        let code = if crate::elevation::process_is_elevated() {
            service_mgmt::elevated_uninstall()
        } else {
            let args = vec![UNINSTALL_FLAG.to_string()];
            match crate::elevation::run_elevated_self_and_wait(&args, 30_000) {
                Ok(c) => c as i32,
                Err(crate::elevation::ElevatedRunError::Cancelled) => {
                    return Err("Administratoradgang blev annulleret".to_string());
                }
                Err(crate::elevation::ElevatedRunError::Failed(msg)) => return Err(msg),
            }
        };
        if code != EXIT_OK {
            return Err(exit_code_message(code).to_string());
        }
        reset_backoff();
        Ok(service_mgmt::query_status())
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        Err("CPU-sensortjenesten understøttes kun på Windows (x64)".to_string())
    }
}

fn exit_code_message(code: i32) -> &'static str {
    match code {
        EXIT_NOT_ELEVATED => "Kræver administratorrettigheder for at ændre baggrundstjenesten",
        EXIT_HELPER_MISSING => "CPU-sensor hjælpefiler blev ikke fundet",
        EXIT_INTEGRITY_FAILED => "Integritetstjek af CPU-sensor filer fejlede",
        EXIT_STOP_FAILED => "Kunne ikke stoppe den eksisterende CPU-sensortjeneste",
        EXIT_COPY_FAILED => "Kunne ikke kopiere CPU-sensor filer til Program Files",
        EXIT_REGISTER_FAILED => "Kunne ikke registrere Windows-tjenesten",
        EXIT_START_FAILED => "Tjenesten blev installeret, men kunne ikke startes",
        EXIT_REMOVE_FAILED => "Kunne ikke fjerne CPU-sensortjenesten helt",
        EXIT_DRIVER_FAILED => "Kunne ikke installere PawnIO-driveren",
        _ => "Uventet fejl under konfiguration af CPU-sensortjenesten",
    }
}

fn reset_backoff() {
    let mut state = STATE.lock();
    state.retry_at = None;
    state.last_status = None;
}

pub fn sample_celsius() -> Option<f32> {
    let mut state = STATE.lock();
    if state.retry_at.is_some_and(|at| Instant::now() < at) {
        return None;
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    let result = sample_windows(&mut state);

    #[cfg(not(all(windows, target_arch = "x86_64")))]
    let result: Result<Value, String> = Ok(unavailable("platform-unsupported"));

    let value = match result {
        Ok(val) => val,
        Err(err) => {
            log::warn!(target: "provider:system", "CPU-sensor fejl: {err}");
            unavailable("helper-error")
        }
    };

    let status = value["status"].as_str().unwrap_or("helper-error");
    if status != "ready" {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            state.pipe = None;
            state.worker = None;
        }
        if state.last_status.as_deref() != Some(status) {
            log::info!(target: "provider:system", "CPU-sensor status: {status}");
        }
        let backoff_secs = match status {
            "driver-required" | "admin-required" => 30,
            "service-unavailable" => 5,
            _ => 10,
        };
        state.retry_at = Some(Instant::now() + Duration::from_secs(backoff_secs));
        state.last_status = Some(status.to_string());
        None
    } else {
        if state.last_status.as_deref() != Some("ready") {
            log::info!(
                target: "provider:system",
                "CPU-sensor aktiv ({})",
                value["sensor"].as_str().unwrap_or("package")
            );
        }
        state.retry_at = None;
        state.last_status = Some("ready".to_string());
        value["celsius"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0 && *v < 125.0)
            .map(|v| v as f32)
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn sample_windows(state: &mut SensorState) -> Result<Value, String> {
    // 1. Prefer the local Windows Service over Named Pipe (works without admin in Open Dock).
    if state.pipe.is_some() {
        if let Some(client) = state.pipe.as_mut() {
            match client.sample() {
                Ok(val) => return Ok(val),
                Err(err) => {
                    log::debug!(target: "provider:system", "CPU-sensor pipe afbrudt: {err}");
                    state.pipe = None;
                }
            }
        }
    }

    if state.worker.is_none() {
        match pipe_client::PipeClient::connect() {
            Ok(mut client) => {
                let val = client.sample();
                match val {
                    Ok(v) => {
                        state.pipe = Some(client);
                        return Ok(v);
                    }
                    Err(err) => {
                        log::debug!(target: "provider:system", "CPU-sensor pipe svar fejlede: {err}");
                    }
                }
            }
            Err(pipe_client::ConnectError::NotAvailable) => {}
            Err(pipe_client::ConnectError::Other(err)) => {
                log::debug!(target: "provider:system", "CPU-sensor pipe kunne ikke åbnes: {err}");
            }
        }
    }

    // 2. Fallback: if Open Dock itself happens to be running elevated, spawn the local worker directly.
    if !crate::elevation::process_is_elevated() {
        return Ok(unavailable("service-unavailable"));
    }

    if state.worker.is_none() {
        let directory = helper_directory(RESOURCE_DIR.get().map(PathBuf::as_path))?;
        state.worker = Some(worker::Worker::start(&directory)?);
    }
    let worker = state
        .worker
        .as_mut()
        .ok_or_else(|| "CPU sensor worker unavailable".to_string())?;
    worker.sample()
}

pub fn shutdown() {
    if let Some(mut state) = STATE.try_lock() {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            state.pipe = None;
            state.worker = None;
        }
        state.retry_at = None;
        state.last_status = None;
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn expected_helper_hash() -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/OpenDock.CpuSensors.exe"
        )))
    )
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn manifest_files() -> Result<Vec<(String, String)>, String> {
    let manifest: Value =
        serde_json::from_str(include_str!("../../resources/cpu-sensor/libraries.json"))
            .map_err(|e| e.to_string())?;
    let mut list = Vec::new();
    for entry in manifest["files"]
        .as_array()
        .ok_or("missing CPU sensor assets")?
    {
        let name = entry["file"].as_str().ok_or("invalid CPU sensor asset")?;
        if name.contains(['/', '\\']) {
            return Err("invalid CPU sensor asset path".into());
        }
        let hash = entry["sha256"].as_str().unwrap_or("").to_string();
        list.push((name.to_string(), hash));
    }
    Ok(list)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn pawnio_installer() -> Result<(String, String), String> {
    let manifest: Value =
        serde_json::from_str(include_str!("../../resources/cpu-sensor/libraries.json"))
            .map_err(|e| e.to_string())?;
    let name = manifest["pawnio"]["file"]
        .as_str()
        .ok_or("missing PawnIO installer asset")?;
    if name.contains(['/', '\\']) || !name.ends_with(".exe") {
        return Err("invalid PawnIO installer path".into());
    }
    let hash = manifest["pawnio"]["sha256"].as_str().unwrap_or("").to_string();
    Ok((name.to_string(), hash))
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod pipe_client {
    use super::*;
    use std::{
        fs::OpenOptions,
        io::{BufRead, BufReader, ErrorKind, Read, Write},
        sync::mpsc::{self, Receiver, SyncSender},
        thread,
    };

    pub(super) enum ConnectError {
        NotAvailable,
        Other(String),
    }

    pub(super) struct PipeClient {
        requests: Option<SyncSender<()>>,
        replies: Receiver<Result<Vec<u8>, String>>,
        sequence: u64,
    }

    impl PipeClient {
        pub(super) fn connect() -> Result<Self, ConnectError> {
            let mut last_err = None;
            for attempt in 0..4 {
                match OpenOptions::new().read(true).write(true).open(PIPE_PATH) {
                    Ok(pipe) => {
                        let (req_tx, req_rx) = mpsc::sync_channel::<()>(1);
                        let (rep_tx, rep_rx) = mpsc::sync_channel::<Result<Vec<u8>, String>>(1);

                        // Owns the synchronous pipe handle on a single thread so
                        // WriteFile and ReadFile never run concurrently on it.
                        thread::spawn(move || {
                            let mut reader = BufReader::new(pipe);
                            while req_rx.recv().is_ok() {
                                if let Err(err) = reader
                                    .get_mut()
                                    .write_all(b"sample\n")
                                    .and_then(|_| reader.get_mut().flush())
                                {
                                    let _ = rep_tx.send(Err(err.to_string()));
                                    break;
                                }
                                let mut line = Vec::new();
                                match reader.by_ref().take(4097).read_until(b'\n', &mut line) {
                                    Ok(0) => {
                                        let _ = rep_tx.send(Err("pipe closed".into()));
                                        break;
                                    }
                                    Ok(_) if line.len() <= 4096 => {
                                        if rep_tx.send(Ok(line)).is_err() {
                                            break;
                                        }
                                    }
                                    Ok(_) => {
                                        let _ = rep_tx.send(Err("oversized pipe reply".into()));
                                        break;
                                    }
                                    Err(err) => {
                                        let _ = rep_tx.send(Err(err.to_string()));
                                        break;
                                    }
                                }
                            }
                        });

                        return Ok(Self {
                            requests: Some(req_tx),
                            replies: rep_rx,
                            sequence: 0,
                        });
                    }
                    Err(err) if err.raw_os_error() == Some(231) && attempt < 3 => {
                        // ERROR_PIPE_BUSY: brief retry while previous instance resets.
                        thread::sleep(Duration::from_millis(50));
                        last_err = Some(err);
                    }
                    Err(err) if err.kind() == ErrorKind::NotFound => {
                        return Err(ConnectError::NotAvailable);
                    }
                    Err(err) => {
                        last_err = Some(err);
                        break;
                    }
                }
            }
            Err(ConnectError::Other(
                last_err
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "pipe unavailable".to_string()),
            ))
        }

        pub(super) fn sample(&mut self) -> Result<Value, String> {
            self.sequence += 1;
            let req = self.requests.as_ref().ok_or("CPU sensor pipe closed")?;
            req.send(()).map_err(|_| "CPU sensor pipe thread closed")?;
            let raw = self
                .replies
                .recv_timeout(Duration::from_secs(if self.sequence == 1 { 5 } else { 2 }))
                .map_err(|_| "CPU sensor service timeout or exit")??;
            parse_reply(&raw, self.sequence, unix_now_ms())
        }
    }

    impl Drop for PipeClient {
        fn drop(&mut self) {
            self.requests.take();
        }
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod service_mgmt {
    use super::*;
    use std::{
        fs,
        os::windows::process::CommandExt,
        process::Command,
        thread,
    };

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    fn sc_exe() -> PathBuf {
        PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32")
            .join("sc.exe")
    }

    pub(super) fn install_directory() -> PathBuf {
        PathBuf::from(
            std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into()),
        )
        .join("Open Dock Sensor")
    }

    fn run_sc(args: &[&str]) -> Option<(Option<i32>, String, String)> {
        let output = Command::new(sc_exe())
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        Some((
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }

    fn query_named_service(name: &str) -> CpuSensorServiceState {
        match run_sc(&["query", name]) {
            Some((code, stdout, stderr)) => parse_sc_query_state(code, &stdout, &stderr),
            None => CpuSensorServiceState::NotInstalled,
        }
    }

    // LibreHardwareMonitor kræver PawnIO >= 2.2 og slår dens version op i
    // uninstall-nøglen; vi spejler det så status matcher helperens driver-check.
    fn pawnio_version() -> Option<(u32, u32, u32)> {
        use windows::{
            core::w,
            Win32::{
                Foundation::ERROR_SUCCESS,
                System::Registry::{
                    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE,
                    KEY_READ, REG_SZ, REG_VALUE_TYPE,
                },
            },
        };

        struct KeyGuard(HKEY);
        impl Drop for KeyGuard {
            fn drop(&mut self) {
                if !self.0.is_invalid() {
                    unsafe {
                        let _ = RegCloseKey(self.0);
                    }
                }
            }
        }

        let mut hkey = HKEY::default();
        let open = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PawnIO"),
                None,
                KEY_READ,
                &mut hkey,
            )
        };
        if open != ERROR_SUCCESS {
            return None;
        }
        let _guard = KeyGuard(hkey);

        let mut value_type = REG_VALUE_TYPE::default();
        let mut buf = [0u16; 64];
        let mut size = (buf.len() * 2) as u32;
        let query = unsafe {
            RegQueryValueExW(
                hkey,
                w!("DisplayVersion"),
                None,
                Some(&mut value_type),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut size),
            )
        };
        if query != ERROR_SUCCESS || value_type != REG_SZ || size == 0 {
            return None;
        }
        let text = String::from_utf16_lossy(&buf);
        let trimmed = text.trim_matches('\0').trim();
        let mut parts = trimmed.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Some((major, minor, patch))
    }

    fn driver_ready() -> bool {
        query_named_service("PawnIO") != CpuSensorServiceState::NotInstalled
            && pawnio_version().is_some_and(|v| v >= (2, 2, 0))
    }

    pub(super) fn query_status() -> CpuSensorStatus {
        let state = query_named_service(SERVICE_NAME);
        let driver_installed = driver_ready();
        let outdated = if state == CpuSensorServiceState::NotInstalled {
            false
        } else {
            let installed_exe = install_directory().join("OpenDock.CpuSensors.exe");
            worker::checked_file(&installed_exe, &expected_helper_hash()).is_err()
        };
        let elevated = crate::elevation::process_is_elevated();
        let elevated_fallback = elevated && state != CpuSensorServiceState::Running;
        let active =
            driver_installed && (state == CpuSensorServiceState::Running || elevated);

        CpuSensorStatus {
            state,
            driver_installed,
            outdated,
            elevated_fallback,
            active,
        }
    }

    fn stop_and_wait() -> bool {
        let state = query_named_service(SERVICE_NAME);
        if state == CpuSensorServiceState::NotInstalled || state == CpuSensorServiceState::Stopped {
            return true;
        }
        let _ = run_sc(&["stop", SERVICE_NAME]);
        for _ in 0..40 {
            thread::sleep(Duration::from_millis(150));
            let current = query_named_service(SERVICE_NAME);
            if current == CpuSensorServiceState::Stopped
                || current == CpuSensorServiceState::NotInstalled
            {
                return true;
            }
        }
        false
    }

    fn copy_with_retry(src: &Path, dst: &Path) -> bool {
        for attempt in 0..10 {
            if fs::copy(src, dst).is_ok() {
                return true;
            }
            if attempt < 9 {
                thread::sleep(Duration::from_millis(150));
            }
        }
        false
    }

    pub(super) fn elevated_install(explicit_source: Option<&Path>) -> i32 {
        if !crate::elevation::process_is_elevated() {
            return EXIT_NOT_ELEVATED;
        }

        let source_dir = match explicit_source {
            Some(dir) if has_helper_files(dir) => dir.to_path_buf(),
            _ => match helper_directory(None) {
                Ok(dir) => dir,
                Err(_) => return EXIT_HELPER_MISSING,
            },
        };

        let files = match manifest_files() {
            Ok(list) => list,
            Err(_) => return EXIT_INTEGRITY_FAILED,
        };

        // Verify SHA-256 of every single asset against compile-time pinned hashes
        // before copying into the privileged Program Files directory.
        let src_exe = source_dir.join("OpenDock.CpuSensors.exe");
        let Ok(_exe_guard) = worker::checked_file(&src_exe, &expected_helper_hash()) else {
            return EXIT_INTEGRITY_FAILED;
        };
        let mut _dll_guards = Vec::with_capacity(files.len());
        for (name, hash) in &files {
            let Ok(guard) = worker::checked_file(&source_dir.join(name), hash) else {
                return EXIT_INTEGRITY_FAILED;
            };
            _dll_guards.push(guard);
        }

        if !stop_and_wait() {
            return EXIT_STOP_FAILED;
        }

        let target_dir = install_directory();
        if fs::create_dir_all(&target_dir).is_err() {
            return EXIT_COPY_FAILED;
        }
        let target_exe = target_dir.join("OpenDock.CpuSensors.exe");
        if !copy_with_retry(&src_exe, &target_exe) {
            return EXIT_COPY_FAILED;
        }
        for (name, _) in &files {
            if !copy_with_retry(&source_dir.join(name), &target_dir.join(name)) {
                return EXIT_COPY_FAILED;
            }
        }

        let bin_path = format!("\"{}\" --service", target_exe.display());
        let existing = query_named_service(SERVICE_NAME) != CpuSensorServiceState::NotInstalled;
        let verb = if existing { "config" } else { "create" };
        let reg_ok = run_sc(&[
            verb,
            SERVICE_NAME,
            "binPath=",
            &bin_path,
            "start=",
            "auto",
            "DisplayName=",
            SERVICE_DISPLAY_NAME,
        ])
        .is_some_and(|(code, _, _)| code == Some(0));

        if !reg_ok {
            return EXIT_REGISTER_FAILED;
        }

        let _ = run_sc(&["description", SERVICE_NAME, SERVICE_DESCRIPTION]);
        let _ = run_sc(&[
            "failure",
            SERVICE_NAME,
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/15000//0",
        ]);

        let start_res = run_sc(&["start", SERVICE_NAME]);
        let started_ok = start_res
            .is_some_and(|(code, _, _)| code == Some(0) || code == Some(1056));
        if !started_ok {
            return EXIT_START_FAILED;
        }

        let mut running = false;
        for _ in 0..40 {
            if query_named_service(SERVICE_NAME) == CpuSensorServiceState::Running {
                running = true;
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
        if !running {
            return EXIT_START_FAILED;
        }

        if driver_ready() {
            return EXIT_OK;
        }

        install_pawnio_driver(&source_dir)
    }

    // Bundlet, hash-pinned officiel PawnIO-installer (må redistribueres uændret).
    // Kører silent i samme eleverede session som tjeneste-installationen.
    fn install_pawnio_driver(source_dir: &Path) -> i32 {
        let (setup_name, setup_hash) = match pawnio_installer() {
            Ok(pair) => pair,
            Err(_) => return EXIT_HELPER_MISSING,
        };
        let setup = source_dir.join(&setup_name);
        let Ok(_guard) = worker::checked_file(&setup, &setup_hash) else {
            return EXIT_INTEGRITY_FAILED;
        };
        let ran = Command::new(&setup)
            .args(["-install", "-silent"])
            .current_dir(source_dir)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ran {
            return EXIT_DRIVER_FAILED;
        }
        for _ in 0..60 {
            if driver_ready() {
                return EXIT_OK;
            }
            thread::sleep(Duration::from_millis(250));
        }
        EXIT_DRIVER_FAILED
    }

    pub(super) fn elevated_uninstall() -> i32 {
        if !crate::elevation::process_is_elevated() {
            return EXIT_NOT_ELEVATED;
        }

        if query_named_service(SERVICE_NAME) != CpuSensorServiceState::NotInstalled {
            let _ = stop_and_wait();
            let deleted = run_sc(&["delete", SERVICE_NAME])
                .is_some_and(|(code, _, _)| code == Some(0) || code == Some(1060) || code == Some(1072));
            if !deleted {
                return EXIT_REMOVE_FAILED;
            }
            for _ in 0..20 {
                if query_named_service(SERVICE_NAME) == CpuSensorServiceState::NotInstalled {
                    break;
                }
                thread::sleep(Duration::from_millis(150));
            }
        }

        let target_dir = install_directory();
        if target_dir.exists() {
            for attempt in 0..15 {
                if fs::remove_dir_all(&target_dir).is_ok() {
                    break;
                }
                if attempt < 14 {
                    thread::sleep(Duration::from_millis(150));
                }
            }
        }

        EXIT_OK
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod worker {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        fs::{File, OpenOptions},
        io::{BufRead, BufReader, Read, Write},
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle, process::CommandExt},
        process::{Child, ChildStdin, Command, Stdio},
        sync::mpsc::{self, Receiver},
        thread,
    };
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    struct Job(HANDLE);
    unsafe impl Send for Job {}
    impl Drop for Job {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                let _ = unsafe { CloseHandle(self.0) };
            }
        }
    }

    pub(super) struct Worker {
        child: Child,
        input: Option<ChildStdin>,
        replies: Receiver<Result<Vec<u8>, String>>,
        sequence: u64,
        _job: Job,
        _assets: Vec<File>,
    }

    pub(super) fn checked_file(path: &Path, expected: &str) -> Result<File, String> {
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(path)
            .map_err(|e| format!("CPU sensor asset {}: {e}", path.display()))?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 16384];
        loop {
            let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        if format!("{:x}", hash.finalize()) != expected {
            return Err(format!(
                "CPU sensor asset integrity failed: {}",
                path.display()
            ));
        }
        Ok(file)
    }

    impl Worker {
        pub(super) fn start(directory: &Path) -> Result<Self, String> {
            let files = manifest_files()?;
            let mut assets = Vec::new();
            let executable = directory.join("OpenDock.CpuSensors.exe");
            assets.push(checked_file(&executable, &expected_helper_hash())?);

            for (name, hash) in &files {
                assets.push(checked_file(&directory.join(name), hash)?);
            }

            let job = Job(unsafe { CreateJobObjectW(None, None) }.map_err(|e| e.to_string())?);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                )
            }
            .map_err(|e| e.to_string())?;

            let mut command = Command::new(&executable);
            command
                .current_dir(directory)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .creation_flags(0x08000000);

            let mut child = command
                .spawn()
                .map_err(|e| format!("CPU sensor helper start: {e}"))?;

            if let Err(error) =
                unsafe { AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle())) }
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("CPU sensor process tracking: {error}"));
            }

            let input = child.stdin.take();
            if let Some(error_output) = child.stderr.take() {
                thread::spawn(move || {
                    let mut reader = BufReader::new(error_output);
                    for _ in 0..8 {
                        let mut line = String::new();
                        if reader.by_ref().take(2048).read_line(&mut line).unwrap_or(0) == 0 {
                            break;
                        }
                        log::warn!(target: "provider:system", "{}", line.trim());
                    }
                });
            }

            let output = child.stdout.take().ok_or("CPU sensor output unavailable")?;
            let (sender, replies) = mpsc::sync_channel(1);
            thread::spawn(move || {
                let mut reader = BufReader::new(output);
                loop {
                    let mut line = Vec::new();
                    let read = reader.by_ref().take(4097).read_until(b'\n', &mut line);
                    match read {
                        Ok(0) => break,
                        Ok(_) if line.len() <= 4096 => {
                            if sender.send(Ok(line)).is_err() {
                                break;
                            }
                        }
                        _ => {
                            let _ = sender.send(Err("invalid CPU sensor output".into()));
                            break;
                        }
                    }
                }
            });

            Ok(Self {
                child,
                input,
                replies,
                sequence: 0,
                _job: job,
                _assets: assets,
            })
        }

        pub(super) fn sample(&mut self) -> Result<Value, String> {
            self.sequence += 1;
            let input = self.input.as_mut().ok_or("CPU sensor input closed")?;
            input
                .write_all(b"sample\n")
                .and_then(|_| input.flush())
                .map_err(|e| e.to_string())?;
            let raw = self
                .replies
                .recv_timeout(Duration::from_secs(if self.sequence == 1 { 5 } else { 2 }))
                .map_err(|_| "CPU sensor helper timeout or exit")??;
            parse_reply(&raw, self.sequence, unix_now_ms())
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            self.input.take();
            for _ in 0..10 {
                if self.child.try_wait().ok().flatten().is_some() {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::checked_file;
        use sha2::{Digest, Sha256};

        #[test]
        fn altered_sensor_assets_are_rejected() {
            let path = std::env::temp_dir()
                .join(format!("open-dock-sensor-integrity-{}.dll", std::process::id()));
            std::fs::write(&path, b"changed library").unwrap();
            let result = checked_file(&path, &format!("{:x}", Sha256::digest(b"original library")));
            std::fs::remove_file(&path).unwrap();
            assert!(result.err().unwrap().contains("integrity failed"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        helper_directory, parse_reply, parse_sc_query_state, CpuSensorServiceState, SOURCE,
    };
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn reply() -> Value {
        json!({
            "schema": 1,
            "sequence": 1,
            "at": 10_000,
            "celsius": 55.25,
            "source": SOURCE,
            "status": "ready",
            "sensor": "Core (Tctl/Tdie)",
            "sensorId": "/amdcpu/0/temperature/0",
            "hardware": "AMD Ryzen 7 7700X"
        })
    }

    fn parse(value: Value) -> Result<Value, String> {
        parse_reply(value.to_string().as_bytes(), 1, 10_000)
    }

    #[test]
    fn resolves_manifest_helper_directory() {
        let dir = helper_directory(None).expect("manifest helper directory should exist after build.rs");
        assert_eq!(
            dir,
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("cpu-sensor")
        );
    }

    #[test]
    fn parses_sc_query_states_across_locales() {
        let running = "SERVICE_NAME: OpenDockCpuSensor\n        STATE              : 4  RUNNING\n";
        assert_eq!(
            parse_sc_query_state(Some(0), running, ""),
            CpuSensorServiceState::Running
        );
        let stopped = "SERVICE_NAME: OpenDockCpuSensor\n        STATE              : 1  STOPPED\n";
        assert_eq!(
            parse_sc_query_state(Some(0), stopped, ""),
            CpuSensorServiceState::Stopped
        );
        assert_eq!(
            parse_sc_query_state(Some(1060), "[SC] OpenService FAILED 1060:", ""),
            CpuSensorServiceState::NotInstalled
        );
    }

    #[test]
    fn valid_package_and_unavailable_states_are_distinct() {
        assert_eq!(parse(reply()).unwrap()["celsius"], 55.25);
        for status in ["driver-required", "admin-required", "sensor-unavailable", "sensor-error"] {
            let mut value = reply();
            value["status"] = json!(status);
            value["celsius"] = Value::Null;
            assert_eq!(parse(value).unwrap()["status"], status);
        }
    }

    #[test]
    fn stale_foreign_and_invalid_sensor_replies_are_rejected() {
        for (key, value) in [
            ("sequence", json!(2)),
            ("schema", json!(2)),
            ("at", json!(1000)),
            ("at", json!(20_000)),
            ("celsius", json!(0)),
            ("celsius", json!(125)),
            ("sensor", json!("CCD1 (Tdie)")),
            ("sensorId", json!("/lpc/0/temperature/0")),
            ("source", json!("acpi")),
            ("status", json!("driver-required")),
        ] {
            let mut input = reply();
            input[key] = value;
            assert!(parse(input).is_err(), "{key}");
        }
        assert!(parse_reply(&vec![b' '; 4097], 1, 10_000).is_err());
        assert!(parse_reply(b"garbage", 1, 10_000).is_err());
    }
}
