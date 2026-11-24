use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use windows::{
    core::{PCWSTR, w},
    Win32::{
        Foundation::ERROR_SUCCESS,
        Graphics::Dxgi::{
            CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
        },
        System::{
            Performance::{
                PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
                PdhGetFormattedCounterArrayW, PdhGetFormattedCounterValue, PdhOpenQueryW,
                PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER,
                PDH_HQUERY, PDH_MORE_DATA,
            },
            Power::{
                CallNtPowerInformation, GetSystemPowerStatus, ProcessorInformation,
                PROCESSOR_POWER_INFORMATION, SYSTEM_POWER_STATUS,
            },
            Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ},
        },
    },
};

#[cfg(feature = "nvidia")]
use nvml_wrapper::{enum_wrappers::device::TemperatureSensor, Nvml};

const NVIDIA_VENDOR_ID: u32 = 0x10de;
const PDH_CSTATUS_ERROR_MASK: u32 = 0x8000_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct SystemCaps {
    pub gpu: bool,
    pub vram: bool,
    pub gpu_temp: bool,
    pub cpu_temp: bool,
    pub disk: bool,
    pub battery: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GpuTickData {
    pub cpu_usage: Option<f32>,
    pub cpu_per_core: Vec<f32>,
    pub usage: Option<f32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub temp_c: Option<f32>,
    pub name: Option<String>,
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
    pub disk_active_pct: Option<f64>,
    pub cpu_perf_pct: Option<f64>,
    pub cpu_base_mhz: Option<u32>,
}

pub struct HardwareSampler {
    query: Option<PDH_HQUERY>,
    c_cpu_utility: Option<PDH_HCOUNTER>,
    c_cpu_total: Option<PDH_HCOUNTER>,
    c_cpu_fallback: Option<PDH_HCOUNTER>,
    c_cpu_per_core: Option<PDH_HCOUNTER>,
    cpu_utility_health: CpuUtilityHealth,
    tm_time_based: bool,
    c_gpu_engine: Option<PDH_HCOUNTER>,
    c_vram: Option<PDH_HCOUNTER>,
    c_disk_read: Option<PDH_HCOUNTER>,
    c_disk_write: Option<PDH_HCOUNTER>,
    c_disk_active: Option<PDH_HCOUNTER>,
    c_cpu_perf: Option<PDH_HCOUNTER>,
    c_cpu_freq_base: Option<PDH_HCOUNTER>,
    baseline_collected: bool,
    gpu_name: Option<String>,
    gpu_luid: Option<String>,
    vram_total_bytes: Option<u64>,
    cpu_nominal_mhz: Option<u32>,
    #[cfg(feature = "nvidia")]
    nvml: Option<Nvml>,
    pub caps: SystemCaps,
    array_buf: Vec<u64>,
}

// Windows PDH and NVML handles are only accessed by the single sampler owner.
unsafe impl Send for HardwareSampler {}

impl Drop for HardwareSampler {
    fn drop(&mut self) {
        if let Some(query) = self.query.take() {
            unsafe {
                let _ = PdhCloseQuery(query);
            }
        }
    }
}

impl Default for HardwareSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl HardwareSampler {
    pub fn new() -> Self {
        let (mut gpu_name, mut vram_total_bytes, gpu_luid, gpu_vendor_id) = detect_dxgi_adapter();
        let battery = detect_battery();

        let use_nvidia = gpu_vendor_id.is_none() || gpu_vendor_id == Some(NVIDIA_VENDOR_ID);

        #[cfg(feature = "nvidia")]
        let nvml = if use_nvidia {
            match Nvml::init() {
                Ok(instance) => match instance.device_by_index(0) {
                    Ok(device) => {
                        if let Ok(name) = device.name() {
                            let short = shorten_gpu_name(&name);
                            if !short.is_empty() {
                                gpu_name = Some(short);
                            }
                        }
                        if let Ok(mem) = device.memory_info() {
                            if mem.total > 0 {
                                vram_total_bytes = Some(mem.total);
                            }
                        }
                        Some(instance)
                    }
                    Err(err) => {
                        log::info!(target: "provider:system", "Ingen NVIDIA GPU fundet via NVML: {err}");
                        None
                    }
                },
                Err(err) => {
                    log::info!(target: "provider:system", "NVML ikke tilgængelig: {err}");
                    None
                }
            }
        } else {
            None
        };

        #[cfg(not(feature = "nvidia"))]
        let _ = use_nvidia;

        #[cfg(feature = "nvidia")]
        let has_nvml = nvml.is_some();
        #[cfg(not(feature = "nvidia"))]
        let has_nvml = false;

        let mut query = PDH_HQUERY::default();
        let open_status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };

        let (
            query_opt,
            c_cpu_utility,
            c_cpu_total,
            c_cpu_fallback,
            c_cpu_per_core,
            c_gpu_engine,
            c_vram,
            c_disk_read,
            c_disk_write,
            c_disk_active,
            c_cpu_perf,
            c_cpu_freq_base,
            baseline_collected,
        ) = if open_status == ERROR_SUCCESS.0 {
            let c_cpu_utility = add_counter(
                query,
                w!("\\Processor Information(_Total)\\% Processor Utility"),
                "CPU Utility",
            );
            let c_cpu_total = add_counter(
                query,
                w!("\\Processor Information(_Total)\\% Processor Time"),
                "CPU Total",
            );
            let c_cpu_fallback = if c_cpu_utility.is_none() && c_cpu_total.is_none() {
                add_counter(
                    query,
                    w!("\\Processor(_Total)\\% Processor Time"),
                    "CPU Fallback",
                )
            } else {
                None
            };
            let c_cpu_per_core = add_counter(
                query,
                w!("\\Processor Information(*)\\% Processor Time"),
                "CPU per-kerne",
            );
            // PDH/VidSch er samme datakilde som Jobliste på tværs af NVIDIA, AMD og Intel.
            // NVML bruges kun som fallback og til temperatur, fordi NVML's utilization ikke
            // inkluderer fx video-/copy-engines på samme måde som Jobliste.
            let c_gpu_engine = add_counter(
                query,
                w!("\\GPU Engine(*)\\Utilization Percentage"),
                "GPU Engine",
            );
            let c_vram = add_counter(
                query,
                w!("\\GPU Adapter Memory(*)\\Dedicated Usage"),
                "GPU VRAM",
            );
            let c_disk_read = add_counter(
                query,
                w!("\\PhysicalDisk(_Total)\\Disk Read Bytes/sec"),
                "Disk read",
            );
            let c_disk_write = add_counter(
                query,
                w!("\\PhysicalDisk(_Total)\\Disk Write Bytes/sec"),
                "Disk write",
            );
            // Joblistens diskprocent er aktiv tid, ikke en vilkårlig hastighedsgrænse.
            let c_disk_active = add_counter(
                query,
                w!("\\PhysicalDisk(_Total)\\% Disk Time"),
                "Disk aktiv tid",
            );
            let c_cpu_perf = add_counter(
                query,
                w!("\\Processor Information(_Total)\\% Processor Performance"),
                "CPU Perf",
            );
            let c_cpu_freq_base = add_counter(
                query,
                w!("\\Processor Information(_Total)\\Processor Frequency"),
                "CPU basefrekvens",
            );

            let baseline_collected = unsafe { PdhCollectQueryData(query) } == ERROR_SUCCESS.0;

            (
                Some(query),
                c_cpu_utility,
                c_cpu_total,
                c_cpu_fallback,
                c_cpu_per_core,
                c_gpu_engine,
                c_vram,
                c_disk_read,
                c_disk_write,
                c_disk_active,
                c_cpu_perf,
                c_cpu_freq_base,
                baseline_collected,
            )
        } else {
            log::warn!(target: "provider:system", "PdhOpenQueryW fejlede med kode 0x{open_status:08x}");
            (None, None, None, None, None, None, None, None, None, None, None, None, false)
        };

        let caps = SystemCaps {
            gpu: c_gpu_engine.is_some() || has_nvml,
            vram: (c_vram.is_some() && vram_total_bytes.is_some())
                || (has_nvml && vram_total_bytes.is_some()),
            gpu_temp: has_nvml,
            cpu_temp: super::cpu_sensor::is_available(),
            disk: c_disk_read.is_some() && c_disk_write.is_some(),
            battery,
        };

        Self {
            query: query_opt,
            c_cpu_utility,
            c_cpu_total,
            cpu_utility_health: CpuUtilityHealth::default(),
            tm_time_based: task_manager_uses_time_based_cpu(),
            c_cpu_fallback,
            c_cpu_per_core,
            c_gpu_engine,
            c_vram,
            c_disk_read,
            c_disk_write,
            c_disk_active,
            c_cpu_perf,
            c_cpu_freq_base,
            baseline_collected,
            gpu_name,
            gpu_luid,
            vram_total_bytes,
            cpu_nominal_mhz: processor_nominal_frequency_mhz(),
            #[cfg(feature = "nvidia")]
            nvml,
            caps,
            array_buf: Vec::with_capacity(4 * 1024),
        }
    }

    pub fn sample(&mut self) -> GpuTickData {
        let mut out = GpuTickData {
            vram_total_bytes: self.vram_total_bytes,
            name: self.gpu_name.clone(),
            ..Default::default()
        };

        if let Some(query) = self.query {
            let status = unsafe { PdhCollectQueryData(query) };
            if status == ERROR_SUCCESS.0 {
                if !self.baseline_collected {
                    self.baseline_collected = true;
                } else {
                    let cpu_perf = self
                        .c_cpu_perf
                        .and_then(sample_scalar_counter)
                        .filter(|v| v.is_finite() && *v > 1.0 && *v < 500.0);
                    out.cpu_perf_pct = cpu_perf;

                    // MaxMhz fra power-API'et er nominel frekvens. PDH's Processor Frequency
                    // er kun fallback, fordi Microsoft dokumenterer at den kan være unøjagtig
                    // på CPU'er hvor frekvensen styres uden om Windows.
                    out.cpu_base_mhz = self.cpu_nominal_mhz.or_else(|| {
                        self.c_cpu_freq_base
                            .and_then(sample_scalar_counter)
                            .filter(|v| v.is_finite() && *v > 0.0 && *v < 20_000.0)
                            .map(|v| v.round() as u32)
                    });

                    let cpu_utility = self.c_cpu_utility.and_then(sample_scalar_counter);
                    let cpu_time = self.c_cpu_total.and_then(sample_scalar_counter);
                    let cpu_fallback = self.c_cpu_fallback.and_then(sample_scalar_counter);
                    out.cpu_usage = self.cpu_utility_health.select(
                        cpu_utility,
                        cpu_time,
                        cpu_perf,
                        cpu_fallback,
                        self.tm_time_based,
                    );

                    if let Some(counter) = self.c_cpu_per_core {
                        if let Some(raw_items) =
                            read_counter_array_items(counter, &mut self.array_buf, false)
                        {
                            let (_global, per_core) = parse_processor_info_samples(&raw_items);
                            out.cpu_per_core = per_core;
                        }
                    }

                    if let Some(counter) = self.c_gpu_engine {
                        if let Some(raw_items) =
                            read_counter_array_items(counter, &mut self.array_buf, false)
                        {
                            out.usage =
                                aggregate_gpu_engine_usage(&raw_items, self.gpu_luid.as_deref());
                        }
                    }
                    if let Some(counter) = self.c_vram {
                        if let Some(raw_items) =
                            read_counter_array_items(counter, &mut self.array_buf, false)
                        {
                            out.vram_used_bytes =
                                aggregate_vram_used(&raw_items, self.gpu_luid.as_deref());
                        }
                    }
                    if let Some(counter) = self.c_disk_read {
                        out.disk_read_bps = sample_scalar_counter(counter).unwrap_or(0.0).max(0.0);
                    }
                    if let Some(counter) = self.c_disk_write {
                        out.disk_write_bps = sample_scalar_counter(counter).unwrap_or(0.0).max(0.0);
                    }
                    if let Some(counter) = self.c_disk_active {
                        out.disk_active_pct = sample_scalar_counter(counter)
                            .filter(|v| v.is_finite() && *v >= 0.0)
                            .map(|v| v.min(100.0));
                    }
                }
            }
        }

        #[cfg(feature = "nvidia")]
        if let Some(nvml) = &self.nvml {
            if let Ok(device) = nvml.device_by_index(0) {
                if let Ok(temp) = device.temperature(TemperatureSensor::Gpu) {
                    if temp > 0 && temp < 125 {
                        out.temp_c = Some(temp as f32);
                    }
                }
                if out.usage.is_none() {
                    if let Ok(util) = device.utilization_rates() {
                        out.usage = Some((util.gpu as f32).clamp(0.0, 100.0));
                    }
                }
                if out.vram_used_bytes.is_none() {
                    if let Ok(mem) = device.memory_info() {
                        out.vram_used_bytes = Some(mem.used);
                        if out.vram_total_bytes.is_none() && mem.total > 0 {
                            out.vram_total_bytes = Some(mem.total);
                        }
                    }
                }
            }
        }

        out
    }
}

