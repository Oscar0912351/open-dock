use super::schema::{
    clamp_glass_strength, clamp_scale, clamp_wallpaper_brightness, clamp_wallpaper_speed,
    normalize_system_interval, normalize_wallpaper_fps, Config, CONFIG_VERSION,
};

pub fn migrate(value: serde_json::Value) -> Result<Config, String> {
    if !value.is_object() {
        return Err("Konfigurationen skal være et JSON-objekt".into());
    }
    let version = value
        .get("version")
        .and_then(|v| v.as_u64())
        .unwrap_or(CONFIG_VERSION as u64) as u32;

    let mut value = value;
    strip_legacy_focus(&mut value);
    let mut config = match version {
        1 => serde_json::from_value::<Config>(value).map_err(|error| error.to_string())?,
        other => return Err(format!("Ukendt config-version: {other}")),
    };

    sanitize(&mut config);
    Ok(config)
}

/// Fokus-widgetten (Pomodoro) er fjernet. Ældre layouts kan indeholde fokus-widgets,
/// fokus-moduler i klynger og `expandWhenRunning`-auto-regler; de fjernes før typet
/// parsing, så resten af layoutet bevares intakt.
fn strip_legacy_focus(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    obj.remove("focusStats");
    let Some(widgets) = obj.get_mut("widgets").and_then(|w| w.as_array_mut()) else {
        return;
    };
    widgets.retain(|w| w.get("kind").and_then(|k| k.as_str()) != Some("focus"));
    for widget in widgets.iter_mut() {
        let Some(modules) = widget
            .get_mut("cluster")
            .and_then(|c| c.get_mut("modules"))
            .and_then(|m| m.as_array_mut())
        else {
            continue;
        };
        modules.retain(|m| m.get("kind").and_then(|k| k.as_str()) != Some("focus"));
        for module in modules.iter_mut() {
            if module.get("autoRule").and_then(|r| r.as_str()) == Some("expandWhenRunning") {
                module["autoRule"] = serde_json::json!("none");
            }
        }
        // En klynge skal have 2–5 moduler; blev den for lille, fyldes et ur på.
        if modules.len() == 1 {
            modules.push(serde_json::json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "kind": "clock",
                "defaultMode": "compact",
                "autoRule": "none",
                "settings": {
                    "style": "digital",
                    "showSeconds": false,
                    "showDate": true,
                    "timezone": null,
                    "label": "",
                    "extraZones": []
                }
            }));
        }
    }
    widgets.retain(|w| {
        w.get("cluster")
            .and_then(|c| c.get("modules"))
            .and_then(|m| m.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(true)
    });
}

pub fn sanitize(config: &mut Config) {
    config.version = CONFIG_VERSION;
    config.general.system_interval_ms =
        normalize_system_interval(config.general.system_interval_ms);
    if config.general.hotkeys.edit_mode.trim().is_empty() {
        config.general.hotkeys.edit_mode = "Ctrl+Alt+E".into();
    }
    if config.general.hotkeys.toggle_visible.trim().is_empty() {
        config.general.hotkeys.toggle_visible = "Ctrl+Alt+H".into();
    }
    config.appearance.glass_strength = clamp_glass_strength(config.appearance.glass_strength);
    config.wallpaper.engine.fps_cap = normalize_wallpaper_fps(config.wallpaper.engine.fps_cap);
    for assignment in config.wallpaper.assignments.values_mut() {
        assignment.speed = clamp_wallpaper_speed(assignment.speed);
        assignment.brightness = clamp_wallpaper_brightness(assignment.brightness);
    }

    for widget in &mut config.widgets {
        widget.scale = clamp_scale(widget.scale);
        if let Some(glass) = widget.glass_override {
            widget.glass_override = Some(clamp_glass_strength(glass));
        }
        if !widget.kind.allows_size(widget.size) {
            widget.size = widget.kind.default_size();
        }
    }
}
