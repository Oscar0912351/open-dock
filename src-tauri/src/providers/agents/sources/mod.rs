//! Lokale adaptere for rigtige agent-datakilder (SQLite + begrænsede fil-haler).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use super::{
    process::ProcessIndex,
    sqlite::db_fingerprint,
    AgentSession, AgentStatus,
};

mod adapters;
mod common;

#[derive(Debug, Clone)]
pub struct AgentPaths {
    pub agy_db: PathBuf,
    pub agy_brain: PathBuf,
    pub devin_db: Option<PathBuf>,
    pub codex_db: PathBuf,
    pub codex_sessions: PathBuf,
    pub claude_projects: PathBuf,
    pub claude_todos: PathBuf,
    pub cursor_db: Option<PathBuf>,
    pub gemini_tmp: PathBuf,
    pub opencode_dirs: Vec<PathBuf>,
    pub windsurf_cascade: PathBuf,
    pub appdata: Option<PathBuf>,
    pub hermes_root: Option<PathBuf>,
    pub grokbot_persistence: Option<PathBuf>,
    pub grokbot_daemon: PathBuf,
    pub qoder_db: Option<PathBuf>,
    pub qoder_turn_db: Option<PathBuf>,
}

impl AgentPaths {
    pub fn from_env() -> Self {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/"));
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
        let local_appdata = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                appdata
                    .as_ref()
                    .and_then(|p| p.parent().map(|base| base.join("Local")))
            });
        Self::from_roots(&home, appdata.as_deref(), local_appdata.as_deref())
    }

    pub fn from_roots(home: &Path, appdata: Option<&Path>, local_appdata: Option<&Path>) -> Self {
        let appdata_buf = appdata.map(Path::to_path_buf);
        Self {
            agy_db: home.join(".gemini/antigravity/conversation_summaries.db"),
            agy_brain: home.join(".gemini/antigravity/brain"),
            devin_db: appdata_buf
                .as_ref()
                .map(|p| p.join("devin/cli/sessions.db")),
            codex_db: home.join(".codex/sqlite/codex.db"),
            codex_sessions: home.join(".codex/sessions"),
            claude_projects: home.join(".claude/projects"),
            claude_todos: home.join(".claude/todos"),
            cursor_db: appdata_buf
                .as_ref()
                .map(|p| p.join("Cursor/User/globalStorage/state.vscdb")),
            gemini_tmp: home.join(".gemini/tmp"),
            opencode_dirs: vec![
                home.join(".local/share/opencode"),
                home.join(".config/opencode"),
            ],
            windsurf_cascade: home.join(".codeium/windsurf/cascade"),
            hermes_root: local_appdata.map(|p| p.join("hermes")),
            grokbot_persistence: appdata_buf
                .as_ref()
                .map(|p| p.join("Grok Bot/sand-client-persistence")),
            grokbot_daemon: home.join(".grokbot/local-exec-daemon.json"),
            qoder_db: appdata_buf
                .as_ref()
                .map(|p| p.join("com.qoder.app.stable/main.sqlite")),
            qoder_turn_db: appdata_buf
                .as_ref()
                .map(|p| p.join("com.qoder.app.stable/chat-session-turn-payload-buffer.sqlite")),
            appdata: appdata_buf,
        }
    }
}

/// Cache af DB-resultater, parsed fil-haler og manglende mapper så uændrede kilder aldrig genlæses fra disk.
const MISSING_DIR_TTL_MS: i64 = 60_000;
const MAX_CACHED_FILES: usize = 256;

#[derive(Default)]
pub struct SourceCache {
    db_entries: HashMap<PathBuf, CachedSource>,
    missing_dirs: HashMap<PathBuf, i64>,
    file_entries: HashMap<PathBuf, CachedFileSession>,
    devin_sessions: HashMap<String, CachedDevinMsg>,
}

struct CachedSource {
    fingerprint: (i64, u64),
    sessions: Vec<AgentSession>,
}

/// Rå message_nodes-række for en Devin-session, nøglet på
/// (max row_id, last_activity_at) — genbruges når nøglen er uændret.
struct CachedDevinMsg {
    key: (i64, i64, Option<i64>),
    msg_row: Option<DevinMsgRow>,
}

type DevinMsgRow = (i64, String, i64, Option<String>, i64, i64);

#[derive(Clone)]
struct CachedFileSession {
    fingerprint: (i64, u64),
    session: AgentSession,
    /// For Codex .jsonl: den udtrukne thread_id som nøgle i `by_thread`.
    merge_key: Option<String>,
    /// Rå tail-status fra seneste læsning (så `allow_working` kan gen-evalueres i RAM).
    raw_working_status: Option<AgentStatus>,
    raw_working_detail: Option<String>,
}

impl SourceCache {
    fn dir_exists(&mut self, dir: &Path, now: i64) -> bool {
        if let Some(&checked_at) = self.missing_dirs.get(dir) {
            if now - checked_at < MISSING_DIR_TTL_MS {
                return false;
            }
        }
        if dir.is_dir() {
            self.missing_dirs.remove(dir);
            true
        } else {
            self.missing_dirs.insert(dir.to_path_buf(), now);
            false
        }
    }

