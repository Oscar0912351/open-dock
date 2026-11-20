use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use ts_rs::TS;

use super::{
    discord::{DiscordProvider, DiscordSnapshot},
    media::{MediaProvider, MediaSnapshot},
    system::{SystemProvider, SystemSample},
    weather::{WeatherProvider, WeatherSnapshot},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum ProviderKind {
    Media,
    System,
    Weather,
    Discord,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubKey {
    pub kind: ProviderKind,
    pub key: Option<String>,
}

impl SubKey {
    pub fn normalized(kind: ProviderKind, key: Option<String>) -> Self {
        let clean = key.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() || (kind == ProviderKind::Media && trimmed == "auto") {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        Self { kind, key: clean }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(untagged)]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum ProviderSnapshot {
    Media(MediaSnapshot),
    System(SystemSample),
    Weather(WeatherSnapshot),
    Discord(DiscordSnapshot),
}

pub struct Scheduler {
    subs: Mutex<HashMap<SubKey, HashMap<String, usize>>>,
    paused: AtomicBool,
    app: AppHandle,
    media: MediaProvider,
    system: SystemProvider,
    weather: WeatherProvider,
    discord: DiscordProvider,
}

impl Scheduler {
    pub fn new(app: AppHandle) -> Self {
        Self {
            subs: Mutex::new(HashMap::new()),
            paused: AtomicBool::new(false),
            app,
            media: MediaProvider::new(),
            system: SystemProvider::new(),
            weather: WeatherProvider::new(),
            discord: DiscordProvider::new(),
        }
    }

    pub fn media(&self) -> &MediaProvider {
        &self.media
    }

    pub fn system(&self) -> &SystemProvider {
        &self.system
    }

    pub fn weather(&self) -> &WeatherProvider {
        &self.weather
    }

    pub fn discord(&self) -> &DiscordProvider {
        &self.discord
    }

    pub fn subscribe(
        &self,
        window_label: String,
        kind: ProviderKind,
        key: Option<String>,
    ) -> Option<ProviderSnapshot> {
        let sub_key = SubKey::normalized(kind, key);
        let should_start = {
            let mut subs = self.subs.lock();
            let counts = subs.entry(sub_key.clone()).or_default();
            let first_for_key = counts.is_empty();
            *counts.entry(window_label).or_insert(0) += 1;

            if !first_for_key {
                false
            } else {
                !self.paused.load(Ordering::Acquire)
            }
        };

        if should_start {
            self.start_key(&sub_key);
        } else if kind == ProviderKind::Media
            && !self.paused.load(Ordering::Acquire)
            && sub_key.key.is_some()
        {
            self.media.set_preferred(sub_key.key.clone());
        }

        match kind {
            ProviderKind::Media => self.media.snapshot().map(ProviderSnapshot::Media),
            ProviderKind::System => self.system.snapshot().map(ProviderSnapshot::System),
            ProviderKind::Weather => sub_key
                .key
                .as_deref()
                .and_then(|loc_id| self.weather.snapshot(loc_id))
                .map(ProviderSnapshot::Weather),
            ProviderKind::Discord => self.discord.snapshot().map(ProviderSnapshot::Discord),
        }
    }

    pub fn unsubscribe(&self, window_label: &str, kind: ProviderKind, key: Option<String>) {
        let sub_key = SubKey::normalized(kind, key);
        let stop_actions = {
            let mut subs = self.subs.lock();
            if let Some(counts) = subs.get_mut(&sub_key) {
                if let Some(count) = counts.get_mut(window_label) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        counts.remove(window_label);
                    }
                }
                if counts.is_empty() {
                    subs.remove(&sub_key);
                    self.compute_stops_for_removed(&subs, &sub_key)
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        };

        for key in stop_actions {
            self.stop_key(&key);
        }
    }

    pub fn unsubscribe_window(&self, window_label: &str) {
        let to_stop = {
            let mut subs = self.subs.lock();
            let mut removed_keys = Vec::new();
            subs.retain(|key, counts| {
                counts.remove(window_label);
                if counts.is_empty() {
                    removed_keys.push(key.clone());
                    false
                } else {
                    true
                }
            });

            let mut stops = Vec::new();
            for key in removed_keys {
                stops.extend(self.compute_stops_for_removed(&subs, &key));
            }
            stops
        };

        for key in to_stop {
            self.stop_key(&key);
        }
    }

    pub fn set_paused(&self, paused: bool) {
        let previous = self.paused.swap(paused, Ordering::AcqRel);
        if previous == paused {
            return;
        }

        let active_keys: Vec<SubKey> = self.subs.lock().keys().cloned().collect();
        if paused {
            log::info!(target: "scheduler", "Pauser aktive providers");
            self.media.stop();
            self.system.stop();
            self.weather.stop_all();
            self.discord.stop();
        } else {
            log::info!(target: "scheduler", "Genoptager aktive providers");
            for key in active_keys {
                self.start_key(&key);
            }
            self.weather.resume_check();
        }
    }

    fn compute_stops_for_removed(
        &self,
        subs: &HashMap<SubKey, HashMap<String, usize>>,
        removed: &SubKey,
    ) -> Vec<SubKey> {
        match removed.kind {
            ProviderKind::Weather => vec![removed.clone()],
            ProviderKind::Media | ProviderKind::System | ProviderKind::Discord => {
                let any_remaining = subs.keys().any(|k| k.kind == removed.kind);
                if !any_remaining {
                    vec![SubKey {
                        kind: removed.kind,
                        key: None,
                    }]
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn start_key(&self, key: &SubKey) {
        match key.kind {
            ProviderKind::Media => {
                self.media.start(&self.app, key.key.clone());
            }
            ProviderKind::System => {
                self.system.start(&self.app);
            }
            ProviderKind::Weather => {
                if let Some(loc_id) = key.key.as_deref() {
                    self.weather.start_location(&self.app, loc_id);
                }
            }
            ProviderKind::Discord => {
                self.discord.start(&self.app);
            }
        }
    }

    fn stop_key(&self, key: &SubKey) {
        match key.kind {
            ProviderKind::Media => self.media.stop(),
            ProviderKind::System => self.system.stop(),
            ProviderKind::Weather => {
                if let Some(loc_id) = key.key.as_deref() {
                    self.weather.stop_location(loc_id);
                }
            }
            ProviderKind::Discord => self.discord.stop(),
        }
    }
}
