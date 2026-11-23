use reqwest::Client;
use serde::Deserialize;

use crate::config::schema::Location;

use super::{
    format_civil_date, unix_now_ms, WeatherCurrent, WeatherDaily, WeatherHourly, WeatherSnapshot,
};

pub async fn fetch_snapshot(
    client: &Client,
    location: &Location,
    location_id: &str,
) -> Result<WeatherSnapshot, String> {
    let lat = format!("{:.5}", location.lat);
    let lon = format!("{:.5}", location.lon);
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,apparent_temperature,relative_humidity_2m,is_day,precipitation,weather_code,wind_speed_10m,wind_gusts_10m,wind_direction_10m&hourly=temperature_2m,weather_code,precipitation_probability,is_day&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset&timezone=auto&forecast_days=7&wind_speed_unit=kmh&timeformat=unixtime"
    );

    let response = client.get(&url).send().await.map_err(|err| {
        log::warn!(target: "provider:weather", "Netværksfejl for {}: {err}", location.name);
        "network".to_string()
    })?;

    if !response.status().is_success() {
        log::warn!(
            target: "provider:weather",
            "HTTP {} fra Open-Meteo for {}",
            response.status(),
            location.name
        );
        return Err(format!("http_{}", response.status().as_u16()));
    }

    let raw: OpenMeteoResponse = response.json().await.map_err(|err| {
        log::warn!(target: "provider:weather", "Parse-fejl for {}: {err}", location.name);
        "parse".to_string()
    })?;

    Ok(build_snapshot(location_id, raw))
}

fn build_snapshot(location_id: &str, raw: OpenMeteoResponse) -> WeatherSnapshot {
    let now_ms = unix_now_ms();
    let now_s = now_ms / 1000;
    let offset_s = raw.utc_offset_seconds.unwrap_or(0);

    let current = raw.current.map(|c| WeatherCurrent {
        temp_c: c.temperature_2m,
        feels_c: c.apparent_temperature,
        code: c.weather_code,
        is_day: c.is_day != 0,
        wind_kmh: c.wind_speed_10m,
        gust_kmh: c.wind_gusts_10m,
        wind_dir: c.wind_direction_10m,
        humidity: c.relative_humidity_2m,
        precip_mm: c.precipitation,
    });

    let mut hourly = Vec::with_capacity(24);
    if let Some(h) = raw.hourly {
        let len = h.time.len();
        let start_idx = h
            .time
            .iter()
            .position(|&t| t >= now_s - 3600)
            .unwrap_or(0);
        for i in start_idx..len {
            if hourly.len() >= 24 {
                break;
            }
            hourly.push(WeatherHourly {
                t: h.time[i] * 1000,
                temp_c: h.temperature_2m.get(i).copied().unwrap_or(0.0),
                code: h.weather_code.get(i).copied().unwrap_or(0),
                precip_prob: h
                    .precipitation_probability
                    .as_ref()
                    .and_then(|v| v.get(i).copied())
                    .flatten()
                    .unwrap_or(0.0),
                is_day: h
                    .is_day
                    .as_ref()
                    .and_then(|v| v.get(i).copied())
                    .map(|v| v != 0)
                    .unwrap_or(true),
            });
        }
    }

    let mut daily = Vec::with_capacity(7);
    if let Some(d) = raw.daily {
        for i in 0..d.time.len().min(7) {
            let day_date = format_civil_date(d.time[i] + offset_s);
            daily.push(WeatherDaily {
                date: day_date,
                min_c: d.temperature_2m_min.get(i).copied().unwrap_or(0.0),
                max_c: d.temperature_2m_max.get(i).copied().unwrap_or(0.0),
                code: d.weather_code.get(i).copied().unwrap_or(0),
                precip_prob: d
                    .precipitation_probability_max
                    .as_ref()
                    .and_then(|v| v.get(i).copied())
                    .flatten()
                    .unwrap_or(0.0),
                sunrise: d.sunrise.get(i).copied().unwrap_or(0) * 1000,
                sunset: d.sunset.get(i).copied().unwrap_or(0) * 1000,
            });
        }
    }

    WeatherSnapshot {
        location_id: location_id.to_string(),
        fetched_at: now_ms,
        stale: false,
        error: None,
        source: "Open-Meteo".to_string(),
        current,
        hourly,
        daily,
    }
}

#[derive(Deserialize)]
struct OpenMeteoResponse {
    utc_offset_seconds: Option<i64>,
    current: Option<OpenMeteoCurrent>,
    hourly: Option<OpenMeteoHourly>,
    daily: Option<OpenMeteoDaily>,
}

#[derive(Deserialize)]
struct OpenMeteoCurrent {
    temperature_2m: f64,
    apparent_temperature: f64,
    relative_humidity_2m: f64,
    is_day: i32,
    precipitation: f64,
    weather_code: u16,
    wind_speed_10m: f64,
    wind_gusts_10m: f64,
    wind_direction_10m: f64,
}

#[derive(Deserialize)]
struct OpenMeteoHourly {
    time: Vec<i64>,
    temperature_2m: Vec<f64>,
    weather_code: Vec<u16>,
    precipitation_probability: Option<Vec<Option<f64>>>,
    is_day: Option<Vec<i32>>,
}

#[derive(Deserialize)]
struct OpenMeteoDaily {
    time: Vec<i64>,
    weather_code: Vec<u16>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    precipitation_probability_max: Option<Vec<Option<f64>>>,
    sunrise: Vec<i64>,
    sunset: Vec<i64>,
}
