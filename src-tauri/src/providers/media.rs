use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use image::{imageops::FilterType, RgbImage};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;
use windows::{
    core::PCWSTR,
    Foundation::TypedEventHandler,
    Media::{
        Control::{
            GlobalSystemMediaTransportControlsSession as Session,
            GlobalSystemMediaTransportControlsSessionManager as GsmtcManager,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus as WinPlaybackStatus,
        },
        MediaPlaybackAutoRepeatMode,
    },
    Storage::Streams::{DataReader, InputStreamOptions},
    Win32::{
        System::{
            Com::CoTaskMemFree,
            WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
        },
        UI::Shell::{
            FOLDERID_AppsFolder, IShellItem, SHCreateItemInKnownFolder, KNOWN_FOLDER_FLAG,
            SIGDN_NORMALDISPLAY,
        },
    },
};

const MAX_ART_FILES: usize = 60;
const GRACE_PERIOD: Duration = Duration::from_secs(2);
const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
const TIMELINE_HYDRATE_DELAYS_MS: [u64; 4] = [250, 1_000, 3_000, 5_000];

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
        #[ts(type = "number")]
        ms: u64,
    },
    Shuffle,
    Repeat,
}

enum Cmd {
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
}

struct MediaWorker {
    tx: Sender<Cmd>,
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
                                let elapsed = (now - current.updated_at).max(0) as u64;
                                current.position_ms = current
                                    .position_ms
                                    .saturating_add(elapsed)
                                    .min(current.duration_ms.max(current.position_ms));
                                current.status = PlaybackStatus::Paused;
                            } else {
                                current.status = PlaybackStatus::Playing;
                            }
                            current.updated_at = now;
                        }
                        MediaAction::Seek { ms } => {
                            if current.can_seek {
                                current.position_ms = (*ms).min(current.duration_ms.max(*ms));
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
            let _ = app.emit("provider://media", &snap);
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
        let latest_ptr = self as *const MediaProvider as usize;

        // SAFETY: MediaProvider lives inside Scheduler which is managed in AppState for app lifetime,
        // and we stop/join the thread on stop(). We still use an Arc<Mutex<Option<MediaSnapshot>>> via AppState/Scheduler instead!
        let _ = latest_ptr;

        let art_dir = app
            .path()
            .app_cache_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("art");
        let _ = fs::create_dir_all(&art_dir);

        let latest_snapshot_sink = app_handle.clone();
        match std::thread::Builder::new()
            .name("od-media".into())
            .spawn(move || {
                run_media_thread(app_handle, tx_clone, rx, art_dir, preferred_source, latest_snapshot_sink);
            }) {
            Ok(join) => {
                log::info!(target: "provider:media", "Starter media-provider (od-media)");
                *guard = Some(MediaWorker {
                    tx,
                    join: Some(join),
                });
            }
            Err(err) => {
                log::warn!(target: "provider:media", "Kunne ikke starte od-media-tråd: {err}");
            }
        }
    }

    pub fn stop(&self) {
        let worker = self.runtime.lock().take();
        if let Some(mut worker) = worker {
            log::info!(target: "provider:media", "Stopper media-provider");
            let _ = worker.tx.send(Cmd::Shutdown);
            if let Some(join) = worker.join.take() {
                let _ = join.join();
            }
        }
    }

    pub fn update_latest(&self, snapshot: MediaSnapshot) {
        *self.latest.lock() = Some(snapshot);
    }
}

struct SessionHooks {
    session: Session,
    media_tok: Option<i64>,
    playback_tok: Option<i64>,
    timeline_tok: Option<i64>,
}

