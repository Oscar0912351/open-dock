use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri::{async_runtime::JoinHandle, AppHandle, Manager};
use tokio::sync::Notify;
use ts_rs::TS;
use uuid::Uuid;

use crate::{error::AppError, state::AppState, util::unix_now_ms};

mod metno;
mod openmeteo;

// api.met.no kraever en identificerbar User-Agent med projekt/kontakt-info.
const USER_AGENT: &str = "Open-Dock/0.1 https://github.com/Oscar0912351/open-dock";
const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const STALE_AFTER_MS: i64 = 45 * 60 * 1000;
const RESUME_REFRESH_MS: i64 = 15 * 60 * 1000;
const BACKOFF_SECS: [u64; 4] = [30, 120, 300, 900];

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WeatherCurrent {
    pub temp_c: f64,
    pub feels_c: f64,
    pub code: u16,
    pub is_day: bool,
    pub wind_kmh: f64,
    pub gust_kmh: f64,
    pub wind_dir: f64,
    pub humidity: f64,
    pub precip_mm: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WeatherHourly {
    pub t: i64,
    pub temp_c: f64,
    pub code: u16,
    pub precip_prob: f64,
    pub is_day: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WeatherDaily {
    pub date: String,
    pub min_c: f64,
    pub max_c: f64,
    pub code: u16,
    pub precip_prob: f64,
    pub sunrise: i64,
    pub sunset: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WeatherSnapshot {
    pub location_id: String,
    pub fetched_at: i64,
    pub stale: bool,
    pub error: Option<String>,
    pub current: Option<WeatherCurrent>,
    pub hourly: Vec<WeatherHourly>,
    pub daily: Vec<WeatherDaily>,
    /// Kilde-attribution vist i widgetten ("Yr.no" / "Open-Meteo").
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct GeoResult {
    pub name: String,
    pub admin1: Option<String>,
    pub country: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: String,
}

struct LocationWorker {
    task: JoinHandle<()>,
    refresh_now: Arc<Notify>,
}

pub struct WeatherProvider {
    client: Client,
    workers: Mutex<HashMap<String, LocationWorker>>,
    snapshots: Arc<Mutex<HashMap<String, WeatherSnapshot>>>,
}

impl Default for WeatherProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl WeatherProvider {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            workers: Mutex::new(HashMap::new()),
            snapshots: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn snapshot(&self, location_id: &str) -> Option<WeatherSnapshot> {
        let mut guard = self.snapshots.lock();
        if let Some(snap) = guard.get_mut(location_id) {
            if snap.current.is_some() && snap.fetched_at > 0 {
                snap.stale = unix_now_ms() - snap.fetched_at > STALE_AFTER_MS;
            }
            return Some(snap.clone());
        }
        None
    }

    pub fn start_location(&self, app: &AppHandle, location_id: &str) {
        if location_id.is_empty() {
            return;
        }
        let mut guard = self.workers.lock();
        if guard.contains_key(location_id) {
            return;
        }

        let cache_dir = cache_dir_for(app);
        if let Some(mut cached) = read_cache(&cache_dir, location_id) {
            cached.stale = unix_now_ms() - cached.fetched_at > STALE_AFTER_MS;
            self.snapshots
                .lock()
                .insert(location_id.to_string(), cached.clone());
            super::scheduler::emit_provider(
                app,
                super::scheduler::ProviderKind::Weather,
                Some(location_id),
                "provider://weather",
                &cached,
            );
        }

        let app_handle = app.clone();
        let client = self.client.clone();
        let loc_id = location_id.to_string();
        let snapshots = Arc::clone(&self.snapshots);
        let refresh_now = Arc::new(Notify::new());
        let refresh_rx = Arc::clone(&refresh_now);

        log::info!(target: "provider:weather", "Starter vejr-provider for {loc_id}");

        let task = tauri::async_runtime::spawn(async move {
            let mut backoff_idx: usize = 0;

            loop {
                let wait_dur = match fetch_and_emit(
                    &app_handle,
                    &client,
                    &cache_dir,
                    &loc_id,
                    &snapshots,
                )
                .await
                {
                    Ok(()) => {
                        backoff_idx = 0;
                        REFRESH_INTERVAL
                    }
                    Err(code) => {
                        let delay = BACKOFF_SECS[backoff_idx.min(BACKOFF_SECS.len() - 1)];
                        backoff_idx = (backoff_idx + 1).min(BACKOFF_SECS.len() - 1);
                        emit_error_or_stale(&app_handle, &loc_id, code, &snapshots);
                        Duration::from_secs(delay)
                    }
                };

                tokio::select! {
                    _ = tokio::time::sleep(wait_dur) => {},
                    _ = refresh_rx.notified() => {},
                }
            }
        });

        guard.insert(
            location_id.to_string(),
            LocationWorker { task, refresh_now },
        );
    }

    pub fn stop_location(&self, location_id: &str) {
        if let Some(worker) = self.workers.lock().remove(location_id) {
            log::info!(target: "provider:weather", "Stopper vejr-provider for {location_id}");
            worker.task.abort();
        }
    }

    pub fn stop_all(&self) {
        let workers: Vec<(String, LocationWorker)> = self.workers.lock().drain().collect();
        for (id, worker) in workers {
            log::info!(target: "provider:weather", "Stopper vejr-provider for {id}");
            worker.task.abort();
        }
    }

    pub fn refresh_location(&self, location_id: &str) {
        if let Some(worker) = self.workers.lock().get(location_id) {
            worker.refresh_now.notify_one();
        }
    }

    pub fn resume_check(&self) {
        let now = unix_now_ms();
        let snapshots = self.snapshots.lock();
        let workers = self.workers.lock();
        for (id, worker) in workers.iter() {
            let should_refresh = match snapshots.get(id) {
                Some(snap) => now - snap.fetched_at > RESUME_REFRESH_MS,
                None => true,
            };
            if should_refresh {
                worker.refresh_now.notify_one();
            }
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<GeoResult>, AppError> {
        let trimmed = query.trim();
        if trimmed.chars().count() < 2 {
            return Ok(Vec::new());
        }

        let encoded = percent_encode_query(trimmed);
        let url = format!(
            "https://geocoding-api.open-meteo.com/v1/search?name={encoded}&count=8&language=da&format=json"
        );
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(AppError::Network)?;

        let payload: GeoResponse = response.json().await.map_err(AppError::Network)?;
        Ok(payload
            .results
            .unwrap_or_default()
            .into_iter()
            .map(|r| GeoResult {
                name: r.name,
                admin1: r.admin1,
                country: r.country.unwrap_or_default(),
                latitude: r.latitude,
                longitude: r.longitude,
                timezone: r.timezone.unwrap_or_else(|| "auto".into()),
            })
            .collect())
    }
}

async fn fetch_and_emit(
    app: &AppHandle,
    client: &Client,
    cache_dir: &Path,
    location_id: &str,
    snapshots: &Arc<Mutex<HashMap<String, WeatherSnapshot>>>,
) -> Result<(), String> {
    let target_uuid = Uuid::parse_str(location_id).map_err(|_| "invalid_location".to_string())?;
    let location = app
        .state::<AppState>()
        .config
        .read_with(|cfg| {
            cfg.weather
                .locations
                .iter()
                .find(|loc| loc.id == target_uuid)
                .cloned()
        })
        .ok_or_else(|| "missing_location".to_string())?;

    // Yr.no (MET Norway) er primaerkilde — bedre data i Norden og
    // egentlige nedboerssandsynligheder. Open-Meteo er fallback hvis
    // MET-svaret mangler eller fejler.
    let snapshot = match metno::fetch_snapshot(client, &location, location_id).await {
        Ok(snapshot) => snapshot,
        Err(code) => {
            log::info!(
                target: "provider:weather",
                "MET Norway fejlede for {} ({code}) — proever Open-Meteo",
                location.name
            );
            openmeteo::fetch_snapshot(client, &location, location_id).await?
        }
    };
    write_cache(cache_dir, location_id, &snapshot);
    snapshots
        .lock()
        .insert(location_id.to_string(), snapshot.clone());
    super::scheduler::emit_provider(
        app,
        super::scheduler::ProviderKind::Weather,
        Some(location_id),
        "provider://weather",
        &snapshot,
    );
    Ok(())
}

fn emit_error_or_stale(
    app: &AppHandle,
    location_id: &str,
    code: String,
    snapshots: &Arc<Mutex<HashMap<String, WeatherSnapshot>>>,
) {
    let payload = {
        let mut guard = snapshots.lock();
        if let Some(cached) = guard.get_mut(location_id) {
            if cached.current.is_some() {
                cached.stale = true;
                cached.error = Some(code);
                cached.clone()
            } else {
                let err_snap = error_snapshot(location_id, code);
                *cached = err_snap.clone();
                err_snap
            }
        } else {
            let err_snap = error_snapshot(location_id, code);
            guard.insert(location_id.to_string(), err_snap.clone());
            err_snap
        }
    };
    super::scheduler::emit_provider(
        app,
        super::scheduler::ProviderKind::Weather,
        Some(location_id),
        "provider://weather",
        &payload,
    );
}

fn error_snapshot(location_id: &str, code: String) -> WeatherSnapshot {
    WeatherSnapshot {
        location_id: location_id.to_string(),
        fetched_at: 0,
        stale: true,
        error: Some(code),
        current: None,
        hourly: Vec::new(),
        daily: Vec::new(),
        source: String::new(),
    }
}

fn cache_dir_for(_app: &AppHandle) -> PathBuf {
    crate::paths::data_root()
}

fn cache_path(cache_dir: &Path, location_id: &str) -> PathBuf {
    cache_dir.join(format!("weather-{location_id}.json"))
}

fn read_cache(cache_dir: &Path, location_id: &str) -> Option<WeatherSnapshot> {
    let path = cache_path(cache_dir, location_id);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(cache_dir: &Path, location_id: &str, snapshot: &WeatherSnapshot) {
    let _ = fs::create_dir_all(cache_dir);
    let path = cache_path(cache_dir, location_id);
    let tmp = cache_dir.join(format!("weather-{location_id}.json.tmp"));
    if let Ok(json) = serde_json::to_string(snapshot) {
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(tmp, path);
        }
    }
}

/// Howard Hinnant's civil-from-days algorithm in pure Rust (avoids pulling chrono).
pub fn format_civil_date(local_unix_secs: i64) -> String {
    let z = local_unix_secs.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}")
}

fn percent_encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}


#[derive(Deserialize)]
struct GeoResponse {
    results: Option<Vec<GeoItem>>,
}

#[derive(Deserialize)]
struct GeoItem {
    name: String,
    admin1: Option<String>,
    country: Option<String>,
    latitude: f64,
    longitude: f64,
    timezone: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::format_civil_date;

    #[test]
    fn converts_unix_timestamp_to_civil_date() {
        // 2026-11-17 00:00:00 UTC -> let's check known epoch 2024-01-01 = 1704067200
        assert_eq!(format_civil_date(1_704_067_200), "2024-01-01");
        assert_eq!(format_civil_date(1_709_164_800), "2024-02-29");
    }
}
