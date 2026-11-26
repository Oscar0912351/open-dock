use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use parking_lot::{Mutex, RwLock};
use tauri::{AppHandle, Emitter};

use crate::{desktop::monitors::MonitorInfo, error::AppError};

use super::{defaults, migrate, schema::{Config, CONFIG_VERSION}};

const CONFIG_FILE: &str = "config.json";
const BACKUP_FILE: &str = "config.bak.json";
const TEMP_FILE: &str = "config.json.tmp";
const SAVE_DEBOUNCE_MS: u64 = 300;
pub const RESTORED_BACKUP_MESSAGE: &str =
    "Dit layout kunne ikke indlæses. Vi har gendannet den seneste sikkerhedskopi.";
pub const RESTORED_DEFAULTS_MESSAGE: &str =
    "Dit layout kunne ikke indlæses. Standardopsætningen er gendannet.";

pub struct ConfigStore {
    dir: PathBuf,
    config: RwLock<Config>,
    app: Option<AppHandle>,
    save_seq: AtomicU64,
    backed_up_session: AtomicBool,
    had_valid_file_on_load: AtomicBool,
    restore_notice: Mutex<Option<String>>,
}

impl ConfigStore {
    pub fn load(app: &AppHandle, primary_monitor: Option<&MonitorInfo>) -> Arc<Self> {
        let dir = crate::paths::data_root();
        let store = Arc::new(Self::load_from_dir(dir, Some(app.clone()), primary_monitor));
        if let Some(message) = store.peek_restore_notice() {
            let _ = app.emit("config://restored", &message);
        }
        store
    }

    pub fn load_from_dir(
        dir: PathBuf,
        app: Option<AppHandle>,
        primary_monitor: Option<&MonitorInfo>,
    ) -> Self {
        let _ = fs::create_dir_all(&dir);
        let config_path = dir.join(CONFIG_FILE);
        let backup_path = dir.join(BACKUP_FILE);

        let mut had_valid = false;
        let mut restored_from_backup = false;
        let mut restore_notice: Option<String> = None;

        let config = match read_config_file(&config_path) {
            Ok(Some((cfg, migrated))) => {
                had_valid = true;
                if migrated {
                    if let Err(error) = write_atomic(&dir, &cfg) {
                        log::warn!("Kunne ikke gemme migreret config: {error:?}");
                    }
                }
                cfg
            }
            Ok(None) => {
                let cfg = defaults::default_config(primary_monitor);
                if let Err(error) = write_atomic(&dir, &cfg) {
                    log::warn!("Kunne ikke skrive standardconfig ved opstart: {error:?}");
                } else {
                    had_valid = true;
                    let _ = fs::copy(&config_path, &backup_path);
                }
                cfg
            }
            Err(primary_err) => {
                log::warn!("config.json kunne ikke indlæses ({primary_err}); prøver sikkerhedskopi");
                match read_config_file(&backup_path) {
                    Ok(Some((bak, _))) => {
                        restored_from_backup = true;
                        restore_notice = Some(RESTORED_BACKUP_MESSAGE.into());
                        if let Err(error) = write_atomic(&dir, &bak) {
                            log::warn!("Kunne ikke genskabe config.json fra sikkerhedskopi: {error:?}");
                        }
                        bak
                    }
                    Ok(None) | Err(_) => {
                        restore_notice = Some(RESTORED_DEFAULTS_MESSAGE.into());
                        let cfg = defaults::default_config(primary_monitor);
                        if let Err(error) = write_atomic(&dir, &cfg) {
                            log::warn!("Kunne ikke skrive standardconfig efter fejl: {error:?}");
                        }
                        cfg
                    }
                }
            }
        };

        Self {
            dir,
            config: RwLock::new(config),
            app,
            save_seq: AtomicU64::new(0),
            backed_up_session: AtomicBool::new(restored_from_backup),
            had_valid_file_on_load: AtomicBool::new(had_valid),
            restore_notice: Mutex::new(restore_notice),
        }
    }

