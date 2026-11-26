use std::path::Path;
use std::collections::HashSet;

use serde_json::Value;

use super::super::common::*;
use super::super::{AgentPaths, CachedDevinMsg, DevinMsgRow, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{
    process::ProcessIndex, sqlite::open_readonly, AgentSession, AgentStatus, AgentTodoProgress,
};

pub(super) struct DevinAdapter;

impl AgentAdapter for DevinAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        mut cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        let Some(db_path) = paths.devin_db.as_deref() else {
            return Vec::new();
        };
        SourceCache::scan_db_cached(&mut cache, db_path, procs, now, |fp| fp, |inner| {
            scan_devin_cached(db_path, procs, now, inner)
        })
    }
}

// ---------------------------------------------------------------------------
// 2. Devin (AppData\Roaming\devin\cli\sessions.db)
// ---------------------------------------------------------------------------

struct DevinSessionRow {
    id: String,
    title: String,
    working_directory: String,
    last_activity_at: i64,
    main_chain_id: Option<i64>,
}

#[derive(Default)]
struct DevinSubagentStats {
    latest_ms: i64,
    active_count: u32,
}

fn devin_live_row_activity_ms(conn: &rusqlite::Connection, session_id: &str) -> i64 {
    conn.query_row(
        "SELECT started_at FROM live_row_times \
         WHERE session_id = ?1 \
         ORDER BY started_at DESC \
         LIMIT 1",
        [session_id],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|raw| parse_timestamp_ms(&raw))
    .unwrap_or(0)
}

fn devin_message_is_finished(raw: &str) -> bool {
    let Ok(val) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    let role_is_assistant = val.get("role").and_then(Value::as_str) == Some("assistant");
    let tool_count = val
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let has_content = val
        .get("content")
        .and_then(Value::as_str)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    role_is_assistant && tool_count == 0 && has_content
}

fn devin_subagent_stats(
    conn: &rusqlite::Connection,
    session_id: &str,
    now: i64,
) -> DevinSubagentStats {
    let mut stats = DevinSubagentStats::default();
    let Ok(mut stmt) = conn.prepare(
        "SELECT h.updated_at, m.chat_message \
         FROM subagent_heads h \
         LEFT JOIN message_nodes m \
           ON m.session_id = h.session_id AND m.node_id = h.chain_node_id \
         WHERE h.session_id = ?1",
    ) else {
        return stats;
    };
    let Ok(rows) = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, i64>(0).unwrap_or(0),
            row.get::<_, Option<String>>(1).ok().flatten(),
        ))
    }) else {
        return stats;
    };

    for (updated_at, msg) in rows.flatten() {
        let updated_ms = normalize_epoch_ms(updated_at);
        stats.latest_ms = stats.latest_ms.max(updated_ms);
        if updated_ms <= 0 || now - updated_ms > WORKING_TIMEOUT_MS {
            continue;
        }
        let finished = msg
            .as_deref()
            .map(devin_message_is_finished)
            .unwrap_or(false);
        if !finished {
            stats.active_count += 1;
        }
    }
    stats
}

fn devin_relevant_msg_row(row_id: i64, raw: &str, created_at: i64) -> Option<DevinMsgRow> {
    let val = serde_json::from_str::<Value>(raw).ok()?;
    let role = val
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let content = val
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool_count = val
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0) as i64;
    let first_tool_name = val
        .get("tool_calls")
        .and_then(Value::as_array)
        .and_then(|calls| calls.first())
        .and_then(|call| call.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let has_content: i64 = if content.is_empty() { 0 } else { 1 };

    let relevant = match role.as_str() {
        "tool" => true,
        "assistant" => {
            !content.trim_start().starts_with("<summary>")
                && val.pointer("/metadata/query_label").is_none()
                && (tool_count > 0 || has_content != 0)
        }
        "user" => {
            let is_user_input = val
                .pointer("/metadata/is_user_input")
                .and_then(|v| v.as_i64().or_else(|| v.as_bool().map(|b| b as i64)))
                .unwrap_or(0)
                != 0;
            is_user_input
                || (val.pointer("/metadata/is_user_input").is_none()
                    && !content.contains("Conversation to summarize")
                    && !content.starts_with("Now summarize"))
        }
        _ => false,
    };
    relevant.then_some((row_id, role, tool_count, first_tool_name, has_content, created_at))
}

