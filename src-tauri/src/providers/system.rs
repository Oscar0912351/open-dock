use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};
use tauri::{AppHandle, Manager};
use ts_rs::TS;

use crate::{state::AppState, util::unix_now_ms};

use super::{
    gpu::{compute_effective_freq_mhz, HardwareSampler, SystemCaps},
    worker::{StopToken, Worker},
};

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
    worker: Mutex<Option<Worker>>,
    history: Arc<Mutex<VecDeque<SystemSample>>>,
    caps_cache: Arc<Mutex<Option<SystemCaps>>>,
    pending_sampler: Arc<Mutex<Option<HardwareSampler>>>,
}

impl Default for SystemProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemProvider {
    pub fn new() -> Self {
        Self {
            worker: Mutex::new(None),
            history: Arc::new(Mutex::new(VecDeque::with_capacity(HISTORY_CAP))),
            caps_cache: Arc::new(Mutex::new(None)),
            pending_sampler: Arc::new(Mutex::new(None)),
        }
    }

    pub fn caps(&self) -> SystemCaps {
        let mut cache = self.caps_cache.lock();
        if let Some(caps) = cache.as_ref() {
            return caps.clone();
        }
        let sampler = HardwareSampler::new();
        let fresh = sampler.caps.clone();
        *cache = Some(fresh.clone());
        drop(cache);
        *self.pending_sampler.lock() = Some(sampler);
        fresh
    }

    pub fn snapshot(&self) -> Option<SystemSample> {
        self.history.lock().back().cloned()
    }

    pub fn history(&self) -> Vec<SystemSample> {
        self.history.lock().iter().cloned().collect()
    }

    pub fn start(&self, app: &AppHandle) {
        let mut slot = self.worker.lock();
        if slot.as_ref().is_some_and(|worker| !worker.is_finished()) {
            return;
        }
        log::info!(target: "provider:system", "Starter system-provider (od-system)");

        let app_handle = app.clone();
        let history = Arc::clone(&self.history);
        let caps_cache = Arc::clone(&self.caps_cache);
        let pending_sampler = Arc::clone(&self.pending_sampler);
        *slot = Worker::spawn("od-system", "provider:system", move |stop| {
            let hw = pending_sampler
                .lock()
                .take()
                .unwrap_or_else(HardwareSampler::new);
            run_system_thread(app_handle, history, caps_cache, stop, hw);
        });
    }

    pub fn stop(&self) {
        let Some(worker) = self.worker.lock().take() else {
            return;
        };
        log::info!(target: "provider:system", "Stopper system-provider");
        drop(worker);
        self.history.lock().clear();
    }
}

fn run_system_thread(
    app_handle: AppHandle,
    history: Arc<Mutex<VecDeque<SystemSample>>>,
    caps_cache: Arc<Mutex<Option<SystemCaps>>>,
    stop: StopToken,
    mut hw: HardwareSampler,
) {
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

    *caps_cache.lock() = Some(hw.caps.clone());
    let mut last_tick = Instant::now();
    let mut first_tick = true;
    let mut tick_count: u64 = 0;
    let mut last_cpu_temp_c: Option<f32> = None;

    loop {
        if stop.is_stopped() {
            break;
        }
        let sleep_ms = if first_tick {
            250
        } else {
            app_handle
                .state::<AppState>()
                .config
                .read_with(|c| c.general.system_interval_ms.max(500) as u64)
        };

        if stop.wait(Duration::from_millis(sleep_ms)) {
            break;
        }

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
            let fresh = super::cpu_sensor::sample_celsius();
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

        if stop.is_stopped() {
            break;
        }
        {
            let mut guard = history.lock();
            if guard.len() >= HISTORY_CAP {
                guard.pop_front();
            }
            guard.push_back(sample.clone());
        }

        if stop.is_stopped() {
            break;
        }
        super::scheduler::emit_provider(
            &app_handle,
            super::scheduler::ProviderKind::System,
            None,
            "provider://system",
            &sample,
        );
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
