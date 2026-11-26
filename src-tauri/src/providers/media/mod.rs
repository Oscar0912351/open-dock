use std::{
    collections::HashMap,
    fs,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    thread::JoinHandle,
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use ts_rs::TS;

use crate::util::unix_now_ms;

mod aumid;
mod artwork;
mod color;
mod gsmtc;
mod persist;
mod youtube;

pub use aumid::format_source_name;
pub use color::extract_ambient_color;
pub use youtube::{clean_media_title, extract_matching_youtube_duration_ms, parse_duration_text};

const MAX_UNOBSERVED_GAP_MS: i64 = 120_000;

#[derive(Debug, Clone)]
struct TrackFallbackTimer {
    track_key: String,
    accumulated_ms: u64,
    playing_since_ms: Option<i64>,
    known_duration_ms: u64,
    anchored: bool,
    unverified: bool,
    timeline_backed: bool,
    last_seen_ms: i64,
}

impl TrackFallbackTimer {
    fn update_and_get(
        &mut self,
        track_key: &str,
        status: PlaybackStatus,
        now_ms: i64,
        known_pos_ms: Option<u64>,
    ) -> (u64, bool) {
        if self.track_key != track_key {
            self.track_key.clear();
            self.track_key.push_str(track_key);
            self.accumulated_ms = known_pos_ms.unwrap_or(0);
            self.playing_since_ms = (status == PlaybackStatus::Playing).then_some(now_ms);
            self.known_duration_ms = 0;
            self.anchored = known_pos_ms.is_some() || !self.unverified;
            self.timeline_backed = known_pos_ms.is_some();
            self.unverified = false;
            self.last_seen_ms = now_ms;
            return (self.accumulated_ms, self.anchored);
        }

        let gap = (now_ms - self.last_seen_ms).max(0);
        if gap > MAX_UNOBSERVED_GAP_MS && (self.unverified || self.playing_since_ms.is_some()) {
            self.anchored = false;
        }
        self.unverified = false;
        self.last_seen_ms = now_ms;

        if let Some(pos) = known_pos_ms {
            self.accumulated_ms = pos;
            self.playing_since_ms = (status == PlaybackStatus::Playing).then_some(now_ms);
            self.anchored = true;
            self.timeline_backed = true;
            return (pos, true);
        }

        match status {
            PlaybackStatus::Playing => {
                if self.playing_since_ms.is_none() {
                    self.playing_since_ms = Some(now_ms);
                }
            }
            PlaybackStatus::Paused => {
                if let Some(since) = self.playing_since_ms.take() {
                    let delta = (now_ms - since).max(0) as u64;
                    self.accumulated_ms = self.accumulated_ms.saturating_add(delta);
                }
            }
            PlaybackStatus::Stopped | PlaybackStatus::None => {
                self.accumulated_ms = 0;
                self.playing_since_ms = None;
            }
        }

        let pos = match self.playing_since_ms {
            Some(since) => {
                let delta = (now_ms - since).max(0) as u64;
                self.accumulated_ms.saturating_add(delta)
            }
            None => self.accumulated_ms,
        };
        if self.anchored {
            (pos, true)
        } else {
            (0, false)
        }
    }

    fn reset(&mut self, status: PlaybackStatus, now_ms: i64, pos_ms: u64) {
        self.accumulated_ms = pos_ms;
        self.playing_since_ms = (status == PlaybackStatus::Playing).then_some(now_ms);
    }
}

#[allow(clippy::too_many_arguments)]
fn update_fallback_timer(
    timers: &mut HashMap<String, TrackFallbackTimer>,
    source: &str,
    title: &str,
    artist: &str,
    status: PlaybackStatus,
    now_ms: i64,
    known_pos_ms: Option<u64>,
    fresh_is_start: bool,
) -> (u64, bool) {
    if source.is_empty() || (title.is_empty() && artist.is_empty()) {
        timers.remove(source);
        return (known_pos_ms.unwrap_or(0), known_pos_ms.is_some());
    }
    let track_key = format!("{}|{}", title.trim(), artist.trim());
    let entry = timers.entry(source.to_string()).or_insert_with(|| TrackFallbackTimer {
        track_key: track_key.clone(),
        accumulated_ms: known_pos_ms.unwrap_or(0),
        playing_since_ms: (status == PlaybackStatus::Playing).then_some(now_ms),
        known_duration_ms: 0,
        anchored: known_pos_ms.is_some() || fresh_is_start,
        unverified: false,
        timeline_backed: known_pos_ms.is_some(),
        last_seen_ms: now_ms,
    });
    entry.update_and_get(&track_key, status, now_ms, known_pos_ms)
}

fn mark_timeline_missing(timers: &mut HashMap<String, TrackFallbackTimer>, source: &str) {
    if let Some(timer) = timers.get_mut(source) {
        timer.timeline_backed = false;
    }
}

fn sync_fallback_duration(
    timers: &mut HashMap<String, TrackFallbackTimer>,
    source: &str,
    gsmtc_dur_ms: u64,
) -> u64 {
    if let Some(timer) = timers.get_mut(source) {
        if gsmtc_dur_ms > 0 {
            timer.known_duration_ms = gsmtc_dur_ms;
            return gsmtc_dur_ms;
        }
        return timer.known_duration_ms;
    }
    gsmtc_dur_ms
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum PlaybackStatus {
    None,
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum RepeatMode {
    None,
    Track,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct MediaSnapshot {
    pub status: PlaybackStatus,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: Option<String>,
    pub art_color: Option<[u8; 3]>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub position_known: bool,
    pub updated_at: i64,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub can_seek: bool,
    pub shuffle: Option<bool>,
    pub repeat: Option<RepeatMode>,
    pub available_sources: Vec<String>,
    pub sessions: Vec<MediaSessionInfo>,
}

impl Default for MediaSnapshot {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::None,
            source: String::new(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            art_url: None,
            art_color: None,
            position_ms: 0,
            duration_ms: 0,
            position_known: false,
            updated_at: unix_now_ms(),
            can_play_pause: false,
            can_next: false,
            can_prev: false,
            can_seek: false,
            shuffle: None,
            repeat: None,
            available_sources: Vec::new(),
            sessions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct MediaSessionInfo {
    pub source: String,
    pub title: String,
    pub artist: String,
    pub status: PlaybackStatus,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum MediaAction {
    PlayPause,
    Next,
    Previous,
    Seek {
        ms: u64,
    },
    Shuffle,
    Repeat,
}

pub(super) enum Cmd {
    Rescan,
    Refresh,
    SetPreferred(Option<String>),
    Control {
        action: MediaAction,
        source: Option<String>,
    },
    Shutdown,
}

pub struct MediaProvider {
    runtime: Mutex<Option<MediaWorker>>,
    latest: Mutex<Option<MediaSnapshot>>,
    generation: Arc<AtomicU64>,
}

struct MediaWorker {
    tx: Sender<Cmd>,
    stopped: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Default for MediaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaProvider {
    pub fn new() -> Self {
        Self {
            runtime: Mutex::new(None),
            latest: Mutex::new(None),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn snapshot(&self) -> Option<MediaSnapshot> {
        self.latest.lock().clone()
    }

    pub fn set_preferred(&self, source: Option<String>) {
        if let Some(worker) = self.runtime.lock().as_ref() {
            let _ = worker.tx.send(Cmd::SetPreferred(source));
        }
    }

    pub fn control(&self, app: &AppHandle, action: MediaAction, source: Option<String>) {
        // Optimistic immediate UI update before WinRT command settles.
        let optimistic = {
            let mut guard = self.latest.lock();
            if let Some(current) = guard.as_mut() {
                if current.status != PlaybackStatus::None {
                    let now = unix_now_ms();
                    let mut should_emit = true;
                    match &action {
                        MediaAction::PlayPause => {
                            if current.status == PlaybackStatus::Playing {
                                if current.position_known {
                                    let elapsed = (now - current.updated_at).max(0) as u64;
                                    let next_pos = current.position_ms.saturating_add(elapsed);
                                    current.position_ms = if current.duration_ms > 0 {
                                        next_pos.min(current.duration_ms)
                                    } else {
                                        next_pos
                                    };
                                }
                                current.status = PlaybackStatus::Paused;
                            } else {
                                current.status = PlaybackStatus::Playing;
                            }
                            current.updated_at = now;
                        }
                        MediaAction::Seek { ms } => {
                            if current.can_seek {
                                current.position_ms = if current.duration_ms > 0 {
                                    (*ms).min(current.duration_ms)
                                } else {
                                    *ms
                                };
                                current.updated_at = now;
                            } else {
                                should_emit = false;
                            }
                        }
                        MediaAction::Shuffle => {
                            if let Some(sh) = current.shuffle {
                                current.shuffle = Some(!sh);
                            }
                        }
                        MediaAction::Repeat => {
                            current.repeat = current.repeat.map(|mode| match mode {
                                RepeatMode::None => RepeatMode::List,
                                RepeatMode::List => RepeatMode::Track,
                                RepeatMode::Track => RepeatMode::None,
                            });
                        }
                        MediaAction::Next | MediaAction::Previous => {
                            current.position_ms = 0;
                            current.updated_at = now;
                        }
                    }
                    should_emit.then(|| current.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(snap) = optimistic {
            super::scheduler::emit_provider(
                app,
                super::scheduler::ProviderKind::Media,
                None,
                "provider://media",
                &snap,
            );
        }

        if let Some(worker) = self.runtime.lock().as_ref() {
            let _ = worker.tx.send(Cmd::Control { action, source });
        }
    }

    pub fn start(&self, app: &AppHandle, preferred_source: Option<String>) {
        let mut guard = self.runtime.lock();
        if let Some(existing) = guard.as_ref() {
            if let Some(pref) = preferred_source {
                let _ = existing.tx.send(Cmd::SetPreferred(Some(pref)));
            }
            return;
        }

        let (tx, rx) = mpsc::channel();
        let tx_clone = tx.clone();
        let app_handle = app.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped_thread = Arc::clone(&stopped);
        let my_gen = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let generation = Arc::clone(&self.generation);

        let art_dir = crate::paths::data_root().join("art");
        let _ = fs::create_dir_all(&art_dir);
        artwork::purge_legacy_art_files(&art_dir);

        match std::thread::Builder::new()
            .name("od-media".into())
            .spawn(move || {
                gsmtc::run_media_thread(
                    app_handle,
                    tx_clone,
                    rx,
                    art_dir,
                    preferred_source,
                    stopped_thread,
                    my_gen,
                    generation,
                );
            }) {
            Ok(join) => {
                log::info!(target: "provider:media", "Starter media-provider (od-media)");
                *guard = Some(MediaWorker {
                    tx,
                    stopped,
                    join: Some(join),
                });
            }
            Err(err) => {
                log::warn!(target: "provider:media", "Kunne ikke starte od-media-tråd: {err}");
            }
        }
    }

    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let worker = self.runtime.lock().take();
        if let Some(mut worker) = worker {
            log::info!(target: "provider:media", "Stopper media-provider");
            worker.stopped.store(true, Ordering::Release);
            let _ = worker.tx.send(Cmd::Shutdown);
            if let Some(join) = worker.join.take() {
                drop(join);
            }
            drop(worker);
        }
    }

    pub fn update_latest(&self, snapshot: MediaSnapshot) {
        *self.latest.lock() = Some(snapshot);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compute_timeline_ms(
    start_ms: u64,
    raw_pos_ms: u64,
    end_ms: u64,
    min_seek_ms: u64,
    max_seek_ms: u64,
    last_updated_ms: i64,
    now_ms: i64,
    status: PlaybackStatus,
) -> (u64, u64) {
    let effective_end = if end_ms > 0 { end_ms } else { max_seek_ms };
    let effective_start = if start_ms > 0 {
        start_ms
    } else if end_ms == 0 {
        min_seek_ms
    } else {
        0
    };

    let (pos, dur) = if effective_end > effective_start && effective_start > 0 {
        (
            raw_pos_ms.saturating_sub(effective_start),
            effective_end - effective_start,
        )
    } else {
        (raw_pos_ms, effective_end)
    };

    // Browsers like Firefox on Windows do not report GSMTC timeline properties
    // (EndTime=0, MaxSeekTime=0, Position=0), yet Windows GSMTC still refreshes
    // LastUpdatedTime every ~60s. Ignoring LastUpdatedTime when both dur and pos
    // are 0 prevents fabricating a 0..60s timer that resets every minute.
    if dur == 0 && pos == 0 {
        return (0, 0);
    }

    let adjusted = if status == PlaybackStatus::Playing && last_updated_ms > 0 && now_ms > last_updated_ms {
        let delta = (now_ms - last_updated_ms) as u64;
        pos.saturating_add(delta)
    } else {
        pos
    };

    let clamped = if dur > 0 { adjusted.min(dur) } else { adjusted };
    (clamped, dur)
}

fn snapshots_differ(prev: Option<&MediaSnapshot>, next: &MediaSnapshot) -> bool {
    let Some(prev) = prev else {
        return true;
    };
    if prev.status == next.status
        && prev.source == next.source
        && prev.title == next.title
        && prev.artist == next.artist
        && prev.album == next.album
        && prev.art_url == next.art_url
        && prev.art_color == next.art_color
        && prev.duration_ms == next.duration_ms
        && prev.position_known == next.position_known
        && prev.can_play_pause == next.can_play_pause
        && prev.can_next == next.can_next
        && prev.can_prev == next.can_prev
        && prev.can_seek == next.can_seek
        && prev.shuffle == next.shuffle
        && prev.repeat == next.repeat
        && prev.available_sources == next.available_sources
        && prev.sessions == next.sessions
    {
        if !next.position_known {
            return false;
        }
        if prev.status == PlaybackStatus::Playing {
            let elapsed = (next.updated_at - prev.updated_at).max(0) as u64;
            let raw_expected = prev.position_ms.saturating_add(elapsed);
            let expected = if prev.duration_ms > 0 {
                raw_expected.min(prev.duration_ms)
            } else {
                raw_expected
            };
            return expected.abs_diff(next.position_ms) >= 750;
        }
        return prev.position_ms.abs_diff(next.position_ms) >= 250;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{
        clean_media_title, compute_timeline_ms, extract_ambient_color,
        extract_matching_youtube_duration_ms, format_source_name, parse_duration_text,
        snapshots_differ, sync_fallback_duration, MediaSnapshot, PlaybackStatus,
        TrackFallbackTimer,
    };
    use super::persist::{persistable, restore, PersistedState, PersistedTimer};
    use image::{Rgb, RgbImage};
    use std::collections::{BTreeMap, HashMap};

    #[allow(clippy::too_many_arguments)]
    fn update_fallback_timer(
        timers: &mut HashMap<String, TrackFallbackTimer>,
        source: &str,
        title: &str,
        artist: &str,
        status: PlaybackStatus,
        now_ms: i64,
        known_pos_ms: Option<u64>,
    ) -> u64 {
        super::update_fallback_timer(timers, source, title, artist, status, now_ms, known_pos_ms, true).0
    }

    #[test]
    fn formats_known_and_unknown_sources() {
        assert_eq!(format_source_name("Spotify.exe"), "Spotify");
        assert_eq!(format_source_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(format_source_name("msedge.exe"), "Microsoft Edge");
        assert_eq!(format_source_name("308046B0AF4A39CB"), "Firefox");
        assert_eq!(format_source_name("foobar2000.exe"), "Foobar2000");
    }

    #[test]
    fn computes_timeline_with_max_seek_fallback_and_start_offset() {
        assert_eq!(
            compute_timeline_ms(0, 12_000, 0, 0, 180_000, 1_000, 3_500, PlaybackStatus::Playing),
            (14_500, 180_000)
        );
        assert_eq!(
            compute_timeline_ms(5_000, 25_000, 65_000, 0, 0, 1_000, 1_000, PlaybackStatus::Paused),
            (20_000, 60_000)
        );
        assert_eq!(
            compute_timeline_ms(0, 10_000, 90_000, 0, 0, 0, 50_000, PlaybackStatus::Playing),
            (10_000, 90_000)
        );
        assert_eq!(
            compute_timeline_ms(0, 89_000, 90_000, 0, 0, 1_000, 10_000, PlaybackStatus::Playing),
            (90_000, 90_000)
        );
        // Empty GSMTC timeline (Firefox) must return (0, 0) rather than fabricating now - last_updated.
        assert_eq!(
            compute_timeline_ms(0, 0, 0, 0, 0, 10_000, 15_000, PlaybackStatus::Playing),
            (0, 0)
        );
    }

    #[test]
    fn fallback_timer_accumulates_across_heartbeats_and_pauses_without_resetting() {
        let mut timers: HashMap<String, TrackFallbackTimer> = HashMap::new();
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "KØRER GALT I TUNET VOLVO!",
                "Granturismo Cars",
                PlaybackStatus::Playing,
                1_000,
                None,
            ),
            0
        );
        // 5 seconds later
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "KØRER GALT I TUNET VOLVO!",
                "Granturismo Cars",
                PlaybackStatus::Playing,
                6_000,
                None,
            ),
            5_000
        );
        // 65 seconds later (across 60s GSMTC heartbeat) — never resets!
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "KØRER GALT I TUNET VOLVO!",
                "Granturismo Cars",
                PlaybackStatus::Playing,
                66_000,
                None,
            ),
            65_000
        );
        // Pause at t=70_000 -> freezes at 69_000
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "KØRER GALT I TUNET VOLVO!",
                "Granturismo Cars",
                PlaybackStatus::Paused,
                70_000,
                None,
            ),
            69_000
        );
        // Still paused at t=100_000 -> remains 69_000
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "KØRER GALT I TUNET VOLVO!",
                "Granturismo Cars",
                PlaybackStatus::Paused,
                100_000,
                None,
            ),
            69_000
        );
        // Resume at t=100_000 and read at t=110_000 -> 79_000
        let _ = update_fallback_timer(
            &mut timers,
            "Firefox",
            "KØRER GALT I TUNET VOLVO!",
            "Granturismo Cars",
            PlaybackStatus::Playing,
            100_000,
            None,
        );
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "KØRER GALT I TUNET VOLVO!",
                "Granturismo Cars",
                PlaybackStatus::Playing,
                110_000,
                None,
            ),
            79_000
        );
        // New video title resets to 0
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "Ny Video",
                "Granturismo Cars",
                PlaybackStatus::Playing,
                115_000,
                None,
            ),
            0
        );
    }

    #[test]
    fn remembers_duration_across_pause_resume_and_resets_on_track_change() {
        let mut timers: HashMap<String, TrackFallbackTimer> = HashMap::new();
        // Track starts with real GSMTC timeline (pos=5s, dur=3:40)
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "The Weeknd - Trust Issues",
                "_tony1",
                PlaybackStatus::Playing,
                1_000,
                Some(5_000),
            ),
            5_000
        );
        assert_eq!(sync_fallback_duration(&mut timers, "Firefox", 220_000), 220_000);

        // Pause -> Firefox clears GSMTC timeline (pos=0, dur=0 -> None / 0)
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "The Weeknd - Trust Issues",
                "_tony1",
                PlaybackStatus::Paused,
                38_000,
                None,
            ),
            42_000
        );
        assert_eq!(sync_fallback_duration(&mut timers, "Firefox", 0), 220_000);

        // Resume -> still empty GSMTC timeline, duration stays remembered and position resumes
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "The Weeknd - Trust Issues",
                "_tony1",
                PlaybackStatus::Playing,
                50_000,
                None,
            ),
            42_000
        );
        assert_eq!(sync_fallback_duration(&mut timers, "Firefox", 0), 220_000);

        // Next track with empty initial timeline resets remembered duration to 0
        assert_eq!(
            update_fallback_timer(
                &mut timers,
                "Firefox",
                "Nyt Track",
                "_tony1",
                PlaybackStatus::Playing,
                55_000,
                None,
            ),
            0
        );
        assert_eq!(sync_fallback_duration(&mut timers, "Firefox", 0), 0);
    }

    #[test]
    fn parses_youtube_duration_and_matches_video_renderer() {
        assert_eq!(clean_media_title("(3) KØRER GALT I TUNET VOLVO! - YouTube"), "KØRER GALT I TUNET VOLVO!");
        assert_eq!(parse_duration_text("52:13"), Some(3_133_000));
        assert_eq!(parse_duration_text("1:02:05"), Some(3_725_000));
        assert_eq!(parse_duration_text("LIVE"), None);

        let sample = serde_json::json!({
            "contents": {
                "sectionListRenderer": {
                    "contents": [
                        {
                            "videoRenderer": {
                                "videoId": "4LnvM9aNRHM",
                                "title": { "runs": [{ "text": "KØRER GALT I TUNET VOLVO!" }] },
                                "ownerText": { "runs": [{ "text": "Granturismo Cars" }] },
                                "lengthText": { "simpleText": "52:13" }
                            }
                        }
                    ]
                }
            }
        });
        assert_eq!(
            extract_matching_youtube_duration_ms(
                &sample,
                "KØRER GALT I TUNET VOLVO!",
                "Granturismo Cars",
            ),
            Some(3_133_000)
        );
        assert_eq!(
            extract_matching_youtube_duration_ms(&sample, "Helt Anden Video", "Anden Kanal"),
            None
        );
    }

    #[test]
    fn dedup_accounts_for_elapsed_playback_and_detects_seeks() {
        let a = MediaSnapshot {
            status: PlaybackStatus::Playing,
            source: "Spotify".into(),
            title: "Song".into(),
            position_ms: 10_000,
            duration_ms: 120_000,
            position_known: true,
            updated_at: 1_000,
            ..MediaSnapshot::default()
        };
        let normal_advance = MediaSnapshot {
            position_ms: 15_100,
            updated_at: 6_000,
            ..a.clone()
        };
        let backward_seek = MediaSnapshot {
            position_ms: 3_000,
            updated_at: 2_000,
            ..a.clone()
        };
        let forward_seek = MediaSnapshot {
            position_ms: 25_000,
            updated_at: 2_000,
            ..a.clone()
        };
        assert!(!snapshots_differ(Some(&a), &normal_advance));
        assert!(snapshots_differ(Some(&a), &backward_seek));
        assert!(snapshots_differ(Some(&a), &forward_seek));

        let paused = MediaSnapshot {
            status: PlaybackStatus::Paused,
            position_ms: 40_000,
            updated_at: 5_000,
            ..a.clone()
        };
        let paused_jitter = MediaSnapshot {
            position_ms: 40_100,
            updated_at: 6_000,
            ..paused.clone()
        };
        let paused_seek = MediaSnapshot {
            position_ms: 40_500,
            updated_at: 6_000,
            ..paused.clone()
        };
        assert!(!snapshots_differ(Some(&paused), &paused_jitter));
        assert!(snapshots_differ(Some(&paused), &paused_seek));
    }

    #[test]
    fn fallback_position_is_unknown_for_track_already_running_at_startup() {
        let mut timers: HashMap<String, TrackFallbackTimer> = HashMap::new();
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 1_000, None, false,
            ),
            (0, false)
        );
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 6_000, None, false,
            ),
            (0, false)
        );
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Anden Video", "Kanal", PlaybackStatus::Playing, 10_000, None, false,
            ),
            (0, true)
        );
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Anden Video", "Kanal", PlaybackStatus::Playing, 15_000, None, false,
            ),
            (5_000, true)
        );
    }

    #[test]
    fn gsmtc_position_anchors_unknown_timer() {
        let mut timers: HashMap<String, TrackFallbackTimer> = HashMap::new();
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 1_000, Some(5_000), false,
            ),
            (5_000, true)
        );
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 11_000, None, false,
            ),
            (15_000, true)
        );
    }

    #[test]
    fn unobserved_gap_while_playing_makes_position_unknown() {
        let mut timers: HashMap<String, TrackFallbackTimer> = HashMap::new();
        let first = super::update_fallback_timer(
            &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 1_000, None, true,
        );
        assert_eq!(first, (0, true));
        let after_gap = super::update_fallback_timer(
            &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 1_000 + 121_000, None, true,
        );
        assert!(!after_gap.1);

        let mut timers: HashMap<String, TrackFallbackTimer> = HashMap::new();
        let _ = super::update_fallback_timer(
            &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 1_000, None, true,
        );
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Paused, 6_000, None, true,
            ),
            (5_000, true)
        );
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Paused, 6_000 + 600_000, None, true,
            ),
            (5_000, true)
        );
    }

    fn restored_state() -> PersistedState {
        let mut timers = BTreeMap::new();
        timers.insert(
            "Firefox".to_string(),
            PersistedTimer {
                track_key: "Video|Kanal".to_string(),
                accumulated_ms: 60_000,
                playing_since_ms: Some(90_000),
                known_duration_ms: 0,
            },
        );
        PersistedState { version: 1, saved_at_ms: 100_000, timers }
    }

    #[test]
    fn restored_timer_resumes_when_fresh_and_track_matches() {
        let mut timers = restore(restored_state());
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 130_000, None, false,
            ),
            (100_000, true)
        );
    }

    #[test]
    fn restored_timer_is_unknown_when_stale_or_track_changed() {
        let mut timers = restore(restored_state());
        let stale = super::update_fallback_timer(
            &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 100_000 + 121_000, None, false,
        );
        assert!(!stale.1);

        let mut timers = restore(restored_state());
        assert_eq!(
            super::update_fallback_timer(
                &mut timers, "Firefox", "Helt Anden Video", "Kanal", PlaybackStatus::Playing, 110_000, None, false,
            ),
            (0, false)
        );
    }

    #[test]
    fn persistable_skips_unknown_and_timeline_backed_timers() {
        let mut timers: HashMap<String, TrackFallbackTimer> = HashMap::new();
        let _ = super::update_fallback_timer(
            &mut timers, "Firefox", "Video", "Kanal", PlaybackStatus::Playing, 1_000, None, false,
        );
        let _ = super::update_fallback_timer(
            &mut timers, "Spotify", "Sang", "Kunstner", PlaybackStatus::Playing, 1_000, Some(2_000), false,
        );
        let _ = super::update_fallback_timer(
            &mut timers, "Edge", "Klip", "Kanal", PlaybackStatus::Playing, 1_000, None, true,
        );
        let map = persistable(&timers);
        assert!(!map.contains_key("Firefox"));
        assert!(!map.contains_key("Spotify"));
        assert!(map.contains_key("Edge"));
    }

    #[test]
    fn dedup_ignores_position_when_unknown() {
        let a = MediaSnapshot {
            status: PlaybackStatus::Playing,
            source: "Firefox".into(),
            title: "Video".into(),
            position_ms: 0,
            duration_ms: 120_000,
            position_known: false,
            updated_at: 1_000,
            ..MediaSnapshot::default()
        };
        let b = MediaSnapshot { updated_at: 60_000, ..a.clone() };
        assert!(!snapshots_differ(Some(&a), &b));
        let c = MediaSnapshot { position_known: true, ..b.clone() };
        assert!(snapshots_differ(Some(&a), &c));
    }

    #[test]
    fn dampens_ambient_color_from_vibrant_image() {
        let img = RgbImage::from_pixel(32, 32, Rgb([255, 60, 40]));
        let color = extract_ambient_color(&img).expect("should find vibrant pixels");
        // Lightness should be dampened (~0.45) rather than full neon white/yellow.
        assert!(color[0] > color[1] && color[0] > color[2]);
        assert!(color[0] < 210);
    }
}
