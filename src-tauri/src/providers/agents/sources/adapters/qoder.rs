use std::path::Path;
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::super::common::*;
use super::super::{AgentPaths, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{
    process::ProcessIndex,
    sqlite::{db_fingerprint, open_readonly},
    AgentSession, AgentStatus,
};

pub(super) struct QoderAdapter;

impl AgentAdapter for QoderAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_qoder_cached(paths, procs, now, cache)
    }
}

// ---------------------------------------------------------------------------
// 10. Qoder (%APPDATA%\com.qoder.app.stable\main.sqlite + turn-payload-buffer)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn scan_qoder(paths: &AgentPaths, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let Some(db) = paths.qoder_db.as_deref() else {
        return Vec::new();
    };
    scan_qoder_impl(db, paths.qoder_turn_db.as_deref(), procs, now)
}

fn scan_qoder_cached(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let Some(db_path) = paths.qoder_db.as_deref() else {
        return Vec::new();
    };
    // Turn-bufferen skrives under aktiv generering, så dens fingeraftryk
    // foldes ind i cachenøglen — ellers ville live-status aldrig genlæses.
    let turn_fp = paths.qoder_turn_db.as_deref().and_then(db_fingerprint);
    SourceCache::scan_db_cached(
        &mut cache,
        db_path,
        procs,
        now,
        |fp| {
            turn_fp
                .map(|(m, l)| (fp.0.max(m), fp.1.saturating_add(l)))
                .unwrap_or(fp)
        },
        |_| scan_qoder_impl(db_path, paths.qoder_turn_db.as_deref(), procs, now),
    )
}

struct QoderLiveTurn {
    /// Seneste created_at på tværs af buffer-items og pending-projektioner.
    last_ms: i64,
    /// item_type for det sidste buffer-item (eller "pending" fra køen).
    last_item_type: String,
    /// item_json for seneste assistant-item (parset først ved behov).
    last_assistant_json: Option<String>,
    subagent_count: u32,
}

fn read_qoder_live_turns(path: Option<&Path>, now: i64) -> HashMap<String, QoderLiveTurn> {
    let mut by_session: HashMap<String, QoderLiveTurn> = HashMap::new();
    let Some(path) = path else {
        return by_session;
    };
    let Some(conn) = open_readonly(path) else {
        return by_session;
    };

    if let Ok(mut stmt) = conn.prepare(
        "SELECT session_id, item_type, COALESCE(parent_tool_use_id, ''), item_json, created_at \
         FROM active_turn_projection_items \
         ORDER BY session_id, ordinal ASC",
    ) {
        if let Ok(iter) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        }) {
            let mut recent_parents: HashMap<String, HashSet<String>> = HashMap::new();
            for (sid, item_type, parent, item_json, created_at) in iter.flatten() {
                if sid.is_empty() {
                    continue;
                }
                let created_ms = normalize_epoch_ms(created_at);
                let turn = by_session.entry(sid.clone()).or_insert(QoderLiveTurn {
                    last_ms: 0,
                    last_item_type: String::new(),
                    last_assistant_json: None,
                    subagent_count: 0,
                });
                turn.last_ms = turn.last_ms.max(created_ms);
                turn.last_item_type = item_type.clone();
                if item_type == "assistant" {
                    turn.last_assistant_json = Some(item_json);
                }
                if !parent.is_empty() && now - created_ms <= SUBAGENT_ACTIVE_MS {
                    recent_parents.entry(sid).or_default().insert(parent);
                }
            }
            for (sid, parents) in recent_parents {
                if let Some(turn) = by_session.get_mut(&sid) {
                    turn.subagent_count = parents.len() as u32;
                }
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT session_id, MAX(created_at) FROM pending_turn_projections GROUP BY session_id",
    ) {
        if let Ok(iter) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        }) {
            for (sid, created_at) in iter.flatten() {
                if sid.is_empty() {
                    continue;
                }
                let ms = normalize_epoch_ms(created_at);
                let turn = by_session.entry(sid).or_insert(QoderLiveTurn {
                    last_ms: 0,
                    last_item_type: String::new(),
                    last_assistant_json: None,
                    subagent_count: 0,
                });
                turn.last_ms = turn.last_ms.max(ms);
                if turn.last_item_type.is_empty() {
                    turn.last_item_type = "pending".to_string();
                }
            }
        }
    }

    by_session
}

/// Sidste tool_use-navn i et live-buffer assistant-item (Claude-format:
/// `message.content[]` med `tool_use`-indgange). `None` = kun tekst/tænkning.
fn qoder_buffer_tool_name(item_json: &str) -> Option<String> {
    let val = serde_json::from_str::<Value>(item_json).ok()?;
    let content = val.pointer("/message/content")?.as_array()?;
    content.iter().rev().find_map(|c| {
        if c.get("type").and_then(Value::as_str) == Some("tool_use") {
            c.get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        } else {
            None
        }
    })
}

/// Sidste værktøjsnavn i en chat_session_messages-payload (parts-format).
/// Foretrækker et tool-part med status "running" frem for blot det sidste.
fn qoder_payload_tool_name(payload_json: &str) -> Option<String> {
    let val = serde_json::from_str::<Value>(payload_json).ok()?;
    let parts = val.get("parts")?.as_array()?;
    let mut last_tool: Option<&str> = None;
    for part in parts {
        if part.get("type").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        let Some(tool) = part.get("tool") else {
            continue;
        };
        let name = tool.get("name").and_then(Value::as_str);
        let status = tool.get("status").and_then(Value::as_str).unwrap_or("");
        if status == "running" {
            return name.map(str::to_string);
        }
        last_tool = name;
    }
    last_tool.map(str::to_string)
}

