use std::path::Path;
use serde_json::Value;

use super::super::common::*;
use super::super::{AgentPaths, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{
    process::ProcessIndex, sqlite::open_readonly, AgentSession, AgentStatus,
};

pub(super) struct CursorAdapter;

impl AgentAdapter for CursorAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        mut cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        let Some(db_path) = paths.cursor_db.as_deref() else {
            return Vec::new();
        };
        SourceCache::scan_db_cached(&mut cache, db_path, procs, now, |fp| fp, |_| {
            scan_cursor(db_path, procs, now)
        })
    }
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
