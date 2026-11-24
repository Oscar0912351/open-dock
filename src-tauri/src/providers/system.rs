use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};
use tauri::{async_runtime::JoinHandle, AppHandle, Manager};
use ts_rs::TS;

use crate::state::AppState;

use super::gpu::{compute_effective_freq_mhz, HardwareSampler, SystemCaps};

const HISTORY_CAP: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct CpuSample {
    pub usage: f32,
    pub core_count: u32,
    pub per_core: Vec<f32>,
    pub freq_mhz: Option<u32>,
    pub temp_c: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct MemSample {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct GpuSample {
    pub usage: Option<f32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub temp_c: Option<f32>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct NetSample {
    pub rx_bps: f64,
    pub tx_bps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct DiskSample {
    pub read_bps: f64,
    pub write_bps: f64,
    pub active_pct: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct SystemSample {
    pub t: i64,
    pub uptime_s: u64,
    pub cpu: CpuSample,
    pub mem: MemSample,
    pub gpu: Option<GpuSample>,
    pub net: NetSample,
    pub disk: DiskSample,
}

pub struct SystemProvider {
    running: AtomicBool,
    task: Mutex<Option<JoinHandle<()>>>,
    history: Arc<Mutex<VecDeque<SystemSample>>>,
    caps_cache: Arc<Mutex<Option<SystemCaps>>>,
}

impl Default for SystemProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemProvider {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            task: Mutex::new(None),
            history: Arc::new(Mutex::new(VecDeque::with_capacity(HISTORY_CAP))),
            caps_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn caps(&self) -> SystemCaps {
        let cached = self.caps_cache.lock().clone();
        if let Some(caps) = cached {
            return caps;
        }
        let fresh = HardwareSampler::new().caps.clone();
        *self.caps_cache.lock() = Some(fresh.clone());
        fresh
    }

    pub fn snapshot(&self) -> Option<SystemSample> {
        self.history.lock().back().cloned()
    }

    pub fn history(&self) -> Vec<SystemSample> {
        self.history.lock().iter().cloned().collect()
    }

    pub fn start(&self, app: &AppHandle) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        log::info!(target: "provider:system", "Starter system-provider");

        let app_handle = app.clone();
        let history = Arc::clone(&self.history);
        let caps_cache = Arc::clone(&self.caps_cache);

        let handle = tauri::async_runtime::spawn(async move {
            let mut sys = match System::new_with_specifics(
                RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::nothing().with_frequency())
                    .with_memory(MemoryRefreshKind::nothing().with_ram()),
            ) {
                Ok(system) => system,
                Err(err) => {
                    log::warn!(target: "provider:system", "System::new_with_specifics fejlede: {err}");
                    return;
                }
            };

            let mut nets = match Networks::new_with_refreshed_list() {
                Ok(n) => Some(n),
                Err(err) => {
                    log::warn!(target: "provider:system", "Networks::new_with_refreshed_list fejlede: {err}");
                    None
                }
            };

            let base_freq_mhz = {
                let cpus = sys.cpus();
                if cpus.is_empty() {
                    None
                } else {
                    let sum: u64 = cpus.iter().map(|c| c.frequency()).sum();
                    let avg = (sum / cpus.len() as u64) as u32;
                    (avg > 0).then_some(avg)
                }
            };
            // Antallet af logiske CPU'er er hardwaren, ikke processens affinity-maske.
            // available_parallelism() kan underrapportere på maskiner med processorgrupper
            // eller begrænset affinity, og det er ikke det tal Jobliste viser.
            let core_count = sys.cpus().len().max(1) as u32;

            let mut hw = HardwareSampler::new();
            *caps_cache.lock() = Some(hw.caps.clone());
            let mut last_tick = Instant::now();
            let mut first_tick = true;
            let mut tick_count: u64 = 0;
            let mut last_cpu_temp_c: Option<f32> = None;

            loop {
                let sleep_ms = if first_tick {
                    250
                } else {
                    app_handle
                        .state::<AppState>()
                        .config
                        .get()
                        .general
                        .system_interval_ms
                        .max(500) as u64
                };
                tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                first_tick = false;
                tick_count = tick_count.wrapping_add(1);

                sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
                if let Some(ref mut net_list) = nets {
                    net_list.refresh(tick_count.is_multiple_of(60));
                }

                let now = Instant::now();
                let elapsed = (now - last_tick).as_secs_f64().max(0.1);
                last_tick = now;

                let hw_data = hw.sample();
                let cpu_temp_c = if tick_count == 1 || tick_count.is_multiple_of(2) {
                    let fresh = tokio::task::spawn_blocking(super::cpu_sensor::sample_celsius)
                        .await
                        .ok()
                        .flatten();
                    last_cpu_temp_c = fresh;
                    fresh
                } else {
                    last_cpu_temp_c
                };

                if let Some(ref mut caps) = *caps_cache.lock() {
                    if cpu_temp_c.is_some() {
                        caps.cpu_temp = true;
                    }
                }

                let Some(cpu_usage) = hw_data.cpu_usage else {
                    continue;
                };

                let freq_mhz = compute_effective_freq_mhz(
                    hw_data.cpu_base_mhz.or(base_freq_mhz),
                    hw_data.cpu_perf_pct,
                );

                let (mut rx_bytes, mut tx_bytes) = (0u64, 0u64);
                if let Some(ref net_list) = nets {
                    for (iface, data) in net_list {
                        let name = iface.to_string_lossy();
                        if is_virtual_interface(&name) {
                            continue;
                        }
                        rx_bytes = rx_bytes.saturating_add(data.received());
                        tx_bytes = tx_bytes.saturating_add(data.transmitted());
                    }
                }

                let gpu = if hw.caps.gpu || hw.caps.vram || hw_data.name.is_some() {
                    if hw.caps.gpu || hw.caps.vram {
                        Some(GpuSample {
                            usage: hw_data.usage,
                            vram_used_bytes: hw_data.vram_used_bytes,
                            vram_total_bytes: hw_data.vram_total_bytes,
                            temp_c: hw_data.temp_c,
                            name: hw_data.name,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                let sample = SystemSample {
                    t: unix_now_ms(),
                    uptime_s: System::uptime().unwrap_or(0),
                    cpu: CpuSample {
                        usage: cpu_usage,
                        core_count,
                        per_core: hw_data.cpu_per_core.clone(),
                        freq_mhz,
                        temp_c: cpu_temp_c,
                    },
                    mem: MemSample {
                        used_bytes: sys.used_memory(),
                        total_bytes: sys.total_memory(),
                    },
                    gpu,
                    net: NetSample {
                        rx_bps: rx_bytes as f64 / elapsed,
                        tx_bps: tx_bytes as f64 / elapsed,
                    },
                    disk: DiskSample {
                        read_bps: hw_data.disk_read_bps,
                        write_bps: hw_data.disk_write_bps,
                        active_pct: hw_data.disk_active_pct.map(|v| v as f32),
                    },
                };

                {
                    let mut guard = history.lock();
                    if guard.len() >= HISTORY_CAP {
                        guard.pop_front();
                    }
                    guard.push_back(sample.clone());
                }

                super::scheduler::emit_provider(
                    &app_handle,
                    super::scheduler::ProviderKind::System,
                    None,
                    "provider://system",
                    &sample,
                );
            }
        });

        *self.task.lock() = Some(handle);
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::AcqRel) {
            return;
        }
        log::info!(target: "provider:system", "Stopper system-provider");
        if let Some(handle) = self.task.lock().take() {
            handle.abort();
        }
        super::cpu_sensor::shutdown();
        self.history.lock().clear();
    }
}

fn is_virtual_interface(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("loopback")
        || lower.contains("vethernet")
        || lower.contains("hyper-v")
        || lower.contains("virtualbox")
        || lower.contains("vmware")
        || lower.contains("wsl")
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
