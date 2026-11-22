//! Lokale adaptere for rigtige agent-datakilder (SQLite + begrænsede fil-haler).

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde_json::Value;

use super::{
    process::ProcessIndex,
    sqlite::{db_fingerprint, open_readonly},
    AgentSession, AgentStatus, AgentTodoProgress,
};

/// Sessioner ældre end 1 time returneres ikke (UI filtrerer videre efter brugervalg).
pub const MAX_RECENT_MS: i64 = 60 * 60 * 1000;
/// Timeout for at nedgradere stale "arbejder"-signaler uden friske opdateringer.
pub const WORKING_TIMEOUT_MS: i64 = 3 * 60 * 1000;
/// Korte fil-læsninger når processen ikke kan bekræftes (f.eks. hurtig CLI-kørsel).
const SHORT_ACTIVE_MS: i64 = 45 * 1000;
/// Subagenter betragtes som aktive i dette vindue.
const SUBAGENT_ACTIVE_MS: i64 = 90 * 1000;
const HEAD_BYTES: usize = 8 * 1024;
const TAIL_BYTES: usize = 16 * 1024;
const TODO_TAIL_BYTES: usize = 64 * 1024;

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
}

impl AgentPaths {
    pub fn from_env() -> Self {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/"));
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
        Self::from_roots(&home, appdata.as_deref())
    }

    pub fn from_roots(home: &Path, appdata: Option<&Path>) -> Self {
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
            appdata: appdata_buf,
        }
    }
}

/// Letvægts cache af DB-resultater så uændrede SQLite-filer ikke genlæses unødigt.
#[derive(Default)]
pub struct SourceCache {
    db_entries: HashMap<PathBuf, CachedSource>,
}

struct CachedSource {
    fingerprint: (i64, u64),
    cached_at_ms: i64,
    sessions: Vec<AgentSession>,
}

impl SourceCache {
    fn get_if_fresh(
        &self,
        path: &Path,
        fp: (i64, u64),
        now: i64,
    ) -> Option<Vec<AgentSession>> {
        let entry = self.db_entries.get(path)?;
        if entry.fingerprint != fp {
            return None;
        }
        // Hvis nogen session stadig er aktiv, genlæs altid så status/timeout opdateres præcist.
        if entry
            .sessions
            .iter()
            .any(|s| matches!(s.status, AgentStatus::Working | AgentStatus::Waiting))
        {
            return None;
        }
        if now - entry.cached_at_ms > 10_000 {
            return None;
        }
        Some(entry.sessions.clone())
    }

    fn store(&mut self, path: &Path, fp: Option<(i64, u64)>, now: i64, sessions: &[AgentSession]) {
        if let Some(fingerprint) = fp {
            self.db_entries.insert(
                path.to_path_buf(),
                CachedSource {
                    fingerprint,
                    cached_at_ms: now,
                    sessions: sessions.to_vec(),
                },
            );
        }
    }
}

pub fn scan_all_sessions(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let mut sessions = Vec::new();

    if let Some(cached) = cache
        .as_deref()
        .and_then(|c| db_fingerprint(&paths.agy_db).and_then(|fp| c.get_if_fresh(&paths.agy_db, fp, now)))
    {
        sessions.extend(cached);
    } else {
        let agy = scan_antigravity(paths, procs, now);
        if let Some(c) = cache.as_deref_mut() {
            c.store(&paths.agy_db, db_fingerprint(&paths.agy_db), now, &agy);
        }
        sessions.extend(agy);
    }

    if let Some(devin_db) = paths.devin_db.as_deref() {
        if let Some(cached) = cache
            .as_deref()
            .and_then(|c| db_fingerprint(devin_db).and_then(|fp| c.get_if_fresh(devin_db, fp, now)))
        {
            sessions.extend(cached);
        } else {
            let devin = scan_devin(devin_db, procs, now);
            if let Some(c) = cache.as_deref_mut() {
                c.store(devin_db, db_fingerprint(devin_db), now, &devin);
            }
            sessions.extend(devin);
        }
    }

    sessions.extend(scan_codex(paths, procs, now));
    sessions.extend(scan_claude_code(paths, procs, now));

    if let Some(cursor_db) = paths.cursor_db.as_deref() {
        if let Some(cached) = cache
            .as_deref()
            .and_then(|c| db_fingerprint(cursor_db).and_then(|fp| c.get_if_fresh(cursor_db, fp, now)))
        {
            sessions.extend(cached);
        } else {
            let cursor = scan_cursor(cursor_db, procs, now);
            if let Some(c) = cache.as_deref_mut() {
                c.store(cursor_db, db_fingerprint(cursor_db), now, &cursor);
            }
            sessions.extend(cursor);
        }
    }

    if let Some(appdata) = paths.appdata.as_deref() {
        sessions.extend(scan_editor_tasks(appdata, procs, now));
    }

    sessions.extend(scan_gemini_cli(&paths.gemini_tmp, procs, now));
    sessions.extend(scan_opencode(&paths.opencode_dirs, procs, now));
    sessions.extend(scan_windsurf_cascade(&paths.windsurf_cascade, procs, now));
    sessions.extend(scan_aider(procs, now));

    sessions.sort_by(|a, b| {
        b.status_rank()
            .cmp(&a.status_rank())
            .then_with(|| b.last_active_ms.cmp(&a.last_active_ms))
            .then_with(|| a.id.cmp(&b.id))
    });
    sessions
}

// ---------------------------------------------------------------------------
// 1. Antigravity (conversation_summaries.db + transcript.jsonl)
// ---------------------------------------------------------------------------

struct AgyRow {
    id: String,
    title: String,
    status: String,
    not_fully_idle: i64,
    nesting_depth: i64,
    parent_id: String,
    workspace_uris: String,
    last_modified_time: String,
}

