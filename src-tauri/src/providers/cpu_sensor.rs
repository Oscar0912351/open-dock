use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde_json::{json, Value};

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();
static STATE: LazyLock<Mutex<SensorState>> = LazyLock::new(|| Mutex::new(SensorState::default()));

const SOURCE: &str = "lhm-direct-package";

pub fn configure(resource_dir: PathBuf) {
    let _ = RESOURCE_DIR.set(resource_dir);
}

#[derive(Default)]
struct SensorState {
    #[cfg(all(windows, target_arch = "x86_64"))]
    worker: Option<worker::Worker>,
    retry_at: Option<Instant>,
    last_status: Option<String>,
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

pub fn sample_celsius() -> Option<f32> {
    if !crate::elevation::process_is_elevated() {
        return None;
    }

    let mut state = STATE.lock();
    if state.retry_at.is_some_and(|at| Instant::now() < at) {
        return None;
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    let result = (|| {
        if state.worker.is_none() {
            let directory = helper_directory(RESOURCE_DIR.get().map(PathBuf::as_path))?;
            state.worker = Some(worker::Worker::start(&directory)?);
        }
        let worker = state
            .worker
            .as_mut()
            .ok_or_else(|| "CPU sensor worker unavailable".to_string())?;
        worker.sample()
    })();

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
            state.worker = None;
        }
        if state.last_status.as_deref() != Some(status) {
            log::info!(target: "provider:system", "CPU-sensor status: {status}");
        }
        let backoff_secs = if matches!(status, "driver-required" | "admin-required") {
            30
        } else {
            10
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

pub fn shutdown() {
    if let Some(mut state) = STATE.try_lock() {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            state.worker = None;
        }
        state.retry_at = None;
        state.last_status = None;
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
            let manifest: Value =
                serde_json::from_str(include_str!("../../resources/cpu-sensor/libraries.json"))
                    .map_err(|e| e.to_string())?;
            let mut assets = Vec::new();
            let executable = directory.join("OpenDock.CpuSensors.exe");
            let helper_hash = format!(
                "{:x}",
                Sha256::digest(include_bytes!(concat!(
                    env!("OUT_DIR"),
                    "/OpenDock.CpuSensors.exe"
                )))
            );
            assets.push(checked_file(&executable, &helper_hash)?);

            for entry in manifest["files"]
                .as_array()
                .ok_or("missing CPU sensor assets")?
            {
                let name = entry["file"].as_str().ok_or("invalid CPU sensor asset")?;
                if name.contains(['/', '\\']) {
                    return Err("invalid CPU sensor asset path".into());
                }
                assets.push(checked_file(
                    &directory.join(name),
                    entry["sha256"].as_str().unwrap_or(""),
                )?);
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
    use super::{helper_directory, parse_reply, SOURCE};
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