impl SessionHooks {
    fn attach(session: Session, tx: &Sender<Cmd>) -> Self {
        let tx1 = tx.clone();
        let media_tok = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx1.send(Cmd::Refresh);
                Ok(())
            }))
            .ok();

        let tx2 = tx.clone();
        let playback_tok = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx2.send(Cmd::Refresh);
                Ok(())
            }))
            .ok();

        let tx3 = tx.clone();
        let timeline_tok = session
            .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx3.send(Cmd::Refresh);
                Ok(())
            }))
            .ok();

        Self {
            session,
            media_tok,
            playback_tok,
            timeline_tok,
        }
    }

    fn detach(self) {
        if let Some(tok) = self.media_tok {
            let _ = self.session.RemoveMediaPropertiesChanged(tok);
        }
        if let Some(tok) = self.playback_tok {
            let _ = self.session.RemovePlaybackInfoChanged(tok);
        }
        if let Some(tok) = self.timeline_tok {
            let _ = self.session.RemoveTimelinePropertiesChanged(tok);
        }
    }
}

struct BgSessionHook {
    session: Session,
    playback_tok: Option<i64>,
}

impl BgSessionHook {
    fn attach(session: Session, tx: &Sender<Cmd>) -> Self {
        let tx_pb = tx.clone();
        let playback_tok = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx_pb.send(Cmd::Rescan);
                Ok(())
            }))
            .ok();
        Self {
            session,
            playback_tok,
        }
    }

    fn detach(self) {
        if let Some(tok) = self.playback_tok {
            let _ = self.session.RemovePlaybackInfoChanged(tok);
        }
    }
}

fn attach_bg_hooks(manager: &GsmtcManager, active: &Session, tx: &Sender<Cmd>) -> Vec<BgSessionHook> {
    let mut out = Vec::new();
    let Ok(sessions) = manager.GetSessions() else {
        return out;
    };
    let count = sessions.Size().unwrap_or(0);
    for i in 0..count {
        let Ok(sess) = sessions.GetAt(i) else {
            continue;
        };
        if &sess == active {
            continue;
        }
        out.push(BgSessionHook::attach(sess, tx));
    }
    out
}

