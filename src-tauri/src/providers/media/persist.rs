use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io,
};

use serde::{Deserialize, Serialize};

use super::TrackFallbackTimer;

const PERSIST_FILE: &str = "media-positions.json";
const PERSIST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PersistedState {
    pub version: u32,
    pub saved_at_ms: i64,
    pub timers: BTreeMap<String, PersistedTimer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PersistedTimer {
    pub track_key: String,
    pub accumulated_ms: u64,
    pub playing_since_ms: Option<i64>,
    pub known_duration_ms: u64,
}

pub(super) fn persistable(timers: &HashMap<String, TrackFallbackTimer>) -> BTreeMap<String, PersistedTimer> {
    timers
        .iter()
        .filter(|(_, timer)| timer.anchored && !timer.timeline_backed)
        .map(|(source, timer)| {
            (
                source.clone(),
                PersistedTimer {
                    track_key: timer.track_key.clone(),
                    accumulated_ms: timer.accumulated_ms,
                    playing_since_ms: timer.playing_since_ms,
                    known_duration_ms: timer.known_duration_ms,
                },
            )
        })
        .collect()
}

pub(super) fn restore(state: PersistedState) -> HashMap<String, TrackFallbackTimer> {
    state
        .timers
        .into_iter()
        .map(|(source, timer)| {
            (
                source,
                TrackFallbackTimer {
                    track_key: timer.track_key,
                    accumulated_ms: timer.accumulated_ms,
                    playing_since_ms: timer.playing_since_ms,
                    known_duration_ms: timer.known_duration_ms,
                    anchored: true,
                    unverified: true,
                    timeline_backed: false,
                    last_seen_ms: state.saved_at_ms,
                },
            )
        })
        .collect()
}

pub(super) fn load() -> HashMap<String, TrackFallbackTimer> {
    let path = crate::paths::data_root().join(PERSIST_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return HashMap::new(),
        Err(error) => {
            log::warn!(target: "provider:media", "Kunne ikke læse media-positions.json: {error}");
            return HashMap::new();
        }
    };
    match serde_json::from_slice::<PersistedState>(&bytes) {
        Ok(state) if state.version == PERSIST_VERSION => restore(state),
        Ok(_) => {
            log::warn!(target: "provider:media", "media-positions.json har ukendt version og ignoreres");
            HashMap::new()
        }
        Err(error) => {
            log::warn!(target: "provider:media", "Kunne ikke parse media-positions.json: {error}");
            HashMap::new()
        }
    }
}

pub(super) fn save(timers: &BTreeMap<String, PersistedTimer>, now_ms: i64) {
    let state = PersistedState {
        version: PERSIST_VERSION,
        saved_at_ms: now_ms,
        timers: timers.clone(),
    };
    let payload = match serde_json::to_vec_pretty(&state) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!(target: "provider:media", "Kunne ikke serialisere media-positions.json: {error}");
            return;
        }
    };
    let path = crate::paths::data_root().join(PERSIST_FILE);
    if let Some(dir) = path.parent() {
        if let Err(error) = fs::create_dir_all(dir) {
            log::warn!(target: "provider:media", "Kunne ikke oprette datamappe: {error}");
            return;
        }
    }
    let tmp = path.with_extension("json.tmp");
    if let Err(error) = fs::write(&tmp, payload).and_then(|_| fs::rename(&tmp, &path)) {
        log::warn!(target: "provider:media", "Kunne ikke gemme media-positions.json: {error}");
    }
}