#[derive(Debug, Default)]
struct CpuUtilityHealth {
    pinned: bool,
    pinned_streak: u8,
    healthy_streak: u8,
}

impl CpuUtilityHealth {
    fn select(
        &mut self,
        utility: Option<f64>,
        time: Option<f64>,
        perf: Option<f64>,
        fallback: Option<f64>,
        prefer_time: bool,
    ) -> Option<f32> {
        let valid = |v: Option<f64>| v.filter(|x| x.is_finite() && *x >= 0.0);
        let u = valid(utility);
        let t = valid(time);
        let p = valid(perf).filter(|x| *x > 10.0);
        let f = valid(fallback);

        if prefer_time {
            return t.or(f).or(u).map(|v| (v as f32).clamp(0.0, 100.0));
        }

        let pinned_now = match (u, t, p) {
            (Some(u), Some(t), Some(p)) => {
                let tol = (p * 0.03).max(2.0);
                (u - p).abs() <= tol && t < 90.0
            }
            _ => false,
        };

        let clearly_healthy = match (u, p) {
            (Some(u), Some(p)) => u < p * 0.90,
            _ => false,
        };

        if pinned_now {
            self.healthy_streak = 0;
            self.pinned_streak = self.pinned_streak.saturating_add(1);
            if !self.pinned && self.pinned_streak >= 2 {
                self.pinned = true;
                log::info!(
                    target: "provider:system",
                    "PDH % Processor Utility følger % Processor Performance uden idle-states — skifter til % Processor Time"
                );
            }
        } else if clearly_healthy {
            self.pinned_streak = 0;
            self.healthy_streak = self.healthy_streak.saturating_add(1);
            if self.pinned && self.healthy_streak >= 2 {
                self.pinned = false;
            }
        } else {
            self.pinned_streak = 0;
            self.healthy_streak = 0;
        }

        let chosen = if self.pinned || pinned_now {
            t.or(f).or(u)
        } else {
            u.or(t).or(f)
        };
        chosen.map(|v| (v as f32).clamp(0.0, 100.0))
    }
}

