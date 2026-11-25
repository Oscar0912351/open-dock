//! MET Norway Locationforecast (yr.no's datakilde) som primaer vejrkilde.
//!
//! Felter der mangler i MET-svaret udfyldes aerligt: `sunrise`/`sunset`
//! beregnes lokalt med NOAA's solformel, og manglende symboler afledes af
//! skydaekke + nedboersmaengde. `apparent_air_temperature` og
//! `probability_of_precipitation` kommer direkte fra `/complete`-varianten.

use std::collections::BTreeMap;
use std::str::FromStr;

use jiff::{civil::Date, tz::TimeZone, Timestamp};
use reqwest::Client;
use serde::Deserialize;

use crate::config::schema::Location;

use super::{unix_now_ms, WeatherCurrent, WeatherDaily, WeatherHourly, WeatherSnapshot};

const ZENITH: f64 = 90.833;

pub async fn fetch_snapshot(
    client: &Client,
    location: &Location,
    location_id: &str,
) -> Result<WeatherSnapshot, String> {
    // api.met.no kraever hoejst 4 decimaler paa koordinater.
    let url = format!(
        "https://api.met.no/weatherapi/locationforecast/2.0/complete?lat={:.4}&lon={:.4}",
        location.lat, location.lon
    );

    let response = client.get(&url).send().await.map_err(|err| {
        log::warn!(target: "provider:weather", "MET Norway netvaerksfejl for {}: {err}", location.name);
        "network".to_string()
    })?;

    if !response.status().is_success() {
        log::warn!(
            target: "provider:weather",
            "HTTP {} fra MET Norway for {}",
            response.status(),
            location.name
        );
        return Err(format!("http_{}", response.status().as_u16()));
    }

    let raw: MetResponse = response.json().await.map_err(|err| {
        log::warn!(target: "provider:weather", "MET Norway parse-fejl for {}: {err}", location.name);
        "parse".to_string()
    })?;

    build_snapshot(location, location_id, raw)
}

fn build_snapshot(
    location: &Location,
    location_id: &str,
    raw: MetResponse,
) -> Result<WeatherSnapshot, String> {
    let now_ms = unix_now_ms();
    let tz = TimeZone::get(&location.tz).unwrap_or(TimeZone::UTC);

    let mut entries = Vec::with_capacity(raw.properties.timeseries.len());
    for item in &raw.properties.timeseries {
        if let Ok(ts) = Timestamp::from_str(&item.time) {
            entries.push(ParsedEntry {
                ts_ms: ts.as_millisecond(),
                item,
                tz: tz.clone(),
            });
        }
    }
    if entries.is_empty() {
        log::warn!(target: "provider:weather", "Tom timeseries fra MET Norway for {}", location.name);
        return Err("empty".to_string());
    }

    let current = entries.first().and_then(|e| e.to_current(location));

    let mut hourly = Vec::with_capacity(24);
    for entry in &entries {
        if entry.ts_ms < now_ms - 3_600_000 {
            continue;
        }
        if hourly.len() >= 24 {
            break;
        }
        hourly.push(entry.to_hourly(location));
    }

    let daily = build_daily(&entries, location, &tz);

    Ok(WeatherSnapshot {
        location_id: location_id.to_string(),
        fetched_at: now_ms,
        stale: false,
        error: None,
        source: "Yr.no".to_string(),
        current,
        hourly,
        daily,
    })
}

struct ParsedEntry<'a> {
    ts_ms: i64,
    item: &'a MetEntry,
    tz: TimeZone,
}