    pub fn get(&self) -> Config {
        self.config.read().clone()
    }

    pub fn notes_dir(&self) -> PathBuf {
        self.dir.join("notes")
    }

    pub fn peek_restore_notice(&self) -> Option<String> {
        self.restore_notice.lock().clone()
    }

    pub fn take_restore_notice(&self) -> Option<String> {
        self.restore_notice.lock().take()
    }

    pub fn mutate<F, R>(self: &Arc<Self>, f: F) -> Result<R, AppError>
    where
        F: FnOnce(&mut Config) -> Result<R, AppError>,
    {
        let (result, snapshot) = {
            let mut guard = self.config.write();
            let mut candidate = guard.clone();
            let result = f(&mut candidate)?;
            migrate::sanitize(&mut candidate);
            *guard = candidate.clone();
            (result, candidate)
        };

        self.schedule_save();
        crate::desktop::hover::poke();
        if let Some(app) = &self.app {
            if let Err(error) = app.emit("config://changed", &snapshot) {
                log::warn!("Kunne ikke sende config://changed: {error}");
            }
            crate::tray::sync(app);
        }
        Ok(result)
    }

    pub fn update_placement_quiet(self: &Arc<Self>, id: uuid::Uuid, x: f64, y: f64) {
        let mut changed = false;
        {
            let mut guard = self.config.write();
            if let Some(target) = guard.widgets.iter_mut().find(|w| w.id == id) {
                if (target.placement.x - x).abs() > 0.25 || (target.placement.y - y).abs() > 0.25 {
                    target.placement.x = x;
                    target.placement.y = y;
                    changed = true;
                }
            }
        }
        if changed {
            self.schedule_save();
        }
    }

    pub fn flush(&self) -> Result<(), AppError> {
        let config_path = self.dir.join(CONFIG_FILE);
        let backup_path = self.dir.join(BACKUP_FILE);

        if !self.backed_up_session.swap(true, Ordering::AcqRel)
            && self.had_valid_file_on_load.load(Ordering::Acquire)
            && config_path.exists()
        {
            if let Err(error) = fs::copy(&config_path, &backup_path) {
                log::warn!("Kunne ikke opdatere config.bak.json: {error}");
            }
        }

        let snapshot = self.get();
        write_atomic(&self.dir, &snapshot)?;

        if !backup_path.exists() {
            let _ = fs::copy(&config_path, &backup_path);
        }
        self.had_valid_file_on_load.store(true, Ordering::Release);
        Ok(())
    }

    fn schedule_save(self: &Arc<Self>) {
        let seq = self.save_seq.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let store = Arc::clone(self);
            handle.spawn(async move {
                tokio::time::sleep(Duration::from_millis(SAVE_DEBOUNCE_MS)).await;
                if store.save_seq.load(Ordering::Acquire) == seq {
                    if let Err(error) = store.flush() {
                        log::error!("Kunne ikke gemme konfiguration: {error:?}");
                    }
                }
            });
        } else if let Err(error) = self.flush() {
            log::error!("Kunne ikke gemme konfiguration synkront: {error:?}");
        }
    }
}