fn registry_dword(value: &str) -> Option<u32> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            PCWSTR::from_raw(wide.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

fn registry_string(value: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut buf = [0u16; 32];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            PCWSTR::from_raw(wide.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS || size < 2 {
        return None;
    }
    let len = ((size as usize) / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn task_manager_time_based_for_build(build: u32, ubr: u32) -> bool {
    build > 26100 || (build == 26100 && ubr >= 4061)
}

fn task_manager_uses_time_based_cpu() -> bool {
    let build = registry_string("CurrentBuildNumber").and_then(|s| s.parse::<u32>().ok());
    let ubr = registry_dword("UBR").unwrap_or(0);
    match build {
        Some(build) => task_manager_time_based_for_build(build, ubr),
        // Ukendt build på en Windows-version vi ikke kan identificere: brug moderne,
        // kerneforbrugsbaseret semantik fremfor legacy Utility-tallet.
        None => true,
    }
}

fn processor_nominal_frequency_mhz() -> Option<u32> {
    let count = std::thread::available_parallelism().ok()?.get();
    let mut infos = vec![PROCESSOR_POWER_INFORMATION::default(); count];
    let size = (count * std::mem::size_of::<PROCESSOR_POWER_INFORMATION>()) as u32;
    let status = unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            None,
            0,
            Some(infos.as_mut_ptr() as *mut core::ffi::c_void),
            size,
        )
    };
    if !status.is_ok() {
        return None;
    }
    let values: Vec<u32> = infos
        .iter()
        .map(|info| info.MaxMhz)
        .filter(|mhz| *mhz > 0)
        .collect();
    if values.is_empty() {
        return None;
    }
    Some((values.iter().map(|v| u64::from(*v)).sum::<u64>() / values.len() as u64) as u32)
}

fn add_counter(query: PDH_HQUERY, path: PCWSTR, label: &str) -> Option<PDH_HCOUNTER> {
    let mut counter = PDH_HCOUNTER::default();
    let status = unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) };
    if status == ERROR_SUCCESS.0 {
        Some(counter)
    } else {
        log::info!(
            target: "provider:system",
            "PDH-tæller '{label}' ikke tilgængelig (0x{status:08x})"
        );
        None
    }
}