fn devin_main_chain_msg_row(
    conn: &rusqlite::Connection,
    session_id: &str,
    main_chain_id: Option<i64>,
) -> Option<DevinMsgRow> {
    let mut node_id = main_chain_id?;
    for _ in 0..256 {
        let row: Option<(i64, Option<i64>, String, i64)> = conn
            .query_row(
                "SELECT row_id, parent_node_id, chat_message, created_at \
                 FROM message_nodes \
                 WHERE session_id = ?1 AND node_id = ?2 \
                 LIMIT 1",
                (session_id, node_id),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let Some((row_id, parent_node_id, raw, created_at)) = row else {
            break;
        };
        if let Some(parsed) = devin_relevant_msg_row(row_id, &raw, created_at) {
            return Some(parsed);
        }
        let Some(parent) = parent_node_id else {
            break;
        };
        if parent == node_id {
            break;
        }
        node_id = parent;
    }
    None
}

#[allow(dead_code)]
pub fn scan_devin(db_path: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    scan_devin_cached(db_path, procs, now, None)
}

pub fn scan_devin_cached(
    db_path: &Path,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let Some(conn) = open_readonly(db_path) else {
        return Vec::new();
    };
    let mut has_main_chain_id = false;
    if let Ok(mut pragma) = conn.prepare("PRAGMA table_info(sessions)") {
        if let Ok(rows) = pragma.query_map([], |row| row.get::<_, String>(1)) {
            has_main_chain_id = rows.flatten().any(|name| name == "main_chain_id");
        }
    }
    let main_chain_expr = if has_main_chain_id {
        "s.main_chain_id"
    } else {
        "NULL"
    };
    let sessions_sql = format!(
        "SELECT s.id, COALESCE(s.title, ''), s.working_directory, s.last_activity_at, {main_chain_expr} \
         FROM sessions s \
         WHERE s.hidden = 0 \
         ORDER BY s.last_activity_at DESC \
         LIMIT 12"
    );
    let mut stmt = match conn.prepare(&sessions_sql) {
        Ok(s) => s,
        Err(err) => {
            log::debug!(target: "provider:agents", "Devin SQL-fejl: {err}");
            return Vec::new();
        }
    };

    let mut rows: Vec<DevinSessionRow> = match stmt.query_map([], |row| {
        Ok(DevinSessionRow {
            id: row.get::<_, String>(0).unwrap_or_default(),
            title: row.get::<_, String>(1).unwrap_or_default(),
            working_directory: row.get::<_, String>(2).unwrap_or_default(),
            last_activity_at: row.get::<_, i64>(3).unwrap_or(0),
            main_chain_id: row.get::<_, Option<i64>>(4).ok().flatten(),
        })
    }) {
        Ok(it) => it.flatten().collect(),
        Err(_) => return Vec::new(),
    };
    drop(stmt);

    // En baggrunds-agent kan fortsætte uden at sessions.last_activity_at opdateres.
    // Hvis listen er fuld, skal sådanne sessioner stadig med, selv om deres
    // sessions-række er ældre end de 12 nyeste hovedsessioner.
    if rows.len() >= 12 {
        let recent_cutoff_sec = (now - MAX_RECENT_MS) / 1000;
        let subagent_sql = format!(
            "SELECT s.id, COALESCE(s.title, ''), s.working_directory, s.last_activity_at, {main_chain_expr} \
             FROM sessions s \
             WHERE s.hidden = 0 \
               AND EXISTS ( \
                 SELECT 1 FROM subagent_heads h \
                 WHERE h.session_id = s.id AND h.updated_at >= ?1 \
               )"
        );
        if let Ok(mut extra_stmt) = conn.prepare(&subagent_sql) {
            if let Ok(extra_rows) = extra_stmt.query_map([recent_cutoff_sec], |row| {
                Ok(DevinSessionRow {
                    id: row.get::<_, String>(0).unwrap_or_default(),
                    title: row.get::<_, String>(1).unwrap_or_default(),
                    working_directory: row.get::<_, String>(2).unwrap_or_default(),
                    last_activity_at: row.get::<_, i64>(3).unwrap_or(0),
                    main_chain_id: row.get::<_, Option<i64>>(4).ok().flatten(),
                })
            }) {
                for extra in extra_rows.flatten() {
                    if !rows.iter().any(|existing| existing.id == extra.id) {
                        rows.push(extra);
                    }
                }
            }
        }
    }

    let devin_running = procs.has_any(&["devin"]);
    let devin_pid = procs.first_pid(&["devin"]);
    let mut out = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for row in rows {
        if row.id.is_empty() {
            continue;
        }
        seen_ids.insert(row.id.clone());
        let subagent_stats = devin_subagent_stats(&conn, &row.id, now);
        let live_activity_ms = devin_live_row_activity_ms(&conn, &row.id);
        let last_active_ms = normalize_epoch_ms(row.last_activity_at)
            .max(live_activity_ms)
            .max(subagent_stats.latest_ms);
        if last_active_ms <= 0 || now - last_active_ms > MAX_RECENT_MS {
            continue;
        }

        // Billig nøgle via idx_message_nodes_session — den tunge json_extract
        // forespørgsel køres kun når sessionen reelt har ændret sig.
        let max_row_id: i64 = conn
            .query_row(
                "SELECT MAX(row_id) FROM message_nodes WHERE session_id = ?1",
                [&row.id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
            .unwrap_or(0);
        let key = (max_row_id, row.last_activity_at, row.main_chain_id);
        let cached_msg = cache
            .as_deref()
            .and_then(|c| c.devin_sessions.get(&row.id))
            .filter(|e| e.key == key)
            .map(|e| e.msg_row.clone());

        let msg_row: Option<DevinMsgRow> = match cached_msg {
            Some(m) => m,
            None => {
                let m: Option<DevinMsgRow> = devin_main_chain_msg_row(
                    &conn,
                    &row.id,
                    row.main_chain_id,
                )
                .or_else(|| {
                    conn.query_row(
                        "SELECT row_id, \
                                COALESCE(json_extract(chat_message, '$.role'), ''), \
                                COALESCE(json_array_length(chat_message, '$.tool_calls'), 0), \
                                json_extract(chat_message, '$.tool_calls[0].name'), \
                                CASE WHEN length(COALESCE(json_extract(chat_message, '$.content'), '')) > 0 THEN 1 ELSE 0 END, \
                                created_at \
                         FROM message_nodes \
                         WHERE session_id = ?1 \
                           AND ( \
                             json_extract(chat_message, '$.role') = 'tool' \
                             OR ( \
                               json_extract(chat_message, '$.role') = 'assistant' \
                               AND COALESCE(substr(ltrim(json_extract(chat_message, '$.content')), 1, 9), '') != '<summary>' \
                               AND json_extract(chat_message, '$.metadata.query_label') IS NULL \
                               AND ( \
                                 COALESCE(json_array_length(chat_message, '$.tool_calls'), 0) > 0 \
                                 OR length(COALESCE(json_extract(chat_message, '$.content'), '')) > 0 \
                               ) \
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
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                    )
                    .ok()
                });
                if let Some(c) = cache.as_deref_mut() {
                    c.devin_sessions.insert(
                        row.id.clone(),
                        CachedDevinMsg {
                            key,
                            msg_row: m.clone(),
                        },
                    );
                }
                m
            }
        };

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

        let (_last_msg_row_id, role, tool_call_count, first_tool_name, has_content, msg_created_at) =
            match msg_row {
                Some((rid, r, c, ft, hc, created)) => (rid, r, c, ft, hc != 0, created),
                None => (0, String::new(), 0, None, false, 0),
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
        let msg_created_ms = normalize_epoch_ms(msg_created_at);
        let has_unfinished_live_row = live_activity_ms > msg_created_ms + 1000
            && now - live_activity_ms <= WORKING_TIMEOUT_MS;
        let has_active_subagent = subagent_stats.active_count > 0;
        let asking_user = matches!(
            first_tool_name.as_deref(),
            Some("ask_user" | "ask_question" | "AskUserQuestion")
        );

        let (status, detail) = if devin_running
            && (has_live_tool
                || (turn_in_progress && now - last_active_ms <= WORKING_TIMEOUT_MS)
                || has_unfinished_live_row
                || has_active_subagent)
        {
            if asking_user {
                (AgentStatus::Waiting, Some("Stiller et spørgsmål".to_string()))
            } else {
                let d = if has_active_subagent {
                    "Kører underagent".to_string()
                } else if has_unfinished_live_row && !turn_in_progress {
                    "Arbejder…".to_string()
                } else {
                    devin_working_detail(
                        tc_kind.as_deref(),
                        first_tool_name.as_deref(),
                        role.as_str(),
                        has_live_tool,
                    )
                };
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
            subagent_count: subagent_stats.active_count,
            todo,
            pid: devin_pid,
            process_running: devin_running,
        });
    }

    if let Some(c) = cache.as_deref_mut() {
        c.devin_sessions
            .retain(|id, _| seen_ids.contains(id));
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