fn run_media_thread(
    app: AppHandle,
    tx: Sender<Cmd>,
    rx: mpsc::Receiver<Cmd>,
    art_dir: PathBuf,
    mut preferred_source: Option<String>,
    _sink: AppHandle,
) {
    let ro_ok = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();

    let manager = match GsmtcManager::RequestAsync().and_then(|op| op.join()) {
        Ok(mgr) => mgr,
        Err(err) => {
            log::warn!(target: "provider:media", "GSMTC Manager::RequestAsync fejlede: {err}");
            if ro_ok {
                unsafe { RoUninitialize() };
            }
            return;
        }
    };

    let tx_sessions = tx.clone();
    let sessions_tok = manager
        .SessionsChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx_sessions.send(Cmd::Rescan);
            Ok(())
        }))
        .ok();

    let tx_current = tx.clone();
    let current_tok = manager
        .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx_current.send(Cmd::Rescan);
            Ok(())
        }))
        .ok();

    let mut active_hooks: Option<SessionHooks> = None;
    let mut bg_hooks: Vec<BgSessionHook> = Vec::new();
    let mut color_cache: HashMap<u64, Option<[u8; 3]>> = HashMap::new();
    let mut source_name_cache: HashMap<String, String> = HashMap::new();
    let mut last_emitted: Option<MediaSnapshot> = None;
    let mut lost_since: Option<Instant> = None;
    let mut pending_reconcile: Option<Instant> = None;
    let mut hydrate_key: Option<String> = None;
    let mut hydrate_step: usize = 0;
    let mut next_hydrate_at: Option<Instant> = None;

    let _ = tx.send(Cmd::Rescan);

    loop {
        let base_timeout = if lost_since.is_some() {
            Duration::from_millis(350)
        } else if last_emitted
            .as_ref()
            .is_some_and(|s| s.status == PlaybackStatus::Playing)
        {
            Duration::from_secs(15)
        } else {
            Duration::from_secs(60)
        };
        let mut timeout = base_timeout;
        if let Some(deadline) = pending_reconcile {
            timeout = timeout.min(deadline.saturating_duration_since(Instant::now()));
        }
        if let Some(deadline) = next_hydrate_at {
            timeout = timeout.min(deadline.saturating_duration_since(Instant::now()));
        }

        let mut should_rescan = false;
        let mut should_refresh = false;
        let mut force_emit = false;

        let cmd = match rx.recv_timeout(timeout) {
            Ok(cmd) => Some(cmd),
            Err(RecvTimeoutError::Timeout) => {
                let now_inst = Instant::now();
                let mut handled = false;
                if pending_reconcile.is_some_and(|deadline| now_inst >= deadline) {
                    pending_reconcile = None;
                    should_refresh = true;
                    force_emit = true;
                    handled = true;
                }
                if next_hydrate_at.is_some_and(|deadline| now_inst >= deadline) {
                    next_hydrate_at = None;
                    hydrate_step = hydrate_step.saturating_add(1);
                    should_rescan = true;
                    should_refresh = true;
                    handled = true;
                }
                if !handled
                    && lost_since.is_none()
                    && last_emitted
                        .as_ref()
                        .is_some_and(|s| s.status == PlaybackStatus::Playing)
                {
                    should_refresh = true;
                }
                None
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };

        if let Some(cmd) = cmd {
            match cmd {
                Cmd::Shutdown => break,
                Cmd::Rescan => should_rescan = true,
                Cmd::Refresh => should_refresh = true,
                Cmd::SetPreferred(pref) => {
                    preferred_source = pref.filter(|s| !s.is_empty() && s != "auto");
                    should_rescan = true;
                }
                Cmd::Control { action, source } => {
                    let target_source = source.or_else(|| preferred_source.clone());
                    let target_session =
                        select_session(&manager, target_source.as_deref(), &mut source_name_cache)
                            .or_else(|| active_hooks.as_ref().map(|h| h.session.clone()));
                    if let Some(session) = target_session {
                        let ok = execute_media_control(&session, action);
                        should_refresh = true;
                        if !ok {
                            pending_reconcile = None;
                            force_emit = true;
                        } else {
                            pending_reconcile = Some(Instant::now() + Duration::from_millis(250));
                        }
                    } else {
                        should_refresh = true;
                        force_emit = true;
                    }
                }
            }
        }

        if should_rescan {
            if let Some(old) = active_hooks.take() {
                old.detach();
            }
            for old_bg in bg_hooks.drain(..) {
                old_bg.detach();
            }
            if let Some(session) =
                select_session(&manager, preferred_source.as_deref(), &mut source_name_cache)
            {
                bg_hooks = attach_bg_hooks(&manager, &session, &tx);
                active_hooks = Some(SessionHooks::attach(session, &tx));
                lost_since = None;
                should_refresh = true;
            } else if lost_since.is_none() {
                lost_since = Some(Instant::now());
            }
        }

        if should_refresh {
            if let Some(hooks) = active_hooks.as_ref() {
                let sessions = collect_sessions(&manager, &mut source_name_cache);
                if let Some(snapshot) = read_session_snapshot(
                    &hooks.session,
                    &art_dir,
                    &mut color_cache,
                    &mut source_name_cache,
                    sessions,
                ) {
                    lost_since = None;
                    if snapshot.status == PlaybackStatus::Playing && snapshot.duration_ms == 0 {
                        let key = format!("{}|{}|{}", snapshot.source, snapshot.title, snapshot.artist);
                        if hydrate_key.as_deref() != Some(key.as_str()) {
                            hydrate_key = Some(key);
                            hydrate_step = 0;
                            next_hydrate_at = None;
                        }
                        if next_hydrate_at.is_none() {
                            if let Some(&delay_ms) = TIMELINE_HYDRATE_DELAYS_MS.get(hydrate_step) {
                                next_hydrate_at =
                                    Some(Instant::now() + Duration::from_millis(delay_ms));
                            }
                        }
                    } else {
                        hydrate_key = None;
                        hydrate_step = 0;
                        next_hydrate_at = None;
                    }

                    if force_emit || snapshots_differ(last_emitted.as_ref(), &snapshot) {
                        emit_snapshot(&app, &snapshot);
                        last_emitted = Some(snapshot);
                    }
                } else {
                    hydrate_key = None;
                    hydrate_step = 0;
                    next_hydrate_at = None;
                    if lost_since.is_none() {
                        lost_since = Some(Instant::now());
                    }
                }
            } else {
                hydrate_key = None;
                hydrate_step = 0;
                next_hydrate_at = None;
                if lost_since.is_none() {
                    lost_since = Some(Instant::now());
                }
            }
        }

        if let Some(since) = lost_since {
            if since.elapsed() >= GRACE_PERIOD {
                lost_since = None;
                hydrate_key = None;
                hydrate_step = 0;
                next_hydrate_at = None;
                let sessions = collect_sessions(&manager, &mut source_name_cache);
                let empty = MediaSnapshot {
                    available_sources: sessions.iter().map(|s| s.source.clone()).collect(),
                    sessions,
                    ..MediaSnapshot::default()
                };
                if last_emitted
                    .as_ref()
                    .map(|s| s.status != PlaybackStatus::None)
                    .unwrap_or(true)
                {
                    emit_snapshot(&app, &empty);
                    last_emitted = Some(empty);
                }
            }
        }
    }

    if let Some(old) = active_hooks.take() {
        old.detach();
    }
    for old_bg in bg_hooks.drain(..) {
        old_bg.detach();
    }
    if let Some(tok) = sessions_tok {
        let _ = manager.RemoveSessionsChanged(tok);
    }
    if let Some(tok) = current_tok {
        let _ = manager.RemoveCurrentSessionChanged(tok);
    }

    if ro_ok {
        unsafe { RoUninitialize() };
    }
}