fn sample_scalar_counter(counter: PDH_HCOUNTER) -> Option<f64> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let status =
        unsafe { PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut value) };
    if status == ERROR_SUCCESS.0 && (value.CStatus & PDH_CSTATUS_ERROR_MASK) == 0 {
        let double_val = unsafe { value.Anonymous.doubleValue };
        if double_val.is_finite() {
            return Some(double_val);
        }
    }
    None
}

fn read_counter_array_items(
    counter: PDH_HCOUNTER,
    buf: &mut Vec<u64>,
    skip_zero: bool,
) -> Option<Vec<(String, f64)>> {
    let mut buffer_size: u32 = 0;
    let mut item_count: u32 = 0;
    let status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buffer_size,
            &mut item_count,
            None,
        )
    };
    if status != PDH_MORE_DATA || buffer_size == 0 || item_count == 0 {
        return None;
    }

    let words = (buffer_size as usize).div_ceil(std::mem::size_of::<u64>());
    if buf.len() < words {
        buf.resize(words, 0);
    }

    let ptr = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
    let status2 = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buffer_size,
            &mut item_count,
            Some(ptr),
        )
    };
    if status2 != ERROR_SUCCESS.0 {
        return None;
    }

    let items = unsafe { std::slice::from_raw_parts(ptr, item_count as usize) };
    let mut out = Vec::with_capacity(if skip_zero { 16 } else { items.len() });
    for item in items {
        if (item.FmtValue.CStatus & PDH_CSTATUS_ERROR_MASK) != 0 {
            continue;
        }
        let value = unsafe { item.FmtValue.Anonymous.doubleValue };
        if !value.is_finite() || (skip_zero && value <= 0.0) {
            continue;
        }
        let name = unsafe { item.szName.to_string().unwrap_or_default() };
        out.push((name, value));
    }
    Some(out)
}

