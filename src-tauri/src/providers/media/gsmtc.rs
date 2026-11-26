use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
        Arc,
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};
use windows::{
    Foundation::TypedEventHandler,
    Media::{
        Control::{
            GlobalSystemMediaTransportControlsSession as Session,
            GlobalSystemMediaTransportControlsSessionManager as GsmtcManager,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus as WinPlaybackStatus,
        },
        MediaPlaybackAutoRepeatMode,
    },
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};

use crate::util::unix_now_ms;

const GRACE_PERIOD: Duration = Duration::from_secs(2);
const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
const TIMELINE_HYDRATE_DELAYS_MS: [u64; 4] = [250, 1_000, 3_000, 5_000];
const ART_HYDRATE_DELAYS_MS: [u64; 4] = [300, 900, 2_000, 4_000];
const STARTUP_WINDOW: Duration = Duration::from_secs(5);
const PERSIST_HEARTBEAT: Duration = Duration::from_secs(20);

use super::{
    aumid::resolve_source_name,
    artwork::resolve_artwork,
    compute_timeline_ms, mark_timeline_missing,
    persist::{self, PersistedTimer},
    snapshots_differ, sync_fallback_duration, update_fallback_timer,
    youtube::{resolve_fallback_duration_ms, DurationLookupState},
    Cmd, MediaAction, MediaSessionInfo, MediaSnapshot, PlaybackStatus, RepeatMode,
    TrackFallbackTimer,
};

