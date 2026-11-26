use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use super::super::common::*;
use super::super::{AgentPaths, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{
    process::ProcessIndex,
    sqlite::{db_fingerprint, open_readonly},
    AgentSession, AgentStatus,
};

pub(super) struct HermesAdapter;

impl AgentAdapter for HermesAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        let Some(root) = paths.hermes_root.as_deref() else {
            return Vec::new();
        };
        scan_hermes_cached(root, procs, now, cache)
    }
}

// ---------------------------------------------------------------------------
// 8. Hermes (%LOCALAPPDATA%\hermes\state.db + profiles\*\state.db)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn scan_hermes(root: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    scan_hermes_cached(root, procs, now, None)
}

fn scan_hermes_cached(
    root: &Path,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let dir_ok = match cache.as_deref_mut() {
        Some(c) => c.dir_exists(root, now),
        None => root.is_dir(),
    };
    if !dir_ok {
        return Vec::new();
    }

    let mut dbs: Vec<(String, PathBuf)> = Vec::new();
    let root_db = root.join("state.db");
    if root_db.is_file() {
        dbs.push(("default".to_string(), root_db));
    }
    let profiles_dir = root.join("profiles");
    let profiles_ok = match cache.as_deref_mut() {
        Some(c) => c.dir_exists(&profiles_dir, now),
        None => profiles_dir.is_dir(),
    };
    if profiles_ok {
        if let Ok(entries) = fs::read_dir(&profiles_dir) {
            for entry in entries.flatten().take(24) {
                let db = entry.path().join("state.db");
                if db.is_file() {
                    let label = entry.file_name().to_string_lossy().to_string();
                    dbs.push((label, db));
                }
            }
        }
    }

    let mut out = Vec::new();
    for (label, db) in dbs {
        let Some(fp) = db_fingerprint(&db) else {
            continue;
        };
        if let Some(cached) = cache
            .as_deref()
            .and_then(|c| c.get_db_if_unchanged(&db, fp, procs, now))
        {
            out.extend(cached);
            continue;
        }
        let fresh = scan_hermes_db(&db, &label, procs, now);
        if let Some(c) = cache.as_deref_mut() {
            c.store_db(&db, Some(fp), &fresh);
        }
        out.extend(fresh);
    }
    out
}

struct HermesRow {
    id: String,
    title: String,
    display_name: String,
    cwd: String,
    profile: String,
    last_activity_at: f64,
    last_activity_desc: String,
    ended: bool,
    parent_id: String,
}

