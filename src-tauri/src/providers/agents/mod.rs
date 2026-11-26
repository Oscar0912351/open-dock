//! AI-agent-aktivitet for udviklere (lokal SQLite + session-haler).
//!
//! Producerer udelukkende ægte, observerbare sessioner fra lokale databaser og
//! logfiler — aldrig falske sessioner blot fordi en IDE som Antigravity,
//! Devin, Cursor eller Windsurf står åben.

mod process;
mod sources;
mod sqlite;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use ts_rs::TS;

pub use process::focus_agent_window;
use process::ProcessIndex;
use sources::{scan_all_sessions, AgentPaths, SourceCache};

const ACTIVE_POLL_INTERVAL: Duration = Duration::from_secs(2);
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(5);
const OVERLAY_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum AgentStatus {
    /// Agenten kører et trin lige nu (tænker, redigerer, kører kommando).
    Working,
    /// Agenten afventer brugerens svar eller godkendelse.
    Waiting,
    /// Agentens tur er netop afsluttet og klar til næste prompt.
    Ready,
    /// Sessionen har nyere historik men er passiv.
    Idle,
}

impl AgentStatus {
    pub fn rank(self) -> u8 {
        match self {
            Self::Working => 3,
            Self::Waiting => 2,
            Self::Ready => 1,
            Self::Idle => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AgentTodoProgress {
    pub done: u32,
    pub total: u32,
    pub current: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AgentSession {
    /// Stabilt id (fx "devin:recondite-archduke") så UI kan bevare rækkefølge/animation.
    pub id: String,
    /// Agent-nøgle (fx "antigravity", "devin", "claude-code", "codex", "cursor").
    pub agent: String,
    /// Visningsnavn for agenten (fx "Antigravity", "Devin").
    pub agent_label: String,
    /// Samtalens eller opgavens titel når tilgængelig.
    pub title: Option<String>,
    /// Projektnavn / mappenavn når tilgængeligt (aldrig blot gentagelse af agentnavn).
    pub project: Option<String>,
    pub status: AgentStatus,
    /// Epoch ms for seneste aktivitet.
    pub last_active_ms: i64,
    /// Kort dansk handlingstekst (fx "Redigerer filer", "Stiller et spørgsmål").
    pub detail: Option<String>,
    /// Antal aktive underagenter knyttet til denne session.
    pub subagent_count: u32,
    /// Valgfri todo/plan-fremgang fra agentens egne strukturerede data.
    pub todo: Option<AgentTodoProgress>,
    /// Valgfri PID brugt til at fokusere det rigtige skrivebordsvindue ved klik.
    pub pid: Option<u32>,
    /// Om agentens proces kører lige nu.
    pub process_running: bool,
}

impl AgentSession {
    pub fn status_rank(&self) -> u8 {
        self.status.rank()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AgentsSnapshot {
    pub sessions: Vec<AgentSession>,
    pub updated_at: i64,
}

pub struct AgentsProvider {
    running: AtomicBool,
    worker: Mutex<Option<Worker>>,
    latest: Arc<Mutex<Option<AgentsSnapshot>>>,
}

struct Worker {
    stop_signal: Arc<(Mutex<bool>, Condvar)>,
    join: Option<JoinHandle<()>>,
}

impl Default for AgentsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentsProvider {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            worker: Mutex::new(None),
            latest: Arc::new(Mutex::new(None)),
        }
    }

    pub fn snapshot(&self) -> Option<AgentsSnapshot> {
        self.latest.lock().clone()
    }

    pub fn start(&self, app: &AppHandle) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        log::info!(target: "provider:agents", "Starter agents-provider v2 (SQLite + fil-haler)");
        let stop_signal = Arc::new((Mutex::new(false), Condvar::new()));
        let stop_thread = Arc::clone(&stop_signal);
        let latest = Arc::clone(&self.latest);
        let app_handle = app.clone();
        let join = std::thread::Builder::new()
            .name("od-agents".into())
            .spawn(move || run_agents_thread(app_handle, stop_thread, latest))
            .ok();
        if join.is_none() {
            log::warn!(target: "provider:agents", "Kunne ikke starte od-agents-tråd");
            self.running.store(false, Ordering::Release);
            return;
        }
        *self.worker.lock() = Some(Worker { stop_signal, join });
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::AcqRel) {
            return;
        }
        log::info!(target: "provider:agents", "Stopper agents-provider");
        if let Some(mut worker) = self.worker.lock().take() {
            let (lock, cvar) = &*worker.stop_signal;
            *lock.lock() = true;
            cvar.notify_all();
            if let Some(join) = worker.join.take() {
                let _ = join.join();
            }
        }
    }
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Default)]
struct SessionOrderMemory {
    positions: HashMap<String, u64>,
    next: u64,
}

impl SessionOrderMemory {
    fn apply(&mut self, sessions: &mut [AgentSession]) {
        for session in sessions.iter() {
            if !self.positions.contains_key(&session.id) {
                self.positions.insert(session.id.clone(), self.next);
                self.next = self.next.saturating_add(1);
            }
        }
        sessions.sort_by(|a, b| {
            b.status_rank()
                .cmp(&a.status_rank())
                .then_with(|| {
                    let a_pos = self.positions.get(&a.id).copied().unwrap_or(u64::MAX);
                    let b_pos = self.positions.get(&b.id).copied().unwrap_or(u64::MAX);
                    a_pos.cmp(&b_pos)
                })
                .then_with(|| a.id.cmp(&b.id))
        });
    }
}

fn same_session_for_ui(a: &AgentSession, b: &AgentSession) -> bool {
    a.id == b.id
        && a.agent == b.agent
        && a.agent_label == b.agent_label
        && a.title == b.title
        && a.project == b.project
        && a.status == b.status
        && a.detail == b.detail
        && a.subagent_count == b.subagent_count
        && a.todo == b.todo
        && a.pid == b.pid
        && a.process_running == b.process_running
}

fn same_sessions_for_ui(a: &[AgentSession], b: &[AgentSession]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| same_session_for_ui(x, y))
}