fn read_config_file(path: &Path) -> Result<Option<(Config, bool)>, String> {
    match fs::read_to_string(path) {
        Ok(raw) => {
            let value: serde_json::Value =
                serde_json::from_str(&raw).map_err(|error| error.to_string())?;
            let raw_version = value
                .get("version")
                .and_then(|v| v.as_u64())
                .unwrap_or(CONFIG_VERSION as u64) as u32;
            migrate::migrate(value).map(|cfg| Some((cfg, raw_version < CONFIG_VERSION)))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn write_atomic(dir: &Path, config: &Config) -> Result<(), AppError> {
    fs::create_dir_all(dir).map_err(AppError::ConfigWrite)?;
    let tmp_path = dir.join(TEMP_FILE);
    let final_path = dir.join(CONFIG_FILE);
    let json = serde_json::to_string_pretty(config)
        .map_err(|error| AppError::Other(format!("JSON-serialisering fejlede: {error}")))?;
    fs::write(&tmp_path, json).map_err(AppError::ConfigWrite)?;
    fs::rename(&tmp_path, &final_path).map_err(AppError::ConfigWrite)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{WidgetKind, WidgetPatch, WidgetSize};

    fn temp_config_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "open_dock_cfg_{}_{}",
            name,
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn clamps_scale_and_validates_allowed_size() {
        let dir = temp_config_dir("clamp");
        let store = Arc::new(ConfigStore::load_from_dir(dir.clone(), None, None));
        let widget_id = store
            .mutate(|cfg| {
                let w = defaults::default_widget(WidgetKind::Clock, None);
                let id = w.id;
                cfg.widgets.push(w);
                Ok(id)
            })
            .unwrap();

        store
            .mutate(|cfg| {
                cfg.widgets[0].apply_patch(WidgetPatch {
                    scale: Some(2.4),
                    ..Default::default()
                })
            })
            .unwrap();
        assert!((store.get().widgets[0].scale - 1.35).abs() < 1e-6);

        let err = store
            .mutate(|cfg| {
                cfg.widgets[0].kind = WidgetKind::Clock;
                cfg.widgets[0].apply_patch(WidgetPatch {
                    size: Some(WidgetSize::L),
                    ..Default::default()
                })
            })
            .unwrap_err();
        assert!(matches!(err, AppError::SizeNotAllowed));
        assert_eq!(store.get().widgets[0].id, widget_id);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_save_creates_backup_and_recovers_corrupt_config() {
        let dir = temp_config_dir("backup");
        {
            let store = Arc::new(ConfigStore::load_from_dir(dir.clone(), None, None));
            store
                .mutate(|cfg| {
                    cfg.widgets.push(defaults::default_widget(WidgetKind::Clock, None));
                    cfg.widgets.push(defaults::default_widget(WidgetKind::Clock, None));
                    Ok(())
                })
                .unwrap();
            assert_eq!(store.get().widgets.len(), 2);
            assert!(dir.join(CONFIG_FILE).exists());
            assert!(dir.join(BACKUP_FILE).exists());
            assert!(!dir.join(TEMP_FILE).exists());
        }

        // Corrupt primary config.json by hand while keeping valid backup from prior session save
        {
            let store = Arc::new(ConfigStore::load_from_dir(dir.clone(), None, None));
            assert_eq!(store.get().widgets.len(), 2);
            // Force a save so config.bak.json receives the 2-widget snapshot
            store.mutate(|_| Ok(())).unwrap();
        }
        fs::write(dir.join(CONFIG_FILE), "{ dette er ikke gyldig json !!!").unwrap();

        let recovered = ConfigStore::load_from_dir(dir.clone(), None, None);
        assert_eq!(recovered.get().widgets.len(), 2);
        assert_eq!(
            recovered.peek_restore_notice().as_deref(),
            Some(RESTORED_BACKUP_MESSAGE)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_legacy_config_without_control_theme_as_dark() {
        let dir = temp_config_dir("legacy_theme");
        let legacy_json = r#"{
          "version": 1,
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
          "widgets": [],
          "onboarded": true,
          "focusStats": { "date": "2026-11-19", "count": 0 }
        }"#;
        fs::write(dir.join(CONFIG_FILE), legacy_json).unwrap();

        let loaded = ConfigStore::load_from_dir(dir.clone(), None, None);
        assert_eq!(
            loaded.get().general.control_theme,
            crate::config::schema::ControlTheme::Dark
        );
        assert_eq!(loaded.get().wallpaper.engine.fps_cap, 30);
        assert_eq!(
            loaded.get().wallpaper.engine.resolution,
            crate::config::schema::WallpaperResolution::Native
        );
        assert!(loaded.get().wallpaper.assignments.is_empty());
        assert!(loaded.peek_restore_notice().is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_v1_discord_to_hidden_expand_when_in_call_and_preserves_v2_explicit() {
        use crate::config::schema::{AutoRule, ModuleMode};

        let dir = temp_config_dir("v1_discord_mig");
        let make_cluster_json = |version: u32| {
            format!(
                r#"{{
                  "version": {version},
                  "general": {{
                    "language": "da",
                    "autostart": true,
                    "pauseOnFullscreen": true,
                    "pauseOnBattery": false,
                    "systemIntervalMs": 1000,
                    "gridSnap": true,
                    "hotkeys": {{ "editMode": "Ctrl+Alt+E", "toggleVisible": "Ctrl+Alt+H" }}
                  }},
                  "appearance": {{
                    "themeId": "aurora",
                    "accent": null,
                    "glassEngine": "wallpaper",
                    "glassStrength": 1.0,
                    "radius": "round",
                    "motion": "full",
                    "autoContrast": true
                  }},
                  "weather": {{ "locations": [] }},
                  "widgets": [
                    {{
                      "id": "ba70eb6d-bf00-43dd-8cc4-69db8d6f0cba",
                      "kind": "cluster",
                      "size": "m",
                      "scale": 1.0,
                      "placement": {{
                        "monitor": {{
                          "name": "DISPLAY1",
                          "rect": {{ "x": 0, "y": 0, "width": 1920, "height": 1080 }}
                        }},
                        "x": 100.0,
                        "y": 20.0
                      }},
                      "layer": "desktop",
                      "clickThrough": false,
                      "settings": {{}},
                      "cluster": {{
                        "presetId": "allInOne",
                        "width": 264,
                        "maxHeight": 320,
                        "resizeBehavior": "adaptive",
                        "anchor": "top",
                        "modules": [
                          {{
                            "id": "15868400-67d2-4568-a045-dac1b57fac7e",
                            "kind": "clock",
                            "defaultMode": "compact",
                            "autoRule": "none",
                            "settings": {{}}
                          }},
                          {{
                            "id": "1cc5ad6c-a13d-4bc7-b150-c7cd7d1668aa",
                            "kind": "discord",
                            "defaultMode": "compact",
                            "autoRule": "none",
                            "settings": {{}}
                          }}
                        ]
                      }},
                      "createdAt": 1700000000
                    }}
                  ],
                  "onboarded": true
                }}"#
            )
        };

        fs::write(dir.join(CONFIG_FILE), make_cluster_json(1)).unwrap();
        let migrated = ConfigStore::load_from_dir(dir.clone(), None, None);
        let cfg = migrated.get();
        assert_eq!(cfg.version, 3);
        let disc = &cfg.widgets[0].cluster.as_ref().unwrap().modules[1];
        assert_eq!(disc.default_mode, ModuleMode::Hidden);
        assert_eq!(disc.auto_rule, AutoRule::ExpandWhenInCall);

        // Bekræft at den migrerede version blev skrevet til disken.
        let disk_raw = fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        let disk_val: serde_json::Value = serde_json::from_str(&disk_raw).unwrap();
        assert_eq!(disk_val.get("version").and_then(|v| v.as_u64()), Some(3));

        // En eksplicit v2-konfiguration må bevare brugerens valg (`compact` + `none`).
        fs::write(dir.join(CONFIG_FILE), make_cluster_json(2)).unwrap();
        let explicit_v2 = ConfigStore::load_from_dir(dir.clone(), None, None);
        let cfg_v2 = explicit_v2.get();
        let disc_v2 = &cfg_v2.widgets[0].cluster.as_ref().unwrap().modules[1];
        assert_eq!(disc_v2.default_mode, ModuleMode::Compact);
        assert_eq!(disc_v2.auto_rule, AutoRule::None);

        let _ = fs::remove_dir_all(&dir);
    }
}