pub fn parse_processor_info_samples(items: &[(String, f64)]) -> (Option<f32>, Vec<f32>) {
    let mut global: Option<f32> = None;
    let mut cores: Vec<((u32, u32), f32)> = Vec::new();

    for (name, val) in items {
        if !val.is_finite() || *val < 0.0 {
            continue;
        }
        let clamped = (*val as f32).clamp(0.0, 100.0);
        let trimmed = name.trim();
        if trimmed == "_Total" {
            global = Some(clamped);
            continue;
        }
        if let Some((grp_s, core_s)) = trimmed.split_once(',') {
            if let (Ok(grp), Ok(core)) = (grp_s.trim().parse::<u32>(), core_s.trim().parse::<u32>()) {
                cores.push(((grp, core), clamped));
            }
        }
    }

    cores.sort_by_key(|(idx, _)| *idx);
    let per_core: Vec<f32> = cores.into_iter().map(|(_, v)| v).collect();

    if global.is_none() && !per_core.is_empty() {
        let sum: f32 = per_core.iter().sum();
        global = Some((sum / per_core.len() as f32).clamp(0.0, 100.0));
    }

    (global, per_core)
}

pub fn format_luid_key(high: i32, low: u32) -> String {
    format!("luid_0x{:08x}_0x{:08x}", high as u32, low)
}

pub fn select_luid_metric(
    per_adapter: &HashMap<String, f64>,
    preferred_luid: Option<&str>,
) -> Option<f64> {
    if let Some(pref) = preferred_luid {
        let needle = pref.to_ascii_lowercase();
        if let Some(&val) = per_adapter.get(&needle) {
            return Some(val);
        }
    }
    per_adapter
        .values()
        .copied()
        .fold(None, |acc: Option<f64>, val| {
            Some(match acc {
                Some(prev) => prev.max(val),
                None => val,
            })
        })
}