fn scan_hermes_db(
    db_path: &Path,
    profile_label: &str,
    procs: &ProcessIndex,
    now: i64,
) -> Vec<AgentSession> {
    let Some(conn) = open_readonly(db_path) else {
        return Vec::new();
    };
    let now_sec = now as f64 / 1000.0;

    let mut live: HashSet<String> = HashSet::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT conversation_id FROM session_turn_leases WHERE expires_at > ?1")
    {
        if let Ok(iter) = stmt.query_map([now_sec], |r| r.get::<_, String>(0)) {
            for id in iter.flatten() {
                live.insert(id);
            }
        }
    }
    if let Ok(mut stmt) =
        conn.prepare("SELECT DISTINCT session_id FROM worker_executions WHERE status = 'running'")
    {
        if let Ok(iter) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for id in iter.flatten() {
                live.insert(id);
            }
        }
    }

    let sub_cutoff = now_sec - (SUBAGENT_ACTIVE_MS as f64 / 1000.0);
    let mut subagent_counts: HashMap<String, u32> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT parent_session_id, COUNT(*) FROM sessions \
         WHERE parent_session_id IS NOT NULL AND parent_session_id <> '' \
           AND ended_at IS NULL AND last_activity_at >= ?1 \
         GROUP BY parent_session_id",
    ) {
        if let Ok(iter) = stmt.query_map([sub_cutoff], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        }) {
            for (pid, n) in iter.flatten() {
                subagent_counts.insert(pid, n as u32);
            }
        }
    }

    let mut stmt = match conn.prepare(
        "SELECT id, COALESCE(title,''), COALESCE(display_name,''), COALESCE(cwd,''), \
                COALESCE(profile_name,''), last_activity_at, \
                COALESCE(last_activity_description,''), ended_at, COALESCE(parent_session_id,'') \
         FROM sessions \
         WHERE hidden = 0 AND archived = 0 \
         ORDER BY last_activity_at DESC \
         LIMIT 20",
    ) {
        Ok(s) => s,
        Err(err) => {
            log::debug!(target: "provider:agents", "Hermes SQL-fejl: {err}");
            return Vec::new();
        }
    };

    let rows_iter = match stmt.query_map([], |row| {
        Ok(HermesRow {
            id: row.get::<_, String>(0).unwrap_or_default(),
            title: row.get::<_, String>(1).unwrap_or_default(),
            display_name: row.get::<_, String>(2).unwrap_or_default(),
            cwd: row.get::<_, String>(3).unwrap_or_default(),
            profile: row.get::<_, String>(4).unwrap_or_default(),
            last_activity_at: row.get::<_, f64>(5).unwrap_or(0.0),
            last_activity_desc: row.get::<_, String>(6).unwrap_or_default(),
            ended: row.get::<_, Option<f64>>(7).unwrap_or(None).is_some(),
            parent_id: row.get::<_, String>(8).unwrap_or_default(),
        })
    }) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let hermes_running = procs.has_any(&["hermes", "hermes-acp"]);
    let hermes_pid = procs.first_pid(&["hermes", "hermes-acp"]);
    let mut out = Vec::new();

    for row in rows_iter.flatten() {
        if row.id.is_empty() || !row.parent_id.is_empty() {
            continue;
        }
        let last_active_ms = float_epoch_to_ms(row.last_activity_at);
        let is_live = live.contains(&row.id);
        if !is_live && (last_active_ms <= 0 || now - last_active_ms > MAX_RECENT_MS) {
            continue;
        }

        let last_msg: Option<(String, String, String, i64)> = conn
            .query_row(
                "SELECT role, COALESCE(tool_name,''), COALESCE(display_kind,''), \
                        CASE WHEN tool_calls IS NULL OR tool_calls IN ('','[]','null') \
                             THEN 0 ELSE 1 END \
                 FROM messages WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
                [&row.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let (role, tool_name, display_kind, has_tool_calls) = last_msg.unwrap_or_default();

        let age = now - last_active_ms;
        let allow_working = hermes_running || age <= SHORT_ACTIVE_MS;
        let mid_turn = matches!(role.as_str(), "user" | "tool")
            || (role == "assistant" && has_tool_calls != 0);

        let (status, detail) = if is_live && allow_working {
            let d = non_empty_trimmed(&row.last_activity_desc)
                .or_else(|| non_empty_trimmed(&tool_name).map(|t| map_hermes_tool_name(&t)))
                .unwrap_or_else(|| "Arbejder…".to_string());
            (AgentStatus::Working, Some(d))
        } else if row.ended {
            (AgentStatus::Idle, None)
        } else if display_kind == "failed_turn" {
            (AgentStatus::Idle, Some("Fejlet".to_string()))
        } else if role == "assistant" && has_tool_calls == 0 {
            (AgentStatus::Ready, Some("Klar til svar".to_string()))
        } else if mid_turn && allow_working && age <= WORKING_TIMEOUT_MS {
            let d = non_empty_trimmed(&row.last_activity_desc)
                .unwrap_or_else(|| "Tænker…".to_string());
            (AgentStatus::Working, Some(d))
        } else {
            (AgentStatus::Idle, None)
        };

        let profile = non_empty_trimmed(&row.profile)
            .unwrap_or_else(|| profile_label.to_string());
        let project = non_empty_trimmed(&row.cwd)
            .as_deref()
            .and_then(project_from_path_str)
            .or_else(|| {
                (profile != "default" && !profile.is_empty()).then_some(profile.clone())
            });

        out.push(AgentSession {
            id: format!("hermes:{profile}:{id}", id = row.id),
            agent: "hermes".to_string(),
            agent_label: "Hermes".to_string(),
            title: non_empty_trimmed(&row.title)
                .or_else(|| non_empty_trimmed(&row.display_name)),
            project,
            status,
            last_active_ms,
            detail,
            subagent_count: subagent_counts.get(&row.id).copied().unwrap_or(0),
            todo: None,
            pid: hermes_pid,
            process_running: hermes_running,
        });
    }

    out
}

fn map_hermes_tool_name(name: &str) -> String {
    match name {
        "terminal" | "execute_code" | "bash" | "shell" | "process" | "run_command" => {
            "Kører kommando"
        }
        "read_file" | "read" | "ls" | "view" | "open" => "Læser filer",
        "write_file" | "edit_file" | "apply_patch" | "patch" | "multi_edit" | "write" => {
            "Redigerer filer"
        }
        "web_search" | "search" | "browse" | "fetch" | "web" => "Søger på nettet",
        "delegate" | "spawn_agent" | "subagent" | "task" | "dispatch" => "Kører underagent",
        _ => "Bruger værktøj",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