/// Klassificér en session ud fra dens seneste bruger/assistent-besked i main-DB.
fn qoder_message_status(
    role: &str,
    msg_status: &str,
    payload: &str,
    created_at: i64,
    qoder_running: bool,
    now: i64,
) -> (AgentStatus, Option<String>) {
    let fresh = qoder_running && now - normalize_epoch_ms(created_at) <= SHORT_ACTIVE_MS;
    match role {
        "user" if fresh => (AgentStatus::Working, Some("Tænker…".to_string())),
        "assistant" => match msg_status {
            "completed" => (AgentStatus::Ready, Some("Klar til svar".to_string())),
            "failed" | "interrupted" => (AgentStatus::Idle, None),
            _ if fresh => {
                // Ikke-terminal besked = tur undervejs.
                match qoder_payload_tool_name(payload).as_deref() {
                    Some("AskUserQuestion") => {
                        (AgentStatus::Waiting, Some("Stiller et spørgsmål".to_string()))
                    }
                    Some(name) => (AgentStatus::Working, Some(super::claude_code::map_claude_tool_name(name))),
                    None => (AgentStatus::Working, Some("Arbejder…".to_string())),
                }
            }
            _ => (AgentStatus::Idle, None),
        },
        _ => (AgentStatus::Idle, None),
    }
}

fn scan_qoder_impl(
    db_path: &Path,
    turn_db_path: Option<&Path>,
    procs: &ProcessIndex,
    now: i64,
) -> Vec<AgentSession> {
    let Some(conn) = open_readonly(db_path) else {
        return Vec::new();
    };
    let qoder_running = procs.has_any(&["qoder"]);
    let qoder_pid = procs.first_pid(&["qoder"]);
    let live_turns = read_qoder_live_turns(turn_db_path, now);

    let mut stmt = match conn.prepare(
        "SELECT session_id, COALESCE(title, ''), COALESCE(cwd, ''), CAST(updated_at AS TEXT) \
         FROM chat_sessions \
         WHERE deleted_at IS NULL AND archived = 0 \
         ORDER BY updated_at DESC \
         LIMIT 16",
    ) {
        Ok(s) => s,
        Err(err) => {
            log::debug!(target: "provider:agents", "Qoder SQL-fejl: {err}");
            return Vec::new();
        }
    };

    let rows_iter = match stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    }) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for (sid, title, cwd, updated_raw) in rows_iter.flatten() {
        if sid.is_empty() {
            continue;
        }
        let mut last_active_ms = parse_timestamp_ms(&updated_raw).unwrap_or(0);

        // Seneste bruger/assistent-besked (system/hook-støj tælles ikke med).
        let msg: Option<(String, String, String, i64)> = conn
            .query_row(
                "SELECT role, status, payload_json, created_at \
                 FROM chat_session_messages \
                 WHERE session_id = ?1 AND role IN ('user', 'assistant') \
                 ORDER BY sequence DESC \
                 LIMIT 1",
                [&sid],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2).unwrap_or_default(),
                        r.get::<_, i64>(3).unwrap_or(0),
                    ))
                },
            )
            .ok();
        if let Some((_, _, _, created_at)) = &msg {
            last_active_ms = last_active_ms.max(normalize_epoch_ms(*created_at));
        }

        let mut status = AgentStatus::Idle;
        let mut detail: Option<String> = None;
        let mut subagent_count = 0u32;
        let mut classified_live = false;

        if let Some(t) = live_turns.get(&sid) {
            last_active_ms = last_active_ms.max(t.last_ms);
            if t.last_ms > 0 && now - t.last_ms <= WORKING_TIMEOUT_MS && qoder_running {
                classified_live = true;
                subagent_count = t.subagent_count;
                match t.last_item_type.as_str() {
                    "assistant" => {
                        match t
                            .last_assistant_json
                            .as_deref()
                            .and_then(qoder_buffer_tool_name)
                            .as_deref()
                        {
                            Some("AskUserQuestion") => {
                                status = AgentStatus::Waiting;
                                detail = Some("Stiller et spørgsmål".to_string());
                            }
                            Some(name) => {
                                status = AgentStatus::Working;
                                detail = Some(super::claude_code::map_claude_tool_name(name));
                            }
                            // Kun tekst tilbage i bufferen = svar netop færdigt.
                            None => {
                                status = AgentStatus::Ready;
                                detail = Some("Klar til svar".to_string());
                            }
                        }
                    }
                    _ => {
                        status = AgentStatus::Working;
                        detail = Some("Tænker…".to_string());
                    }
                }
            }
        }

        if !classified_live {
            if let Some((role, msg_status, payload, created_at)) = msg {
                (status, detail) =
                    qoder_message_status(&role, &msg_status, &payload, created_at, qoder_running, now);
            }
        }

        let is_active = matches!(status, AgentStatus::Working | AgentStatus::Waiting);
        if !is_active && (last_active_ms <= 0 || now - last_active_ms > MAX_RECENT_MS) {
            continue;
        }

        out.push(AgentSession {
            id: format!("qoder:{sid}"),
            agent: "qoder".to_string(),
            agent_label: "Qoder".to_string(),
            title: non_empty_trimmed(&title),
            project: project_from_path_str(&cwd),
            status,
            last_active_ms,
            detail,
            subagent_count,
            todo: None,
            pid: qoder_pid,
            process_running: qoder_running,
        });
    }

    out
}

// ---------------------------------------------------------------------------
