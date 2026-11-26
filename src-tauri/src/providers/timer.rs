//! "Husk-timer" — flere navngivne timere pr. widget-id, ejet af Rust.
//!
//! Timere lever uafhængigt af widget-vinduet: de fortsætter når widgetten
//! genindlæses, når appen er på pause, og (via `timers.json` v2) når appen
//! genstartes. Frontend tæller sekunder lokalt ud fra `ends_at` — der sendes
//! ingen tick-events. Én tokio-task sover til nærmeste deadline og vækkes
//! via `Notify` når tilstanden ændres.
//!
//! Hver widget (eller klynge-modul) har op til 8 timere (`items`) plus en
//! liste over de seneste navngivne varigheder (`recents`, maks 6) som
//! frontend viser som ét-klik-chips. Når en timer udløber: én samlet lyd pr.
//! scheduler-pass, notifikation pr. timer med dens navn, og rækken viser
//! "+1 min"/"+5 min"/"Færdig" indtil brugeren fjerner den. Finished-timere
//! ringer igen hvert 10. sekund i op til 60 sekunder (REPEAT_INTERVAL_MS /
//! REPEAT_WINDOW_MS).

use std::{collections::HashMap, fs, path::PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    config::schema::{Config, Language, WidgetKind},
    error::AppError,
};

