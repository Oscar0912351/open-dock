use std::path::Path;
use std::collections::HashMap;

use serde_json::Value;

use super::super::common::*;
use super::super::{AgentPaths, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{
    process::ProcessIndex, sqlite::open_readonly, AgentSession, AgentStatus,
};

pub(super) struct AntigravityAdapter;

impl AgentAdapter for AntigravityAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        mut cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        SourceCache::scan_db_cached(
            &mut cache,
            &paths.agy_db,
            procs,
            now,
            |fp| fp,
            |_| scan_antigravity(paths, procs, now),
        )
    }
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
                    .or(tail_info.fallback_detail)
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
        _ => "Bruger værktøj",
    }
    .to_string()
}

fn parse_agy_workspace_project(raw_json: &str) -> Option<String> {
    let arr = serde_json::from_str::<Value>(raw_json).ok()?;
    let first = arr.as_array()?.first()?.as_str()?;
    project_from_uri(first)
}

// ---------------------------------------------------------------------------
