//! Call-timer der overlever app-genstart og IPC-genforbindelse.
//!
//! Discord IPC råber ikke hvornår et opkald startede, så start-tidspunktet er
//! i sig selv ikke gendenneligt. Til gengæld har hver voice-session et stabilt,
//! unikt id (RPC `session_id`, eller WASAPI session-instance-id i fallback).
//! Vi gemmer `(session_id, started_at)` på disk mens opkaldet står på: matcher
//! id'et stadig efter en genstart, er opkaldet med garanti det samme, og
//! timeren fortsætter. Matcher det ikke, er det et nyt opkald og timeren
//! starter forfra.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const SESSION_FILE: &str = "discord-call.json";
/// WASAPI session-instance-id'er er ikke garanteret unikke på tværs af calls
/// (audio-session-objektet kan i princippet overleve mellem opkald i samme
/// Discord-proces). Genoptag derfor kun en ikke-autoritativ session hvis
/// heartbeat'et er friskt; ellers antager vi at opkaldet er nyt.
const WEAK_KEY_MAX_RESUME_GAP_MS: i64 = 10 * 60 * 1000;
/// Heartbeat (`updated_at`) skrives højst så ofte til disk.
const HEARTBEAT_WRITE_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedCall {
    key: String,
    started_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Default)]
pub struct CallTimer {
    path: Option<PathBuf>,
    record: Option<PersistedCall>,
    live_key: Option<String>,
    live_started_ms: Option<i64>,
    last_write_ms: i64,
}

impl CallTimer {
    pub fn load(config_dir: &Path) -> Self {
        let path = config_dir.join(SESSION_FILE);
        let record = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PersistedCall>(&bytes).ok());
        Self {
            path: Some(path),
            record,
            ..Default::default()
        }
    }

    /// Kaldes mens et opkald observeres aktivt med et verificérbart session-id.
    /// `authoritative = true` (RPC `session_id`) betyder at nøglen kun kan matche
    /// hvis selve voice-sessionen fortsatte — disk-record må derfor genoptages
    /// uanset alder. `false` (WASAPI) kræver et friskt heartbeat.
    /// Returnerer start-tidspunktet for den igangværende session.
    pub fn observe(&mut self, key: &str, authoritative: bool, now_ms: i64) -> i64 {
        if self.live_key.as_deref() == Some(key) {
            self.touch(now_ms);
            return self.live_started_ms.unwrap_or(now_ms);
        }
        if self.live_key.is_some() {
            // Session-id genforhandlet midt i et observeret opkald
            // (voice-server failover) — behold start-tidspunktet.
            self.live_key = Some(key.to_string());
            if let Some(rec) = self.record.as_mut() {
                rec.key = key.to_string();
                rec.updated_at_ms = now_ms;
            }
            self.persist();
            return self.live_started_ms.unwrap_or(now_ms);
        }

        self.live_key = Some(key.to_string());
        let restorable = self
            .record
            .as_ref()
            .filter(|rec| rec.key == key)
            .filter(|rec| {
                authoritative
                    || now_ms.saturating_sub(rec.updated_at_ms) <= WEAK_KEY_MAX_RESUME_GAP_MS
            });
        match restorable {
            Some(rec) => {
                self.live_started_ms = Some(rec.started_at_ms);
                if let Some(rec) = self.record.as_mut() {
                    rec.updated_at_ms = now_ms;
                }
            }
            None => {
                // `live_started_ms` kan allerede være sat af `observe_unknown` —
                // den beholdes som første observationstidspunkt i denne kørsel.
                let started = self.live_started_ms.unwrap_or(now_ms);
                self.record = Some(PersistedCall {
                    key: key.to_string(),
                    started_at_ms: started,
                    updated_at_ms: now_ms,
                });
            }
        }
        self.persist();
        self.live_started_ms.unwrap_or(now_ms)
    }

    /// Kaldes mens et opkald observeres uden verificérbart session-id. Kan ikke
    /// genoptage fra disk (identitet ukendt), men holder timeren stabil indtil
    /// et session-id eventuelt dukker op.
    pub fn observe_unknown(&mut self, now_ms: i64) -> i64 {
        *self.live_started_ms.get_or_insert(now_ms)
    }

    /// Opkaldet er slut — ryd live-tilstand og slet disk-recordet.
    pub fn clear(&mut self) {
        if self.live_key.is_none() && self.live_started_ms.is_none() && self.record.is_none() {
            return;
        }
        self.live_key = None;
        self.live_started_ms = None;
        self.record = None;
        self.last_write_ms = 0;
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
    }

    fn touch(&mut self, now_ms: i64) {
        if let Some(rec) = self.record.as_mut() {
            rec.updated_at_ms = now_ms;
        }
        if now_ms.saturating_sub(self.last_write_ms) >= HEARTBEAT_WRITE_MS {
            self.persist();
        }
    }

    fn persist(&mut self) {
        let (Some(path), Some(rec)) = (&self.path, &self.record) else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec(rec) else {
            return;
        };
        if fs::write(path, &bytes).is_ok() {
            self.last_write_ms = rec.updated_at_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_session_continues_after_restart() {
        let dir = std::env::temp_dir().join(format!("od_call_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("mkdir");

        let mut t = CallTimer::load(&dir);
        assert_eq!(t.observe("rpc:session-a", true, 1_000), 1_000);
        drop(t);

        // Genstart: samme session-id => timeren fortsætter fra 1_000.
        let mut t2 = CallTimer::load(&dir);
        assert_eq!(t2.observe("rpc:session-a", true, 60_000), 1_000);
        drop(t2);

        // Genstart: nyt session-id => ny timer.
        let mut t3 = CallTimer::load(&dir);
        assert_eq!(t3.observe("rpc:session-b", true, 70_000), 70_000);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn weak_session_requires_fresh_heartbeat() {
        let dir = std::env::temp_dir().join(format!("od_call_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("mkdir");

        let mut t = CallTimer::load(&dir);
        assert_eq!(t.observe("wasapi:inst-1", false, 1_000), 1_000);
        drop(t);

        // Frisk genstart (< gap) => genoptag.
        let mut t2 = CallTimer::load(&dir);
        assert_eq!(t2.observe("wasapi:inst-1", false, 2 * 60_000), 1_000);
        drop(t2);

        // Forældet heartbeat (> gap) => ny session trods samme id.
        let mut t3 = CallTimer::load(&dir);
        let later = 1_000 + WEAK_KEY_MAX_RESUME_GAP_MS + 2 * 60_000;
        assert_eq!(t3.observe("wasapi:inst-1", false, later), later);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_deletes_persisted_record() {
        let dir = std::env::temp_dir().join(format!("od_call_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("mkdir");

        let mut t = CallTimer::load(&dir);
        t.observe("rpc:session-a", true, 1_000);
        t.clear();
        drop(t);

        let mut t2 = CallTimer::load(&dir);
        assert_eq!(t2.observe("rpc:session-a", true, 5_000), 5_000);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn provisional_start_survives_until_key_arrives() {
        let mut t = CallTimer::default();
        assert_eq!(t.observe_unknown(1_000), 1_000);
        // Manglende nøgle: kan ikke genoptage, behold observationstidspunktet.
        assert_eq!(t.observe("rpc:session-a", true, 2_000), 1_000);
    }
}