impl ParsedEntry<'_> {
    fn instant(&self) -> MetInstantDetails {
        self.item
            .data
            .instant
            .as_ref()
            .and_then(|i| i.details.clone())
            .unwrap_or_default()
    }

    /// Foretraekker det korteste gyldige interval (1t > 6t > 12t) for
    /// symbol og nedboersdata, saa timevisningen er saa lokal som mulig.
    fn period(&self) -> Option<&MetPeriod> {
        self.item
            .data
            .next_1_hours
            .as_ref()
            .or(self.item.data.next_6_hours.as_ref())
            .or(self.item.data.next_12_hours.as_ref())
    }

    fn symbol(&self) -> Option<&str> {
        self.period()
            .and_then(|p| p.summary.as_ref())
            .and_then(|s| s.symbol_code.as_deref())
    }

    fn code(&self) -> u16 {
        if let Some(symbol) = self.symbol() {
            return symbol_to_wmo(symbol);
        }
        let details = self.instant();
        if details.precipitation_amount.unwrap_or(0.0) > 0.0 {
            return if details.air_temperature.unwrap_or(0.0) <= 0.0 {
                71
            } else {
                61
            };
        }
        cloud_to_wmo(details.cloud_area_fraction)
    }

    fn precip_prob(&self) -> f64 {
        self.period()
            .and_then(|p| p.details.as_ref())
            .and_then(|d| d.probability_of_precipitation)
            .unwrap_or(0.0)
    }

    fn is_day(&self, location: &Location) -> bool {
        match self.symbol().and_then(symbol_phase) {
            Some(is_day) => is_day,
            None => is_day_at(location.lat, location.lon, self.ts_ms, self.local_date()),
        }
    }

    fn local_date(&self) -> Option<Date> {
        Timestamp::from_millisecond(self.ts_ms)
            .ok()
            .map(|ts| ts.to_zoned(self.tz.clone()).date())
    }

    fn to_current(&self, location: &Location) -> Option<WeatherCurrent> {
        let details = self.instant();
        let temp_c = details.air_temperature?;
        let wind_ms = details.wind_speed.unwrap_or(0.0);
        Some(WeatherCurrent {
            temp_c,
            feels_c: details.apparent_air_temperature.unwrap_or(temp_c),
            code: self.code(),
            is_day: self.is_day(location),
            wind_kmh: wind_ms * 3.6,
            gust_kmh: details.wind_speed_of_gust.unwrap_or(wind_ms) * 3.6,
            wind_dir: details.wind_from_direction.unwrap_or(0.0),
            humidity: details.relative_humidity.unwrap_or(0.0),
            precip_mm: self
                .period()
                .and_then(|p| p.details.as_ref())
                .and_then(|d| d.precipitation_amount)
                .unwrap_or(0.0),
        })
    }

    fn to_hourly(&self, location: &Location) -> WeatherHourly {
        let details = self.instant();
        WeatherHourly {
            t: self.ts_ms,
            temp_c: details.air_temperature.unwrap_or(0.0),
            code: self.code(),
            precip_prob: self.precip_prob(),
            is_day: self.is_day(location),
        }
    }
}

struct DayAgg {
    min_c: f64,
    max_c: f64,
    best_code: u16,
    best_prob: f64,
    noon_symbol: Option<(i32, u16)>,
    precip_prob: f64,
}