fn emit_snapshot(app: &AppHandle, snapshot: &MediaSnapshot) {
    if let Some(scheduler) = app.try_state::<super::scheduler::Scheduler>() {
        scheduler.media().update_latest(snapshot.clone());
    }
    if let Err(err) = app.emit("provider://media", snapshot) {
        log::warn!(target: "provider:media", "Kunne ikke emitte provider://media: {err}");
    }
}

fn collect_sessions(
    manager: &GsmtcManager,
    source_name_cache: &mut HashMap<String, String>,
) -> Vec<MediaSessionInfo> {
    let mut out: Vec<MediaSessionInfo> = Vec::new();
    let Ok(sessions) = manager.GetSessions() else {
        return out;
    };
    let count = sessions.Size().unwrap_or(0);
    for i in 0..count {
        let Ok(session) = sessions.GetAt(i) else {
            continue;
        };
        let Ok(aumid) = session.SourceAppUserModelId() else {
            continue;
        };
        let label = resolve_source_name(&aumid.to_string_lossy(), source_name_cache);
        if label.is_empty() || out.iter().any(|s| s.source == label) {
            continue;
        }
        let status = session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .map(|s| match s {
                WinPlaybackStatus::Playing => PlaybackStatus::Playing,
                WinPlaybackStatus::Paused => PlaybackStatus::Paused,
                WinPlaybackStatus::Stopped => PlaybackStatus::Stopped,
                _ => PlaybackStatus::None,
            })
            .unwrap_or(PlaybackStatus::None);
        let (title, artist) = session
            .TryGetMediaPropertiesAsync()
            .ok()
            .and_then(|op| op.join().ok())
            .map(|props| {
                (
                    props.Title().map(|t| t.to_string_lossy()).unwrap_or_default(),
                    props.Artist().map(|a| a.to_string_lossy()).unwrap_or_default(),
                )
            })
            .unwrap_or_default();
        if status == PlaybackStatus::None && title.is_empty() && artist.is_empty() {
            continue;
        }
        out.push(MediaSessionInfo {
            source: label,
            title,
            artist,
            status,
        });
    }
    out
}

