use std::path::Path;

use serde_json::Value;

use super::super::common::*;
use super::super::{AgentPaths, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{
    process::ProcessIndex,
    sqlite::open_readonly,
    AgentSession, AgentStatus,
};

pub(super) struct OpenCodeAdapter;

impl AgentAdapter for OpenCodeAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_opencode_cached(paths, procs, now, cache)
    }
}

// ---------------------------------------------------------------------------
// 8. OpenCode (%USERPROFILE%\.local\share\opencode\opencode.db — session_v2)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn scan_opencode(paths: &AgentPaths, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    scan_opencode_impl(&paths.opencode_db, procs, now)
}

fn scan_opencode_cached(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    SourceCache::scan_db_cached(
        &mut cache,
        &paths.opencode_db,
        procs,
        now,
        |fp| fp,
        |_| scan_opencode_impl(&paths.opencode_db, procs, now),
    )
}

fn map_opencode_tool_name(name: &str) -> String {
    match name {
        "shell" | "execute" => "Kører kommando",
        "edit" | "write" => "Redigerer filer",
        "read" | "grep" | "glob" => "Læser filer",
        "websearch" | "webfetch" => "Søger på nettet",
        "subagent" => "Kører underagent",
        "question" => "Stiller et spørgsmål",
        _ => "Bruger værktøj",
    }
    .to_string()
}

fn tool_in_progress(status: &str) -> bool {
    !matches!(status, "completed" | "error")
}

/// Klassificér seneste assistant-besked: sidste content-element styrer detaljen.
fn opencode_working_detail(data_json: &str) -> (AgentStatus, String) {
    let default = || (AgentStatus::Working, "Arbejder…".to_string());
    let Ok(val) = serde_json::from_str::<Value>(data_json) else {
        return default();
    };
    let Some(content) = val.get("content").and_then(Value::as_array) else {
        return default();
    };
    let Some(last) = content.last() else {
        return default();
    };
    match last.get("type").and_then(Value::as_str) {
        Some("tool") => {
            let name = last.get("name").and_then(Value::as_str).unwrap_or("");
            let status = last
                .pointer("/state/status")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !tool_in_progress(status) {
                return default();
            }
            if name == "question" {
                (AgentStatus::Waiting, "Stiller et spørgsmål".to_string())
            } else {
                (AgentStatus::Working, map_opencode_tool_name(name))
            }
        }
        Some("reasoning") => (AgentStatus::Working, "Tænker…".to_string()),
        _ => default(),
    }
}

fn scan_opencode_impl(db_path: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    let Some(conn) = open_readonly(db_path) else {
        return Vec::new();
    };
    let running = procs.has_any(&["opencode", "opencode-cli"]);
    let pid = procs.first_pid(&["opencode", "opencode-cli"]);
    let recent_cutoff = now - MAX_RECENT_MS;
    let child_cutoff = now - WORKING_TIMEOUT_MS;

    let mut stmt = match conn.prepare(
        "SELECT id, COALESCE(title, ''), directory, time_updated, time_busy, time_idle, idle_outcome \
         FROM session_v2 \
         WHERE parent_id IS NULL AND time_archived IS NULL \
           AND (time_updated >= ?1 OR (time_busy IS NOT NULL AND (time_idle IS NULL OR time_busy > time_idle))) \
         ORDER BY time_updated DESC \
         LIMIT 16",
    ) {
        Ok(s) => s,
        Err(err) => {
            log::warn!(target: "provider:agents", "OpenCode SQL-fejl (session_v2): {err}");
            return Vec::new();
        }
    };

    let rows: Vec<(String, String, String, i64, Option<i64>, Option<i64>, Option<String>)> =
        match stmt.query_map([recent_cutoff], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1).unwrap_or_default(),
                r.get::<_, String>(2).unwrap_or_default(),
                r.get::<_, i64>(3).unwrap_or(0),
                r.get::<_, Option<i64>>(4).unwrap_or(None),
                r.get::<_, Option<i64>>(5).unwrap_or(None),
                r.get::<_, Option<String>>(6).unwrap_or(None),
            ))
        }) {
            Ok(iter) => iter.flatten().collect(),
            Err(err) => {
                log::warn!(target: "provider:agents", "OpenCode query-fejl (session_v2): {err}");
                return Vec::new();
            }
        };

    let mut out = Vec::new();
    for (sid, title, directory, time_updated, time_busy, time_idle, idle_outcome) in rows {
        if sid.is_empty() {
            continue;
        }
        let busy = match (time_busy, time_idle) {
            (Some(tb), ti) => ti.map(|i| tb > i).unwrap_or(true),
            _ => false,
        };

        // Børn (underagent-sessioner): antal busy + seneste aktivitet.
        let (busy_children, child_max_updated) = conn
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN time_busy IS NOT NULL \
                     AND (time_idle IS NULL OR time_busy > time_idle) \
                     AND time_updated >= ?2 THEN 1 ELSE 0 END), 0), \
                    COALESCE(MAX(time_updated), 0) \
                 FROM session_v2 WHERE parent_id = ?1",
                rusqlite::params![sid, child_cutoff],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .unwrap_or((0, 0));

        let mut last_active_ms = time_updated.max(child_max_updated);
        if let Some(tb) = time_busy {
            last_active_ms = last_active_ms.max(tb);
        }

        let status;
        let detail: Option<String>;
        let mut subagent_count = 0u32;

        if busy && running {
            subagent_count = busy_children.max(0) as u32;
            let msg_data: Option<String> = conn
                .query_row(
                    "SELECT data FROM session_message \
                     WHERE session_id = ?1 AND type = 'assistant' \
                     ORDER BY seq DESC LIMIT 1",
                    [&sid],
                    |r| r.get::<_, String>(0),
                )
                .ok();
            let (st, det) = msg_data
                .as_deref()
                .map(opencode_working_detail)
                .unwrap_or((AgentStatus::Working, "Arbejder…".to_string()));
            status = st;
            detail = Some(det);
        } else if busy {
            // Proces ikke i live — ingen falsk "arbejder".
            status = AgentStatus::Idle;
            detail = None;
        } else {
            let failed = idle_outcome
                .as_deref()
                .map(|o| matches!(o, "interrupted" | "failed" | "error"))
                .unwrap_or(false);
            if failed {
                status = AgentStatus::Idle;
                detail = None;
            } else {
                status = AgentStatus::Ready;
                detail = Some("Klar til svar".to_string());
            }
        }

        let is_active = matches!(status, AgentStatus::Working | AgentStatus::Waiting);
        if !is_active && (last_active_ms <= 0 || now - last_active_ms > MAX_RECENT_MS) {
            continue;
        }

        out.push(AgentSession {
            id: format!("opencode:{sid}"),
            agent: "opencode".to_string(),
            agent_label: "OpenCode".to_string(),
            title: non_empty_trimmed(&title),
            project: project_from_path_str(&directory),
            status,
            last_active_ms,
            detail,
            subagent_count,
            todo: None,
            pid,
            process_running: running,
        });
    }

    out
}