const MIN_DURATION_MS: i64 = 1_000;
const MAX_DURATION_MS: i64 = 24 * 60 * 60 * 1000; // 24 timer
const MAX_ITEMS: usize = 8;
const MAX_LABEL_CHARS: usize = 60;
const MAX_RECENTS: usize = 6;
const REPEAT_INTERVAL_MS: i64 = 10_000;
const REPEAT_WINDOW_MS: i64 = 60_000;
const PERSIST_FILE: &str = "timers.json";
const PERSIST_VERSION: u32 = 2;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum TimerStatus {
    Running,
    Paused,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct TimerItem {
    pub id: Uuid,
    pub label: String,
    pub status: TimerStatus,
    /// Kun sat mens Running — epoch-ms for hvornår timeren udløber.
    pub ends_at: Option<i64>,
    /// Paused: resterende tid. Running: resterende tid ved seneste commit.
    /// Finished: 0.
    pub remaining_ms: i64,
    /// Nævner for progress; vokser ved +min.
    pub total_ms: i64,
    /// Oprindelig varighed — fallback-titel og recents.
    pub initial_ms: i64,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    /// Antal gentag-ringer afspillet for en finished timer (intern, persisteres
    /// men sendes ikke til frontend).
    #[serde(default)]
    #[ts(skip)]
    pub repeat_count: u8,
}

impl TimerItem {
    /// Næste gentag-ringetidspunkt for en finished timer (hver 10. s, op til
    /// 60 s efter finished_at). `None` for ikke-finished eller uden for vinduet.
    fn next_repeat_at(&self) -> Option<i64> {
        if self.status != TimerStatus::Finished {
            return None;
        }
        let finished_at = self.finished_at?;
        let next = finished_at + (self.repeat_count as i64 + 1) * REPEAT_INTERVAL_MS;
        if next <= finished_at + REPEAT_WINDOW_MS {
            Some(next)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct RecentTimer {
    pub label: String,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct TimerListSnapshot {
    pub widget_id: Uuid,
    pub items: Vec<TimerItem>,
    pub recents: Vec<RecentTimer>,
}

/// Timer-tilstanden for én widget (eller ét klynge-modul) — ren logik der
/// tager `now` som parameter, så den kan unit-testes uden AppHandle.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WidgetTimers {
    items: Vec<TimerItem>,
    recents: Vec<RecentTimer>,
}

impl WidgetTimers {
    fn snapshot(&self, widget_id: Uuid) -> TimerListSnapshot {
        TimerListSnapshot {
            widget_id,
            items: self.items.clone(),
            recents: self.recents.clone(),
        }
    }

    fn add(&mut self, raw_label: &str, duration_ms: i64, now: i64) -> Result<Uuid, AppError> {
        let label: String = raw_label.trim().chars().take(MAX_LABEL_CHARS).collect();
        if !(MIN_DURATION_MS..=MAX_DURATION_MS).contains(&duration_ms) {
            return Err(AppError::Other(
                "Vælg en tid mellem 1 sekund og 24 timer".into(),
            ));
        }
        if self.items.len() >= MAX_ITEMS {
            return Err(AppError::Other(
                "Du kan højst have 8 timere i gang".into(),
            ));
        }
        let id = Uuid::new_v4();
        self.items.push(TimerItem {
            id,
            label: label.clone(),
            status: TimerStatus::Running,
            ends_at: Some(now + duration_ms),
            remaining_ms: duration_ms,
            total_ms: duration_ms,
            initial_ms: duration_ms,
            created_at: now,
            finished_at: None,
            repeat_count: 0,
        });
        if !label.is_empty() {
            let key = label.to_lowercase();
            self.recents.retain(|r| r.label.to_lowercase() != key);
            self.recents.insert(
                0,
                RecentTimer {
                    label,
                    duration_ms,
                },
            );
            self.recents.truncate(MAX_RECENTS);
        }
        Ok(id)
    }

    fn pause(&mut self, item_id: Uuid, now: i64) -> bool {
        let Some(item) = self.items.iter_mut().find(|i| i.id == item_id) else {
            return false;
        };
        if item.status != TimerStatus::Running {
            return false;
        }
        if let Some(ends) = item.ends_at {
            item.remaining_ms = (ends - now).max(0);
        }
        item.ends_at = None;
        item.status = TimerStatus::Paused;
        true
    }

    fn resume(&mut self, item_id: Uuid, now: i64) -> bool {
        let Some(item) = self.items.iter_mut().find(|i| i.id == item_id) else {
            return false;
        };
        if item.status != TimerStatus::Paused || item.remaining_ms <= 0 {
            return false;
        }
        item.ends_at = Some(now + item.remaining_ms);
        item.status = TimerStatus::Running;
        true
    }

    /// Justerer resterende tid med `delta_ms` (negativ = kortere).
    /// Running: ny remaining måles fra nu, ends_at forskydes tilsvarende.
    /// Finished: kun positiv delta genstarter timeren med delta som varighed.
    fn adjust(&mut self, item_id: Uuid, delta_ms: i64, now: i64) -> bool {
        let Some(item) = self.items.iter_mut().find(|i| i.id == item_id) else {
            return false;
        };
        match item.status {
            TimerStatus::Running => {
                let rem = item
                    .ends_at
                    .map(|ends| (ends - now).max(0))
                    .unwrap_or(item.remaining_ms);
                let new = (rem + delta_ms).clamp(MIN_DURATION_MS, MAX_DURATION_MS);
                item.ends_at = Some(now + new);
                item.remaining_ms = new;
                item.total_ms = (item.total_ms + (new - rem)).max(new);
                true
            }
            TimerStatus::Paused => {
                let rem = item.remaining_ms;
                let new = (rem + delta_ms).clamp(MIN_DURATION_MS, MAX_DURATION_MS);
                item.remaining_ms = new;
                item.total_ms = (item.total_ms + (new - rem)).max(new);
                true
            }
            TimerStatus::Finished => {
                if delta_ms <= 0 {
                    return false;
                }
                let new = delta_ms.clamp(MIN_DURATION_MS, MAX_DURATION_MS);
                item.status = TimerStatus::Running;
                item.ends_at = Some(now + new);
                item.remaining_ms = new;
                item.total_ms = new;
                item.finished_at = None;
                item.repeat_count = 0;
                true
            }
        }
    }

    fn remove(&mut self, item_id: Uuid) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != item_id);
        self.items.len() != before
    }

    /// Markerer en running timer som færdig. `at` er den rigtige udløbstid,
    /// så overtid tælles fra den — ikke fra behandlingstidspunktet.
    fn finish(&mut self, item_id: Uuid, at: i64) -> bool {
        let Some(item) = self.items.iter_mut().find(|i| i.id == item_id) else {
            return false;
        };
        if item.status != TimerStatus::Running {
            return false;
        }
        item.status = TimerStatus::Finished;
        item.ends_at = None;
        item.remaining_ms = 0;
        item.finished_at = Some(at);
        item.repeat_count = 0;
        true
    }

    /// Running-timere hvis ends_at er nået — returnerer (item-id, ends_at).
    fn due(&self, now: i64) -> Vec<(Uuid, i64)> {
        self.items
            .iter()
            .filter_map(|item| {
                if item.status != TimerStatus::Running {
                    return None;
                }
                item.ends_at
                    .filter(|ends| *ends <= now)
                    .map(|ends| (item.id, ends))
            })
            .collect()
    }

    /// Næste tidspunkt scheduler'en skal wake: minimum af fremtidige ends_at
    /// (running) og næste repeat-ring (finished).
    fn next_wake(&self, now: i64) -> Option<i64> {
        let mut best: Option<i64> = None;
        let mut consider = |candidate: i64| {
            best = Some(best.map_or(candidate, |b| b.min(candidate)));
        };
        for item in &self.items {
            if item.status == TimerStatus::Running {
                if let Some(ends) = item.ends_at {
                    if ends > now {
                        consider(ends);
                    }
                }
            }
            if let Some(next) = item.next_repeat_at() {
                consider(next);
            }
        }
        best
    }
}

/// timers.json v2: `{ "version": 2, "widgets": { "<uuid>": WidgetTimers } }`.
#[derive(Debug, Serialize, Deserialize)]
struct TimerFile {
    version: u32,
    #[serde(default)]
    widgets: HashMap<Uuid, WidgetTimers>,
}

/// Indstillinger timeren læser fra widget-instansens (eller klynge-modulets)
/// settings ved brand-tidspunktet, så ændringer i kontrolcentret altid
/// gælder for næste ring. Kun lyd-relaterede felter — resten er væk med
/// pomodoro.
#[derive(Debug, Clone)]
pub struct TimerSettings {
    pub sound: String,
    pub volume: u8,
    pub notify: bool,
}

impl TimerSettings {
    fn from_value(value: &serde_json::Value) -> Self {
        Self {
            sound: value
                .get("sound")
                .and_then(|v| v.as_str())
                .unwrap_or("chime")
                .to_string(),
            volume: value
                .get("volume")
                .and_then(|v| v.as_u64())
                .map(|v| v.min(100) as u8)
                .unwrap_or(80),
            notify: value
                .get("notify")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }
    }
}

impl Default for TimerSettings {
    fn default() -> Self {
        Self::from_value(&serde_json::Value::Null)
    }
}

/// Finder settings for et timer-id — enten en fri widget eller et klynge-modul.
fn find_timer_settings(config: &Config, id: Uuid) -> Option<serde_json::Value> {
    for widget in &config.widgets {
        if widget.id == id && widget.kind == WidgetKind::Timer {
            return Some(widget.settings.clone());
        }
        if let Some(cluster) = &widget.cluster {
            for module in &cluster.modules {
                if module.id == id && module.kind == WidgetKind::Timer {
                    return Some(module.settings.clone());
                }
            }
        }
    }
    None
}

pub struct TimerManager {
    app: AppHandle,
    entries: Mutex<HashMap<Uuid, WidgetTimers>>,
    rearm: Notify,
}

impl TimerManager {
    pub fn new(app: AppHandle) -> Self {
        let entries = load_persisted();
        Self {
            app,
            entries: Mutex::new(entries),
            rearm: Notify::new(),
        }
    }

    fn persist_path() -> PathBuf {
        crate::paths::data_root().join(PERSIST_FILE)
    }

    fn persist(&self) {
        let entries = self.entries.lock();
        let payload = match serde_json::to_vec_pretty(&TimerFile {
            version: PERSIST_VERSION,
            widgets: entries.clone(),
        }) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!(target: "timer", "Kunne ikke serialisere timers.json: {error}");
                return;
            }
        };
        drop(entries);
        let path = Self::persist_path();
        if let Some(dir) = path.parent() {
            if let Err(error) = fs::create_dir_all(dir) {
                log::warn!(target: "timer", "Kunne ikke oprette datamappe: {error}");
                return;
            }
        }
        let tmp = path.with_extension("json.tmp");
        if let Err(error) = fs::write(&tmp, payload).and_then(|_| fs::rename(&tmp, &path)) {
            log::warn!(target: "timer", "Kunne ikke gemme timers.json: {error}");
        }
    }

    fn emit(&self, id: Uuid) {
        let snapshot = self.snapshot(id);
        if let Err(error) = self.app.emit("timer://state", &snapshot) {
            log::warn!(target: "timer", "Kunne ikke emit'e timer-state: {error}");
        }
    }

    /// Opdaterer tilstanden for `id`, persisterer, emitter og vækker
    /// scheduleren. `entries.entry(id).or_default()` opretter en tom post ved
    /// behov — harmløs, forsvinder ved næste reload hvis den forbliver tom.
    fn commit<R>(&self, id: Uuid, mutate: impl FnOnce(&mut WidgetTimers) -> R) -> R {
        let result = {
            let mut entries = self.entries.lock();
            mutate(entries.entry(id).or_default())
        };
        self.persist();
        self.emit(id);
        self.rearm.notify_one();
        result
    }

    pub fn settings_for(&self, id: Uuid) -> TimerSettings {
        let config = self.app.state::<crate::state::AppState>().config.get();
        find_timer_settings(&config, id)
            .map(|value| TimerSettings::from_value(&value))
            .unwrap_or_default()
    }

    pub fn snapshot(&self, id: Uuid) -> TimerListSnapshot {
        self.entries
            .lock()
            .get(&id)
            .map(|timers| timers.snapshot(id))
            .unwrap_or_else(|| TimerListSnapshot {
                widget_id: id,
                items: Vec::new(),
                recents: Vec::new(),
            })
    }

    pub fn add(&self, id: Uuid, label: &str, duration_ms: i64) -> Result<(), AppError> {
        let now = now_ms();
        self.commit(id, |timers| timers.add(label, duration_ms, now))
            .map(|_| ())
    }

    pub fn pause(&self, id: Uuid, item: Uuid) {
        let now = now_ms();
        self.commit(id, |timers| {
            timers.pause(item, now);
        });
    }

    pub fn resume(&self, id: Uuid, item: Uuid) {
        let now = now_ms();
        self.commit(id, |timers| {
            timers.resume(item, now);
        });
    }

    pub fn adjust(&self, id: Uuid, item: Uuid, delta_ms: i64) {
        let now = now_ms();
        self.commit(id, |timers| {
            timers.adjust(item, delta_ms, now);
        });
    }

    /// Fjerner én timer-række (ikke hele widgetten — det er `remove`).
    pub fn remove_item(&self, id: Uuid, item: Uuid) {
        self.commit(id, |timers| {
            timers.remove(item);
        });
    }

    /// Fjerner hele widgettens timer-post — bruges af widget_remove.
    pub fn remove(&self, id: Uuid) {
        let existed = {
            let mut entries = self.entries.lock();
            entries.remove(&id).is_some()
        };
        if existed {
            self.persist();
            self.rearm.notify_one();
        }
    }

    /// Én scheduler-pass over alle widgets: timers der er udløbet finishes
    /// med deres rigtige ends_at, med én samlet lyd og én notifikation pr.
    /// timer. `due` er (widget-id, item-id, ends_at).
    fn finish_due(&self, due: Vec<(Uuid, Uuid, i64)>) {
        if due.is_empty() {
            return;
        }
        let mut finished: Vec<(Uuid, String)> = Vec::new();
        {
            let mut entries = self.entries.lock();
            for (widget_id, item_id, ends_at) in &due {
                let Some(timers) = entries.get_mut(widget_id) else {
                    continue;
                };
                if timers.finish(*item_id, *ends_at) {
                    let label = timers
                        .items
                        .iter()
                        .find(|item| item.id == *item_id)
                        .map(|item| item.label.clone())
                        .unwrap_or_default();
                    finished.push((*widget_id, label));
                }
            }
        }
        if finished.is_empty() {
            return;
        }
        // Én lyd pr. pass uanset antal udløbne timere — første berørte
        // widgets indstillinger afgør lyd og volumen.
        let first_settings = self.settings_for(finished[0].0);
        super::sound::play(&self.app, &first_settings.sound, first_settings.volume);
        for (widget_id, label) in &finished {
            if self.settings_for(*widget_id).notify {
                self.notify_finished(label);
            }
        }
        self.persist();
        let mut emitted: Vec<Uuid> = Vec::new();
        for (widget_id, _) in &finished {
            if !emitted.contains(widget_id) {
                emitted.push(*widget_id);
                self.emit(*widget_id);
            }
        }
        self.rearm.notify_one();
    }

    /// Gentag-ringer for finished-timere: én lyd pr. pass, repeat_count
    /// inkrementeres på alle forfaldne. `due` er (widget-id, item-id).
    fn ring_repeats(&self, due: Vec<(Uuid, Uuid)>) {
        if due.is_empty() {
            return;
        }
        let mut affected: Vec<Uuid> = Vec::new();
        {
            let mut entries = self.entries.lock();
            let now = now_ms();
            for (widget_id, item_id) in &due {
                let Some(timers) = entries.get_mut(widget_id) else {
                    continue;
                };
                let Some(item) = timers.items.iter_mut().find(|i| i.id == *item_id) else {
                    continue;
                };
                if item.next_repeat_at().is_some_and(|t| t <= now) {
                    item.repeat_count = item.repeat_count.saturating_add(1);
                    if !affected.contains(widget_id) {
                        affected.push(*widget_id);
                    }
                }
            }
        }
        if affected.is_empty() {
            return;
        }
        let settings = self.settings_for(affected[0]);
        super::sound::play(&self.app, &settings.sound, settings.volume);
        self.persist();
        for widget_id in &affected {
            self.emit(*widget_id);
        }
        self.rearm.notify_one();
    }

    fn notify_finished(&self, label: &str) {
        use tauri_plugin_notification::NotificationExt;
        let language = self
            .app
            .state::<crate::state::AppState>()
            .config
            .get()
            .general
            .language;
        let (title, fallback) = match language {
            Language::En => ("Time is up", "Your timer is done"),
            Language::Da => ("Tiden er gået", "Din timer er færdig"),
        };
        let body = if label.trim().is_empty() {
            fallback.to_string()
        } else {
            label.to_string()
        };
        if let Err(error) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(&body)
            .show()
        {
            log::warn!(target: "timer", "Kunne ikke vise timer-notifikation: {error}");
        }
    }

    /// Ved opstart: fyr timere der udløb mens appen var lukket — samme
    /// due-behandling som scheduler-passet.
    fn fire_overdue(&self) {
        let now = now_ms();
        let due: Vec<(Uuid, Uuid, i64)> = {
            let entries = self.entries.lock();
            entries
                .iter()
                .flat_map(|(widget_id, timers)| {
                    timers
                        .due(now)
                        .into_iter()
                        .map(|(item_id, ends_at)| (*widget_id, item_id, ends_at))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        for (_, item_id, _) in &due {
            log::info!(target: "timer", "Timer {item_id} udløb mens appen var lukket — fyrer nu");
        }
        self.finish_due(due);
    }
}

fn decode_persist(bytes: &[u8]) -> HashMap<Uuid, WidgetTimers> {
    match serde_json::from_slice::<TimerFile>(bytes) {
        Ok(file) if file.version == PERSIST_VERSION => file.widgets,
        _ => {
            log::info!(target: "timer", "timers.json fra tidligere format ignoreres");
            HashMap::new()
        }
    }
}

fn load_persisted() -> HashMap<Uuid, WidgetTimers> {
    let path = crate::paths::data_root().join(PERSIST_FILE);
    let Ok(bytes) = fs::read(&path) else {
        return HashMap::new();
    };
    decode_persist(&bytes)
}

/// Installerer manageren som Tauri-state og starter scheduler-loopet.
pub fn install(app: &AppHandle) {
    super::sound::clean_cache();
    let manager = TimerManager::new(app.clone());
    manager.fire_overdue();
    app.manage(manager);

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let manager = app_handle.state::<TimerManager>();
            let now = now_ms();
            let mut wake_at: Option<i64> = None;
            let mut due: Vec<(Uuid, Uuid, i64)> = Vec::new();
            let mut repeat_due: Vec<(Uuid, Uuid)> = Vec::new();

            {
                let entries = manager.entries.lock();
                for (widget_id, timers) in entries.iter() {
                    for (item_id, ends_at) in timers.due(now) {
                        due.push((*widget_id, item_id, ends_at));
                    }
                    for item in &timers.items {
                        if let Some(next) = item.next_repeat_at() {
                            if next <= now {
                                repeat_due.push((*widget_id, item.id));
                            }
                        }
                    }
                    if let Some(next) = timers.next_wake(now) {
                        wake_at = Some(wake_at.map_or(next, |w: i64| w.min(next)));
                    }
                }
            }

            manager.finish_due(due);
            manager.ring_repeats(repeat_due);

            // notify_one() gemmer én permit selv uden ventere, så en commit der
            // lander mens vi behandler, vækker os straks ved næste await.
            let rearm_signal = manager.rearm.notified();
            match wake_at {
                Some(target) => {
                    let delay = (target - now_ms()).max(50) as u64;
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(delay),
                        rearm_signal,
                    )
                    .await;
                }
                None => {
                    rearm_signal.await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn timers() -> WidgetTimers {
        WidgetTimers::default()
    }

    fn running(timers: &mut WidgetTimers, label: &str, duration_ms: i64) -> Uuid {
        timers.add(label, duration_ms, NOW).expect("add skal virke")
    }

    fn item_mut<'a>(timers: &'a mut WidgetTimers, id: Uuid) -> &'a mut TimerItem {
        timers.items.iter_mut().find(|i| i.id == id).expect("item")
    }

    fn item(timers: &WidgetTimers, id: Uuid) -> &TimerItem {
        timers.items.iter().find(|i| i.id == id).expect("item")
    }

    #[test]
    fn add_ok_running() {
        let mut t = timers();
        let id = running(&mut t, "Pasta", 600_000);
        let it = item(&t, id);
        assert_eq!(it.status, TimerStatus::Running);
        assert_eq!(it.ends_at, Some(NOW + 600_000));
        assert_eq!(it.remaining_ms, 600_000);
        assert_eq!(it.total_ms, 600_000);
        assert_eq!(it.initial_ms, 600_000);
        assert_eq!(it.created_at, NOW);
        assert_eq!(it.label, "Pasta");
    }

    #[test]
    fn add_trimmer_label_og_kapper_ved_60_tegn() {
        let mut t = timers();
        let long = format!("  {}  ", "x".repeat(80));
        let id = t.add(&long, 60_000, NOW).unwrap();
        assert_eq!(item(&t, id).label.chars().count(), 60);
        let id2 = t.add("  te  ", 60_000, NOW).unwrap();
        assert_eq!(item(&t, id2).label, "te");
    }

    #[test]
    fn add_afviser_varigheder_uden_for_omraade() {
        let mut t = timers();
        assert!(t.add("a", 999, NOW).is_err());
        assert!(t.add("a", 0, NOW).is_err());
        assert!(t.add("a", 86_400_001, NOW).is_err());
        assert!(t.add("a", 1_000, NOW).is_ok());
        assert!(t.add("a", 86_400_000, NOW).is_ok());
    }

    #[test]
    fn add_maks_otte_timere() {
        let mut t = timers();
        for i in 0..8 {
            assert!(t.add(&format!("t{i}"), 60_000, NOW).is_ok());
        }
        assert!(t.add("for mange", 60_000, NOW).is_err());
    }

    #[test]
    fn recents_dedupe_rækkefølge_max6() {
        let mut t = timers();
        running(&mut t, "Pasta", 600_000);
        running(&mut t, "Te", 90_000);
        running(&mut t, "pasta", 300_000); // case-insensitiv dedupe
        assert_eq!(t.recents.len(), 2);
        assert_eq!(t.recents[0].label, "pasta");
        assert_eq!(t.recents[0].duration_ms, 300_000);
        assert_eq!(t.recents[1].label, "Te");
        // seks forskellige navne, nyeste først, ældste ryger ud
        let mut t2 = timers();
        for i in 0..7 {
            running(&mut t2, &format!("navn{i}"), 60_000);
        }
        assert_eq!(t2.recents.len(), 6);
        assert_eq!(t2.recents[0].label, "navn6");
        assert!(!t2.recents.iter().any(|r| r.label == "navn0"));
    }

    #[test]
    fn tom_label_kommer_ikke_i_recents() {
        let mut t = timers();
        running(&mut t, "   ", 60_000);
        assert!(t.recents.is_empty());
        running(&mut t, "", 60_000);
        assert!(t.recents.is_empty());
    }

    #[test]
    fn pause_resume() {
        let mut t = timers();
        let id = running(&mut t, "a", 600_000);
        // 100s inde i en 600s timer
        let later = NOW + 100_000;
        assert!(t.pause(id, later));
        let it = item(&t, id);
        assert_eq!(it.status, TimerStatus::Paused);
        assert_eq!(it.ends_at, None);
        assert_eq!(it.remaining_ms, 500_000);
        // pause på paused = no-op
        assert!(!t.pause(id, later));
        // resume
        let later2 = later + 50_000;
        assert!(t.resume(id, later2));
        let it = item(&t, id);
        assert_eq!(it.status, TimerStatus::Running);
        assert_eq!(it.ends_at, Some(later2 + 500_000));
        // resume på running = no-op
        assert!(!t.resume(id, later2));
    }

    #[test]
    fn adjust_running_plus_minus() {
        let mut t = timers();
        let id = running(&mut t, "a", 600_000);
        // 100s tilbage
        let now = NOW + 500_000;
        assert!(t.adjust(id, 60_000, now)); // +1 min → 160s
        let it = item(&t, id);
        assert_eq!(it.ends_at, Some(now + 160_000));
        assert_eq!(it.remaining_ms, 160_000);
        assert_eq!(it.total_ms, 660_000); // total vokser med delta
        assert!(t.adjust(id, -60_000, now)); // -1 min → 100s
        let it = item(&t, id);
        assert_eq!(it.remaining_ms, 100_000);
        assert_eq!(it.ends_at, Some(now + 100_000));
        assert_eq!(it.total_ms, 600_000);
    }

    #[test]
    fn adjust_clamper_til_1s() {
        let mut t = timers();
        let id = running(&mut t, "a", 60_000);
        let now = NOW + 59_500; // 500ms tilbage
        assert!(t.adjust(id, -60_000, now));
        let it = item(&t, id);
        assert_eq!(it.remaining_ms, 1_000);
        assert_eq!(it.ends_at, Some(now + 1_000));
    }

    #[test]
    fn adjust_paused() {
        let mut t = timers();
        let id = running(&mut t, "a", 600_000);
        let now = NOW + 100_000;
        t.pause(id, now);
        assert!(t.adjust(id, 60_000, now + 10_000));
        let it = item(&t, id);
        assert_eq!(it.status, TimerStatus::Paused);
        assert_eq!(it.ends_at, None);
        assert_eq!(it.remaining_ms, 560_000); // 500k + 60k
        assert_eq!(it.total_ms, 660_000);
    }

    #[test]
    fn adjust_finished_genstarter_ved_positiv_delta() {
        let mut t = timers();
        let id = running(&mut t, "a", 600_000);
        let ends = NOW + 600_000;
        assert!(t.finish(id, ends));
        let now = ends + 30_000;
        // negativ delta på finished = no-op
        assert!(!t.adjust(id, -60_000, now));
        assert_eq!(item(&t, id).status, TimerStatus::Finished);
        // +5 min genstarter
        assert!(t.adjust(id, 300_000, now));
        let it = item(&t, id);
        assert_eq!(it.status, TimerStatus::Running);
        assert_eq!(it.ends_at, Some(now + 300_000));
        assert_eq!(it.remaining_ms, 300_000);
        assert_eq!(it.total_ms, 300_000);
        assert_eq!(it.finished_at, None);
        assert_eq!(it.repeat_count, 0);
        // initial_ms bevares
        assert_eq!(it.initial_ms, 600_000);
    }

    #[test]
    fn finish_og_due() {
        let mut t = timers();
        let id1 = running(&mut t, "a", 60_000);
        let id2 = running(&mut t, "b", 120_000);
        let id3 = running(&mut t, "c", 30_000);
        t.pause(id3, NOW);
        let now = NOW + 70_000;
        let due = t.due(now);
        assert_eq!(due, vec![(id1, NOW + 60_000)]);
        // finish sætter finished_at til udløbstid
        assert!(t.finish(id1, NOW + 60_000));
        let it = item(&t, id1);
        assert_eq!(it.status, TimerStatus::Finished);
        assert_eq!(it.finished_at, Some(NOW + 60_000));
        assert_eq!(it.remaining_ms, 0);
        assert_eq!(it.ends_at, None);
        // id2 udløber ved 120s, id3 er paused
        assert_eq!(t.due(now + 60_000), vec![(id2, NOW + 120_000)]);
    }

    #[test]
    fn next_repeat_at_vindue() {
        let mut t = timers();
        let id = running(&mut t, "a", 60_000);
        let finished_at = NOW + 60_000;
        assert!(t.finish(id, finished_at));
        let it = item(&t, id);
        assert_eq!(it.next_repeat_at(), Some(finished_at + 10_000));
        item_mut(&mut t, id).repeat_count = 5;
        assert_eq!(item(&t, id).next_repeat_at(), Some(finished_at + 60_000));
        item_mut(&mut t, id).repeat_count = 6;
        assert_eq!(item(&t, id).next_repeat_at(), None);
        // kun finished
        let id2 = running(&mut t, "b", 60_000);
        assert_eq!(item(&t, id2).next_repeat_at(), None);
    }

    #[test]
    fn next_wake_tager_min_af_ends_og_repeat() {
        let mut t = timers();
        let running_id = running(&mut t, "a", 100_000);
        let finished_id = running(&mut t, "b", 50_000);
        t.finish(finished_id, NOW + 50_000);
        // finished repeat ved +60s, running ends ved +100s → wake = 60s
        assert_eq!(t.next_wake(NOW), Some(NOW + 60_000));
        // efter running fjernes står kun repeat tilbage
        t.remove(running_id);
        assert_eq!(t.next_wake(NOW), Some(NOW + 60_000));
        // uden noget → None
        let t2 = timers();
        assert_eq!(t2.next_wake(NOW), None);
    }

    #[test]
    fn settings_parse_defaults() {
        let settings = TimerSettings::from_value(&serde_json::json!({}));
        assert_eq!(settings.sound, "chime");
        assert_eq!(settings.volume, 80);
        assert!(settings.notify);
    }

    #[test]
    fn settings_parse_volume_clamp() {
        let settings = TimerSettings::from_value(&serde_json::json!({
            "sound": "marimba", "volume": 140, "notify": false
        }));
        assert_eq!(settings.sound, "marimba");
        assert_eq!(settings.volume, 100);
        assert!(!settings.notify);
        let settings2 = TimerSettings::from_value(&serde_json::json!({ "volume": 30 }));
        assert_eq!(settings2.volume, 30);
    }

    #[test]
    fn remove_item() {
        let mut t = timers();
        let id = running(&mut t, "a", 60_000);
        assert!(t.remove(id));
        assert!(!t.remove(id));
        assert!(t.items.is_empty());
    }

    #[test]
    fn persist_v1_fil_ignoreres() {
        // gammelt format: bart HashMap<Uuid, TimerEntry> uden version/widgets
        let v1 = br#"{"550e8400-e29b-41d4-a716-446655440000":{"label":"old","durationMs":60000}}"#;
        assert!(decode_persist(v1).is_empty());
        // forkert version
        let wrong_version = br#"{"version":1,"widgets":{}}"#;
        assert!(decode_persist(wrong_version).is_empty());
        // ødelagt json
        assert!(decode_persist(b"not json").is_empty());
        // gyldig v2
        let mut t = timers();
        running(&mut t, "a", 60_000);
        let widget_id = Uuid::new_v4();
        let file = TimerFile {
            version: PERSIST_VERSION,
            widgets: HashMap::from([(widget_id, t)]),
        };
        let bytes = serde_json::to_vec(&file).unwrap();
        let decoded = decode_persist(&bytes);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[&widget_id].items.len(), 1);
    }
}