fn select_session(
    manager: &GsmtcManager,
    preferred: Option<&str>,
    source_name_cache: &mut HashMap<String, String>,
) -> Option<Session> {
    let sessions = manager.GetSessions().ok()?;
    let count = sessions.Size().unwrap_or(0);
    let current = manager.GetCurrentSession().ok();

    if let Some(pref) = preferred.filter(|p| !p.is_empty() && *p != "auto") {
        let needle = pref.to_ascii_lowercase();
        for i in 0..count {
            if let Ok(session) = sessions.GetAt(i) {
                if let Ok(aumid) = session.SourceAppUserModelId() {
                    let raw = aumid.to_string_lossy();
                    let resolved = resolve_source_name(&raw, source_name_cache);
                    if raw.to_ascii_lowercase().contains(&needle)
                        || resolved.to_ascii_lowercase().contains(&needle)
                    {
                        return Some(session);
                    }
                }
            }
        }
    }

    if let Some(cur) = &current {
        if is_session_playing(cur) {
            return Some(cur.clone());
        }
    }

    for i in 0..count {
        if let Ok(session) = sessions.GetAt(i) {
            if is_session_playing(&session) {
                return Some(session);
            }
        }
    }

    current
}

fn is_session_playing(session: &Session) -> bool {
    session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .map(|s| s == WinPlaybackStatus::Playing)
        .unwrap_or(false)
}

fn read_session_snapshot(
    session: &Session,
    art_dir: &Path,
    color_cache: &mut HashMap<u64, Option<[u8; 3]>>,
    source_name_cache: &mut HashMap<String, String>,
    sessions: Vec<MediaSessionInfo>,
) -> Option<MediaSnapshot> {
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let info = session.GetPlaybackInfo().ok()?;
    let controls = info.Controls().ok();
    let timeline = session.GetTimelineProperties().ok();

    let source_raw = session
        .SourceAppUserModelId()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let source = resolve_source_name(&source_raw, source_name_cache);

    let title = props.Title().map(|s| s.to_string_lossy()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string_lossy()).unwrap_or_default();
    let album = props.AlbumTitle().map(|s| s.to_string_lossy()).unwrap_or_default();

    let status = match info.PlaybackStatus().unwrap_or(WinPlaybackStatus::Closed) {
        WinPlaybackStatus::Playing => PlaybackStatus::Playing,
        WinPlaybackStatus::Paused => PlaybackStatus::Paused,
        WinPlaybackStatus::Stopped => PlaybackStatus::Stopped,
        WinPlaybackStatus::Closed | WinPlaybackStatus::Opened | WinPlaybackStatus::Changing => {
            if title.is_empty() && artist.is_empty() {
                PlaybackStatus::None
            } else {
                PlaybackStatus::Paused
            }
        }
        _ => PlaybackStatus::None,
    };

    if status == PlaybackStatus::None {
        return None;
    }

    let now_ms = unix_now_ms();
    let (position_ms, duration_ms) = if let Some(tl) = timeline {
        let start_ms = tl
            .StartTime()
            .map(|t| (t.Duration / 10_000).max(0) as u64)
            .unwrap_or(0);
        let raw_pos_ms = tl
            .Position()
            .map(|t| (t.Duration / 10_000).max(0) as u64)
            .unwrap_or(0);
        let end_ms = tl
            .EndTime()
            .map(|t| (t.Duration / 10_000).max(0) as u64)
            .unwrap_or(0);
        let min_seek_ms = tl
            .MinSeekTime()
            .map(|t| (t.Duration / 10_000).max(0) as u64)
            .unwrap_or(0);
        let max_seek_ms = tl
            .MaxSeekTime()
            .map(|t| (t.Duration / 10_000).max(0) as u64)
            .unwrap_or(0);
        let last_updated_ms = tl
            .LastUpdatedTime()
            .map(|dt| (dt.UniversalTime - EPOCH_DIFF_100NS) / 10_000)
            .unwrap_or(now_ms);
        compute_timeline_ms(
            start_ms,
            raw_pos_ms,
            end_ms,
            min_seek_ms,
            max_seek_ms,
            last_updated_ms,
            now_ms,
            status,
        )
    } else {
        (0, 0)
    };

    let (art_url, art_color) =
        resolve_artwork(&props, &title, &artist, &album, art_dir, color_cache);

    let can_play_pause = controls
        .as_ref()
        .and_then(|c| c.IsPlayPauseToggleEnabled().ok())
        .unwrap_or(false);
    let can_next = controls
        .as_ref()
        .and_then(|c| c.IsNextEnabled().ok())
        .unwrap_or(false);
    let can_prev = controls
        .as_ref()
        .and_then(|c| c.IsPreviousEnabled().ok())
        .unwrap_or(false);
    let can_seek = controls
        .as_ref()
        .and_then(|c| c.IsPlaybackPositionEnabled().ok())
        .unwrap_or(false);

    let shuffle = info
        .IsShuffleActive()
        .ok()
        .and_then(|r| r.Value().ok());
    let repeat = info
        .AutoRepeatMode()
        .ok()
        .and_then(|r| r.Value().ok())
        .map(|mode| match mode {
            MediaPlaybackAutoRepeatMode::Track => RepeatMode::Track,
            MediaPlaybackAutoRepeatMode::List => RepeatMode::List,
            _ => RepeatMode::None,
        });

    Some(MediaSnapshot {
        status,
        source,
        title,
        artist,
        album,
        art_url,
        art_color,
        position_ms,
        duration_ms,
        updated_at: now_ms,
        can_play_pause,
        can_next,
        can_prev,
        can_seek,
        shuffle,
        repeat,
        available_sources: sessions.iter().map(|s| s.source.clone()).collect(),
        sessions,
    })
}