    fn get_db_if_unchanged(
        &self,
        path: &Path,
        fp: (i64, u64),
        procs: &ProcessIndex,
        now: i64,
    ) -> Option<Vec<AgentSession>> {
        let entry = self.db_entries.get(path)?;
        if entry.fingerprint != fp {
            return None;
        }
        Some(reconcile_cached_sessions(&entry.sessions, procs, now))
    }

    fn store_db(&mut self, path: &Path, fp: Option<(i64, u64)>, sessions: &[AgentSession]) {
        if let Some(fingerprint) = fp {
            self.db_entries.insert(
                path.to_path_buf(),
                CachedSource {
                    fingerprint,
                    sessions: sessions.to_vec(),
                },
            );
        }
    }

    fn get_file(&self, path: &Path, fp: (i64, u64)) -> Option<&CachedFileSession> {
        self.file_entries.get(path).filter(|e| e.fingerprint == fp)
    }

    fn store_file(&mut self, path: PathBuf, entry: CachedFileSession) {
        if self.file_entries.len() >= MAX_CACHED_FILES && !self.file_entries.contains_key(&path) {
            self.file_entries.clear();
        }
        self.file_entries.insert(path, entry);
    }
}

fn exes_for_cached_agent(agent: &str) -> &'static [&'static str] {
    match agent {
        "antigravity" => &["antigravity", "language_server"],
        "devin" => &["devin"],
        "codex" => &["codex", "codex-cli"],
        "claude-code" => &["claude", "claude-code"],
        "cursor" => &["cursor"],
        "gemini" => &["gemini"],
        "opencode" => &["opencode"],
        "windsurf" => &["windsurf"],
        "grok-bot" => &["grok bot"],
        "hermes" => &["hermes", "hermes-acp"],
        "qoder" => &["qoder"],
        _ => &[],
    }
}

fn reconcile_cached_sessions(
    cached: &[AgentSession],
    procs: &ProcessIndex,
    now: i64,
) -> Vec<AgentSession> {
    let mut out = Vec::with_capacity(cached.len());
    for s in cached {
        let mut item = s.clone();
        let exes = exes_for_cached_agent(&item.agent);
        if !exes.is_empty() {
            item.process_running = procs.has_any(exes);
            item.pid = procs.first_pid(exes);
        }
        let age = now - item.last_active_ms;
        if matches!(item.status, AgentStatus::Working | AgentStatus::Waiting)
            && (!item.process_running || age > WORKING_TIMEOUT_MS)
        {
            item.status = AgentStatus::Idle;
            item.detail = None;
            item.subagent_count = 0;
        }
        let still_active = matches!(item.status, AgentStatus::Working | AgentStatus::Waiting);
        if still_active || (item.last_active_ms > 0 && age <= MAX_RECENT_MS) {
            out.push(item);
        }
    }
    out
}

impl SourceCache {
    pub(super) fn scan_db_cached(
        cache: &mut Option<&mut SourceCache>,
        path: &Path,
        procs: &ProcessIndex,
        now: i64,
        key: impl FnOnce((i64, u64)) -> (i64, u64),
        scan: impl FnOnce(Option<&mut SourceCache>) -> Vec<AgentSession>,
    ) -> Vec<AgentSession> {
        let Some(base_fp) = db_fingerprint(path) else {
            return Vec::new();
        };
        let fp = key(base_fp);
        if let Some(cached) = cache
            .as_deref()
            .and_then(|c| c.get_db_if_unchanged(path, fp, procs, now))
        {
            return cached;
        }
        let fresh = scan(cache.as_deref_mut());
        if let Some(c) = cache.as_deref_mut() {
            c.store_db(path, Some(fp), &fresh);
        }
        fresh
    }
}

pub fn scan_all_sessions(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let mut sessions = Vec::new();
    for adapter in adapters::ALL_ADAPTERS {
        sessions.extend(adapter.scan(paths, procs, now, cache.as_deref_mut()));
    }

    sessions.sort_by(|a, b| {
        b.status_rank()
            .cmp(&a.status_rank())
            .then_with(|| b.last_active_ms.cmp(&a.last_active_ms))
            .then_with(|| a.id.cmp(&b.id))
    });
    sessions
}

#[allow(unused_imports)]
pub use adapters::{
    aider::scan_aider,
    antigravity::scan_antigravity,
    claude_code::scan_claude_code,
    codex::scan_codex,
    cursor::scan_cursor,
    devin::{scan_devin, scan_devin_cached},
    editor_tasks::scan_editor_tasks,
    gemini_cli::scan_gemini_cli,
    grok_bot::scan_grok_bot,
    hermes::scan_hermes,
    opencode::scan_opencode,
    qoder::scan_qoder,
    windsurf::scan_windsurf_cascade,
};
#[allow(unused_imports)]
pub use common::{
    parse_markdown_todos, parse_timestamp_ms, project_from_path_str, project_from_uri,
};
pub use common::{MAX_RECENT_MS, WORKING_TIMEOUT_MS};