fn build_daily(
    entries: &[ParsedEntry<'_>],
    location: &Location,
    tz: &TimeZone,
) -> Vec<WeatherDaily> {
    let mut days: BTreeMap<String, DayAgg> = BTreeMap::new();
    let mut dates: BTreeMap<String, Date> = BTreeMap::new();

    for entry in entries {
        let Ok(ts) = Timestamp::from_millisecond(entry.ts_ms) else {
            continue;
        };
        let zoned = ts.to_zoned(tz.clone());
        let date = zoned.date();
        let key = date.to_string();
        let details = entry.instant();

        let prob = entry.precip_prob();
        let code = entry.code();
        let noon_dist = (i32::from(zoned.hour()) - 12).abs();

        // Kun 6-timers blokke baerer egentlige min/max-tal; instant-temp er fallback.
        let mut low = details.air_temperature.unwrap_or(f64::MAX);
        let mut high = details.air_temperature.unwrap_or(f64::MIN);
        if let Some(d) = entry
            .item
            .data
            .next_6_hours
            .as_ref()
            .and_then(|p| p.details.as_ref())
        {
            if let Some(v) = d.air_temperature_min {
                low = low.min(v);
            }
            if let Some(v) = d.air_temperature_max {
                high = high.max(v);
            }
        }
        if low > high {
            continue;
        }

        let agg = days.entry(key.clone()).or_insert(DayAgg {
            min_c: f64::MAX,
            max_c: f64::MIN,
            best_code: code,
            best_prob: f64::NEG_INFINITY,
            noon_symbol: None,
            precip_prob: 0.0,
        });
        agg.min_c = agg.min_c.min(low);
        agg.max_c = agg.max_c.max(high);
        agg.precip_prob = agg.precip_prob.max(prob);

        let better_noon = match &agg.noon_symbol {
            Some((dist, _)) => noon_dist < *dist,
            None => true,
        };
        if better_noon {
            agg.noon_symbol = Some((noon_dist, code));
        }
        if prob > agg.best_prob {
            agg.best_prob = prob;
            agg.best_code = code;
        }
        dates.entry(key).or_insert(date);
    }

    let mut daily = Vec::with_capacity(7);
    for (key, agg) in days.into_iter().take(7) {
        let date = dates[&key];
        let (sunrise, sunset) = solar_events(location.lat, location.lon, date)
            .map(|(r, s)| (r, s))
            .unwrap_or((0, 0));
        let code = if agg.best_prob > 0.0 {
            agg.best_code
        } else {
            agg.noon_symbol.map(|(_, c)| c).unwrap_or(agg.best_code)
        };
        daily.push(WeatherDaily {
            date: key,
            min_c: agg.min_c,
            max_c: agg.max_c,
            code,
            precip_prob: agg.precip_prob,
            sunrise,
            sunset,
        });
    }
    daily
}

/// MET-symboler -> WMO 4677-koder som `wmo.ts` kender.
fn symbol_to_wmo(symbol: &str) -> u16 {
    let base = symbol
        .strip_suffix("_polartwilight")
        .or_else(|| symbol.strip_suffix("_day"))
        .or_else(|| symbol.strip_suffix("_night"))
        .unwrap_or(symbol);
    match base {
        "clearsky" => 0,
        "fair" => 1,
        "partlycloudy" => 2,
        "cloudy" => 3,
        "fog" => 45,
        "lightrain" => 61,
        "rain" => 63,
        "heavyrain" => 65,
        "lightrainshowers" => 80,
        "rainshowers" => 81,
        "heavyrainshowers" => 82,
        "lightsnow" => 71,
        "snow" => 73,
        "heavysnow" => 75,
        "lightsnowshowers" => 85,
        "snowshowers" => 85,
        "heavysnowshowers" => 86,
        // Slud/isregn mappes til taaeteste WMO-kode i widget-tabellen.
        "lightsleet" | "lightsleetshowers" | "sleet" | "sleetshowers" => 66,
        "heavysleet" | "heavysleetshowers" => 67,
        "lightrainandthunder" | "rainandthunder" | "lightrainshowersandthunder"
        | "rainshowersandthunder" => 95,
        "lightsleetandthunder" | "sleetandthunder" | "lightsleetshowersandthunder"
        | "sleetshowersandthunder" | "lightsnowandthunder" | "snowandthunder"
        | "lightsnowshowersandthunder" | "snowshowersandthunder" | "heavyrainandthunder"
        | "heavyrainshowersandthunder" => 96,
        "heavysleetandthunder" | "heavysleetshowersandthunder" | "heavysnowandthunder"
        | "heavysnowshowersandthunder" => 99,
        other => {
            if other.contains("thunder") {
                95
            } else if other.contains("snow") {
                73
            } else if other.contains("sleet") {
                66
            } else if other.contains("rain") {
                63
            } else if other.contains("fog") {
                45
            } else {
                2
            }
        }
    }
}

fn cloud_to_wmo(cloud_area_fraction: Option<f64>) -> u16 {
    match cloud_area_fraction.unwrap_or(50.0) {
        c if c < 12.5 => 0,
        c if c < 50.0 => 1,
        c if c < 87.5 => 2,
        _ => 3,
    }
}

fn symbol_phase(symbol: &str) -> Option<bool> {
    if symbol.ends_with("_day") {
        Some(true)
    } else if symbol.ends_with("_night") || symbol.ends_with("_polartwilight") {
        Some(false)
    } else {
        None
    }
}

/// NOAA/Meeus solberegning — præcis nok (±2 min) til sunrise/sunset-felterne.
/// Returnerer (sunrise_ms, sunset_ms) i epoch, eller None ved polarnat/-dag.
fn solar_events(lat: f64, lon: f64, date: Date) -> Option<(i64, i64)> {
    let day_start_ms = date
        .to_zoned(TimeZone::UTC)
        .ok()?
        .timestamp()
        .as_millisecond();
    let rise = solar_utc_hours(lat, lon, date, true)?;
    let set = solar_utc_hours(lat, lon, date, false)?;
    Some((
        day_start_ms + (rise * 3_600_000.0) as i64,
        day_start_ms + (set * 3_600_000.0) as i64,
    ))
}

fn solar_utc_hours(lat: f64, lon: f64, date: Date, sunrise: bool) -> Option<f64> {
    let n = f64::from(date.day_of_year());
    let lng_hour = lon / 15.0;
    let t = n + (if sunrise { 6.0 } else { 18.0 } - lng_hour) / 24.0;
    let m = (0.9856 * t - 3.289).to_radians();
    let mut l = m.to_degrees() + 1.916 * m.sin() + 0.020 * (2.0 * m).sin() + 282.634;
    l = l.rem_euclid(360.0);
    let l_rad = l.to_radians();
    let mut ra = (0.91764 * l_rad.tan()).atan().to_degrees();
    ra = ra.rem_euclid(360.0);
    ra += (l.div_euclid(90.0) - ra.div_euclid(90.0)) * 90.0;
    ra /= 15.0;
    let sin_dec = 0.39782 * l_rad.sin();
    let cos_dec = (1.0 - sin_dec * sin_dec).sqrt();
    let lat_rad = lat.to_radians();
    let cos_h = (ZENITH.to_radians().cos() - sin_dec * lat_rad.sin())
        / (cos_dec * lat_rad.cos());
    if !(-1.0..=1.0).contains(&cos_h) {
        return None;
    }
    let mut h = cos_h.acos().to_degrees() / 15.0;
    if sunrise {
        h = 24.0 - h;
    }
    let local_mean = h + ra - 0.06571 * t - 6.622;
    Some((local_mean - lng_hour).rem_euclid(24.0))
}

fn is_day_at(lat: f64, lon: f64, ts_ms: i64, date: Option<Date>) -> bool {
    let Some(date) = date else {
        return true;
    };
    match solar_events(lat, lon, date) {
        Some((rise, set)) => ts_ms >= rise && ts_ms < set,
        // Polarnat/-dag: ingen op-/nedgang — brug breddegradens hemisffere.
        None => lat * solar_declination_hint(date) > 0.0,
    }
}

fn solar_declination_hint(date: Date) -> f64 {
    let n = f64::from(date.day_of_year());
    let l = (0.9856 * n - 3.289 + 282.634_f64).rem_euclid(360.0).to_radians();
    0.39782 * l.sin()
}

#[derive(Deserialize)]
struct MetResponse {
    properties: MetProperties,
}

#[derive(Deserialize)]
struct MetProperties {
    timeseries: Vec<MetEntry>,
}

#[derive(Deserialize)]
struct MetEntry {
    time: String,
    data: MetData,
}

#[derive(Deserialize)]
struct MetData {
    instant: Option<MetInstant>,
    next_1_hours: Option<MetPeriod>,
    next_6_hours: Option<MetPeriod>,
    next_12_hours: Option<MetPeriod>,
}

#[derive(Deserialize)]
struct MetInstant {
    details: Option<MetInstantDetails>,
}

#[derive(Deserialize, Default, Clone)]
struct MetInstantDetails {
    air_temperature: Option<f64>,
    apparent_air_temperature: Option<f64>,
    relative_humidity: Option<f64>,
    cloud_area_fraction: Option<f64>,
    precipitation_amount: Option<f64>,
    wind_from_direction: Option<f64>,
    wind_speed: Option<f64>,
    wind_speed_of_gust: Option<f64>,
}

#[derive(Deserialize)]
struct MetPeriod {
    summary: Option<MetSummary>,
    details: Option<MetPeriodDetails>,
}

#[derive(Deserialize)]
struct MetSummary {
    symbol_code: Option<String>,
}

#[derive(Deserialize)]
struct MetPeriodDetails {
    precipitation_amount: Option<f64>,
    probability_of_precipitation: Option<f64>,
    air_temperature_min: Option<f64>,
    air_temperature_max: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::Location;

    fn loc() -> Location {
        Location {
            id: uuid::Uuid::nil(),
            name: "Aarhus".into(),
            admin: None,
            country: "DK".into(),
            lat: 56.16,
            lon: 10.20,
            tz: "Europe/Copenhagen".into(),
        }
    }

    #[test]
    fn maps_met_symbols_to_wmo() {
        assert_eq!(symbol_to_wmo("clearsky_day"), 0);
        assert_eq!(symbol_to_wmo("clearsky_night"), 0);
        assert_eq!(symbol_to_wmo("partlycloudy_polartwilight"), 2);
        assert_eq!(symbol_to_wmo("heavyrain"), 65);
        assert_eq!(symbol_to_wmo("lightrainshowers"), 80);
        assert_eq!(symbol_to_wmo("heavysnow"), 75);
        assert_eq!(symbol_to_wmo("sleet"), 66);
        assert_eq!(symbol_to_wmo("heavysleetshowers"), 67);
        assert_eq!(symbol_to_wmo("rainandthunder"), 95);
        assert_eq!(symbol_to_wmo("heavysnowandthunder"), 99);
        assert_eq!(symbol_to_wmo("ukendt_symbol"), 2);
    }

    #[test]
    fn symbol_phase_reads_day_night_suffix() {
        assert_eq!(symbol_phase("clearsky_day"), Some(true));
        assert_eq!(symbol_phase("cloudy_night"), Some(false));
        assert_eq!(symbol_phase("fair_polartwilight"), Some(false));
        assert_eq!(symbol_phase("cloudy"), None);
    }

    #[test]
    fn solar_events_match_known_copenhagen_solstice() {
        let date = Date::from_str("2025-06-21").unwrap();
        let (rise, set) = solar_events(55.6761, 12.5683, date).unwrap();
        // Koebenhavn 21. juni: solopgang ~02:25 UTC, solnedgang ~19:58 UTC.
        let rise_min = (rise / 60_000) % (24 * 60);
        let set_min = (set / 60_000) % (24 * 60);
        assert!((rise_min - 145).abs() <= 6, "sunrise {rise_min} min");
        assert!((set_min - 1198).abs() <= 6, "sunset {set_min} min");
    }

    #[test]
    fn solar_events_none_in_polar_night() {
        // Tromsoe i december: ingen solopgang.
        let date = Date::from_str("2025-12-21").unwrap();
        assert!(solar_events(69.65, 18.96, date).is_none());
    }

    #[test]
    fn builds_snapshot_from_complete_payload() {
        // Tiderne laves dynamisk: entry1 er kl. 23 lokal tid i dag, entry2 er
        // midnat (i morgen kl. 00 lokal tid) — så straddler de altid to lokale
        // dage OG ligger inden for hourly-filterets vindue (>= nu-1t), uanset
        // hvornår testen kører. Faste datoer gjorde testen til en tidsbombe.
        let tz = TimeZone::get("Europe/Copenhagen").unwrap();
        let today = Timestamp::now().to_zoned(tz.clone()).date();
        let tomorrow = today.tomorrow().unwrap();
        let midnight_ms = tomorrow
            .to_zoned(tz)
            .unwrap()
            .timestamp()
            .as_millisecond();
        let t2 = Timestamp::from_millisecond(midnight_ms).unwrap();
        let t1 = Timestamp::from_millisecond(midnight_ms - 3_600_000).unwrap();
        let json = r#"{
            "properties": { "timeseries": [
                { "time": "__T1__", "data": {
                    "instant": { "details": {
                        "air_temperature": 1.8, "apparent_air_temperature": -1.9,
                        "relative_humidity": 85.6, "cloud_area_fraction": 97.1,
                        "wind_from_direction": 329.0, "wind_speed": 3.7,
                        "wind_speed_of_gust": 6.5
                    }},
                    "next_1_hours": {
                        "summary": { "symbol_code": "cloudy" },
                        "details": { "precipitation_amount": 0.0,
                                     "probability_of_precipitation": 0.0 }
                    },
                    "next_6_hours": {
                        "summary": { "symbol_code": "cloudy" },
                        "details": { "air_temperature_min": 0.9, "air_temperature_max": 1.7,
                                     "precipitation_amount": 0.0,
                                     "probability_of_precipitation": 0.0 }
                    }
                }},
                { "time": "__T2__", "data": {
                    "instant": { "details": {
                        "air_temperature": 1.2, "apparent_air_temperature": -1.8,
                        "relative_humidity": 93.0, "cloud_area_fraction": 79.8,
                        "wind_speed": 2.7, "wind_from_direction": 335.0
                    }},
                    "next_1_hours": {
                        "summary": { "symbol_code": "lightrain_night" },
                        "details": { "precipitation_amount": 0.3,
                                     "probability_of_precipitation": 62.0 }
                    }
                }}
            ]}
        }"#
        .replace("__T1__", &t1.to_string())
        .replace("__T2__", &t2.to_string());
        let raw: MetResponse = serde_json::from_str(&json).unwrap();
        let snap = build_snapshot(&loc(), "loc-1", raw).unwrap();

        assert_eq!(snap.source, "Yr.no");
        let cur = snap.current.unwrap();
        assert_eq!(cur.temp_c, 1.8);
        assert_eq!(cur.feels_c, -1.9);
        assert_eq!(cur.code, 3);
        assert!((cur.gust_kmh - 23.4).abs() < 0.01);
        assert_eq!(cur.precip_mm, 0.0);

        assert_eq!(snap.hourly.len(), 2);
        assert_eq!(snap.hourly[1].code, 61);
        assert_eq!(snap.hourly[1].precip_prob, 62.0);
        assert!(!snap.hourly[1].is_day);

        // 23:00 og 00:00 dansk tid ligger på hhv. i dag og i morgen — to lokale dage.
        assert_eq!(snap.daily.len(), 2);
        assert_eq!(snap.daily[0].date, today.to_string());
        assert_eq!(snap.daily[0].min_c, 0.9);
        assert_eq!(snap.daily[0].max_c, 1.8);
        assert_eq!(snap.daily[0].precip_prob, 0.0);
        assert!(snap.daily[0].sunrise > 0);
        assert_eq!(snap.daily[1].date, tomorrow.to_string());
        assert_eq!(snap.daily[1].min_c, 1.2);
        assert_eq!(snap.daily[1].max_c, 1.2);
        assert_eq!(snap.daily[1].precip_prob, 62.0);
        assert_eq!(snap.daily[1].code, 61);
    }
}