fn execute_media_control(session: &Session, action: MediaAction) -> bool {
    match action {
        MediaAction::PlayPause => session
            .TryTogglePlayPauseAsync()
            .and_then(|op| op.join())
            .unwrap_or(false),
        MediaAction::Next => session
            .TrySkipNextAsync()
            .and_then(|op| op.join())
            .unwrap_or(false),
        MediaAction::Previous => session
            .TrySkipPreviousAsync()
            .and_then(|op| op.join())
            .unwrap_or(false),
        MediaAction::Seek { ms } => session
            .TryChangePlaybackPositionAsync((ms as i64) * 10_000)
            .and_then(|op| op.join())
            .unwrap_or(false),
        MediaAction::Shuffle => {
            let current = session
                .GetPlaybackInfo()
                .ok()
                .and_then(|i| i.IsShuffleActive().ok())
                .and_then(|r| r.Value().ok())
                .unwrap_or(false);
            session
                .TryChangeShuffleActiveAsync(!current)
                .and_then(|op| op.join())
                .unwrap_or(false)
        }
        MediaAction::Repeat => {
            let current = session
                .GetPlaybackInfo()
                .ok()
                .and_then(|i| i.AutoRepeatMode().ok())
                .and_then(|r| r.Value().ok())
                .unwrap_or(MediaPlaybackAutoRepeatMode::None);
            let next = match current {
                MediaPlaybackAutoRepeatMode::None => MediaPlaybackAutoRepeatMode::List,
                MediaPlaybackAutoRepeatMode::List => MediaPlaybackAutoRepeatMode::Track,
                _ => MediaPlaybackAutoRepeatMode::None,
            };
            session
                .TryChangeAutoRepeatModeAsync(next)
                .and_then(|op| op.join())
                .unwrap_or(false)
        }
    }
}

