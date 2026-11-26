use super::schema::{
    clamp_glass_strength, clamp_scale, clamp_wallpaper_brightness, clamp_wallpaper_speed,
    normalize_system_interval, normalize_wallpaper_fps, AutoRule, Config, ModuleMode, WidgetKind,
    CONFIG_VERSION,
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
    backfill_reveal_on_hover(&mut value);
    map_legacy_layers(&mut value);
    let mut config = match version {
        1 | 2 | 3 | 4 => serde_json::from_value::<Config>(value).map_err(|error| error.to_string())?,
        other => return Err(format!("Ukendt config-version: {other}")),
    };

    if version < 2 {
        migrate_v1_discord_auto_hide(&mut config);
    }
    if version < 4 {
        migrate_v3_timer_show_on_hover(&mut config);
    }

    sanitize(&mut config);
    Ok(config)
}

/// I v1 startede Discord-modulet i klynger som `compact` + `none`, hvilket holdt
/// "Discord lukket" / "Ikke i call" synlig konstant. I v2 skjules det automatisk,
/// når brugeren ikke er i et call.
fn migrate_v1_discord_auto_hide(config: &mut Config) {
    for widget in &mut config.widgets {
        let Some(cluster) = &mut widget.cluster else {
            continue;
        };
        for module in &mut cluster.modules {
            if module.kind == WidgetKind::Discord
                && module.default_mode == ModuleMode::Compact
                && module.auto_rule == AutoRule::None
            {
                module.default_mode = ModuleMode::Hidden;
                module.auto_rule = AutoRule::ExpandWhenInCall;
            }
        }
    }
}

/// I v3 startede timer-moduler i klynger som `compact` + `none`, hvilket holdt
/// en tom timer synlig konstant. I v4 vises de kun når musen er over klyngen.
fn migrate_v3_timer_show_on_hover(config: &mut Config) {
    for widget in &mut config.widgets {
        let Some(cluster) = &mut widget.cluster else {
            continue;
        };
        for module in &mut cluster.modules {
            if module.kind == WidgetKind::Timer
                && module.default_mode == ModuleMode::Compact
                && module.auto_rule == AutoRule::None
            {
                module.default_mode = ModuleMode::Hidden;
                module.auto_rule = AutoRule::ShowOnHover;
            }
        }
    }
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

/// `revealOnHover` er tilføjet med Timer 3.0. Eksisterende timere skal have
/// det slået TIL som standard, alle andre widgets FRA. Manglende nøgle
/// udfyldes på JSON-niveau før typet parsing, så eksplicit gemte
/// true/false-værdier bevares.
fn backfill_reveal_on_hover(value: &mut serde_json::Value) {
    let Some(widgets) = value.get_mut("widgets").and_then(|w| w.as_array_mut()) else {
        return;
    };
    for widget in widgets.iter_mut() {
        if widget.get("revealOnHover").is_none() {
            widget["revealOnHover"] = serde_json::json!(
                widget.get("kind").and_then(|k| k.as_str()) == Some("timer")
            );
        }
    }
}

/// `normal`-laget er fjernet i v3 – widgets kan nu kun ligge på skrivebordet
/// eller altid øverst. Gamle `normal`-widgets mappes til skrivebordslaget.
fn map_legacy_layers(value: &mut serde_json::Value) {
    let Some(widgets) = value
        .get_mut("widgets")
        .and_then(|w| w.as_array_mut())
    else {
        return;
    };
    for widget in widgets.iter_mut() {
        if widget.get("layer").and_then(|l| l.as_str()) == Some("normal") {
            widget["layer"] = serde_json::json!("desktop");
        }
    }
}

pub fn sanitize(config: &mut Config) {
    config.version = CONFIG_VERSION;
    config.general.system_interval_ms =
        normalize_system_interval(config.general.system_interval_ms);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_reveal_on_hover_giver_timer_true_og_bevarer_explicitte_vaerdier() {
        let mut value = serde_json::json!({
            "widgets": [
                { "kind": "timer" },
                { "kind": "clock" },
                { "kind": "timer", "revealOnHover": false },
                { "kind": "note", "revealOnHover": true },
                { "kind": "system" }
            ]
        });
        backfill_reveal_on_hover(&mut value);
        let widgets = value["widgets"].as_array().unwrap();
        assert_eq!(widgets[0]["revealOnHover"], serde_json::json!(true));
        assert_eq!(widgets[1]["revealOnHover"], serde_json::json!(false));
        assert_eq!(widgets[2]["revealOnHover"], serde_json::json!(false));
        assert_eq!(widgets[3]["revealOnHover"], serde_json::json!(true));
        assert_eq!(widgets[4]["revealOnHover"], serde_json::json!(false));
    }

    fn timer_cluster_json(version: u32) -> serde_json::Value {
        serde_json::json!({
            "version": version,
            "general": {
                "language": "da",
                "autostart": true,
                "pauseOnFullscreen": true,
                "pauseOnBattery": false,
                "systemIntervalMs": 1000,
                "gridSnap": true,
                "hotkeys": { "editMode": "Ctrl+Alt+E", "toggleVisible": "Ctrl+Alt+H" }
            },
            "appearance": {
                "themeId": "aurora",
                "accent": null,
                "glassEngine": "wallpaper",
                "glassStrength": 1.0,
                "radius": "round",
                "motion": "full",
                "autoContrast": true
            },
            "weather": { "locations": [] },
            "widgets": [
                {
                    "id": "ba70eb6d-bf00-43dd-8cc4-69db8d6f0cba",
                    "kind": "cluster",
                    "size": "m",
                    "scale": 1.0,
                    "placement": {
                        "monitor": {
                            "name": "DISPLAY1",
                            "rect": { "x": 0, "y": 0, "width": 1920, "height": 1080 }
                        },
                        "x": 100.0,
                        "y": 20.0
                    },
                    "layer": "desktop",
                    "clickThrough": false,
                    "settings": {},
                    "cluster": {
                        "presetId": "allInOne",
                        "width": 264,
                        "maxHeight": 320,
                        "resizeBehavior": "adaptive",
                        "anchor": "top",
                        "modules": [
                            {
                                "id": "1cc5ad6c-a13d-4bc7-b150-c7cd7d1668aa",
                                "kind": "timer",
                                "defaultMode": "compact",
                                "autoRule": "none",
                                "settings": {}
                            }
                        ]
                    },
                    "createdAt": 1700000000
                }
            ],
            "onboarded": true
        })
    }

    #[test]
    fn migraterer_v3_timer_compact_til_show_on_hover_og_bevarer_v4_explicit() {
        let v3 = migrate(timer_cluster_json(3)).unwrap();
        let timer_v3 = &v3.widgets[0].cluster.as_ref().unwrap().modules[0];
        assert_eq!(timer_v3.default_mode, ModuleMode::Hidden);
        assert_eq!(timer_v3.auto_rule, AutoRule::ShowOnHover);

        let v4 = migrate(timer_cluster_json(4)).unwrap();
        let timer_v4 = &v4.widgets[0].cluster.as_ref().unwrap().modules[0];
        assert_eq!(timer_v4.default_mode, ModuleMode::Compact);
        assert_eq!(timer_v4.auto_rule, AutoRule::None);
    }
}
