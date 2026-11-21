use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use uuid::Uuid;

use crate::desktop::monitors::MonitorInfo;

use super::schema::{
    Appearance, AutoRule, ClusterConfig, ClusterModule, Config, General, Layer,
    ModuleMode, MonitorKey, Placement, RectI, ResizeBehavior, VerticalAnchor, WallpaperConfig,
    WeatherConfig, WidgetInstance, WidgetKind, CONFIG_VERSION,
};

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn default_settings(kind: WidgetKind) -> serde_json::Value {
    match kind {
        WidgetKind::Clock => json!({
            "style": "digital",
            "showSeconds": false,
            "showDate": true,
            "timezone": null,
            "label": "",
            "extraZones": []
        }),
        WidgetKind::NowPlaying => json!({
            "preferredSource": "auto",
            "ambient": true,
            "showProgress": true,
            "hideWhenIdle": false
        }),
        WidgetKind::System => json!({
            "metrics": ["cpu", "mem", "gpu"],
            "primary": "cpu",
            "showTemps": true,
            "unitsBits": false
        }),
        WidgetKind::Weather => json!({
            "locationId": null,
            "unit": "c",
            "chart": "hourly"
        }),
        WidgetKind::Note => json!({
            "title": "Note",
            "mode": "text",
            "fontSize": "m"
        }),
        WidgetKind::Discord => json!({}),
        WidgetKind::Cluster => json!({}),
        WidgetKind::Agents => json!({}),
    }
}

pub fn monitor_key(monitor: Option<&MonitorInfo>) -> MonitorKey {
    match monitor {
        Some(m) => MonitorKey {
            name: m.name.clone(),
            rect: m.rect,
        },
        None => MonitorKey {
            name: r"\\.\DISPLAY1".into(),
            rect: RectI {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
        },
    }
}

pub fn default_placement(monitor: Option<&MonitorInfo>, base_size: (f64, f64)) -> Placement {
    let key = monitor_key(monitor);
    let (work_w, _work_h) = match monitor {
        Some(m) => {
            let scale = m.scale.max(0.5);
            (m.work_area.width as f64 / scale, m.work_area.height as f64 / scale)
        }
        None => (1920.0, 1040.0),
    };
    let x = (work_w - base_size.0 - 48.0).max(20.0);
    let y = 48.0;
    Placement {
        monitor: key,
        x,
        y,
    }
}

pub fn default_widget(kind: WidgetKind, monitor: Option<&MonitorInfo>) -> WidgetInstance {
    let size = kind.default_size();
    WidgetInstance {
        id: Uuid::new_v4(),
        kind,
        size,
        scale: 1.0,
        placement: default_placement(monitor, size.base_size()),
        layer: Layer::Desktop,
        click_through: false,
        theme_override: None,
        glass_override: None,
        glass_override_engine: None,
        settings: default_settings(kind),
        cluster: None,
        created_at: unix_now(),
    }
}

pub fn default_config(_primary_monitor: Option<&MonitorInfo>) -> Config {
    Config {
        version: CONFIG_VERSION,
        general: General::default(),
        appearance: Appearance::default(),
        weather: WeatherConfig::default(),
        wallpaper: WallpaperConfig::default(),
        widgets: Vec::new(),
        onboarded: false,
    }
}

fn make_module(kind: WidgetKind, default_mode: ModuleMode, auto_rule: AutoRule) -> ClusterModule {
    ClusterModule {
        id: Uuid::new_v4(),
        kind,
        default_mode,
        auto_rule,
        settings: default_settings(kind),
    }
}

pub fn cluster_preset_configs(preset_id: &str) -> Option<Vec<ClusterConfig>> {
    match preset_id {
        "allInOne" => Some(vec![ClusterConfig {
            preset_id: Some("allInOne".into()),
            width: 264,
            max_height: 320,
            resize_behavior: ResizeBehavior::Adaptive,
            anchor: VerticalAnchor::Top,
            modules: vec![
                make_module(WidgetKind::Clock, ModuleMode::Compact, AutoRule::None),
                make_module(
                    WidgetKind::Weather,
                    ModuleMode::Compact,
                    AutoRule::FillRemainingSpace,
                ),
                make_module(
                    WidgetKind::System,
                    ModuleMode::Compact,
                    AutoRule::ExpandWhenHighLoad,
                ),
                make_module(
                    WidgetKind::NowPlaying,
                    ModuleMode::Hidden,
                    AutoRule::ExpandWhenPlaying,
                ),
                make_module(WidgetKind::Discord, ModuleMode::Compact, AutoRule::None),
            ],
        }]),
        "miniTrio" => {
            let mut note_mod = make_module(WidgetKind::Note, ModuleMode::Compact, AutoRule::None);
            note_mod.settings = json!({
                "title": "Indkøb",
                "mode": "checklist",
                "fontSize": "m"
            });
            Some(vec![
                ClusterConfig {
                    preset_id: Some("miniTrioA".into()),
                    width: 232,
                    max_height: 240,
                    resize_behavior: ResizeBehavior::Adaptive,
                    anchor: VerticalAnchor::Top,
                    modules: vec![
                        make_module(WidgetKind::Clock, ModuleMode::Compact, AutoRule::None),
                        make_module(WidgetKind::Weather, ModuleMode::Compact, AutoRule::None),
                    ],
                },
                ClusterConfig {
                    preset_id: Some("miniTrioB".into()),
                    width: 232,
                    max_height: 240,
                    resize_behavior: ResizeBehavior::Adaptive,
                    anchor: VerticalAnchor::Top,
                    modules: vec![
                        make_module(
                            WidgetKind::NowPlaying,
                            ModuleMode::Hidden,
                            AutoRule::ExpandWhenPlaying,
                        ),
                        make_module(WidgetKind::Clock, ModuleMode::Compact, AutoRule::None),
                    ],
                },
                ClusterConfig {
                    preset_id: Some("miniTrioC".into()),
                    width: 232,
                    max_height: 240,
                    resize_behavior: ResizeBehavior::Adaptive,
                    anchor: VerticalAnchor::Top,
                    modules: vec![
                        make_module(
                            WidgetKind::System,
                            ModuleMode::Compact,
                            AutoRule::ExpandWhenHighLoad,
                        ),
                        note_mod,
                    ],
                },
            ])
        }
        "pillBar" => Some(vec![ClusterConfig {
            preset_id: Some("pillBar".into()),
            width: 264,
            max_height: 240,
            resize_behavior: ResizeBehavior::Adaptive,
            anchor: VerticalAnchor::Top,
            modules: vec![
                make_module(WidgetKind::Clock, ModuleMode::Compact, AutoRule::None),
                make_module(WidgetKind::Weather, ModuleMode::Compact, AutoRule::None),
                make_module(WidgetKind::System, ModuleMode::Compact, AutoRule::None),
                make_module(
                    WidgetKind::NowPlaying,
                    ModuleMode::Hidden,
                    AutoRule::ExpandWhenPlaying,
                ),
            ],
        }]),
        _ => None,
    }
}