fn resolve_artwork(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
    title: &str,
    artist: &str,
    album: &str,
    art_dir: &Path,
    color_cache: &mut HashMap<u64, Option<[u8; 3]>>,
) -> (Option<String>, Option<[u8; 3]>) {
    if title.is_empty() && artist.is_empty() {
        return (None, None);
    }

    let key = fnv1a_64(&format!("{title}|{artist}|{album}"));
    let file_path = art_dir.join(format!("{key:016x}.jpg"));
    if file_path.exists() {
        let color = if let Some(cached) = color_cache.get(&key) {
            *cached
        } else {
            let computed = image::open(&file_path)
                .ok()
                .and_then(|img| extract_ambient_color(&img.to_rgb8()));
            color_cache.insert(key, computed);
            computed
        };
        return (Some(file_path.to_string_lossy().into_owned()), color);
    }

    let Ok(thumb_ref) = props.Thumbnail() else {
        return (None, None);
    };
    let Ok(stream) = thumb_ref.OpenReadAsync().and_then(|op| op.join()) else {
        return (None, None);
    };
    let Ok(size_u64) = stream.Size() else {
        return (None, None);
    };
    if size_u64 == 0 || size_u64 > 16 * 1024 * 1024 {
        return (None, None);
    }
    let size = size_u64 as u32;
    let Ok(input) = stream.GetInputStreamAt(0) else {
        return (None, None);
    };
    let Ok(reader) = DataReader::CreateDataReader(&input) else {
        return (None, None);
    };
    let _ = reader.SetInputStreamOptions(InputStreamOptions::ReadAhead);
    if reader.LoadAsync(size).and_then(|op| op.join()).is_err() {
        return (None, None);
    }
    let mut bytes = vec![0u8; size as usize];
    if reader.ReadBytes(&mut bytes).is_err() {
        return (None, None);
    }

    let Ok(decoded) = image::load_from_memory(&bytes) else {
        return (None, None);
    };
    let cropped = crop_square_320(&decoded);
    let rgb = cropped.to_rgb8();
    let ambient = extract_ambient_color(&rgb);

    if let Err(err) = save_jpeg_q86(&rgb, &file_path) {
        log::warn!(target: "provider:media", "Kunne ikke gemme cover-cache: {err}");
        return (None, ambient);
    }
    evict_old_art(art_dir, MAX_ART_FILES);
    color_cache.insert(key, ambient);

    (Some(file_path.to_string_lossy().into_owned()), ambient)
}

fn crop_square_320(img: &image::DynamicImage) -> image::DynamicImage {
    let w = img.width();
    let h = img.height();
    let side = w.min(h).max(1);
    let x = (w - side) / 2;
    let y = (h - side) / 2;
    img.crop_imm(x, y, side, side)
        .resize_exact(320, 320, FilterType::Lanczos3)
}

fn save_jpeg_q86(rgb: &RgbImage, target: &Path) -> Result<(), image::ImageError> {
    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let file = fs::File::create(target).map_err(image::ImageError::IoError)?;
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, 86);
    encoder.encode_image(rgb)
}

fn evict_old_art(dir: &Path, max_files: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, SystemTime)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(UNIX_EPOCH);
            Some((path, modified))
        })
        .collect();

    if files.len() <= max_files {
        return;
    }

    files.sort_by_key(|(_, time)| *time);
    let remove_count = files.len() - max_files;
    for (path, _) in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