fn run_agents_thread(
    app: AppHandle,
    stop_signal: Arc<(Mutex<bool>, Condvar)>,
    latest: Arc<Mutex<Option<AgentsSnapshot>>>,
) {
    let paths = AgentPaths::from_env();
    let mut cache = SourceCache::default();
    let mut order_memory = SessionOrderMemory::default();
    let mut last_emitted_sessions: Option<Vec<AgentSession>> = None;

    let (stop_lock, stop_cvar) = &*stop_signal;
    loop {
        if *stop_lock.lock() {
            break;
        }
        let now = unix_now_ms();
        let procs = ProcessIndex::scan_tracked();
        let mut sessions = scan_all_sessions(&paths, &procs, now, Some(&mut cache));
        order_memory.apply(&mut sessions);
        let any_active = sessions
            .iter()
            .any(|s| matches!(s.status, AgentStatus::Working | AgentStatus::Waiting));
        let snapshot = AgentsSnapshot {
            sessions: sessions.clone(),
            updated_at: now,
        };
        *latest.lock() = Some(snapshot.clone());

        let changed_for_ui = last_emitted_sessions
            .as_deref()
            .map(|prev| !same_sessions_for_ui(prev, sessions.as_slice()))
            .unwrap_or(true);
        if changed_for_ui {
            last_emitted_sessions = Some(sessions);
            super::scheduler::emit_provider(
                &app,
                super::scheduler::ProviderKind::Agents,
                None,
                "provider://agents",
                &snapshot,
            );
        }

        let in_overlay = app
            .try_state::<crate::state::AppState>()
            .map(|s| s.game_overlay_active.load(Ordering::Acquire))
            .unwrap_or(false);
        let wait = if in_overlay {
            OVERLAY_POLL_INTERVAL
        } else if any_active {
            ACTIVE_POLL_INTERVAL
        } else {
            IDLE_POLL_INTERVAL
        };
        let mut stopped = stop_lock.lock();
        if *stopped {
            break;
        }
        stop_cvar.wait_for(&mut stopped, wait);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    struct TestDir {
        root: PathBuf,
    }

    impl TestDir {
        fn new(tag: &str) -> Self {
            let uniq = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let root = std::env::temp_dir().join(format!(
                "open_dock_agents_{tag}_{}_{uniq}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn home(&self) -> PathBuf {
            self.root.join("home")
        }

        fn appdata(&self) -> PathBuf {
            self.root.join("appdata")
        }

        fn local_appdata(&self) -> PathBuf {
            self.root.join("local_appdata")
        }

        fn paths(&self) -> AgentPaths {
            AgentPaths::from_roots(
                &self.home(),
                Some(&self.appdata()),
                Some(&self.local_appdata()),
            )
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_file(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    fn test_session(id: &str, status: AgentStatus, last_active_ms: i64) -> AgentSession {
        AgentSession {
            id: id.to_string(),
            agent: "devin".to_string(),
            agent_label: "Devin".to_string(),
            title: None,
            project: None,
            status,
            last_active_ms,
            detail: None,
            subagent_count: 0,
            todo: None,
            pid: None,
            process_running: true,
        }
    }

    #[test]
    fn session_order_is_stable_when_only_activity_timing_changes() {
        let mut memory = SessionOrderMemory::default();
        let mut first = vec![
            test_session("devin:a", AgentStatus::Working, 100),
            test_session("devin:b", AgentStatus::Working, 200),
            test_session("devin:c", AgentStatus::Ready, 300),
        ];
        memory.apply(&mut first);
        assert_eq!(
            first.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["devin:a", "devin:b", "devin:c"]
        );

        let mut second = vec![
            test_session("devin:b", AgentStatus::Working, 500),
            test_session("devin:a", AgentStatus::Working, 400),
            test_session("devin:c", AgentStatus::Ready, 600),
        ];
        memory.apply(&mut second);
        assert_eq!(
            second.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["devin:a", "devin:b", "devin:c"]
        );
    }

    #[test]
    fn ui_comparison_ignores_last_active_but_not_visible_changes() {
        let a = vec![test_session("devin:a", AgentStatus::Working, 100)];
        let mut b = vec![test_session("devin:a", AgentStatus::Working, 2_100)];
        assert!(same_sessions_for_ui(&a, &b));

        b[0].detail = Some("Redigerer filer".to_string());
        assert!(!same_sessions_for_ui(&a, &b));

        let reordered = vec![
            test_session("devin:b", AgentStatus::Working, 100),
            test_session("devin:a", AgentStatus::Working, 100),
        ];
        let original_order = vec![
            test_session("devin:a", AgentStatus::Working, 100),
            test_session("devin:b", AgentStatus::Working, 100),
        ];
        assert!(!same_sessions_for_ui(&original_order, &reordered));
    }

    #[test]
    fn parses_rfc3339_and_workspace_uri() {
        let ms = sources::parse_timestamp_ms("2026-11-21 23:48:21.4639664+00:00").unwrap();
        let ms_utc_z = sources::parse_timestamp_ms("2026-11-21T23:48:21.463Z").unwrap();
        assert_eq!(ms, ms_utc_z);

        let ms_offset = sources::parse_timestamp_ms("2026-11-22T00:48:21.463+01:00").unwrap();
        assert_eq!(ms_offset, ms_utc_z);

        assert_eq!(
            sources::project_from_uri("file:///c%3A/Users/oscar/Desktop/APPS"),
            Some("APPS".to_string())
        );
        assert_eq!(
            sources::project_from_uri("file:///C:/Users/oscar/Desktop/Open_Dock/"),
            Some("Open_Dock".to_string())
        );
    }

    #[test]
    fn open_idle_ides_produce_zero_ghost_sessions() {
        let td = TestDir::new("no_ghost");
        let paths = td.paths();
        let procs = ProcessIndex::from_exes(&["antigravity", "devin", "cursor", "windsurf"]);
        let now = 1_795_305_000_000;

        // Opret Antigravity-DB hvor alle samtaler er idle og ældre end 1 time
        fs::create_dir_all(paths.agy_db.parent().unwrap()).unwrap();
        let conn = Connection::open(&paths.agy_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE conversation_summaries (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                not_fully_idle INTEGER NOT NULL,
                nesting_depth INTEGER NOT NULL,
                parent_conversation_id TEXT NOT NULL,
                workspace_uris TEXT NOT NULL,
                last_modified_time TEXT NOT NULL,
                step_count INTEGER NOT NULL,
                killed INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO conversation_summaries VALUES (?1, ?2, 'CASCADE_RUN_STATUS_IDLE', 0, 0, '', ?3, ?4, 12, 0)",
            (
                "old-convo",
                "Gammel opgave",
                r#"["file:///c%3A/Users/oscar/Desktop/APPS"]"#,
                "2026-11-21 10:00:00+00:00",
            ),
        )
        .unwrap();

        let sessions = scan_all_sessions(&paths, &procs, now, None);
        assert!(sessions.is_empty(), "Forventede 0 ghost-sessioner, fik: {sessions:?}");
    }

    #[test]
    fn detects_antigravity_running_subagents_and_waiting_or_crashed_states() {
        let td = TestDir::new("agy");
        let paths = td.paths();
        let now = sources::parse_timestamp_ms("2026-11-21 23:48:25+00:00").unwrap();

        fs::create_dir_all(paths.agy_db.parent().unwrap()).unwrap();
        let conn = Connection::open(&paths.agy_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE conversation_summaries (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                not_fully_idle INTEGER NOT NULL,
                nesting_depth INTEGER NOT NULL,
                parent_conversation_id TEXT NOT NULL,
                workspace_uris TEXT NOT NULL,
                last_modified_time TEXT NOT NULL,
                step_count INTEGER NOT NULL,
                killed INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO conversation_summaries VALUES ('c1', 'Byg ny widget', 'CASCADE_RUN_STATUS_RUNNING', 1, 0, '', '[\"file:///c%3A/Users/oscar/Desktop/APPS\"]', '2026-11-21 23:48:21.4639664+00:00', 10, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO conversation_summaries VALUES ('sub1', '', 'CASCADE_RUN_STATUS_RUNNING', 1, 1, 'c1', '[]', '2026-11-21 23:48:22+00:00', 4, 0)",
            [],
        )
        .unwrap();

        write_file(
            &paths.agy_brain.join("c1/.system_generated/logs/transcript.jsonl"),
            concat!(
                r#"{"step_index":1,"type":"USER_INPUT","status":"DONE"}"#,
                "\n",
                r#"{"step_index":2,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"edit_file"}]}"#,
                "\n"
            ),
        );

        // Når task.md findes med tjekliste, udtrækkes todo-fremgang
        write_file(
            &paths.agy_brain.join("c1/task.md"),
            "- [x] Undersøg kodebase\n- [/] **Byg semantiske prikker** <!-- id: 2 -->\n- [ ] Kør tests\n",
        );

        let procs_live = ProcessIndex::from_exes(&["Antigravity.exe"]);
        let live = sources::scan_antigravity(&paths, &procs_live, now);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].title.as_deref(), Some("Byg ny widget"));
        assert_eq!(live[0].project.as_deref(), Some("APPS"));
        assert_eq!(live[0].status, AgentStatus::Working);
        assert_eq!(live[0].detail.as_deref(), Some("Redigerer filer"));
        assert_eq!(live[0].subagent_count, 1);
        let todo = live[0].todo.as_ref().unwrap();
        assert_eq!(todo.done, 1);
        assert_eq!(todo.total, 3);
        assert_eq!(todo.current.as_deref(), Some("Byg semantiske prikker"));

        // Når ask_question kaldes sidst, vises Waiting
        write_file(
            &paths.agy_brain.join("c1/.system_generated/logs/transcript.jsonl"),
            r#"{"step_index":3,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"ask_question"}]}"#,
        );
        let waiting = sources::scan_antigravity(&paths, &procs_live, now);
        assert_eq!(waiting[0].status, AgentStatus::Waiting);
        assert_eq!(waiting[0].detail.as_deref(), Some("Stiller et spørgsmål"));

        // Når processen er lukket (crash), må den aldrig stå som Working
        let procs_dead = ProcessIndex::empty();
        let crashed = sources::scan_antigravity(&paths, &procs_dead, now);
        assert_ne!(crashed[0].status, AgentStatus::Working);
        assert_eq!(crashed[0].subagent_count, 0);
    }

    #[test]
    fn detects_devin_working_and_ready_and_ignores_stale_failed_tools() {
        let td = TestDir::new("devin");
        let paths = td.paths();
        let db = paths.devin_db.as_ref().unwrap();
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                working_directory TEXT NOT NULL,
                backend_type TEXT NOT NULL,
                model TEXT NOT NULL,
                agent_mode TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL,
                title TEXT,
                hidden INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE message_nodes (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                node_id INTEGER NOT NULL,
                chat_message TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE tool_call_state (
                session_id TEXT NOT NULL,
                tool_call_id TEXT NOT NULL,
                tool_call_json TEXT,
                tool_call_update_json TEXT,
                PRIMARY KEY (session_id, tool_call_id)
            );",
        )
        .unwrap();

        let now_sec: i64 = 1_795_305_000;
        let now_ms = now_sec * 1000;

        conn.execute(
            "INSERT INTO sessions VALUES ('s-work', 'C:\\Users\\oscar\\Desktop\\APPS\\SIDEQUESTS\\Open_Dock', 'acp', 'm', 'a', ?1, ?1, 'Ret tema-kort', 0)",
            [now_sec - 5],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-work', 1, ?1, ?2)",
            (
                r#"{"role":"user","content":"Start","metadata":{"is_user_input":1}}"#,
                now_sec - 10,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-work', 2, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"t0","name":"todo_write","arguments":{"todos":[{"content":"Hent data","status":"completed"},{"content":"Opdatér UI","status":"in_progress"},{"content":"Verificér","status":"pending"}]}}]}"#,
                now_sec - 8,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-work', 3, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"1","name":"edit_file"}]}"#,
                now_sec - 5,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tool_call_state VALUES ('s-work', 'c1', '{\"kind\":\"edit\"}', NULL)",
            [],
        )
        .unwrap();

        // Færdig session (tom tool_calls: [], content != "", selvom summarizer-rækker ligger bagefter + stale todo før ny prompt)
        conn.execute(
            "INSERT INTO sessions VALUES ('s-done', 'C:\\Users\\oscar\\Desktop\\APPS', 'acp', 'm', 'a', ?1, ?1, 'Gammelt fail-tool men færdig', 0)",
            [now_sec - 20],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-done', 1, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"t-old","name":"todo_write","arguments":{"todos":[{"content":"Gammel task","status":"completed"}]}}]}"#,
                now_sec - 40,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-done', 2, ?1, ?2)",
            (
                r#"{"role":"user","content":"Nyt spørgsmål uden todos","metadata":{"is_user_input":1}}"#,
                now_sec - 30,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-done', 3, ?1, ?2)",
            (r#"{"role":"assistant","content":"Færdig!","tool_calls":[]}"#, now_sec - 20),
        )
        .unwrap();
        // Devin baggrunds-summarizer efter afsluttet svar
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-done', 4, ?1, ?2)",
            (r#"{"role":"system","content":"You are a Summarizer"}"#, now_sec - 19),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-done', 5, ?1, ?2)",
            (r#"{"role":"user","content":"Conversation to summarize:\n..."}"#, now_sec - 19),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-done', 6, ?1, ?2)",
            (r#"{"role":"user","content":"Now summarize the conversation above"}"#, now_sec - 18),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-done', 7, ?1, ?2)",
            (r#"{"role":"assistant","content":"<summary>1. Request...</summary>"}"#, now_sec - 17),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tool_call_state VALUES ('s-done', 'old-fail', '{\"kind\":\"execute\"}', '{\"status\":\"failed\"}')",
            [],
        )
        .unwrap();

        let procs = ProcessIndex::from_exes(&["Devin.exe", "devin.exe"]);
        let res = sources::scan_devin(db, &procs, now_ms);
        assert_eq!(res.len(), 2);

        let work = res.iter().find(|s| s.id == "devin:s-work").unwrap();
        assert_eq!(work.status, AgentStatus::Working);
        assert_eq!(work.detail.as_deref(), Some("Redigerer filer"));
        assert_eq!(work.project.as_deref(), Some("Open_Dock"));
        let wtodo = work.todo.as_ref().unwrap();
        assert_eq!(wtodo.done, 1);
        assert_eq!(wtodo.total, 3);
        assert_eq!(wtodo.current.as_deref(), Some("Opdatér UI"));

        let done = res.iter().find(|s| s.id == "devin:s-done").unwrap();
        assert_eq!(done.status, AgentStatus::Ready);
        assert_eq!(done.detail.as_deref(), Some("Klar til svar"));
        assert!(done.todo.is_none(), "Stale færdig todo før ny prompt skal ignoreres");
    }

    #[test]
    fn devin_subagent_activity_overrides_stale_session_timestamp() {
        let td = TestDir::new("devin_subagent");
        let paths = td.paths();
        let db = paths.devin_db.as_ref().unwrap();
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                working_directory TEXT NOT NULL,
                backend_type TEXT NOT NULL,
                model TEXT NOT NULL,
                agent_mode TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL,
                title TEXT,
                hidden INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE message_nodes (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                node_id INTEGER NOT NULL,
                chat_message TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE tool_call_state (
                session_id TEXT NOT NULL,
                tool_call_id TEXT NOT NULL,
                tool_call_json TEXT,
                tool_call_update_json TEXT,
                PRIMARY KEY (session_id, tool_call_id)
            );
            CREATE TABLE subagent_heads (
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                chain_node_id INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (session_id, agent_id)
            );
            CREATE TABLE live_row_times (
                session_id TEXT NOT NULL,
                row_key TEXT NOT NULL,
                started_at TEXT NOT NULL,
                PRIMARY KEY (session_id, row_key)
            );",
        )
        .unwrap();

        let now_sec: i64 = 1_795_305_000;
        let now_ms = now_sec * 1000;
        conn.execute(
            "INSERT INTO sessions VALUES ('s-sub', 'C:\\Users\\oscar\\proj', 'acp', 'm', 'a', ?1, ?1, 'Med subagent', 0)",
            [now_sec - 1200],
        )
        .unwrap();
        // Fyld listen med nyere hovedsessioner, så den stale subagent-session kun
        // findes via subagent_heads og ikke via de 12 nyeste sessions-rækker.
        for i in 0..12 {
            conn.execute(
                "INSERT INTO sessions VALUES (?1, 'C:\\Users\\oscar\\proj', 'acp', 'm', 'a', ?2, ?2, 'Filler', 0)",
                (format!("s-filler-{i}"), now_sec - i64::from(i)),
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-sub', 1, ?1, ?2)",
            (
                r#"{"role":"user","content":"Start","metadata":{"is_user_input":1}}"#,
                now_sec - 1200,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-sub', 3, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"sk1","name":"exec"}]}"#,
                now_sec - 700,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-sub', 4, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"Hovedsvaret er givet","tool_calls":[]}"#,
                now_sec - 600,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO subagent_heads VALUES ('s-sub', 'sidekick', 3, ?1)",
            [now_sec - 5],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO live_row_times VALUES ('s-sub', 'tool:sk::call-1', ?1)",
            [(now_sec - 5).to_string()],
        )
        .unwrap();

        let procs = ProcessIndex::from_exes(&["devin.exe"]);
        let res = sources::scan_devin(db, &procs, now_ms);
        assert_eq!(res.len(), 13);
        let sub = res.iter().find(|s| s.id == "devin:s-sub").unwrap();
        assert_eq!(sub.status, AgentStatus::Working);
        assert_eq!(sub.detail.as_deref(), Some("Kører underagent"));
        assert_eq!(sub.subagent_count, 1);
        assert_eq!(sub.last_active_ms, (now_sec - 5) * 1000);

        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-sub', 5, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"Sidekick er færdig","tool_calls":[]}"#,
                now_sec - 4,
            ),
        )
        .unwrap();
        conn.execute(
            "UPDATE subagent_heads SET chain_node_id = 5, updated_at = ?1 WHERE session_id = 's-sub'",
            [now_sec - 4],
        )
        .unwrap();

        let done = sources::scan_devin(db, &procs, now_ms);
        let done_sub = done.iter().find(|s| s.id == "devin:s-sub").unwrap();
        assert_eq!(done_sub.status, AgentStatus::Ready);
        assert_eq!(done_sub.subagent_count, 0);
    }

    #[test]
    fn devin_status_follows_main_chain_instead_of_newer_side_branch() {
        let td = TestDir::new("devin_main_chain");
        let paths = td.paths();
        let db = paths.devin_db.as_ref().unwrap();
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                working_directory TEXT NOT NULL,
                backend_type TEXT NOT NULL,
                model TEXT NOT NULL,
                agent_mode TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL,
                title TEXT,
                main_chain_id INTEGER,
                hidden INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE message_nodes (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                node_id INTEGER NOT NULL,
                parent_node_id INTEGER,
                chat_message TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE tool_call_state (
                session_id TEXT NOT NULL,
                tool_call_id TEXT NOT NULL,
                tool_call_json TEXT,
                tool_call_update_json TEXT,
                PRIMARY KEY (session_id, tool_call_id)
            );",
        )
        .unwrap();

        let now_sec: i64 = 1_795_305_000;
        let now_ms = now_sec * 1000;
        conn.execute(
            "INSERT INTO sessions (id, working_directory, backend_type, model, agent_mode, created_at, last_activity_at, title, main_chain_id, hidden) VALUES ('s-chain', 'C:\\Users\\oscar\\proj', 'acp', 'm', 'a', ?1, ?1, 'Gren-test', 5, 0)",
            [now_sec - 5],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, parent_node_id, chat_message, created_at) VALUES ('s-chain', 1, NULL, ?1, ?2)",
            (
                r#"{"role":"user","content":"Start","metadata":{"is_user_input":1}}"#,
                now_sec - 30,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, parent_node_id, chat_message, created_at) VALUES ('s-chain', 2, 1, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"t1","name":"exec"}]}"#,
                now_sec - 20,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, parent_node_id, chat_message, created_at) VALUES ('s-chain', 3, 2, ?1, ?2)",
            (
                r#"{"role":"tool","content":"ok"}"#,
                now_sec - 10,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, parent_node_id, chat_message, created_at) VALUES ('s-chain', 4, 1, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"Sidegrenen ser færdig ud","tool_calls":[]}"#,
                now_sec - 4,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, parent_node_id, chat_message, created_at) VALUES ('s-chain', 5, 3, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"","tool_calls":[],"metadata":{"query_label":"title"}}"#,
                now_sec - 3,
            ),
        )
        .unwrap();

        let procs = ProcessIndex::from_exes(&["devin.exe"]);
        let res = sources::scan_devin(db, &procs, now_ms);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].status, AgentStatus::Working);
        assert_ne!(res[0].detail.as_deref(), Some("Klar til svar"));
    }

    #[test]
    fn devin_live_rows_keep_active_generation_working_until_final_message() {
        let td = TestDir::new("devin_live_rows");
        let paths = td.paths();
        let db = paths.devin_db.as_ref().unwrap();
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                working_directory TEXT NOT NULL,
                backend_type TEXT NOT NULL,
                model TEXT NOT NULL,
                agent_mode TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL,
                title TEXT,
                hidden INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE message_nodes (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                node_id INTEGER NOT NULL,
                chat_message TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE tool_call_state (
                session_id TEXT NOT NULL,
                tool_call_id TEXT NOT NULL,
                tool_call_json TEXT,
                tool_call_update_json TEXT,
                PRIMARY KEY (session_id, tool_call_id)
            );
            CREATE TABLE live_row_times (
                session_id TEXT NOT NULL,
                row_key TEXT NOT NULL,
                started_at TEXT NOT NULL,
                PRIMARY KEY (session_id, row_key)
            );",
        )
        .unwrap();

        let now_sec: i64 = 1_795_305_000;
        let now_ms = now_sec * 1000;
        conn.execute(
            "INSERT INTO sessions VALUES ('s-live', 'C:\\Users\\oscar\\proj', 'acp', 'm', 'a', ?1, ?1, 'Live generation', 0)",
            [now_sec - 30],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-live', 1, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"Tidligere svar","tool_calls":[]}"#,
                now_sec - 30,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO live_row_times VALUES ('s-live', 'thought:new-turn', ?1)",
            [(now_sec - 5).to_string()],
        )
        .unwrap();

        let procs = ProcessIndex::from_exes(&["devin.exe"]);
        let active = sources::scan_devin(db, &procs, now_ms);
        assert_eq!(active[0].status, AgentStatus::Working);
        assert_eq!(active[0].detail.as_deref(), Some("Arbejder…"));

        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s-live', 2, ?1, ?2)",
            (
                r#"{"role":"assistant","content":"Nyt færdigt svar","tool_calls":[]}"#,
                now_sec - 2,
            ),
        )
        .unwrap();
        let done = sources::scan_devin(db, &procs, now_ms);
        assert_eq!(done[0].status, AgentStatus::Ready);
    }

    #[test]
    fn db_fingerprint_ignores_shm_read_marks() {
        let td = TestDir::new("wal_fp");
        let db = td.home().join("wal.db");
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let writer = Connection::open(&db).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL; CREATE TABLE t(v TEXT); INSERT INTO t VALUES('a');",
            )
            .unwrap();

        let fp1 = sqlite::db_fingerprint(&db).unwrap();
        {
            let ro = sqlite::open_readonly(&db).unwrap();
            let v: String = ro
                .query_row("SELECT v FROM t LIMIT 1", [], |r| r.get(0))
                .unwrap();
            assert_eq!(v, "a");
        }
        std::thread::sleep(Duration::from_millis(30));
        let fp2 = sqlite::db_fingerprint(&db).unwrap();
        assert_eq!(fp1, fp2, "read-only scan må ikke invalidere DB-cachen");

        writer.execute("INSERT INTO t VALUES('b')", []).unwrap();
        let fp3 = sqlite::db_fingerprint(&db).unwrap();
        assert_ne!(fp2, fp3);
    }

    #[test]
    fn devin_msg_cache_reuses_rows_until_session_changes() {
        let td = TestDir::new("devin_memo");
        let paths = td.paths();
        let db = paths.devin_db.as_ref().unwrap();
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                working_directory TEXT NOT NULL,
                backend_type TEXT NOT NULL,
                model TEXT NOT NULL,
                agent_mode TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL,
                title TEXT,
                hidden INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE message_nodes (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                node_id INTEGER NOT NULL,
                chat_message TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE tool_call_state (
                session_id TEXT NOT NULL,
                tool_call_id TEXT NOT NULL,
                tool_call_json TEXT,
                tool_call_update_json TEXT,
                PRIMARY KEY (session_id, tool_call_id)
            );",
        )
        .unwrap();

        let now_sec: i64 = 1_795_305_000;
        let now_ms = now_sec * 1000;
        conn.execute(
            "INSERT INTO sessions VALUES ('s1', 'C:\\Users\\oscar\\proj', 'acp', 'm', 'a', ?1, ?1, 'T1', 0)",
            [now_sec - 5],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s1', 1, ?1, ?2)",
            (
                r#"{"role":"user","content":"Start","metadata":{"is_user_input":1}}"#,
                now_sec - 5,
            ),
        )
        .unwrap();

        let procs = ProcessIndex::from_exes(&["devin.exe"]);
        let mut cache = SourceCache::default();
        let r1 = sources::scan_devin_cached(db, &procs, now_ms, Some(&mut cache));
        let r2 = sources::scan_devin_cached(db, &procs, now_ms, Some(&mut cache));
        assert_eq!(r1, r2, "Anden scan med varm cache skal give samme resultat");
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].status, AgentStatus::Working);

        // Ny besked-ralje => cachenøglen rammer forbi og status genberegnes.
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at) VALUES ('s1', 2, ?1, ?2)",
            (r#"{"role":"assistant","content":"Færdig!","tool_calls":[]}"#, now_sec - 3),
        )
        .unwrap();
        let r3 = sources::scan_devin_cached(db, &procs, now_ms, Some(&mut cache));
        assert_eq!(r3[0].status, AgentStatus::Ready);
        assert_eq!(r3[0].detail.as_deref(), Some("Klar til svar"));
    }

    #[test]
    fn claude_code_fixes_inverted_semantics_and_reads_todos() {
        let td = TestDir::new("claude");
        let paths = td.paths();
        let proj = paths.claude_projects.join("C--Users-oscar-Desktop-Open_Dock");

        // 1. Bruger har lige sendt besked / tool_result -> agent TÆNKER (Working) + TodoWrite i hale
        let f_think = proj.join("sess-think.jsonl");
        write_file(
            &f_think,
            concat!(
                r#"{"type":"user","cwd":"C:\\Users\\oscar\\Desktop\\Open_Dock","message":{"content":"Hjælp med X"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"A","status":"completed"},{"content":"B","activeForm":"Arbejder på B","status":"in_progress"}]}}]}}"#,
                "\n",
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"1","content":"ok"}]}}"#,
                "\n"
            ),
        );

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let procs = ProcessIndex::from_exes(&["claude.exe"]);
        let r1 = sources::scan_claude_code(&paths, &procs, now);
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].status, AgentStatus::Working);
        assert_eq!(r1[0].detail.as_deref(), Some("Tænker…"));
        assert_eq!(r1[0].project.as_deref(), Some("Open_Dock"));
        let ctodo = r1[0].todo.as_ref().unwrap();
        assert_eq!(ctodo.done, 1);
        assert_eq!(ctodo.total, 2);
        assert_eq!(ctodo.current.as_deref(), Some("Arbejder på B"));

        // ~/.claude/todos/<stem>.json vinder hvis nyere/tilgængelig
        write_file(
            &paths.claude_todos.join("sess-think.json"),
            r#"[{"content":"1","status":"completed"},{"content":"2","status":"completed"},{"content":"3","status":"in_progress","activeForm":"Tester C"}]"#,
        );
        let r_file = sources::scan_claude_code(&paths, &procs, now);
        assert_eq!(r_file[0].todo.as_ref().map(|t| (t.done, t.total)), Some((2, 3)));
        assert_eq!(
            r_file[0].todo.as_ref().and_then(|t| t.current.as_deref()),
            Some("Tester C")
        );

        // 2. Assistant med AskUserQuestion -> Waiting
        write_file(
            &f_think,
            concat!(
                r#"{"type":"user","cwd":"C:\\Users\\oscar\\Desktop\\Open_Dock","message":{"content":"Hjælp"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"AskUserQuestion"}]}}"#,
                "\n"
            ),
        );
        let r2 = sources::scan_claude_code(&paths, &procs, now);
        assert_eq!(r2[0].status, AgentStatus::Waiting);
        assert_eq!(r2[0].detail.as_deref(), Some("Stiller et spørgsmål"));

        // 3. Assistant uden tool_use -> Ready
        write_file(
            &f_think,
            concat!(
                r#"{"type":"user","cwd":"C:\\Users\\oscar\\Desktop\\Open_Dock","message":{"content":"Hjælp"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Her er svaret."}]}}"#,
                "\n"
            ),
        );
        let r3 = sources::scan_claude_code(&paths, &procs, now);
        assert_eq!(r3[0].status, AgentStatus::Ready);
        assert_eq!(r3[0].detail.as_deref(), Some("Klar til svar"));
    }

    #[test]
    fn cursor_filters_drafts_detects_blocking_waiting_and_todos() {
        let td = TestDir::new("cursor");
        let paths = td.paths();
        let db = paths.cursor_db.as_ref().unwrap();
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            "CREATE TABLE composerHeaders (
                composerId TEXT PRIMARY KEY,
                workspaceId TEXT,
                createdAt INTEGER,
                lastUpdatedAt INTEGER,
                isArchived INTEGER,
                isSubagent INTEGER,
                recency INTEGER,
                value TEXT
            );
            CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();

        let now = 1_795_305_000_000i64;
        conn.execute(
            "INSERT INTO composerHeaders VALUES ('draft-1', 'w', ?1, ?1, 0, 0, ?1, ?2)",
            (now - 1000, r#"{"isDraft":true}"#),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO composerHeaders VALUES ('c-wait', 'w', ?1, ?1, 0, 0, ?1, ?2)",
            (
                now - 1000,
                r#"{"name":"Refactor API","hasBlockingPendingActions":true,"draftTarget":{"environment":{"uri":{"fsPath":"C:\\repo\\Open_Dock"}}}}"#,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV VALUES ('composerData:c-wait', ?1)",
            [r#"{"status":"running","todos":[{"content":"Step 1","status":"completed"},{"content":"Approve SQL","status":"in_progress"}]}"#],
        )
        .unwrap();

        let procs = ProcessIndex::from_exes(&["Cursor.exe"]);
        let out = sources::scan_cursor(db, &procs, now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title.as_deref(), Some("Refactor API"));
        assert_eq!(out[0].project.as_deref(), Some("Open_Dock"));
        assert_eq!(out[0].status, AgentStatus::Waiting);
        let todo = out[0].todo.as_ref().unwrap();
        assert_eq!(todo.done, 1);
        assert_eq!(todo.total, 2);
        assert_eq!(todo.current.as_deref(), Some("Approve SQL"));
    }

    #[test]
    fn codex_extracts_update_plan_todos() {
        let td = TestDir::new("codex");
        let paths = td.paths();
        let rollout = paths.codex_sessions.join("2026/11/22/rollout-abc.jsonl");
        write_file(
            &rollout,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"t1","cwd":"C:\\repo\\Open_Dock"}}"#,
                "\n",
                r#"{"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{\"plan\":[{\"step\":\"Læs kode\",\"status\":\"completed\"},{\"step\":\"Implementér plan\",\"status\":\"in_progress\"},{\"step\":\"Verificér\",\"status\":\"pending\"}]}"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#,
                "\n"
            ),
        );

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let procs = ProcessIndex::from_exes(&["codex.exe"]);
        let res = sources::scan_codex(&paths, &procs, now);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].status, AgentStatus::Ready);
        let todo = res[0].todo.as_ref().unwrap();
        assert_eq!(todo.done, 1);
        assert_eq!(todo.total, 3);
        assert_eq!(todo.current.as_deref(), Some("Implementér plan"));
    }

    fn create_hermes_db(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                source TEXT,
                display_name TEXT,
                parent_session_id TEXT,
                started_at REAL NOT NULL,
                ended_at REAL,
                cwd TEXT,
                title TEXT,
                last_activity_at REAL,
                last_activity_description TEXT,
                profile_name TEXT,
                hidden INTEGER NOT NULL DEFAULT 0,
                archived INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT,
                tool_name TEXT,
                tool_calls TEXT,
                display_kind TEXT,
                timestamp REAL NOT NULL
            );
            CREATE TABLE session_turn_leases (
                conversation_id TEXT PRIMARY KEY,
                holder TEXT NOT NULL,
                acquired_at REAL NOT NULL,
                expires_at REAL NOT NULL
            );
            CREATE TABLE worker_executions (
                execution_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                status TEXT NOT NULL
            );",
        )
        .unwrap();
    }

    #[test]
    fn hermes_reads_profiles_leases_and_ready_sessions() {
        let td = TestDir::new("hermes");
        let paths = td.paths();
        let root = paths.hermes_root.as_ref().unwrap();
        let now_sec: f64 = 1_795_305_000.0;
        let now_ms = (now_sec * 1000.0) as i64;

        // Profil-DB med én live (lease) og én klar session
        let pdb = root.join("profiles/ossi-research/state.db");
        create_hermes_db(&pdb);
        let conn = Connection::open(&pdb).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source, title, cwd, profile_name, started_at, last_activity_at, ended_at, hidden, archived)
             VALUES ('s-live','local','Optimér pc','C:\\repo\\Open_Dock','ossi-research',?1,?1,NULL,0,0)",
            [now_sec - 30.0],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_turn_leases VALUES ('s-live','gw',?1,?2)",
            (now_sec - 10.0, now_sec + 60.0),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, role, tool_name, tool_calls, timestamp)
             VALUES ('s-live','assistant','terminal','[{\"a\":1}]',?1)",
            [now_sec - 20.0],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source, title, cwd, profile_name, started_at, last_activity_at, ended_at, hidden, archived)
             VALUES ('s-ready','desktop','Læs koden',NULL,'ossi-research',?1,?1,NULL,0,0)",
            [now_sec - 120.0],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp)
             VALUES ('s-ready','assistant','Færdig.',?1)",
            [now_sec - 120.0],
        )
        .unwrap();
        // Skjult + subagent rækker må ikke give egne kort
        conn.execute(
            "INSERT INTO sessions (id, source, title, profile_name, started_at, last_activity_at, ended_at, hidden, archived)
             VALUES ('s-hidden','desktop','Hemmelig','ossi-research',?1,?1,NULL,1,0)",
            [now_sec - 10.0],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source, title, profile_name, parent_session_id, started_at, last_activity_at, ended_at, hidden, archived)
             VALUES ('s-sub','desktop','Underagent','ossi-research','s-live',?1,?1,NULL,0,0)",
            [now_sec - 10.0],
        )
        .unwrap();
        drop(conn);

        let procs = ProcessIndex::from_exes(&["hermes.exe"]);
        let out = sources::scan_hermes(root, &procs, now_ms);
        assert_eq!(out.len(), 2, "skjult + subagent filtreres fra: {out:?}");

        let live = out.iter().find(|s| s.id == "hermes:ossi-research:s-live").unwrap();
        assert_eq!(live.status, AgentStatus::Working);
        assert_eq!(live.detail.as_deref(), Some("Kører kommando"));
        assert_eq!(live.project.as_deref(), Some("Open_Dock"));
        assert_eq!(live.subagent_count, 1);

        let ready = out.iter().find(|s| s.id == "hermes:ossi-research:s-ready").unwrap();
        assert_eq!(ready.status, AgentStatus::Ready);
        assert_eq!(ready.project.as_deref(), Some("ossi-research"));

        // Uden kørende proces gælder kort efter stadig SHORT_ACTIVE_MS -> Working tilladt
        let dead = ProcessIndex::empty();
        let out_dead = sources::scan_hermes(root, &dead, now_ms);
        let live_dead = out_dead
            .iter()
            .find(|s| s.id == "hermes:ossi-research:s-live")
            .unwrap();
        assert_eq!(live_dead.status, AgentStatus::Working);
        // Langt senere er leasen udløbet og aktiviteten gammel -> aldrig Working
        let later = sources::scan_hermes(root, &dead, now_ms + 10 * 60 * 1000);
        assert!(later
            .iter()
            .filter(|s| s.id == "hermes:ossi-research:s-live")
            .all(|s| s.status != AgentStatus::Working));
    }

    #[test]
    fn grok_bot_reads_persistence_blobs_and_daemon() {
        let td = TestDir::new("grokbot");
        let paths = td.paths();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let dir = paths.grokbot_persistence.as_ref().unwrap();
        write_file(
            &dir.join("conv1.blob"),
            &format!(
                r#"{{"schemaVersion":1,"value":{{"entries":[
                {{"kind":"message","role":"user","content":"Hej","timestampMs":{}}},
                {{"kind":"message","role":"assistant","content":"Svar","toAgent":{{"id":"a1","name":"Cora"}},"timestampMs":{}}}
            ],"persistedAt":{}}}}}"#,
                now - 60_000,
                now - 5_000,
                now
            ),
        );
        write_file(
            &dir.join("conv2.blob"),
            &format!(
                r#"{{"schemaVersion":1,"value":{{"entries":[
                {{"kind":"message","role":"assistant","content":"Ok","timestampMs":{}}},
                {{"kind":"send-message","message":{{"type":"text","content":"Arbejder…"}},"author":{{"id":"a1","name":"Cora"}},"timestampMs":{}}}
            ],"persistedAt":{}}}}}"#,
                now - 60_000,
                now - 2_000,
                now
            ),
        );
        // Daemon med aktiv computer-use opgave
        write_file(
            &paths.grokbot_daemon,
            r#"{"pid":1,"startedAt":1795304800000,"inflightCount":2}"#,
        );

        let procs = ProcessIndex::from_exes(&["Grok Bot.exe"]);
        let out = sources::scan_grok_bot(&paths, &procs, now);
        assert_eq!(out.len(), 3, "daemon + 2 samtaler: {out:?}");

        let exec = out.iter().find(|s| s.id == "grok-bot:local-exec").unwrap();
        assert_eq!(exec.status, AgentStatus::Working);
        assert_eq!(exec.detail.as_deref(), Some("Udfører 2 lokale handlinger"));

        let ready = out.iter().find(|s| s.id == "grok-bot:conv1").unwrap();
        assert_eq!(ready.status, AgentStatus::Ready);
        assert_eq!(ready.title.as_deref(), Some("Cora"));

        let work = out.iter().find(|s| s.id == "grok-bot:conv2").unwrap();
        assert_eq!(work.status, AgentStatus::Working);

        // Uden process -> ingen Working-tilstand
        let dead = ProcessIndex::empty();
        let out_dead = sources::scan_grok_bot(&paths, &dead, now + 60_000);
        assert!(out_dead.iter().all(|s| s.status != AgentStatus::Working));
    }

    #[test]
    fn detects_qoder_live_turn_ready_and_waiting_states() {
        let td = TestDir::new("qoder");
        let paths = td.paths();
        let now: i64 = 1_795_469_200_000;

        let db = paths.qoder_db.as_ref().unwrap();
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            "CREATE TABLE chat_sessions (
                session_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                cwd TEXT NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0,
                deleted_at TIMESTAMP
            );
            CREATE TABLE chat_session_messages (
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                role TEXT,
                status TEXT NOT NULL DEFAULT 'completed',
                payload_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (session_id, message_id)
            );",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO chat_sessions VALUES ('s1', 'Gennemgå system', 'C:\\Users\\oscar\\Desktop\\APPS', ?1, 0, NULL)",
            [now - 5_000],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_session_messages (session_id, message_id, sequence, role, status, payload_json, created_at, updated_at) \
             VALUES ('s1', 'm1', 1, 'assistant', 'completed', '{}', ?1, ?1)",
            [now - 60_000],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO chat_sessions VALUES ('s2', 'Anden opgave', 'D:\\work\\Proj', ?1, 0, NULL)",
            [now - 10_000],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_session_messages (session_id, message_id, sequence, role, status, payload_json, created_at, updated_at) \
             VALUES ('s2', 'm2', 1, 'assistant', 'completed', '{}', ?1, ?1)",
            [now - 10_000],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO chat_sessions VALUES ('s3', 'Venter på svar', 'D:\\work\\Wait', ?1, 0, NULL)",
            [now - 3_000],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_session_messages (session_id, message_id, sequence, role, status, payload_json, created_at, updated_at) \
             VALUES ('s3', 'm3', 1, 'assistant', 'completed', '{}', ?1, ?1)",
            [now - 3_000],
        )
        .unwrap();

        // Turn-buffer: s1 kører Edit, s3 venter med AskUserQuestion.
        let tdb = paths.qoder_turn_db.as_ref().unwrap();
        let tconn = Connection::open(tdb).unwrap();
        tconn.execute_batch(
            "CREATE TABLE active_turn_projection_items (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                parent_tool_use_id TEXT,
                item_type TEXT NOT NULL,
                item_json TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (session_id, turn_id, item_id)
            );
            CREATE TABLE pending_turn_projections (
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (session_id, message_id)
            );",
        )
        .unwrap();
        tconn.execute(
            "INSERT INTO active_turn_projection_items \
             (session_id, turn_id, item_id, ordinal, parent_tool_use_id, item_type, item_json, created_at) \
             VALUES ('s1', 't1', 'i1', 1, NULL, 'assistant', ?1, ?2)",
            (
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{}}]}}"#,
                now - 2_000,
            ),
        )
        .unwrap();
        tconn.execute(
            "INSERT INTO active_turn_projection_items \
             (session_id, turn_id, item_id, ordinal, parent_tool_use_id, item_type, item_json, created_at) \
             VALUES ('s3', 't3', 'i3', 1, NULL, 'assistant', ?1, ?2)",
            (
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"AskUserQuestion","input":{}}]}}"#,
                now - 1_000,
            ),
        )
        .unwrap();

        let procs = ProcessIndex::from_exes(&["Qoder.exe"]);
        let sessions = sources::scan_qoder(&paths, &procs, now);
        assert_eq!(sessions.len(), 3, "alle tre sessioner: {sessions:?}");

        let s1 = sessions.iter().find(|s| s.id == "qoder:s1").unwrap();
        assert_eq!(s1.status, AgentStatus::Working);
        assert_eq!(s1.detail.as_deref(), Some("Redigerer filer"));
        assert_eq!(s1.title.as_deref(), Some("Gennemgå system"));
        assert_eq!(s1.project.as_deref(), Some("APPS"));
        assert_eq!(s1.agent_label, "Qoder");

        let s2 = sessions.iter().find(|s| s.id == "qoder:s2").unwrap();
        assert_eq!(s2.status, AgentStatus::Ready);
        assert_eq!(s2.project.as_deref(), Some("Proj"));

        let s3 = sessions.iter().find(|s| s.id == "qoder:s3").unwrap();
        assert_eq!(s3.status, AgentStatus::Waiting);
        assert_eq!(s3.detail.as_deref(), Some("Stiller et spørgsmål"));

        // Uden kørende proces nedgraderes live-turns til message-DB-status.
        let dead = sources::scan_qoder(&paths, &ProcessIndex::empty(), now);
        let s1d = dead.iter().find(|s| s.id == "qoder:s1").unwrap();
        assert_eq!(s1d.status, AgentStatus::Ready);
        let s3d = dead.iter().find(|s| s.id == "qoder:s3").unwrap();
        assert_ne!(s3d.status, AgentStatus::Working);
        assert_ne!(s3d.status, AgentStatus::Waiting);
    }

    #[test]
    #[ignore = "Kører mod brugerens rigtige lokale databaser"]
    fn live_scan_smoke() {
        let paths = AgentPaths::from_env();
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
        )
        .unwrap();
        let procs = ProcessIndex::scan(&mut sys);
        let now = unix_now_ms();
        let sessions = scan_all_sessions(&paths, &procs, now, None);
        println!("Live agent-sessioner ({}):", sessions.len());
        for s in &sessions {
            println!(
                "  [{:?}] {} | title={:?} | proj={:?} | detail={:?} | subs={} | todo={:?} | pid={:?}",
                s.status, s.agent_label, s.title, s.project, s.detail, s.subagent_count, s.todo, s.pid
            );
        }
    }
}