pub fn scan_antigravity(paths: &AgentPaths, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let Some(conn) = open_readonly(&paths.agy_db) else {
        return Vec::new();
    };
    let mut stmt = match conn.prepare(
        "SELECT conversation_id, title, status, not_fully_idle, nesting_depth, \
         parent_conversation_id, workspace_uris, last_modified_time \
         FROM conversation_summaries \
         WHERE killed = 0 AND step_count > 0 \
         ORDER BY last_modified_time DESC \
         LIMIT 120",
    ) {
        Ok(s) => s,
        Err(err) => {
            log::debug!(target: "provider:agents", "Antigravity SQL-fejl: {err}");
            return Vec::new();
        }
    };

    let rows_iter = match stmt.query_map([], |row| {
        Ok(AgyRow {
            id: row.get::<_, String>(0).unwrap_or_default(),
            title: row.get::<_, String>(1).unwrap_or_default(),
            status: row.get::<_, String>(2).unwrap_or_default(),
            not_fully_idle: row.get::<_, i64>(3).unwrap_or(0),
            nesting_depth: row.get::<_, i64>(4).unwrap_or(0),
            parent_id: row.get::<_, String>(5).unwrap_or_default(),
            workspace_uris: row.get::<_, String>(6).unwrap_or_default(),
            last_modified_time: row.get::<_, String>(7).unwrap_or_default(),
        })
    }) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let rows: Vec<AgyRow> = rows_iter.flatten().collect();
    let antigravity_running = procs.has_any(&["antigravity", "language_server"]);
    let antigravity_pid = procs.first_pid(&["antigravity"]);

    // Tæl aktive subagenter pr. top-level samtale.
    let mut subagent_counts: HashMap<String, u32> = HashMap::new();
    for r in &rows {
        if r.nesting_depth > 0 && !r.parent_id.is_empty() && antigravity_running {
            let sub_active = r.status.contains("RUNNING")
                || r.status.contains("WAITING")
                || r.not_fully_idle != 0;
            if sub_active {
                *subagent_counts.entry(r.parent_id.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut out = Vec::new();
    for r in rows {
        if r.nesting_depth > 0 || r.id.is_empty() {
            continue;
        }
        let last_active_ms = parse_timestamp_ms(&r.last_modified_time).unwrap_or(0);
        let is_running_flag =
            (r.status.contains("RUNNING") || r.not_fully_idle != 0) && antigravity_running;
        let is_waiting_flag = r.status.contains("WAITING") && antigravity_running;

        if !is_running_flag && !is_waiting_flag && (last_active_ms <= 0 || now - last_active_ms > MAX_RECENT_MS) {
            continue;
        }

        let transcript = paths
            .agy_brain
            .join(&r.id)
            .join(".system_generated/logs/transcript.jsonl");
        let tail_info = read_agy_transcript_tail(&transcript);

        let (status, detail) = if is_waiting_flag {
            let d = tail_info
                .tool_detail
                .filter(|_| tail_info.asked_question)
                .unwrap_or_else(|| "Afventer godkendelse".to_string());
            (AgentStatus::Waiting, Some(d))
        } else if is_running_flag {
            if tail_info.asked_question {
                (AgentStatus::Waiting, Some("Stiller et spørgsmål".to_string()))
            } else {
                let d = tail_info
                    .tool_detail
                    .or_else(|| tail_info.fallback_detail)
                    .unwrap_or_else(|| "Arbejder…".to_string());
                (AgentStatus::Working, Some(d))
            }
        } else if r.status.contains("IDLE") {
            if tail_info.finished_reply {
                (AgentStatus::Ready, Some("Klar til svar".to_string()))
            } else {
                (AgentStatus::Idle, None)
            }
        } else {
            (AgentStatus::Idle, None)
        };

        let title = non_empty_trimmed(&r.title);
        let project = parse_agy_workspace_project(&r.workspace_uris);
        let subagent_count = subagent_counts.get(&r.id).copied().unwrap_or(0);
        let task_path = paths.agy_brain.join(&r.id).join("task.md");
        let todo = read_head(&task_path, HEAD_BYTES * 2)
            .and_then(|md| parse_markdown_todos(&md));

        out.push(AgentSession {
            id: format!("antigravity:{}", r.id),
            agent: "antigravity".to_string(),
            agent_label: "Antigravity".to_string(),
            title,
            project,
            status,
            last_active_ms: last_active_ms.max(0),
            detail,
            subagent_count,
            todo,
            pid: antigravity_pid,
            process_running: antigravity_running,
        });
    }

    out
}

#[derive(Default)]
struct AgyTranscriptTail {
    asked_question: bool,
    tool_detail: Option<String>,
    fallback_detail: Option<String>,
    finished_reply: bool,
}

fn read_agy_transcript_tail(path: &Path) -> AgyTranscriptTail {
    let mut info = AgyTranscriptTail::default();
    let Some(lines) = read_tail_lines(path, TAIL_BYTES) else {
        return info;
    };

    for line in lines.iter().rev().take(12) {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(ty) = val.get("type").and_then(Value::as_str) else {
            continue;
        };
        match ty {
            "USER_INPUT" => {
                info.fallback_detail = Some("Tænker…".to_string());
                break;
            }
            "GENERIC" => {
                info.fallback_detail = Some("Tænker…".to_string());
                break;
            }
            "PLANNER_RESPONSE" => {
                let calls = val
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .filter(|a| !a.is_empty());
                if let Some(calls) = calls {
                    let has_question = calls.iter().any(|c| {
                        c.get("name").and_then(Value::as_str) == Some("ask_question")
                    });
                    if has_question {
                        info.asked_question = true;
                        info.tool_detail = Some("Stiller et spørgsmål".to_string());
                    } else if let Some(name) = calls
                        .last()
                        .and_then(|c| c.get("name"))
                        .and_then(Value::as_str)
                    {
                        info.tool_detail = Some(map_agy_tool_name(name));
                    }
                    break;
                }
                let has_content = val
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
                let has_thinking = val
                    .get("thinking")
                    .and_then(Value::as_str)
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
                if has_content {
                    info.finished_reply = true;
                } else if has_thinking {
                    info.fallback_detail = Some("Tænker…".to_string());
                }
                break;
            }
            _ => {}
        }
    }

    info
}

fn map_agy_tool_name(name: &str) -> String {
    match name {
        "ask_question" => "Stiller et spørgsmål",
        "run_command" => "Kører kommando",
        "edit_file" | "write_file" => "Redigerer filer",
        "read_file" => "Læser filer",
        "search_web" => "Søger på nettet",
        "invoke_subagent" | "swarm" => "Kører underagent",
        "exit_plan_mode" => "Skifter til udførelse",
        "call_mcp_tool" | _ => "Bruger værktøj",
    }
    .to_string()
}

fn parse_agy_workspace_project(raw_json: &str) -> Option<String> {
    let arr = serde_json::from_str::<Value>(raw_json).ok()?;
    let first = arr.as_array()?.first()?.as_str()?;
    project_from_uri(first)
}

// ---------------------------------------------------------------------------
// 2. Devin (AppData\Roaming\devin\cli\sessions.db)
// ---------------------------------------------------------------------------

struct DevinSessionRow {
    id: String,
    title: String,
    working_directory: String,
    last_activity_at: i64,
}

pub fn scan_devin(db_path: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let Some(conn) = open_readonly(db_path) else {
        return Vec::new();
    };
    let mut stmt = match conn.prepare(
        "SELECT id, COALESCE(title, ''), working_directory, last_activity_at \
         FROM sessions \
         WHERE hidden = 0 \
         ORDER BY last_activity_at DESC \
         LIMIT 12",
    ) {
        Ok(s) => s,
        Err(err) => {
            log::debug!(target: "provider:agents", "Devin SQL-fejl: {err}");
            return Vec::new();
        }
    };

    let rows_iter = match stmt.query_map([], |row| {
        Ok(DevinSessionRow {
            id: row.get::<_, String>(0).unwrap_or_default(),
            title: row.get::<_, String>(1).unwrap_or_default(),
            working_directory: row.get::<_, String>(2).unwrap_or_default(),
            last_activity_at: row.get::<_, i64>(3).unwrap_or(0),
        })
    }) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let devin_running = procs.has_any(&["devin"]);
    let devin_pid = procs.first_pid(&["devin"]);
    let mut out = Vec::new();

    for row in rows_iter.flatten() {
        if row.id.is_empty() {
            continue;
        }
        let last_active_ms = normalize_epoch_ms(row.last_activity_at);
        if last_active_ms <= 0 || now - last_active_ms > MAX_RECENT_MS {
            continue;
        }

        let msg_row: Option<(i64, String, i64, Option<String>, i64)> = conn
            .query_row(
                "SELECT row_id, \
                        COALESCE(json_extract(chat_message, '$.role'), ''), \
                        COALESCE(json_array_length(chat_message, '$.tool_calls'), 0), \
                        json_extract(chat_message, '$.tool_calls[0].name'), \
                        CASE WHEN length(COALESCE(json_extract(chat_message, '$.content'), '')) > 0 THEN 1 ELSE 0 END \
                 FROM message_nodes \
                 WHERE session_id = ?1 \
                   AND ( \
                     json_extract(chat_message, '$.role') = 'tool' \
                     OR ( \
                       json_extract(chat_message, '$.role') = 'assistant' \
                       AND COALESCE(substr(ltrim(json_extract(chat_message, '$.content')), 1, 9), '') != '<summary>' \
                     ) \
                     OR ( \
                       json_extract(chat_message, '$.role') = 'user' \
                       AND ( \
                         json_extract(chat_message, '$.metadata.is_user_input') = 1 \
                         OR ( \
                           json_extract(chat_message, '$.metadata.is_user_input') IS NULL \
                           AND COALESCE(json_extract(chat_message, '$.content'), '') NOT LIKE '%Conversation to summarize%' \
                           AND COALESCE(json_extract(chat_message, '$.content'), '') NOT LIKE 'Now summarize%' \
                         ) \
                       ) \
                     ) \
                   ) \
                 ORDER BY row_id DESC \
                 LIMIT 1",
                [&row.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .ok();

        let tc_row: Option<(Option<String>, String)> = conn
            .query_row(
                "SELECT json_extract(tool_call_json, '$.kind'), \
                        CASE WHEN tool_call_update_json IS NULL THEN 'running' \
                             ELSE COALESCE(json_extract(tool_call_update_json, '$.status'), 'unknown') \
                        END \
                 FROM tool_call_state \
                 WHERE session_id = ?1 \
                 ORDER BY rowid DESC \
                 LIMIT 1",
                [&row.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();

        let (_last_msg_row_id, role, tool_call_count, first_tool_name, has_content) = match msg_row {
            Some((rid, r, c, ft, hc)) => (rid, r, c, ft, hc != 0),
            None => (0, String::new(), 0, None, false),
        };
        let (tc_kind, tc_status) = match tc_row {
            Some((k, s)) => (k, s),
            None => (None, String::new()),
        };

        let turn_finished = role == "assistant" && tool_call_count == 0 && has_content;
        let has_live_tool = !turn_finished
            && matches!(tc_status.as_str(), "running" | "pending" | "in_progress");
        let turn_in_progress =
            matches!(role.as_str(), "user" | "tool") || (role == "assistant" && tool_call_count > 0);
        let asking_user = matches!(
            first_tool_name.as_deref(),
            Some("ask_user" | "ask_question" | "AskUserQuestion")
        );

        let (status, detail) = if devin_running
            && (has_live_tool || (turn_in_progress && now - last_active_ms <= WORKING_TIMEOUT_MS))
        {
            if asking_user {
                (AgentStatus::Waiting, Some("Stiller et spørgsmål".to_string()))
            } else {
                let d = devin_working_detail(
                    tc_kind.as_deref(),
                    first_tool_name.as_deref(),
                    role.as_str(),
                    has_live_tool,
                );
                (AgentStatus::Working, Some(d))
            }
        } else if turn_finished {
            (AgentStatus::Ready, Some("Klar til svar".to_string()))
        } else {
            (AgentStatus::Idle, None)
        };

        let todo = extract_devin_todo(&conn, &row.id);
        let title = non_empty_trimmed(&row.title);
        let project = project_from_path_str(&row.working_directory);

        out.push(AgentSession {
            id: format!("devin:{}", row.id),
            agent: "devin".to_string(),
            agent_label: "Devin".to_string(),
            title,
            project,
            status,
            last_active_ms,
            detail,
            subagent_count: 0,
            todo,
            pid: devin_pid,
            process_running: devin_running,
        });
    }

    out
}

fn extract_devin_todo(conn: &rusqlite::Connection, session_id: &str) -> Option<AgentTodoProgress> {
    let last_user_row_id: Option<i64> = conn
        .query_row(
            "SELECT MAX(row_id) FROM message_nodes \
             WHERE session_id = ?1 \
               AND json_extract(chat_message, '$.role') = 'user' \
               AND json_extract(chat_message, '$.metadata.is_user_input') = 1",
            [session_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();

    if let Ok(mut stmt) = conn.prepare(
        "SELECT row_id, chat_message FROM message_nodes \
         WHERE session_id = ?1 \
           AND json_extract(chat_message, '$.role') = 'assistant' \
           AND chat_message LIKE '%todo_write%' \
         ORDER BY row_id DESC \
         LIMIT 3",
    ) {
        if let Ok(iter) = stmt.query_map([session_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        }) {
            for (row_id, raw_msg) in iter.flatten() {
                if let Some(todo) = parse_devin_message_todo(&raw_msg) {
                    if let Some(user_rid) = last_user_row_id {
                        if row_id < user_rid && todo.done >= todo.total {
                            return None;
                        }
                    }
                    return Some(todo);
                }
            }
        }
    }

    // Fallback til tool_call_state hvis ingen message_node blev fundet
    let tc_raw: Option<String> = conn
        .query_row(
            "SELECT tool_call_json FROM tool_call_state \
             WHERE session_id = ?1 AND tool_call_json LIKE '%\"todos\"%' \
             ORDER BY rowid DESC LIMIT 1",
            [session_id],
            |r| r.get(0),
        )
        .ok();
    if let Some(raw) = tc_raw {
        if let Ok(val) = serde_json::from_str::<Value>(&raw) {
            if let Some(items) = val.pointer("/rawInput/todos").and_then(Value::as_array) {
                return parse_todo_array(items);
            }
        }
    }

    None
}

fn parse_devin_message_todo(raw_msg: &str) -> Option<AgentTodoProgress> {
    let val = serde_json::from_str::<Value>(raw_msg).ok()?;
    let calls = val.get("tool_calls")?.as_array()?;
    for tc in calls.iter().rev() {
        if tc.get("name").and_then(Value::as_str) != Some("todo_write") {
            continue;
        }
        if let Some(items) = tc.pointer("/arguments/todos").and_then(Value::as_array) {
            return parse_todo_array(items);
        }
        if let Some(arg_str) = tc.get("arguments").and_then(Value::as_str) {
            if let Ok(parsed) = serde_json::from_str::<Value>(arg_str) {
                if let Some(items) = parsed.get("todos").and_then(Value::as_array) {
                    return parse_todo_array(items);
                }
            }
        }
    }
    None
}

fn devin_working_detail(
    tc_kind: Option<&str>,
    first_tool_name: Option<&str>,
    role: &str,
    has_live_tool: bool,
) -> String {
    if first_tool_name == Some("todo_write") {
        return "Opdaterer plan".to_string();
    }
    if matches!(role, "user" | "tool") && !has_live_tool {
        return "Tænker…".to_string();
    }
    match tc_kind {
        Some("edit") => "Redigerer filer",
        Some("execute") => "Kører kommando",
        Some("read") => "Læser filer",
        Some("search") => "Søger i kode",
        _ => "Arbejder…",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// 3. Codex (~/.codex/sqlite/codex.db + ~/.codex/sessions/**/*.jsonl)
// ---------------------------------------------------------------------------

pub fn scan_codex(paths: &AgentPaths, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let codex_running = procs.has_any(&["codex", "codex-cli"]);
    let codex_pid = procs.first_pid(&["codex", "codex-cli"]);
    let mut by_thread: HashMap<String, AgentSession> = HashMap::new();

    if let Some(conn) = open_readonly(&paths.codex_db) {
        let mut active_threads: HashMap<String, i64> = HashMap::new();
        if let Ok(mut stmt) =
            conn.prepare("SELECT thread_id, observed_at_ms FROM active_chat_turns")
        {
            if let Ok(iter) = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            }) {
                for (tid, obs_ms) in iter.flatten() {
                    active_threads
                        .entry(tid)
                        .and_modify(|prev| *prev = (*prev).max(obs_ms))
                        .or_insert(obs_ms);
                }
            }
        }

        if let Ok(mut stmt) = conn.prepare(
            "SELECT thread_id, display_title, cwd, source_updated_at, source_recency_at \
             FROM local_thread_catalog \
             WHERE missing_candidate = 0 \
             ORDER BY source_recency_at DESC \
             LIMIT 16",
        ) {
            if let Ok(iter) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1).unwrap_or_default(),
                    r.get::<_, Option<String>>(2).unwrap_or(None),
                    r.get::<_, f64>(3).unwrap_or(0.0),
                    r.get::<_, f64>(4).unwrap_or(0.0),
                ))
            }) {
                for (tid, title, cwd, updated_at, recency_at) in iter.flatten() {
                    if tid.is_empty() {
                        continue;
                    }
                    let ts_f = updated_at.max(recency_at);
                    let mut last_active_ms = float_epoch_to_ms(ts_f);
                    let active_obs = active_threads.get(&tid).copied();
                    if let Some(obs) = active_obs {
                        last_active_ms = last_active_ms.max(normalize_epoch_ms(obs));
                    }
                    let is_active_turn = active_obs
                        .map(|obs| now - normalize_epoch_ms(obs) <= WORKING_TIMEOUT_MS && codex_running)
                        .unwrap_or(false);
                    if !is_active_turn && (last_active_ms <= 0 || now - last_active_ms > MAX_RECENT_MS) {
                        continue;
                    }

                    let (status, detail) = if is_active_turn {
                        (AgentStatus::Working, Some("Arbejder…".to_string()))
                    } else {
                        (AgentStatus::Ready, Some("Klar til svar".to_string()))
                    };

                    by_thread.insert(
                        tid.clone(),
                        AgentSession {
                            id: format!("codex:{tid}"),
                            agent: "codex".to_string(),
                            agent_label: "Codex".to_string(),
                            title: non_empty_trimmed(&title),
                            project: cwd.as_deref().and_then(project_from_path_str),
                            status,
                            last_active_ms,
                            detail,
                            subagent_count: 0,
                            todo: None,
                            pid: codex_pid,
                            process_running: codex_running,
                        },
                    );
                }
            }
        }
    }

    // Suppler/opdater fra friske .jsonl-filer (skipper .jsonl.zst).
    for (path, mtime_ms) in collect_recent_files(&paths.codex_sessions, &["jsonl"], 4, MAX_RECENT_MS, now, 12) {
        let head = read_head(&path, HEAD_BYTES).unwrap_or_default();
        let (head_tid, head_cwd, head_title) = parse_codex_head(&head);
        let thread_id = head_tid.unwrap_or_else(|| path.to_string_lossy().to_string());
        let project = head_cwd.as_deref().and_then(project_from_path_str);
        let age = now - mtime_ms;
        let allow_working = age <= WORKING_TIMEOUT_MS && (codex_running || age <= SHORT_ACTIVE_MS);
        let (tail_status, tail_detail, tail_todo) = parse_codex_tail(&path, allow_working);

        if let Some(existing) = by_thread.get_mut(&thread_id) {
            if mtime_ms > existing.last_active_ms {
                existing.last_active_ms = mtime_ms;
            }
            if existing.project.is_none() {
                existing.project = project;
            }
            if existing.title.is_none() {
                existing.title = head_title;
            }
            if existing.todo.is_none() {
                existing.todo = tail_todo;
            }
            if let Some(st) = tail_status {
                if st.rank() >= existing.status_rank() || !codex_running {
                    existing.status = st;
                    existing.detail = tail_detail;
                }
            }
        } else {
            let status = tail_status.unwrap_or(AgentStatus::Idle);
            by_thread.insert(
                thread_id.clone(),
                AgentSession {
                    id: format!("codex:{thread_id}"),
                    agent: "codex".to_string(),
                    agent_label: "Codex".to_string(),
                    title: head_title,
                    project,
                    status,
                    last_active_ms: mtime_ms,
                    detail: tail_detail,
                    subagent_count: 0,
                    todo: tail_todo,
                    pid: codex_pid,
                    process_running: codex_running,
                },
            );
        }
    }

    by_thread.into_values().collect()
}

fn parse_codex_head(head: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut thread_id = None;
    let mut cwd = None;
    let mut title = None;

    for line in head.lines().take(24) {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if thread_id.is_none() {
            thread_id = val
                .pointer("/payload/id")
                .or_else(|| val.get("session_id"))
                .or_else(|| val.get("thread_id"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        if cwd.is_none() {
            cwd = extract_cwd_from_value(&val);
        }
        if title.is_none() {
            let is_user = val.get("type").and_then(Value::as_str) == Some("event_msg")
                && val.pointer("/payload/type").and_then(Value::as_str) == Some("user_message");
            if is_user {
                title = val
                    .pointer("/payload/message")
                    .and_then(Value::as_str)
                    .and_then(short_title);
            }
        }
    }
    (thread_id, cwd, title)
}

fn parse_codex_tail(
    path: &Path,
    allow_working: bool,
) -> (Option<AgentStatus>, Option<String>, Option<AgentTodoProgress>) {
    let Some(lines) = read_tail_lines(path, TODO_TAIL_BYTES) else {
        return (None, None, None);
    };
    let mut status_detail: Option<(AgentStatus, Option<String>)> = None;
    let mut todo: Option<AgentTodoProgress> = None;

    for (idx, line) in lines.iter().rev().enumerate() {
        if idx >= 48 && status_detail.is_some() && todo.is_some() {
            break;
        }
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let top_ty = val.get("type").and_then(Value::as_str).unwrap_or("");
        let sub_ty = val
            .pointer("/payload/type")
            .and_then(Value::as_str)
            .unwrap_or("");

        if todo.is_none() && matches!((top_ty, sub_ty), ("response_item", "function_call") | ("function_call", _)) {
            if let Some(found) = extract_codex_plan(&val) {
                todo = Some(found);
            }
        }

        if status_detail.is_none() && idx < 16 {
            status_detail = match (top_ty, sub_ty) {
                ("event_msg", "task_complete") | ("event_msg", "agent_message") => {
                    Some((AgentStatus::Ready, Some("Klar til svar".to_string())))
                }
                ("response_item", "message")
                    if val.pointer("/payload/role").and_then(Value::as_str) == Some("assistant") =>
                {
                    Some((AgentStatus::Ready, Some("Klar til svar".to_string())))
                }
                ("response_item", "function_call")
                | ("response_item", "custom_tool_call")
                | ("response_item", "local_shell_call")
                | ("function_call", _) => Some(if allow_working {
                    (AgentStatus::Working, Some("Kører kommando".to_string()))
                } else {
                    (AgentStatus::Idle, None)
                }),
                ("response_item", "web_search_call") => Some(if allow_working {
                    (AgentStatus::Working, Some("Søger på nettet".to_string()))
                } else {
                    (AgentStatus::Idle, None)
                }),
                ("event_msg", "user_message")
                | ("user_message", _)
                | ("turn_context", _)
                | ("response_item", "function_call_output")
                | ("function_call_output", _) => Some(if allow_working {
                    (AgentStatus::Working, Some("Tænker…".to_string()))
                } else {
                    (AgentStatus::Idle, None)
                }),
                _ => None,
            };
        }

        if status_detail.is_some() && todo.is_some() {
            break;
        }
    }

    match status_detail {
        Some((st, det)) => (Some(st), det, todo),
        None => (None, None, todo),
    }
}

fn extract_codex_plan(val: &Value) -> Option<AgentTodoProgress> {
    let payload = val.get("payload").unwrap_or(val);
    if payload.get("name").and_then(Value::as_str) != Some("update_plan") {
        return None;
    }
    let args = payload.get("arguments")?;
    if let Some(items) = args.get("plan").and_then(Value::as_array) {
        return parse_todo_array(items);
    }
    if let Some(raw) = args.as_str() {
        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
            if let Some(items) = parsed.get("plan").and_then(Value::as_array) {
                return parse_todo_array(items);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 4. Claude Code (~/.claude/projects/*/*.jsonl)
// ---------------------------------------------------------------------------

pub fn scan_claude_code(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
) -> Vec<AgentSession> {
    let projects_dir = &paths.claude_projects;
    if !projects_dir.is_dir() {
        return Vec::new();
    }

    let claude_running = procs.has_any(&["claude", "claude-code"]);
    let claude_pid = procs.first_pid(&["claude", "claude-code"]);

    let mut newest_main_per_dir: HashMap<PathBuf, (PathBuf, i64)> = HashMap::new();
    let mut active_subagents_per_dir: HashMap<PathBuf, u32> = HashMap::new();

    let mut stack = vec![(projects_dir.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 3 {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(mtime_ms) = modified_ms(&meta) else {
                continue;
            };
            let fname = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();

            if fname.starts_with("agent-") {
                if now - mtime_ms <= SUBAGENT_ACTIVE_MS && claude_running {
                    *active_subagents_per_dir.entry(parent).or_insert(0) += 1;
                }
                continue;
            }
            if now - mtime_ms > MAX_RECENT_MS {
                continue;
            }

            newest_main_per_dir
                .entry(parent)
                .and_modify(|slot| {
                    if mtime_ms > slot.1 {
                        slot.0 = path.clone();
                        slot.1 = mtime_ms;
                    }
                })
                .or_insert((path, mtime_ms));
        }
    }

    let mut sorted: Vec<(PathBuf, PathBuf, i64)> = newest_main_per_dir
        .into_iter()
        .map(|(dir, (file, mtime))| (dir, file, mtime))
        .collect();
    sorted.sort_by(|a, b| b.2.cmp(&a.2));
    sorted.truncate(12);

    let mut out = Vec::new();
    for (dir, path, mtime_ms) in sorted {
        let head = read_head(&path, HEAD_BYTES).unwrap_or_default();
        let (head_cwd, head_title) = parse_claude_head(&head);
        let project = head_cwd
            .as_deref()
            .and_then(project_from_path_str)
            .or_else(|| {
                dir.file_name()
                    .and_then(|n| n.to_str())
                    .map(humanize_slug)
            });

        let age = now - mtime_ms;
        let allow_working = age <= WORKING_TIMEOUT_MS && (claude_running || age <= SHORT_ACTIVE_MS);
        let tail = parse_claude_tail(&path, allow_working);
        let todo = read_claude_todo_file(&paths.claude_todos, &path).or(tail.todo);

        let subagent_count = active_subagents_per_dir.get(&dir).copied().unwrap_or(0);
        out.push(AgentSession {
            id: format!("claude-code:{}", path.display()),
            agent: "claude-code".to_string(),
            agent_label: "Claude Code".to_string(),
            title: tail.summary.or(head_title),
            project,
            status: tail.status,
            last_active_ms: mtime_ms,
            detail: tail.detail,
            subagent_count,
            todo,
            pid: claude_pid,
            process_running: claude_running,
        });
    }

    out
}

fn parse_claude_head(head: &str) -> (Option<String>, Option<String>) {
    let mut cwd = None;
    let mut title = None;

    for line in head.lines().take(24) {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if cwd.is_none() {
            cwd = extract_cwd_from_value(&val);
        }
        if val.get("type").and_then(Value::as_str) == Some("summary") {
            if let Some(s) = val.get("summary").and_then(Value::as_str).and_then(short_title) {
                title = Some(s);
            }
        } else if title.is_none() && val.get("type").and_then(Value::as_str) == Some("user") {
            if let Some(content) = val.pointer("/message/content") {
                if let Some(text) = content.as_str() {
                    title = short_title(text);
                } else if let Some(arr) = content.as_array() {
                    for item in arr {
                        if item.get("type").and_then(Value::as_str) == Some("text") {
                            if let Some(t) = item.get("text").and_then(Value::as_str) {
                                title = short_title(t);
                                if title.is_some() {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    (cwd, title)
}

struct ClaudeTail {
    status: AgentStatus,
    detail: Option<String>,
    summary: Option<String>,
    todo: Option<AgentTodoProgress>,
}

fn read_claude_todo_file(todos_dir: &Path, session_path: &Path) -> Option<AgentTodoProgress> {
    if !todos_dir.is_dir() {
        return None;
    }
    let stem = session_path.file_stem()?.to_str()?;
    if stem.is_empty() {
        return None;
    }
    let prefix_agent = format!("{stem}-agent-");
    let exact_name = format!("{stem}.json");
    let entries = fs::read_dir(todos_dir).ok()?;
    let mut candidates: Vec<(i64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(fname) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if fname == exact_name || fname.starts_with(&prefix_agent) {
            let mtime = entry.metadata().ok().and_then(|m| modified_ms(&m)).unwrap_or(0);
            candidates.push((mtime, p));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in candidates {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&raw) {
                if let Some(todo) = parse_todo_array(&items) {
                    return Some(todo);
                }
            }
        }
    }
    None
}

fn parse_claude_tail(path: &Path, allow_working: bool) -> ClaudeTail {
    let mut out = ClaudeTail {
        status: AgentStatus::Idle,
        detail: None,
        summary: None,
        todo: None,
    };
    let Some(lines) = read_tail_lines(path, TODO_TAIL_BYTES) else {
        return out;
    };

    let mut found_turn = false;
    for (idx, line) in lines.iter().rev().enumerate() {
        if idx >= 48 && found_turn && out.todo.is_some() && out.summary.is_some() {
            break;
        }
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(ty) = val.get("type").and_then(Value::as_str) else {
            continue;
        };
        if ty == "summary" && out.summary.is_none() {
            out.summary = val
                .get("summary")
                .and_then(Value::as_str)
                .and_then(short_title);
            continue;
        }

        if ty == "assistant" && out.todo.is_none() {
            if let Some(items) = val.pointer("/message/content").and_then(Value::as_array) {
                for item in items.iter().rev() {
                    if item.get("type").and_then(Value::as_str) == Some("tool_use")
                        && item.get("name").and_then(Value::as_str) == Some("TodoWrite")
                    {
                        if let Some(todos) = item.pointer("/input/todos").and_then(Value::as_array) {
                            if let Some(parsed) = parse_todo_array(todos) {
                                out.todo = Some(parsed);
                                break;
                            }
                        }
                    }
                }
            }
        }

        if found_turn || idx >= 16 {
            continue;
        }
        match ty {
            "assistant" => {
                let tools: Vec<&str> = val
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter(|i| i.get("type").and_then(Value::as_str) == Some("tool_use"))
                            .filter_map(|i| i.get("name").and_then(Value::as_str))
                            .collect()
                    })
                    .unwrap_or_default();

                if let Some(&last_tool) = tools.last() {
                    if tools.contains(&"AskUserQuestion") {
                        out.status = AgentStatus::Waiting;
                        out.detail = Some("Stiller et spørgsmål".to_string());
                    } else if allow_working {
                        out.status = AgentStatus::Working;
                        out.detail = Some(map_claude_tool_name(last_tool));
                    } else {
                        out.status = AgentStatus::Idle;
                        out.detail = None;
                    }
                } else {
                    out.status = AgentStatus::Ready;
                    out.detail = Some("Klar til svar".to_string());
                }
                found_turn = true;
            }
            "user" | "progress" => {
                if allow_working {
                    out.status = AgentStatus::Working;
                    out.detail = Some("Tænker…".to_string());
                } else {
                    out.status = AgentStatus::Idle;
                    out.detail = None;
                }
                found_turn = true;
            }
            _ => {}
        }
    }

    out
}

fn map_claude_tool_name(name: &str) -> String {
    match name {
        "Bash" => "Kører kommando",
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "Redigerer filer",
        "Read" | "Glob" | "Grep" | "LS" => "Læser filer",
        "WebSearch" | "WebFetch" => "Søger på nettet",
        "Agent" | "Task" => "Kører underagent",
        "AskUserQuestion" => "Stiller et spørgsmål",
        _ => "Bruger værktøj",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// 5. Cursor (%APPDATA%\Cursor\User\globalStorage\state.vscdb)
// ---------------------------------------------------------------------------

pub fn scan_cursor(db_path: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let Some(conn) = open_readonly(db_path) else {
        return Vec::new();
    };
    let cutoff = now - MAX_RECENT_MS;
    let mut stmt = match conn.prepare(
        "SELECT h.composerId, h.lastUpdatedAt, h.value, d.value \
         FROM composerHeaders h \
         LEFT JOIN cursorDiskKV d ON d.key = ('composerData:' || h.composerId) \
         WHERE COALESCE(h.isArchived, 0) = 0 \
           AND COALESCE(h.isSubagent, 0) = 0 \
           AND h.lastUpdatedAt IS NOT NULL \
           AND h.lastUpdatedAt >= ?1 \
         ORDER BY h.lastUpdatedAt DESC \
         LIMIT 12",
    ) {
        Ok(s) => s,
        Err(err) => {
            log::debug!(target: "provider:agents", "Cursor SQL-fejl: {err}");
            return Vec::new();
        }
    };

    let rows_iter = match stmt.query_map([cutoff], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2).unwrap_or_default(),
            r.get::<_, Option<String>>(3).unwrap_or(None),
        ))
    }) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let cursor_running = procs.has_any(&["cursor"]);
    let cursor_pid = procs.first_pid(&["cursor"]);
    let mut out = Vec::new();

    for (cid, last_updated_at, head_raw, data_raw) in rows_iter.flatten() {
        if cid.is_empty() || cid == "empty-state-draft" {
            continue;
        }
        let head = serde_json::from_str::<Value>(&head_raw).unwrap_or(Value::Null);
        let data = data_raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .unwrap_or(Value::Null);

        if head.get("isDraft").and_then(Value::as_bool) == Some(true)
            || head.get("isEphemeral").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }

        let title = head
            .get("name")
            .and_then(Value::as_str)
            .and_then(non_empty_trimmed)
            .or_else(|| {
                data.get("name")
                    .and_then(Value::as_str)
                    .and_then(non_empty_trimmed)
            });

        let raw_status = data
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("none");
        if raw_status == "none" && title.is_none() {
            continue;
        }

        let blocking = head
            .get("hasBlockingPendingActions")
            .and_then(Value::as_bool)
            == Some(true);

        let last_active_ms = normalize_epoch_ms(last_updated_at);
        let (status, detail) = if blocking && cursor_running {
            (AgentStatus::Waiting, Some("Afventer godkendelse".to_string()))
        } else if matches!(raw_status, "running" | "generating" | "streaming") && cursor_running {
            (AgentStatus::Working, Some("Arbejder…".to_string()))
        } else if raw_status == "completed" {
            (AgentStatus::Ready, Some("Klar til svar".to_string()))
        } else {
            (AgentStatus::Idle, None)
        };

        let project = head
            .pointer("/draftTarget/environment/uri/fsPath")
            .and_then(Value::as_str)
            .and_then(project_from_path_str)
            .or_else(|| {
                head.pointer("/draftTarget/environment/uri/external")
                    .and_then(Value::as_str)
                    .and_then(project_from_uri)
            });

        let todo = data
            .get("todos")
            .and_then(Value::as_array)
            .and_then(|arr| parse_todo_array(arr));

        out.push(AgentSession {
            id: format!("cursor:{cid}"),
            agent: "cursor".to_string(),
            agent_label: "Cursor".to_string(),
            title,
            project,
            status,
            last_active_ms,
            detail,
            subagent_count: 0,
            todo,
            pid: cursor_pid,
            process_running: cursor_running,
        });
    }

    out
}

// ---------------------------------------------------------------------------
// 6. Cline / Roo Code / Kilo Code (ui_messages.json i globalStorage)
// ---------------------------------------------------------------------------

pub fn scan_editor_tasks(appdata: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let editors: &[(&str, &str)] = &[
        ("Code", "code"),
        ("Code - Insiders", "code - insiders"),
        ("VSCodium", "vscodium"),
        ("Cursor", "cursor"),
        ("Windsurf", "windsurf"),
        ("Antigravity", "antigravity"),
    ];
    let exts: &[(&str, &str, &str)] = &[
        ("saoudrizwan.claude-dev", "cline", "Cline"),
        ("rooveterinaryinc.roo-cline", "cline", "Roo Code"),
        ("kilocode.kilo-code", "kilo", "Kilo Code"),
    ];

    let mut out = Vec::new();
    for (editor_dir, editor_exe) in editors {
        let editor_running = procs.has_any(&[editor_exe]);
        let editor_pid = procs.first_pid(&[editor_exe]);
        for (ext_id, agent_id, label) in exts {
            let tasks_dir = appdata
                .join(editor_dir)
                .join("User/globalStorage")
                .join(ext_id)
                .join("tasks");
            if !tasks_dir.is_dir() {
                continue;
            }
            let Ok(entries) = fs::read_dir(&tasks_dir) else {
                continue;
            };
            for entry in entries.flatten().take(40) {
                let ui_path = entry.path().join("ui_messages.json");
                let Ok(meta) = fs::metadata(&ui_path) else {
                    continue;
                };
                let Some(mtime_ms) = modified_ms(&meta) else {
                    continue;
                };
                if now - mtime_ms > MAX_RECENT_MS {
                    continue;
                }
                let head = read_head(&ui_path, 2048).unwrap_or_default();
                let title = extract_cline_task_title(&head);
                let tail = read_tail(&ui_path, TAIL_BYTES).unwrap_or_default();
                let age = now - mtime_ms;
                let (status, detail) =
                    classify_cline_tail(&tail, editor_running && age <= SHORT_ACTIVE_MS);
                out.push(AgentSession {
                    id: format!("{agent_id}:{}", ui_path.display()),
                    agent: (*agent_id).to_string(),
                    agent_label: (*label).to_string(),
                    title,
                    project: Some((*editor_dir).to_string()),
                    status,
                    last_active_ms: mtime_ms,
                    detail,
                    subagent_count: 0,
                    todo: None,
                    pid: editor_pid,
                    process_running: editor_running,
                });
            }
        }
    }
    out
}

fn extract_cline_task_title(head: &str) -> Option<String> {
    let idx = head.find("\"text\"")?;
    let rest = &head[idx + 6..];
    let colon = rest.find(':')?;
    let q1 = rest[colon + 1..].find('"')?;
    let val_start = &rest[colon + 1 + q1 + 1..];
    let q2 = val_start.find('"')?;
    short_title(&val_start[..q2])
}

fn classify_cline_tail(tail: &str, can_be_working: bool) -> (AgentStatus, Option<String>) {
    // Find sidste "type": "ask" | "say"
    if let Some(pos) = tail.rfind("\"type\"") {
        let slice = &tail[pos..];
        if slice.contains("\"ask\"") {
            if slice.contains("completion_result") {
                return (AgentStatus::Ready, Some("Færdig".to_string()));
            }
            if slice.contains("resume_task") || slice.contains("resume_completed_task") {
                return (AgentStatus::Idle, None);
            }
            if slice.contains("followup") {
                return (AgentStatus::Waiting, Some("Stiller et spørgsmål".to_string()));
            }
            return (AgentStatus::Waiting, Some("Afventer godkendelse".to_string()));
        }
        if slice.contains("\"say\"") && can_be_working {
            return (AgentStatus::Working, Some("Arbejder…".to_string()));
        }
    }
    (AgentStatus::Idle, None)
}

// ---------------------------------------------------------------------------
// 7. Gemini CLI, OpenCode & Windsurf Cascade + Aider
// ---------------------------------------------------------------------------

pub fn scan_gemini_cli(root: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let running = procs.has_any(&["gemini"]);
    let pid = procs.first_pid(&["gemini"]);
    let files = collect_recent_files(root, &["json"], 4, MAX_RECENT_MS, now, 8);
    let mut out = Vec::new();
    for (path, mtime_ms) in files {
        if !path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|n| n.eq_ignore_ascii_case("chats"))
            .unwrap_or(false)
        {
            continue;
        }
        let age = now - mtime_ms;
        let working = age <= WORKING_TIMEOUT_MS && (running || age <= SHORT_ACTIVE_MS);
        let head = read_head(&path, HEAD_BYTES).unwrap_or_default();
        let project = extract_cwd_from_text(&head)
            .as_deref()
            .and_then(project_from_path_str);
        out.push(AgentSession {
            id: format!("gemini:{}", path.display()),
            agent: "gemini".to_string(),
            agent_label: "Gemini CLI".to_string(),
            title: None,
            project,
            status: if working {
                AgentStatus::Working
            } else {
                AgentStatus::Ready
            },
            last_active_ms: mtime_ms,
            detail: Some(if working {
                "Arbejder…".to_string()
            } else {
                "Klar til svar".to_string()
            }),
            subagent_count: 0,
            todo: None,
            pid,
            process_running: running,
        });
    }
    out
}

pub fn scan_opencode(roots: &[PathBuf], procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let running = procs.has_any(&["opencode"]);
    let pid = procs.first_pid(&["opencode"]);
    let mut out = Vec::new();
    let mut seen_paths = HashSet::new();

    for root in roots {
        for (path, mtime_ms) in collect_recent_files(root, &["json", "jsonl"], 4, MAX_RECENT_MS, now, 8) {
            if !seen_paths.insert(path.clone()) {
                continue;
            }
            let age = now - mtime_ms;
            let working = age <= WORKING_TIMEOUT_MS && (running || age <= SHORT_ACTIVE_MS);
            let head = read_head(&path, HEAD_BYTES).unwrap_or_default();
            let project = extract_cwd_from_text(&head)
                .as_deref()
                .and_then(project_from_path_str);
            out.push(AgentSession {
                id: format!("opencode:{}", path.display()),
                agent: "opencode".to_string(),
                agent_label: "OpenCode".to_string(),
                title: None,
                project,
                status: if working {
                    AgentStatus::Working
                } else {
                    AgentStatus::Ready
                },
                last_active_ms: mtime_ms,
                detail: Some(if working {
                    "Arbejder…".to_string()
                } else {
                    "Klar til svar".to_string()
                }),
                subagent_count: 0,
                todo: None,
                pid,
                process_running: running,
            });
        }
    }
    out
}

pub fn scan_windsurf_cascade(
    cascade_dir: &Path,
    procs: &ProcessIndex,
    now: i64,
) -> Vec<AgentSession> {
    let running = procs.has_any(&["windsurf"]);
    if !running {
        return Vec::new();
    }
    let pid = procs.first_pid(&["windsurf"]);
    let files = collect_recent_files(cascade_dir, &["pb"], 2, MAX_RECENT_MS, now, 6);
    files
        .into_iter()
        .map(|(path, mtime_ms)| {
            let age = now - mtime_ms;
            let working = age <= SHORT_ACTIVE_MS;
            AgentSession {
                id: format!("windsurf:{}", path.display()),
                agent: "windsurf".to_string(),
                agent_label: "Windsurf".to_string(),
                title: None,
                project: None,
                status: if working {
                    AgentStatus::Working
                } else {
                    AgentStatus::Ready
                },
                last_active_ms: mtime_ms,
                detail: Some(if working {
                    "Arbejder…".to_string()
                } else {
                    "Klar til svar".to_string()
                }),
                subagent_count: 0,
                todo: None,
                pid,
                process_running: true,
            }
        })
        .collect()
}

pub fn scan_aider(procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    procs
        .aider_sessions()
        .iter()
        .map(|(pid, project, started_ms)| AgentSession {
            id: format!("aider:{pid}"),
            agent: "aider".to_string(),
            agent_label: "Aider".to_string(),
            title: None,
            project: project.clone(),
            status: AgentStatus::Idle,
            last_active_ms: if *started_ms > 0 { *started_ms } else { now },
            detail: None,
            subagent_count: 0,
            todo: None,
            pid: Some(*pid),
            process_running: true,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fælles hjælpere (dato, URI, fil-haler)
// ---------------------------------------------------------------------------

fn modified_ms(meta: &fs::Metadata) -> Option<i64> {
    meta.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

fn collect_recent_files(
    root: &Path,
    extensions: &[&str],
    max_depth: usize,
    max_age_ms: i64,
    now: i64,
    limit: usize,
) -> Vec<(PathBuf, i64)> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                continue;
            }
            let Some(mtime_ms) = modified_ms(&meta) else {
                continue;
            };
            if now - mtime_ms <= max_age_ms {
                found.push((path, mtime_ms));
            }
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1));
    found.truncate(limit);
    found
}

fn read_head(path: &Path, max_bytes: usize) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(max_bytes.min(HEAD_BYTES));
    file.take(max_bytes as u64).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).to_string())
}

fn read_tail(path: &Path, max_bytes: usize) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes as u64);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = vec![0u8; (len - start) as usize];
    file.read_exact(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    if start > 0 {
        if let Some(pos) = text.find('\n') {
            return Some(text[pos + 1..].to_string());
        }
    }
    Some(text.to_string())
}

fn read_tail_lines(path: &Path, max_bytes: usize) -> Option<Vec<String>> {
    let tail = read_tail(path, max_bytes)?;
    let lines: Vec<String> = tail
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .map(str::to_string)
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

fn extract_cwd_from_value(val: &Value) -> Option<String> {
    for container in [Some(val), val.get("payload")].into_iter().flatten() {
        for key in ["cwd", "workdir", "workspace", "current_dir"] {
            if let Some(dir) = container.get(key).and_then(Value::as_str) {
                if !dir.is_empty() {
                    return Some(dir.to_string());
                }
            }
        }
    }
    None
}

fn extract_cwd_from_text(head: &str) -> Option<String> {
    if let Ok(val) = serde_json::from_str::<Value>(head) {
        if let Some(d) = extract_cwd_from_value(&val) {
            return Some(d);
        }
    }
    for line in head.lines().take(24) {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            if let Some(d) = extract_cwd_from_value(&val) {
                return Some(d);
            }
        }
    }
    None
}

fn non_empty_trimmed(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn parse_todo_array(items: &[Value]) -> Option<AgentTodoProgress> {
    if items.is_empty() {
        return None;
    }
    let total = items.len() as u32;
    let mut done = 0u32;
    let mut current: Option<String> = None;

    for item in items {
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if matches!(status.as_str(), "completed" | "done") {
            done += 1;
        } else if status == "in_progress" && current.is_none() {
            let text = item
                .get("activeForm")
                .or_else(|| item.get("content"))
                .or_else(|| item.get("step"))
                .and_then(Value::as_str)
                .unwrap_or("");
            current = short_title_len(text, 72);
        }
    }

    Some(AgentTodoProgress {
        done,
        total,
        current,
    })
}

pub fn parse_markdown_todos(md: &str) -> Option<AgentTodoProgress> {
    let mut total = 0u32;
    let mut done = 0u32;
    let mut current: Option<String> = None;

    for line in md.lines() {
        let trimmed = line.trim_start();
        let after_bullet = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "));
        let Some(rest) = after_bullet else {
            continue;
        };
        if rest.starts_with("[x]") || rest.starts_with("[X]") {
            total += 1;
            done += 1;
        } else if let Some(body) = rest.strip_prefix("[/]") {
            total += 1;
            if current.is_none() {
                let without_comment = body.split("<!--").next().unwrap_or(body);
                let cleaned = without_comment.replace("**", "").replace('`', "");
                current = short_title_len(&cleaned, 72);
            }
        } else if rest.starts_with("[ ]") {
            total += 1;
        }
    }

    (total > 0).then_some(AgentTodoProgress {
        done,
        total,
        current,
    })
}

fn short_title(raw: &str) -> Option<String> {
    short_title_len(raw, 64)
}

fn short_title_len(raw: &str, max_chars: usize) -> Option<String> {
    let first_line = raw.lines().find(|l| !l.trim().is_empty())?.trim();
    if first_line.starts_with('<') || first_line.is_empty() {
        return None;
    }
    let mut out: String = first_line.chars().take(max_chars).collect();
    if first_line.chars().count() > max_chars {
        out.push('…');
    }
    Some(out)
}

fn normalize_epoch_ms(ts: i64) -> i64 {
    if ts > 0 && ts < 10_000_000_000 {
        ts * 1000
    } else {
        ts
    }
}

fn float_epoch_to_ms(ts: f64) -> i64 {
    if !ts.is_finite() || ts <= 0.0 {
        return 0;
    }
    if ts < 10_000_000_000.0 {
        (ts * 1000.0).round() as i64
    } else {
        ts.round() as i64
    }
}

pub fn project_from_path_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let seg = trimmed
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())?;
    if seg.ends_with(':') {
        return None;
    }
    Some(seg.to_string())
}

pub fn project_from_uri(uri: &str) -> Option<String> {
    let decoded = percent_decode_str(uri.trim());
    let path = decoded
        .strip_prefix("file:///")
        .or_else(|| decoded.strip_prefix("file://"))
        .unwrap_or(&decoded);
    project_from_path_str(path)
}

fn percent_decode_str(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + (b - b'a')),
        b'A'..=b'F' => Some(10 + (b - b'A')),
        _ => None,
    }
}

fn humanize_slug(slug: &str) -> String {
    let parts: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => slug.to_string(),
        1 => parts[0].to_string(),
        n => parts[n - 2..].join("-"),
    }
}

/// Parser RFC3339 / SQLite-datotidsstempel og epoch-strenge til epoch millisekunder.
pub fn parse_timestamp_ms(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(num) = s.parse::<i64>() {
        return Some(normalize_epoch_ms(num));
    }
    if let Ok(num) = s.parse::<f64>() {
        let ms = float_epoch_to_ms(num);
        return (ms > 0).then_some(ms);
    }

    // Forvent "YYYY-MM-DD[T| ]HH:MM:SS[.frac][Z|+HH:MM|-HH:MM]"
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    if s.as_bytes().get(4).copied() != Some(b'-') || s.as_bytes().get(7).copied() != Some(b'-') {
        return None;
    }
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let sep = s.as_bytes().get(10).copied()?;
    if sep != b'T' && sep != b't' && sep != b' ' {
        return None;
    }
    if s.as_bytes().get(13).copied() != Some(b':') || s.as_bytes().get(16).copied() != Some(b':') {
        return None;
    }
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&min)
        || !(0..=60).contains(&sec)
    {
        return None;
    }

    let mut rest = &s[19..];
    let mut sub_ms: i64 = 0;
    if let Some(after_dot) = rest.strip_prefix('.') {
        let frac_end = after_dot
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after_dot.len());
        let frac_digits = &after_dot[..frac_end];
        let mut padded = [b'0'; 3];
        for (idx, b) in frac_digits.bytes().take(3).enumerate() {
            padded[idx] = b;
        }
        sub_ms = std::str::from_utf8(&padded).ok()?.parse().ok()?;
        rest = &after_dot[frac_end..];
    }

    let mut offset_secs: i64 = 0;
    if let Some(sign_ch) = rest.chars().next() {
        if sign_ch == 'Z' || sign_ch == 'z' {
            offset_secs = 0;
        } else if sign_ch == '+' || sign_ch == '-' {
            let sign = if sign_ch == '+' { 1i64 } else { -1i64 };
            let off_str = &rest[1..];
            let oh: i64 = off_str.get(0..2)?.parse().ok()?;
            let om: i64 = if let Some(after_colon) = off_str.get(2..).and_then(|r| r.strip_prefix(':')) {
                after_colon.get(0..2).and_then(|m| m.parse().ok()).unwrap_or(0)
            } else if off_str.len() >= 4 {
                off_str.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0)
            } else {
                0
            };
            offset_secs = sign * (oh * 3600 + om * 60);
        }
    }

    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + min * 60 + sec - offset_secs;
    Some(secs * 1000 + sub_ms)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