pub fn extract_ambient_color(rgb: &RgbImage) -> Option<[u8; 3]> {
    let small = image::imageops::resize(rgb, 32, 32, FilterType::Triangle);
    let mut r_sum: f64 = 0.0;
    let mut g_sum: f64 = 0.0;
    let mut b_sum: f64 = 0.0;
    let mut count: u32 = 0;

    for px in small.pixels() {
        let (_, s, l) = rgb_to_hsl(px[0], px[1], px[2]);
        if s > 0.25 && (0.2..=0.85).contains(&l) {
            r_sum += px[0] as f64;
            g_sum += px[1] as f64;
            b_sum += px[2] as f64;
            count += 1;
        }
    }

    if count == 0 {
        return None;
    }

    let avg_r = (r_sum / count as f64).round().clamp(0.0, 255.0) as u8;
    let avg_g = (g_sum / count as f64).round().clamp(0.0, 255.0) as u8;
    let avg_b = (b_sum / count as f64).round().clamp(0.0, 255.0) as u8;

    let (h, s, _) = rgb_to_hsl(avg_r, avg_g, avg_b);
    let clamped_s = s.min(0.55);
    Some(hsl_to_rgb(h, clamped_s, 0.45))
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d < 1e-5 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let mut h = if (max - rf).abs() < 1e-5 {
        (gf - bf) / d + if gf < bf { 6.0 } else { 0.0 }
    } else if (max - gf).abs() < 1e-5 {
        (bf - rf) / d + 2.0
    } else {
        (rf - gf) / d + 4.0
    };
    h /= 6.0;
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    if s < 1e-5 {
        let v = (l * 255.0).round().clamp(0.0, 255.0) as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let b = hue_to_rgb(p, q, h - 1.0 / 3.0);
    [
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
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

    let adjusted = if status == PlaybackStatus::Playing && last_updated_ms > 0 && now_ms > last_updated_ms {
        let delta = (now_ms - last_updated_ms) as u64;
        pos.saturating_add(delta)
    } else {
        pos
    };

    let clamped = if dur > 0 { adjusted.min(dur) } else { adjusted };
    (clamped, dur)
}

fn match_known_source_name(aumid: &str) -> Option<&'static str> {
    let lower = aumid.to_ascii_lowercase();
    if lower.contains("spotify") {
        return Some("Spotify");
    }
    if lower.contains("msedge") || lower.contains("microsoftedge") {
        return Some("Microsoft Edge");
    }
    if lower.contains("chrome") {
        return Some("Google Chrome");
    }
    if lower.contains("firefox")
        || lower.contains("308046b0af4a39cb")
        || lower.contains("e7cf176e110c211b")
        || lower.contains("f0dc299d809b9700")
        || lower.contains("6f193ccc56814779")
    {
        return Some("Firefox");
    }
    if lower.contains("vlc") {
        return Some("VLC");
    }
    if lower.contains("wmplayer") || lower.contains("zune") || lower.contains("mediaplayer") {
        return Some("Medieafspiller");
    }
    None
}

fn resolve_aumid_display_name(aumid: &str) -> Option<String> {
    let trimmed = aumid.trim();
    if trimmed.is_empty() {
        return None;
    }
    let wide: Vec<u16> = trimmed.encode_utf16().chain(std::iter::once(0)).collect();
    let item: IShellItem = unsafe {
        SHCreateItemInKnownFolder(
            &FOLDERID_AppsFolder,
            KNOWN_FOLDER_FLAG(0),
            PCWSTR(wide.as_ptr()),
        )
        .ok()?
    };
    let pwstr = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()? };
    let text = unsafe { pwstr.to_string().ok() };
    unsafe {
        CoTaskMemFree(Some(pwstr.0 as *const _));
    }
    text.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn resolve_source_name(aumid: &str, cache: &mut HashMap<String, String>) -> String {
    let trimmed = aumid.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(cached) = cache.get(trimmed) {
        return cached.clone();
    }
    let resolved = if let Some(known) = match_known_source_name(trimmed) {
        known.to_string()
    } else if let Some(display_name) = resolve_aumid_display_name(trimmed) {
        display_name
    } else {
        format_source_name(trimmed)
    };
    cache.insert(trimmed.to_string(), resolved.clone());
    resolved
}

pub fn format_source_name(aumid: &str) -> String {
    if let Some(known) = match_known_source_name(aumid) {
        return known.into();
    }

    let base = aumid
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(aumid)
        .split('!')
        .next()
        .unwrap_or(aumid)
        .trim();
    let stripped = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base);
    let mut chars = stripped.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
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
        && prev.can_play_pause == next.can_play_pause
        && prev.can_next == next.can_next
        && prev.can_prev == next.can_prev
        && prev.can_seek == next.can_seek
        && prev.shuffle == next.shuffle
        && prev.repeat == next.repeat
        && prev.available_sources == next.available_sources
        && prev.sessions == next.sessions
    {
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

fn fnv1a_64(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        compute_timeline_ms, extract_ambient_color, format_source_name, MediaSnapshot,
        PlaybackStatus, snapshots_differ,
    };
    use image::{Rgb, RgbImage};

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
    }

    #[test]
    fn dedup_accounts_for_elapsed_playback_and_detects_seeks() {
        let a = MediaSnapshot {
            status: PlaybackStatus::Playing,
            source: "Spotify".into(),
            title: "Song".into(),
            position_ms: 10_000,
            duration_ms: 120_000,
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
    fn dampens_ambient_color_from_vibrant_image() {
        let img = RgbImage::from_pixel(32, 32, Rgb([255, 60, 40]));
        let color = extract_ambient_color(&img).expect("should find vibrant pixels");
        // Lightness should be dampened (~0.45) rather than full neon white/yellow.
        assert!(color[0] > color[1] && color[0] > color[2]);
        assert!(color[0] < 210);
    }
}
