//! Timer-tilstand pr. widget-id, ejet af Rust.
//!
//! Timeren lever uafhængigt af widget-vinduet: den fortsætter når widgetten
//! genindlæses, når appen er på pause, og (via `timers.json`) når appen
//! genstartes. Frontend tæller sekunder lokalt ud fra `ends_at` — der sendes
//! ingen tick-events. Én tokio-task sover til nærmeste deadline og vækkes
//! via `Notify` når tilstanden ændres.
//!
//! Segmenter: `countdown` (nedtælling) eller pomodoro-cyklussen
//! `focus → break/longBreak`. Efter `rounds` fokus-runder kommer lang pause,
//! og derefter er cyklussen færdig (idle). Fokus starter aldrig automatisk
//! efter en pause — brugeren trykker play (matcher plan/06 §5).
//!
//! `round` er 1-baseret indeks for den aktuelle/senest afsluttede fokus-runde.
//! Antal afsluttede runder til prik-UI'et: `round - 1` mens segment == focus,
//! ellers `round`.

use std::{collections::HashMap, fs, path::PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;
use ts_rs::TS;
use uuid::Uuid;

use crate::config::schema::{Config, Language, WidgetKind};

const MAX_DURATION_MS: i64 = 12 * 60 * 60 * 1000; // 12 timer
const MIN_DURATION_MS: i64 = 1000;
const REPEAT_INTERVAL_MS: i64 = 10_000;
const REPEAT_WINDOW_MS: i64 = 60_000;
const PERSIST_FILE: &str = "timers.json";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum TimerSegment {
    Countdown,
    Focus,
    Break,
    LongBreak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum TimerStatus {
    Idle,
    Running,
    Paused,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct TimerSnapshot {
    pub widget_id: Uuid,
    pub segment: TimerSegment,
    pub status: TimerStatus,
    pub ends_at: Option<i64>,
    pub remaining_ms: i64,
    pub total_ms: i64,
    pub round: u8,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimerEntry {
    segment: TimerSegment,
    status: TimerStatus,
    ends_at: Option<i64>,
    remaining_ms: i64,
    total_ms: i64,
    round: u8,
    finished_at: Option<i64>,
    #[serde(default)]
    repeat_count: u8,
    /// Om den seneste fokus-runde blev fuldført naturligt. Skip sætter den ikke,
    /// og rundtællingen stiger kun ved overgangen pause → fokus hvis den er sat.
    #[serde(default)]
    round_counted: bool,
}

impl TimerEntry {
    fn idle() -> Self {
        Self {
            segment: TimerSegment::Countdown,
            status: TimerStatus::Idle,
            ends_at: None,
            remaining_ms: 0,
            total_ms: 0,
            round: 0,
            finished_at: None,
            repeat_count: 0,
            round_counted: false,
        }
    }

    fn snapshot(&self, widget_id: Uuid) -> TimerSnapshot {
        TimerSnapshot {
            widget_id,
            segment: self.segment,
            status: self.status,
            ends_at: self.ends_at,
            remaining_ms: self.remaining_ms,
            total_ms: self.total_ms,
            round: self.round,
            finished_at: self.finished_at,
        }
    }
}

/// Indstillinger timeren læser fra widget-instansens settings ved brand-tidspunktet,
/// så ændringer i kontrolcentret altid gælder for næste ring.
#[derive(Debug, Clone)]
pub struct TimerSettings {
    pub mode: TimerMode,
    pub sound: String,
    pub volume: u8,
    pub notify: bool,
    pub focus_ms: i64,
    pub break_ms: i64,
    pub long_break_ms: i64,
    pub rounds: u8,
    pub auto_start_break: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerMode {
    Countdown,
    Pomodoro,
}

impl TimerSettings {
    fn from_value(value: &serde_json::Value) -> Self {
        let minutes = |key: &str, default: i64| -> i64 {
            value
                .get(key)
                .and_then(|v| v.as_f64())
                .map(|m| (m as i64).clamp(1, 180) * 60_000)
                .unwrap_or(default * 60_000)
        };
        Self {
            mode: if value.get("mode").and_then(|v| v.as_str()) == Some("pomodoro") {
                TimerMode::Pomodoro
            } else {
                TimerMode::Countdown
            },
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
            focus_ms: minutes("focusMin", 25),
            break_ms: minutes("breakMin", 5),
            long_break_ms: minutes("longBreakMin", 15),
            rounds: value
                .get("rounds")
                .and_then(|v| v.as_u64())
                .map(|v| v.clamp(2, 8) as u8)
                .unwrap_or(4),
            auto_start_break: value
                .get("autoStartBreak")
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
    entries: Mutex<HashMap<Uuid, TimerEntry>>,
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
        let map: &HashMap<Uuid, TimerEntry> = &entries;
        let payload = match serde_json::to_vec_pretty(map) {
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

    /// Opdaterer tilstand, persisterer, emitter og vækker scheduleren.
    fn commit(&self, id: Uuid, mutate: impl FnOnce(&mut TimerEntry)) {
        {
            let mut entries = self.entries.lock();
            let entry = entries.entry(id).or_insert_with(TimerEntry::idle);
            mutate(entry);
        }
        self.persist();
        self.emit(id);
        self.rearm.notify_one();
    }

    pub fn settings_for(&self, id: Uuid) -> TimerSettings {
        let config = self.app.state::<crate::state::AppState>().config.get();
        find_timer_settings(&config, id)
            .map(|value| TimerSettings::from_value(&value))
            .unwrap_or_default()
    }

    pub fn snapshot(&self, id: Uuid) -> TimerSnapshot {
        let entries = self.entries.lock();
        entries
            .get(&id)
            .map(|entry| entry.snapshot(id))
            .unwrap_or_else(|| TimerEntry::idle().snapshot(id))
    }

    pub fn start(&self, id: Uuid, duration_ms: Option<i64>) {
        let settings = self.settings_for(id);
        let now = now_ms();
        self.commit(id, |entry| {
            match settings.mode {
                TimerMode::Countdown => {
                    let duration = duration_ms
                        .map(|d| d.clamp(MIN_DURATION_MS, MAX_DURATION_MS))
                        .or(if entry.total_ms >= MIN_DURATION_MS {
                            Some(entry.total_ms)
                        } else {
                            None
                        })
                        .unwrap_or(25 * 60_000);
                    entry.segment = TimerSegment::Countdown;
                    entry.total_ms = duration;
                    entry.round = 0;
                }
                TimerMode::Pomodoro => {
                    entry.segment = TimerSegment::Focus;
                    entry.total_ms = settings.focus_ms;
                    entry.round = 1;
                }
            }
            entry.status = TimerStatus::Running;
            entry.remaining_ms = entry.total_ms;
            entry.ends_at = Some(now + entry.total_ms);
            entry.finished_at = None;
            entry.repeat_count = 0;
        });
    }

    pub fn pause(&self, id: Uuid) {
        let now = now_ms();
        self.commit(id, |entry| {
            if entry.status != TimerStatus::Running {
                return;
            }
            if let Some(ends_at) = entry.ends_at {
                entry.remaining_ms = (ends_at - now).max(0);
            }
            entry.ends_at = None;
            entry.status = TimerStatus::Paused;
        });
    }

    pub fn resume(&self, id: Uuid) {
        let now = now_ms();
        self.commit(id, |entry| {
            if entry.status != TimerStatus::Paused || entry.remaining_ms <= 0 {
                return;
            }
            entry.ends_at = Some(now + entry.remaining_ms);
            entry.status = TimerStatus::Running;
        });
    }

    pub fn reset(&self, id: Uuid) {
        self.commit(id, |entry| {
            let total = entry.total_ms;
            let segment = entry.segment;
            *entry = TimerEntry::idle();
            entry.total_ms = total;
            entry.segment = segment;
        });
    }

    pub fn dismiss(&self, id: Uuid) {
        self.commit(id, |entry| {
            if entry.status != TimerStatus::Finished {
                return;
            }
            let total = entry.total_ms;
            *entry = TimerEntry::idle();
            entry.total_ms = total;
        });
    }

    /// Snooze starter altid en kort nedtælling — uanset tilstand.
    pub fn snooze(&self, id: Uuid, minutes: i64) {
        let now = now_ms();
        let duration = (minutes.clamp(1, 60)) * 60_000;
        self.commit(id, |entry| {
            entry.segment = TimerSegment::Countdown;
            entry.status = TimerStatus::Running;
            entry.total_ms = duration;
            entry.remaining_ms = duration;
            entry.ends_at = Some(now + duration);
            entry.round = 0;
            entry.finished_at = None;
            entry.repeat_count = 0;
        });
    }

    /// Pomodoro: spring til næste segment uden at runden tæller som afsluttet
    /// på samme måde — men overgangen er den samme som ved en egentlig udløb.
    pub fn skip(&self, id: Uuid) {
        let entry_state = {
            let entries = self.entries.lock();
            entries.get(&id).map(|entry| (entry.segment, entry.status))
        };
        let Some((segment, status)) = entry_state else {
            return;
        };
        if status == TimerStatus::Idle {
            return;
        }
        self.advance_segment(id, segment, false);
    }

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

    /// Overgangslogik når et segment udløber (eller skippes).
    /// `counted` = udløbet naturligt (vs. skip — samme overgang, men skip
    /// ringer ikke og viser ikke notifikation).
    fn advance_segment(&self, id: Uuid, segment: TimerSegment, counted: bool) {
        let settings = self.settings_for(id);
        self.commit(id, |entry| {
            entry.ends_at = None;
            entry.finished_at = None;
            entry.repeat_count = 0;
            match segment {
                TimerSegment::Focus => {
                    let last_round = entry.round >= settings.rounds;
                    entry.segment = if last_round {
                        TimerSegment::LongBreak
                    } else {
                        TimerSegment::Break
                    };
                    entry.total_ms = if last_round {
                        settings.long_break_ms
                    } else {
                        settings.break_ms
                    };
                    entry.remaining_ms = entry.total_ms;
                    if settings.auto_start_break && counted {
                        entry.ends_at = Some(now_ms() + entry.total_ms);
                        entry.status = TimerStatus::Running;
                    } else {
                        // Venter på brugeren: viser næste segment klar til start.
                        entry.status = TimerStatus::Paused;
                    }
                }
                TimerSegment::Break => {
                    entry.segment = TimerSegment::Focus;
                    if entry.round_counted {
                        entry.round = entry.round.saturating_add(1).min(settings.rounds);
                    }
                    entry.round_counted = false;
                    entry.total_ms = settings.focus_ms;
                    entry.remaining_ms = entry.total_ms;
                    // Fokus starter aldrig automatisk efter en pause.
                    entry.status = TimerStatus::Paused;
                }
                TimerSegment::LongBreak => {
                    let total = settings.focus_ms;
                    *entry = TimerEntry::idle();
                    entry.total_ms = total;
                }
                TimerSegment::Countdown => {}
            }
        });
    }

    /// Kaldes når ends_at nås: lyd + notifikation + overgang/ringetilstand.
    fn fire(&self, id: Uuid) {
        let settings = self.settings_for(id);
        let (segment, total_ms) = {
            let entries = self.entries.lock();
            match entries.get(&id) {
                Some(entry) => (entry.segment, entry.total_ms),
                None => return,
            }
        };

        super::sound::play(&self.app, &settings.sound, settings.volume);
        if settings.notify {
            self.notify_finished(id, segment, &settings);
        }

        if segment == TimerSegment::Countdown {
            let now = now_ms();
            self.commit(id, |entry| {
                entry.status = TimerStatus::Finished;
                entry.ends_at = None;
                entry.remaining_ms = 0;
                entry.total_ms = total_ms;
                entry.finished_at = Some(now);
                entry.repeat_count = 0;
            });
        } else {
            // En naturligt fuldført fokus-runde tæller — skip gør ikke (plan/06 §5).
            if segment == TimerSegment::Focus {
                let mut entries = self.entries.lock();
                if let Some(entry) = entries.get_mut(&id) {
                    entry.round_counted = true;
                }
            }
            self.advance_segment(id, segment, true);
        }
    }

    fn notify_finished(&self, id: Uuid, segment: TimerSegment, settings: &TimerSettings) {
        use tauri_plugin_notification::NotificationExt;
        let language = self
            .app
            .state::<crate::state::AppState>()
            .config
            .get()
            .general
            .language;
        let note = self.read_note(id);
        let (title, fallback) = match (language, segment) {
            (Language::En, TimerSegment::Countdown) => ("Time is up", "Your timer is done"),
            (Language::En, TimerSegment::Focus) => ("Focus done", "Time for a break"),
            (Language::En, TimerSegment::Break | TimerSegment::LongBreak) => {
                ("Break is over", "Ready for the next round")
            }
            (Language::Da, TimerSegment::Countdown) => ("Tiden er gået", "Din timer er færdig"),
            (Language::Da, TimerSegment::Focus) => ("Fokus færdig", "Tid til en pause"),
            (Language::Da, TimerSegment::Break | TimerSegment::LongBreak) => {
                ("Pausen er ovre", "Klar til næste runde")
            }
        };
        let body = note.unwrap_or_else(|| fallback.to_string());
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
        let _ = settings;
    }

    fn read_note(&self, id: Uuid) -> Option<String> {
        let notes_dir = self.app.state::<crate::state::AppState>().config.notes_dir();
        let path = notes_dir.join(format!("{id}.txt"));
        let content = fs::read_to_string(path).ok()?;
        let first_line = content.lines().next()?.trim();
        if first_line.is_empty() {
            return None;
        }
        let truncated: String = first_line.chars().take(120).collect();
        Some(truncated)
    }

    /// Næste ringetidspunkt for en finished-countdown (gentagelse hvert 10. s,
    /// op til 60 s efter finished_at). `None` hvis vinduet er ovre.
    fn next_repeat_at(entry: &TimerEntry) -> Option<i64> {
        if entry.status != TimerStatus::Finished || entry.segment != TimerSegment::Countdown {
            return None;
        }
        let finished_at = entry.finished_at?;
        let next = finished_at + (entry.repeat_count as i64 + 1) * REPEAT_INTERVAL_MS;
        if next <= finished_at + REPEAT_WINDOW_MS {
            Some(next)
        } else {
            None
        }
    }

    /// Ved opstart: fyr timere der udløb mens appen var lukket.
    /// De ringer én gang og viser notifikation — ellers ville brugeren aldrig høre dem.
    fn fire_overdue(&self) {
        let now = now_ms();
        let overdue: Vec<Uuid> = {
            let entries = self.entries.lock();
            entries
                .iter()
                .filter(|(_, entry)| {
                    entry.status == TimerStatus::Running
                        && entry.ends_at.is_some_and(|ends| ends <= now)
                })
                .map(|(id, _)| *id)
                .collect()
        };
        for id in overdue {
            log::info!(target: "timer", "Timer {id} udløb mens appen var lukket — fyrer nu");
            self.fire(id);
        }
    }
}

fn load_persisted() -> HashMap<Uuid, TimerEntry> {
    let path = crate::paths::data_root().join(PERSIST_FILE);
    let Ok(bytes) = fs::read(&path) else {
        return HashMap::new();
    };
    match serde_json::from_slice::<HashMap<Uuid, TimerEntry>>(&bytes) {
        Ok(map) => map,
        Err(error) => {
            log::warn!(target: "timer", "timers.json var korrupt ({error}) — starter forfra");
            HashMap::new()
        }
    }
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
            let mut due: Vec<Uuid> = Vec::new();
            let mut repeat_due: Vec<Uuid> = Vec::new();

            {
                let entries = manager.entries.lock();
                for (id, entry) in entries.iter() {
                    if entry.status == TimerStatus::Running {
                        if let Some(ends) = entry.ends_at {
                            if ends <= now {
                                due.push(*id);
                            } else {
                                wake_at = Some(wake_at.map_or(ends, |w: i64| w.min(ends)));
                            }
                        }
                    }
                    if let Some(next_ring) = TimerManager::next_repeat_at(entry) {
                        if next_ring <= now {
                            repeat_due.push(*id);
                        } else {
                            wake_at = Some(wake_at.map_or(next_ring, |w: i64| w.min(next_ring)));
                        }
                    }
                }
            }

            for id in due {
                // Gen-tjek under lås: tilstanden kan have ændret sig siden scanningen.
                let still_due = {
                    let entries = manager.entries.lock();
                    entries.get(&id).is_some_and(|entry| {
                        entry.status == TimerStatus::Running
                            && entry.ends_at.is_some_and(|ends| ends <= now_ms())
                    })
                };
                if still_due {
                    manager.fire(id);
                }
            }

            for id in repeat_due {
                let should_ring = {
                    let mut entries = manager.entries.lock();
                    if let Some(entry) = entries.get_mut(&id) {
                        if TimerManager::next_repeat_at(entry).is_some_and(|t| t <= now_ms()) {
                            entry.repeat_count = entry.repeat_count.saturating_add(1);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if should_ring {
                    let settings = manager.settings_for(id);
                    super::sound::play(&manager.app, &settings.sound, settings.volume);
                    manager.persist();
                }
            }

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

    #[test]
    fn repeat_schedule_stops_after_window() {
        let mut entry = TimerEntry::idle();
        entry.status = TimerStatus::Finished;
        entry.finished_at = Some(1_000);
        entry.repeat_count = 0;
        assert_eq!(TimerManager::next_repeat_at(&entry), Some(11_000));
        entry.repeat_count = 5;
        assert_eq!(TimerManager::next_repeat_at(&entry), Some(61_000));
        entry.repeat_count = 6;
        assert_eq!(TimerManager::next_repeat_at(&entry), None);
    }

    #[test]
    fn repeat_only_for_countdown() {
        let mut entry = TimerEntry::idle();
        entry.status = TimerStatus::Finished;
        entry.segment = TimerSegment::Focus;
        entry.finished_at = Some(1_000);
        assert_eq!(TimerManager::next_repeat_at(&entry), None);
    }

    #[test]
    fn settings_parse_defaults() {
        let settings = TimerSettings::from_value(&serde_json::json!({}));
        assert_eq!(settings.sound, "chime");
        assert_eq!(settings.volume, 80);
        assert_eq!(settings.focus_ms, 25 * 60_000);
        assert!(matches!(settings.mode, TimerMode::Countdown));
    }

    #[test]
    fn settings_parse_pomodoro() {
        let settings = TimerSettings::from_value(&serde_json::json!({
            "mode": "pomodoro", "focusMin": 50, "rounds": 6, "volume": 140
        }));
        assert!(matches!(settings.mode, TimerMode::Pomodoro));
        assert_eq!(settings.focus_ms, 50 * 60_000);
        assert_eq!(settings.rounds, 6);
        assert_eq!(settings.volume, 100);
    }
}
