//! AI-agent-aktivitet for udviklere (lokal SQLite + session-haler).
//!
//! Producerer udelukkende ægte, observerbare sessioner fra lokale databaser og
//! logfiler — aldrig falske sessioner blot fordi en IDE som Antigravity,
//! Devin, Cursor eller Windsurf står åben.

mod process;
mod sources;
mod sqlite;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, RefreshKind, System};
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

pub use process::focus_agent_window;
use process::ProcessIndex;
use sources::{scan_all_sessions, AgentPaths, SourceCache};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
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

fn run_agents_thread(
    app: AppHandle,
    stop_signal: Arc<(Mutex<bool>, Condvar)>,
    latest: Arc<Mutex<Option<AgentsSnapshot>>>,
) {
    let paths = AgentPaths::from_env();
    let mut sys = match System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
    ) {
        Ok(sys) => sys,
        Err(err) => {
            log::warn!(target: "provider:agents", "Kunne ikke oprette proces-skanner: {err}");
            return;
        }
    };
    let mut cache = SourceCache::default();
    let mut last_emitted_sessions: Option<Vec<AgentSession>> = None;

    let (stop_lock, stop_cvar) = &*stop_signal;
    loop {
        if *stop_lock.lock() {
            break;
        }
        let now = unix_now_ms();
        let procs = ProcessIndex::scan(&mut sys);
        let sessions = scan_all_sessions(&paths, &procs, now, Some(&mut cache));
        let snapshot = AgentsSnapshot {
            sessions: sessions.clone(),
            updated_at: now,
        };
        *latest.lock() = Some(snapshot.clone());

        if last_emitted_sessions.as_deref() != Some(sessions.as_slice()) {
            last_emitted_sessions = Some(sessions);
            if let Err(err) = app.emit("provider://agents", &snapshot) {
                log::warn!(target: "provider:agents", "Kunne ikke emitte provider://agents: {err}");
            }
        }

        let wait = if app
            .try_state::<crate::state::AppState>()
            .map(|s| s.game_overlay_active.load(Ordering::Acquire))
            .unwrap_or(false)
        {
            OVERLAY_POLL_INTERVAL
        } else {
            POLL_INTERVAL
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

        fn paths(&self) -> AgentPaths {
            AgentPaths::from_roots(&self.home(), Some(&self.appdata()))
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