pub fn compute_effective_freq_mhz(base_mhz: Option<u32>, perf_pct: Option<f64>) -> Option<u32> {
    match (base_mhz, perf_pct) {
        (Some(base), Some(pct)) if base > 0 && pct.is_finite() && pct > 0.0 => {
            Some(((base as f64) * (pct / 100.0)).round().clamp(100.0, 20_000.0) as u32)
        }
        _ => base_mhz,
    }
}

pub fn extract_gpu_luid_and_engine(name: &str) -> Option<(String, String)> {
    let lower = name.to_ascii_lowercase();
    let start = lower.find("luid_")?;
    let tail = &lower[start..];
    if let Some(phys) = tail.find("_phys_") {
        return Some((tail[..phys].to_string(), tail[phys + 1..].to_string()));
    }
    let after_prefix = &tail[5..];
    let mut underscore_iter = after_prefix.match_indices('_').map(|(i, _)| i);
    let _first_sep = underscore_iter.next();
    if let Some(second_sep) = underscore_iter.next() {
        let split_at = 5 + second_sep;
        Some((tail[..split_at].to_string(), tail[split_at + 1..].to_string()))
    } else {
        Some((tail.to_string(), "default".to_string()))
    }
}

pub fn aggregate_gpu_engine_usage(
    items: &[(String, f64)],
    preferred_luid: Option<&str>,
) -> Option<f32> {
    let mut per_engine: HashMap<(String, String), f64> = HashMap::new();
    for (instance_name, value) in items {
        if !value.is_finite() || *value < 0.0 {
            continue;
        }
        let (luid, engine) = extract_gpu_luid_and_engine(instance_name)
            .unwrap_or_else(|| ("default".to_string(), "default".to_string()));
        *per_engine.entry((luid, engine)).or_insert(0.0) += *value;
    }

    let mut per_adapter: HashMap<String, f64> = HashMap::new();
    for ((luid, _engine), engine_sum) in per_engine {
        per_adapter
            .entry(luid)
            .and_modify(|prev| *prev = prev.max(engine_sum))
            .or_insert(engine_sum);
    }

    select_luid_metric(&per_adapter, preferred_luid).map(|v| (v as f32).clamp(0.0, 100.0))
}

pub fn aggregate_vram_used(
    items: &[(String, f64)],
    preferred_luid: Option<&str>,
) -> Option<u64> {
    let mut per_adapter: HashMap<String, f64> = HashMap::new();
    for (instance_name, value) in items {
        if !value.is_finite() || *value < 0.0 {
            continue;
        }
        let luid = extract_luid_key(instance_name)
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "default".to_owned());
        *per_adapter.entry(luid).or_insert(0.0) += *value;
    }

    select_luid_metric(&per_adapter, preferred_luid).map(|v| v.round() as u64)
}

fn extract_luid_key(name: &str) -> Option<&str> {
    let start = name.find("luid_")?;
    let tail = &name[start..];
    if let Some(phys) = tail.find("_phys_") {
        Some(&tail[..phys])
    } else {
        Some(tail)
    }
}

fn detect_dxgi_adapter() -> (Option<String>, Option<u64>, Option<String>, Option<u32>) {
    let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
        Ok(f) => f,
        Err(err) => {
            log::info!(target: "provider:system", "CreateDXGIFactory1 fejlede: {err}");
            return (None, None, None, None);
        }
    };

    let mut best: Option<(String, u64, String, u32)> = None;
    let mut index = 0;
    loop {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(_) => break,
        };
        index += 1;

        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
            continue;
        }
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let raw_name = String::from_utf16_lossy(&desc.Description[..end]);
        let name = shorten_gpu_name(raw_name.trim());
        let vram = desc.DedicatedVideoMemory as u64;
        let luid = format_luid_key(desc.AdapterLuid.HighPart, desc.AdapterLuid.LowPart);

        let replace = match &best {
            Some((_, best_vram, _, _)) => vram > *best_vram,
            None => true,
        };
        if replace {
            best = Some((name, vram, luid, desc.VendorId));
        }
    }

    match best {
        Some((name, vram, luid, vendor_id)) => (
            (!name.is_empty()).then_some(name),
            (vram > 0).then_some(vram),
            Some(luid),
            Some(vendor_id),
        ),
        None => (None, None, None, None),
    }
}