fn persist_timers_if_needed(
    fallback_timers: &HashMap<String, TrackFallbackTimer>,
    last_persisted: &mut BTreeMap<String, PersistedTimer>,
    last_persist_at: &mut Instant,
) {
    let map = persist::persistable(fallback_timers);
    let changed = map != *last_persisted;
    let heartbeat = !map.is_empty() && last_persist_at.elapsed() >= PERSIST_HEARTBEAT;
    if changed || heartbeat {
        persist::save(&map, unix_now_ms());
        *last_persisted = map;
        *last_persist_at = Instant::now();
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

pub(super) fn run_media_thread(
    app: AppHandle,
    tx: Sender<Cmd>,
    rx: mpsc::Receiver<Cmd>,
    art_dir: PathBuf,
    mut preferred_source: Option<String>,
    stopped: Arc<AtomicBool>,
    my_gen: u64,
    generation: Arc<AtomicU64>,
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
    let mut fallback_timers: HashMap<String, TrackFallbackTimer> = persist::load();
    let mut last_track_by_source: HashMap<String, (String, Option<String>)> = HashMap::new();
    let started_at = Instant::now();
    let mut dropped_unverified = false;
    let mut last_persisted: BTreeMap<String, PersistedTimer> = persist::persistable(&fallback_timers);
    let mut last_persist_at = Instant::now();
    let duration_lookup = Arc::new(Mutex::new(DurationLookupState::default()));
    let mut last_emitted: Option<MediaSnapshot> = None;
    let mut lost_since: Option<Instant> = None;
    let mut pending_reconcile: Option<Instant> = None;
    let mut hydrate_key: Option<String> = None;
    let mut hydrate_step: usize = 0;
    let mut next_hydrate_at: Option<Instant> = None;
    let mut art_hydrate_track: Option<String> = None;
    let mut art_hydrate_stale_url: Option<String> = None;
    let mut art_hydrate_step: usize = 0;
    let mut next_art_hydrate_at: Option<Instant> = None;

    let _ = tx.send(Cmd::Rescan);

    loop {
        if stopped.load(Ordering::Acquire) || generation.load(Ordering::Acquire) != my_gen {
            break;
        }
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
        if let Some(deadline) = next_art_hydrate_at {
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
                    should_refresh = true;
                    handled = true;
                }
                if next_art_hydrate_at.is_some_and(|deadline| now_inst >= deadline) {
                    next_art_hydrate_at = None;
                    art_hydrate_step = art_hydrate_step.saturating_add(1);
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

        let fresh_is_start = started_at.elapsed() >= STARTUP_WINDOW;
        if fresh_is_start && !dropped_unverified {
            fallback_timers.retain(|_, timer| !timer.unverified);
            dropped_unverified = true;
        }

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
                        let session_source = session
                            .SourceAppUserModelId()
                            .ok()
                            .map(|s| {
                                resolve_source_name(&s.to_string_lossy(), &mut source_name_cache)
                            })
                            .unwrap_or_default();
                        let now_ms = unix_now_ms();
                        match &action {
                            MediaAction::Next | MediaAction::Previous => {
                                if let Some(timer) = fallback_timers.get_mut(&session_source) {
                                    timer.reset(PlaybackStatus::Playing, now_ms, 0);
                                }
                            }
                            MediaAction::Seek { ms } => {
                                if let Some(timer) = fallback_timers.get_mut(&session_source) {
                                    let cur_status = if timer.playing_since_ms.is_some() {
                                        PlaybackStatus::Playing
                                    } else {
                                        PlaybackStatus::Paused
                                    };
                                    timer.reset(cur_status, now_ms, *ms);
                                }
                            }
                            _ => {}
                        }
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
                let sessions = collect_sessions(
                    &manager,
                    &mut source_name_cache,
                    &mut fallback_timers,
                    fresh_is_start,
                );
                if let Some(snapshot) = read_session_snapshot(
                    &hooks.session,
                    &art_dir,
                    &mut color_cache,
                    &mut source_name_cache,
                    &mut fallback_timers,
                    &duration_lookup,
                    &tx,
                    sessions,
                    fresh_is_start,
                ) {
                    lost_since = None;
                    let full_track_key =
                        format!("{}|{}|{}", snapshot.source, snapshot.title, snapshot.artist);
                    let source_track_key = format!("{}|{}", snapshot.title, snapshot.artist);

                    // Browsers (Firefox/Chrome) often update GSMTC Title/Artist before
                    // downloading the new cover image, leaving the previous track's
                    // thumbnail in GSMTC briefly. Schedule short re-checks until the
                    // thumbnail content hash changes (or retries expire).
                    let prev_for_source = last_track_by_source.get(&snapshot.source).cloned();
                    let track_changed = prev_for_source
                        .as_ref()
                        .is_none_or(|(prev_key, _)| prev_key != &source_track_key);
                    if track_changed {
                        let prev_art = prev_for_source.and_then(|(_, art)| art);
                        let art_looks_stale = snapshot.art_url.is_none()
                            || (prev_art.is_some() && snapshot.art_url == prev_art);
                        if art_looks_stale
                            && (!snapshot.title.is_empty() || !snapshot.artist.is_empty())
                        {
                            art_hydrate_track = Some(full_track_key.clone());
                            art_hydrate_stale_url = prev_art;
                            art_hydrate_step = 0;
                            next_art_hydrate_at = Some(
                                Instant::now() + Duration::from_millis(ART_HYDRATE_DELAYS_MS[0]),
                            );
                        } else {
                            art_hydrate_track = None;
                            art_hydrate_stale_url = None;
                            art_hydrate_step = 0;
                            next_art_hydrate_at = None;
                        }
                    } else if art_hydrate_track.as_deref() == Some(full_track_key.as_str()) {
                        let art_resolved = snapshot.art_url.is_some()
                            && snapshot.art_url != art_hydrate_stale_url;
                        if art_resolved {
                            art_hydrate_track = None;
                            art_hydrate_stale_url = None;
                            art_hydrate_step = 0;
                            next_art_hydrate_at = None;
                        } else if next_art_hydrate_at.is_none() {
                            if let Some(&delay_ms) = ART_HYDRATE_DELAYS_MS.get(art_hydrate_step) {
                                next_art_hydrate_at =
                                    Some(Instant::now() + Duration::from_millis(delay_ms));
                            } else {
                                art_hydrate_track = None;
                                art_hydrate_stale_url = None;
                                art_hydrate_step = 0;
                            }
                        }
                    }
                    last_track_by_source.insert(
                        snapshot.source.clone(),
                        (source_track_key, snapshot.art_url.clone()),
                    );

                    if snapshot.status == PlaybackStatus::Playing && snapshot.duration_ms == 0 {
                        if hydrate_key.as_deref() != Some(full_track_key.as_str()) {
                            hydrate_key = Some(full_track_key);
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
                        emit_snapshot(&app, &snapshot, &stopped, &generation, my_gen);
                        last_emitted = Some(snapshot);
                    }
                } else {
                    hydrate_key = None;
                    hydrate_step = 0;
                    next_hydrate_at = None;
                    art_hydrate_track = None;
                    art_hydrate_stale_url = None;
                    art_hydrate_step = 0;
                    next_art_hydrate_at = None;
                    if lost_since.is_none() {
                        lost_since = Some(Instant::now());
                    }
                }
            } else {
                hydrate_key = None;
                hydrate_step = 0;
                next_hydrate_at = None;
                art_hydrate_track = None;
                art_hydrate_stale_url = None;
                art_hydrate_step = 0;
                next_art_hydrate_at = None;
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
                art_hydrate_track = None;
                art_hydrate_stale_url = None;
                art_hydrate_step = 0;
                next_art_hydrate_at = None;
                last_track_by_source.clear();
                let sessions = collect_sessions(
                    &manager,
                    &mut source_name_cache,
                    &mut fallback_timers,
                    fresh_is_start,
                );
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
                    emit_snapshot(&app, &empty, &stopped, &generation, my_gen);
                    last_emitted = Some(empty);
                }
            }
        }

        persist_timers_if_needed(&fallback_timers, &mut last_persisted, &mut last_persist_at);
    }

    persist_timers_if_needed(&fallback_timers, &mut last_persisted, &mut last_persist_at);

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

fn emit_snapshot(
    app: &AppHandle,
    snapshot: &MediaSnapshot,
    stopped: &AtomicBool,
    generation: &AtomicU64,
    my_gen: u64,
) {
    if stopped.load(Ordering::Acquire) || generation.load(Ordering::Acquire) != my_gen {
        return;
    }
    if let Some(scheduler) = app.try_state::<crate::providers::scheduler::Scheduler>() {
        scheduler.media().update_latest(snapshot.clone());
        if stopped.load(Ordering::Acquire) || generation.load(Ordering::Acquire) != my_gen {
            return;
        }
        scheduler.emit_to_subscribers(
            crate::providers::scheduler::ProviderKind::Media,
            None,
            "provider://media",
            snapshot,
        );
    }
}

fn collect_sessions(
    manager: &GsmtcManager,
    source_name_cache: &mut HashMap<String, String>,
    fallback_timers: &mut HashMap<String, TrackFallbackTimer>,
    fresh_is_start: bool,
) -> Vec<MediaSessionInfo> {
    let mut out: Vec<MediaSessionInfo> = Vec::new();
    let Ok(sessions) = manager.GetSessions() else {
        return out;
    };
    let now_ms = unix_now_ms();
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
            fallback_timers.remove(&label);
            continue;
        }
        update_fallback_timer(fallback_timers, &label, &title, &artist, status, now_ms, None, fresh_is_start);
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

#[allow(clippy::too_many_arguments)]
fn read_session_snapshot(
    session: &Session,
    art_dir: &Path,
    color_cache: &mut HashMap<u64, Option<[u8; 3]>>,
    source_name_cache: &mut HashMap<String, String>,
    fallback_timers: &mut HashMap<String, TrackFallbackTimer>,
    duration_lookup: &Arc<Mutex<DurationLookupState>>,
    tx: &Sender<Cmd>,
    sessions: Vec<MediaSessionInfo>,
    fresh_is_start: bool,
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
    let (gsmtc_pos_ms, gsmtc_dur_ms) = if let Some(tl) = timeline {
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

    let gsmtc_has_timeline = gsmtc_dur_ms > 0 || gsmtc_pos_ms > 0;
    let (raw_position_ms, position_known) = update_fallback_timer(
        fallback_timers,
        &source,
        &title,
        &artist,
        status,
        now_ms,
        gsmtc_has_timeline.then_some(gsmtc_pos_ms),
        fresh_is_start,
    );
    if !gsmtc_has_timeline {
        mark_timeline_missing(fallback_timers, &source);
    }
    let remembered_dur_ms = sync_fallback_duration(fallback_timers, &source, gsmtc_dur_ms);
    let duration_ms = if remembered_dur_ms > 0 {
        remembered_dur_ms
    } else {
        resolve_fallback_duration_ms(duration_lookup, tx, &title, &artist)
    };
    let position_ms = if !position_known {
        0
    } else if duration_ms > 0 {
        raw_position_ms.min(duration_ms)
    } else {
        raw_position_ms
    };

    let (art_url, art_color) =
        resolve_artwork(&props, &title, &artist, art_dir, color_cache);

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
    let can_seek = gsmtc_has_timeline
        && controls
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
        position_known,
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
