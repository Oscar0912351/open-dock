use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    config::schema::{Config, Location, WidgetKind},
    error::AppError,
    providers::{scheduler::Scheduler, weather::GeoResult},
    state::AppState,
};

#[tauri::command]
pub async fn weather_search(app: AppHandle, query: String) -> Result<Vec<GeoResult>, AppError> {
    let result = app.state::<Scheduler>().weather().search(&query).await;
    if let Err(error) = &result {
        log::error!(target: "cmd", "weather_search fejlede: {error:?}");
    }
    result
}

#[tauri::command]
pub fn weather_location_add(app: AppHandle, result: GeoResult) -> Result<Config, AppError> {
    let res = app.state::<AppState>().config.mutate(|cfg| {
        if let Some(existing) = cfg.weather.locations.iter().find(|loc| {
            loc.name.eq_ignore_ascii_case(&result.name)
                && (loc.lat - result.latitude).abs() < 0.01
                && (loc.lon - result.longitude).abs() < 0.01
        }) {
            let existing_id = existing.id.to_string();
            assign_weather_location_if_empty(cfg, &existing_id);
            return Ok(cfg.clone());
        }

        let loc = Location {
            id: Uuid::new_v4(),
            name: result.name,
            admin: result.admin1,
            country: result.country,
            lat: result.latitude,
            lon: result.longitude,
            tz: result.timezone,
        };
        let loc_id = loc.id.to_string();
        cfg.weather.locations.push(loc);
        assign_weather_location_if_empty(cfg, &loc_id);
        Ok(cfg.clone())
    });

    if let Err(error) = &res {
        log::error!(target: "cmd", "weather_location_add fejlede: {error:?}");
    }
    res
}

fn assign_weather_location_if_empty(cfg: &mut Config, location_id: &str) {
    for widget in &mut cfg.widgets {
        if widget.kind == WidgetKind::Weather {
            let current = widget
                .settings
                .get("locationId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if current.is_empty() {
                if let Some(obj) = widget.settings.as_object_mut() {
                    obj.insert(
                        "locationId".to_string(),
                        serde_json::Value::String(location_id.to_string()),
                    );
                }
            }
        }
    }
}

#[tauri::command]
pub fn weather_location_remove(app: AppHandle, id: Uuid) -> Result<Config, AppError> {
    let id_str = id.to_string();
    let res = app.state::<AppState>().config.mutate(|cfg| {
        cfg.weather.locations.retain(|loc| loc.id != id);
        let fallback_id = cfg
            .weather
            .locations
            .first()
            .map(|l| l.id.to_string())
            .unwrap_or_default();

        for widget in &mut cfg.widgets {
            if widget.kind == WidgetKind::Weather {
                let current = widget
                    .settings
                    .get("locationId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if current == id_str {
                    if let Some(obj) = widget.settings.as_object_mut() {
                        if fallback_id.is_empty() {
                            obj.insert("locationId".to_string(), serde_json::Value::Null);
                        } else {
                            obj.insert(
                                "locationId".to_string(),
                                serde_json::Value::String(fallback_id.clone()),
                            );
                        }
                    }
                }
            }
        }
        Ok(cfg.clone())
    });

    app.state::<Scheduler>().weather().stop_location(&id_str);

    if let Err(error) = &res {
        log::error!(target: "cmd", "weather_location_remove fejlede: {error:?}");
    }
    res
}

#[tauri::command]
pub fn weather_refresh(app: AppHandle, location_id: String) -> Result<(), AppError> {
    app.state::<Scheduler>()
        .weather()
        .refresh_location(&location_id);
    Ok(())
}