pub fn shorten_gpu_name(raw: &str) -> String {
    let trimmed = raw.trim();
    for prefix in [
        "NVIDIA GeForce ",
        "NVIDIA ",
        "AMD Radeon(TM) ",
        "AMD Radeon ",
        "Intel(R) Iris(R) ",
        "Intel(R) UHD ",
        "Intel(R) Arc(TM) ",
        "Intel(R) ",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    trimmed.to_string()
}

pub fn detect_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_ok() {
        status.BatteryFlag != 128 && status.BatteryFlag != 255
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{
        aggregate_gpu_engine_usage, aggregate_vram_used, compute_effective_freq_mhz,
        extract_gpu_luid_and_engine, extract_luid_key, format_luid_key,
        parse_processor_info_samples, select_luid_metric, shorten_gpu_name,
    };
    use std::collections::HashMap;

    #[test]
    fn extracts_luid_from_pdh_gpu_engine_instance() {
        let name = "pid_9940_luid_0x00000000_0x00010D9A_phys_0_eng_0_engtype_3D";
        assert_eq!(
            extract_luid_key(name),
            Some("luid_0x00000000_0x00010D9A")
        );
        assert_eq!(
            extract_gpu_luid_and_engine(name),
            Some((
                "luid_0x00000000_0x00010d9a".to_string(),
                "phys_0_eng_0_engtype_3d".to_string()
            ))
        );
    }

    #[test]
    fn parses_processor_information_total_and_sorted_cores() {
        let items = vec![
            ("0,10".to_string(), 35.25),
            ("0,1".to_string(), 4.5),
            ("0,_Total".to_string(), 19.0),
            ("0,0".to_string(), 7.5),
            ("_Total".to_string(), 15.75),
            ("0,2".to_string(), 1.5),
        ];
        let (global, per_core) = parse_processor_info_samples(&items);
        assert_eq!(global, Some(15.75));
        assert_eq!(per_core, vec![7.5, 4.5, 1.5, 35.25]);

        let no_total = vec![("0,0".to_string(), 10.0), ("1,0".to_string(), 30.0)];
        let (fallback_global, sorted) = parse_processor_info_samples(&no_total);
        assert_eq!(fallback_global, Some(20.0));
        assert_eq!(sorted, vec![10.0, 30.0]);
    }

    #[test]
    fn aggregates_gpu_engine_usage_like_task_manager_across_engines_and_pids() {
        let items = vec![
            // Primary adapter: 3D engine shared by two PIDs (12 + 8 = 20%), Copy engine (5%), VideoDecode (14%)
            (
                "pid_100_luid_0x00000000_0x041510EA_phys_0_eng_0_engtype_3D".to_string(),
                12.0,
            ),
            (
                "pid_200_luid_0x00000000_0x041510EA_phys_0_eng_0_engtype_3D".to_string(),
                8.0,
            ),
            (
                "pid_100_luid_0x00000000_0x041510EA_phys_0_eng_1_engtype_Copy".to_string(),
                5.0,
            ),
            (
                "pid_300_luid_0x00000000_0x041510EA_phys_0_eng_2_engtype_VideoDecode".to_string(),
                14.0,
            ),
            // Secondary iGPU: higher load (55%), must not override preferred NVIDIA LUID
            (
                "pid_400_luid_0x00000000_0x0000E7F0_phys_0_eng_0_engtype_3D".to_string(),
                55.0,
            ),
        ];
        assert_eq!(
            aggregate_gpu_engine_usage(&items, Some("luid_0x00000000_0x041510ea")),
            Some(20.0)
        );
        // Without preference, falls back to busiest adapter
        assert_eq!(aggregate_gpu_engine_usage(&items, None), Some(55.0));
    }

    #[test]
    fn aggregates_vram_used_per_luid_and_selects_preferred() {
        let items = vec![
            ("luid_0x00000000_0x041510EA_phys_0".to_string(), 600_000_000.0),
            ("luid_0x00000000_0x041510EA_phys_1".to_string(), 40_000_000.0),
            ("luid_0x00000000_0x0000E7F0_phys_0".to_string(), 128_000_000.0),
        ];
        assert_eq!(
            aggregate_vram_used(&items, Some("luid_0x00000000_0x041510ea")),
            Some(640_000_000)
        );
    }

    #[test]
    fn formats_dxgi_luid_key() {
        assert_eq!(format_luid_key(0, 0x10d9a), "luid_0x00000000_0x00010d9a");
        assert_eq!(format_luid_key(-1, 42), "luid_0xffffffff_0x0000002a");
    }

    #[test]
    fn selects_preferred_luid_or_falls_back_to_max() {
        let mut map = HashMap::new();
        map.insert("luid_0x00000000_0x00010d9a".to_string(), 14.0);
        map.insert("luid_0x00000000_0x00020000".to_string(), 62.0);
        assert_eq!(
            select_luid_metric(&map, Some("luid_0x00000000_0x00010D9A")),
            Some(14.0)
        );
        assert_eq!(
            select_luid_metric(&map, Some("luid_0x00000000_0x99999999")),
            Some(62.0)
        );
        assert_eq!(select_luid_metric(&map, None), Some(62.0));
    }

    #[test]
    fn computes_effective_cpu_frequency_from_perf_pct() {
        assert_eq!(
            compute_effective_freq_mhz(Some(3800), Some(115.0)),
            Some(4370)
        );
        assert_eq!(compute_effective_freq_mhz(Some(3800), None), Some(3800));
        assert_eq!(compute_effective_freq_mhz(None, Some(120.0)), None);
    }

    #[test]
    fn shortens_gpu_vendor_prefixes() {
        assert_eq!(shorten_gpu_name("NVIDIA GeForce RTX 4070"), "RTX 4070");
        assert_eq!(shorten_gpu_name("AMD Radeon RX 7800 XT"), "RX 7800 XT");
        assert_eq!(shorten_gpu_name("Intel(R) Arc A770"), "Arc A770");
    }

    #[test]
    fn cpu_utility_health_uses_utility_when_cstates_work_and_time_when_pinned() {
        let mut normal = super::CpuUtilityHealth::default();
        // Normal PC (C-states ON): Utility tracks actual work (48%) even when Time under-reports (18%)
        assert_eq!(
            normal.select(Some(48.0), Some(18.0), Some(120.0), None, false),
            Some(48.0)
        );
        // Turbo-boosted heavy load (Utility 117%, Perf 180%, Time 65%): not pinned, clamped to 100
        assert_eq!(
            normal.select(Some(117.0), Some(65.0), Some(180.0), None, false),
            Some(100.0)
        );
        // Genuine 100% all-core load (Time 99.5%, Utility == Perf 118%): not flagged as pinned
        assert_eq!(
            normal.select(Some(118.0), Some(99.5), Some(118.0), None, false),
            Some(100.0)
        );
        assert!(!normal.pinned);

        // C-states disabled PC (like Ryzen with C1=C2=C3=0): Utility == Perf (113.8%) while Time is 22.6%
        let mut no_cstates = super::CpuUtilityHealth::default();
        assert_eq!(
            no_cstates.select(Some(113.8), Some(22.6), Some(113.8), None, false),
            Some(22.6)
        );
        assert_eq!(
            no_cstates.select(Some(113.4), Some(26.8), Some(113.4), None, false),
            Some(26.8)
        );
        assert!(no_cstates.pinned);
        // Stays on Time even when real load rises to 95%
        assert_eq!(
            no_cstates.select(Some(110.0), Some(95.0), Some(110.0), None, false),
            Some(95.0)
        );
    }

    #[test]
    fn newer_windows_always_prefers_time_based_task_manager_cpu() {
        let mut modern = super::CpuUtilityHealth::default();
        assert_eq!(
            modern.select(Some(112.0), Some(39.0), Some(112.0), None, true),
            Some(39.0)
        );
        assert!(!modern.pinned);
        assert!(super::task_manager_time_based_for_build(26100, 4061));
        assert!(super::task_manager_time_based_for_build(26200, 0));
        assert!(!super::task_manager_time_based_for_build(26100, 4060));
        assert!(!super::task_manager_time_based_for_build(22631, 5335));
        assert!(!super::task_manager_time_based_for_build(19045, 6000));
    }
}
