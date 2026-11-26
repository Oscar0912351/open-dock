use std::path::Path;
use std::collections::HashMap;

use serde_json::Value;

use super::super::common::*;
use super::super::{AgentPaths, CachedFileSession, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{
    process::ProcessIndex,
    sqlite::{db_fingerprint, open_readonly},
    AgentSession, AgentStatus, AgentTodoProgress,
};

pub(super) struct CodexAdapter;

impl AgentAdapter for CodexAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_codex_cached(paths, procs, now, cache)
    }
}

// ---------------------------------------------------------------------------
// 3. Codex (~/.codex/sqlite/codex.db + ~/.codex/sessions/**/*.jsonl)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn scan_codex(paths: &AgentPaths, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    scan_codex_cached(paths, procs, now, None)
}

fn scan_codex_cached(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let codex_running = procs.has_any(&["codex", "codex-cli"]);
    let codex_pid = procs.first_pid(&["codex", "codex-cli"]);
    let mut by_thread: HashMap<String, AgentSession> = HashMap::new();

    if let Some(fp) = db_fingerprint(&paths.codex_db) {
        let db_sessions = if let Some(cached) = cache
            .as_deref()
            .and_then(|c| c.get_db_if_unchanged(&paths.codex_db, fp, procs, now))
        {
            cached
        } else {
            let mut fresh = Vec::new();
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
                            if !is_active_turn
                                && (last_active_ms <= 0 || now - last_active_ms > MAX_RECENT_MS)
                            {
                                continue;
                            }

                            let (status, detail) = if is_active_turn {
                                (AgentStatus::Working, Some("Arbejder…".to_string()))
                            } else {
                                (AgentStatus::Ready, Some("Klar til svar".to_string()))
                            };

                            fresh.push(AgentSession {
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
                            });
                        }
                    }
                }
            }
            if let Some(c) = cache.as_deref_mut() {
                c.store_db(&paths.codex_db, Some(fp), &fresh);
            }
            fresh
        };
        for s in db_sessions {
            let tid = s.id.strip_prefix("codex:").unwrap_or(&s.id).to_string();
            by_thread.insert(tid, s);
        }
    }

    let sessions_dir_ok = match cache.as_deref_mut() {
        Some(c) => c.dir_exists(&paths.codex_sessions, now),
        None => paths.codex_sessions.is_dir(),
    };
    if !sessions_dir_ok {
        return by_thread.into_values().collect();
    }

    // Suppler/opdater fra friske .jsonl-filer (skipper .jsonl.zst og uændrede filer).
    for (path, mtime_ms, file_len) in
        collect_recent_files(&paths.codex_sessions, &["jsonl"], 4, MAX_RECENT_MS, now, 12)
    {
        let fp = (mtime_ms, file_len);
        let age = now - mtime_ms;
        let allow_working = age <= WORKING_TIMEOUT_MS && (codex_running || age <= SHORT_ACTIVE_MS);

        let (thread_id, project, head_title, tail_status, tail_detail, tail_todo) =
            if let Some(cached) = cache.as_deref().and_then(|c| c.get_file(&path, fp)).cloned() {
                let (st, det) = match cached.raw_working_status {
                    Some(AgentStatus::Working) if !allow_working => (Some(AgentStatus::Idle), None),
                    other => (other, cached.raw_working_detail),
                };
                (
                    cached
                        .merge_key
                        .unwrap_or_else(|| path.to_string_lossy().to_string()),
                    cached.session.project,
                    cached.session.title,
                    st,
                    det,
                    cached.session.todo,
                )
            } else {
                let head = read_head(&path, HEAD_BYTES).unwrap_or_default();
                let (head_tid, head_cwd, head_title) = parse_codex_head(&head);
                let thread_id = head_tid.unwrap_or_else(|| path.to_string_lossy().to_string());
                let project = head_cwd.as_deref().and_then(project_from_path_str);
                let (raw_st, raw_det, tail_todo) = parse_codex_tail(&path, true);
                if let Some(c) = cache.as_deref_mut() {
                    c.store_file(
                        path.clone(),
                        CachedFileSession {
                            fingerprint: fp,
                            merge_key: Some(thread_id.clone()),
                            raw_working_status: raw_st,
                            raw_working_detail: raw_det.clone(),
                            session: AgentSession {
                                id: format!("codex:{thread_id}"),
                                agent: "codex".to_string(),
                                agent_label: "Codex".to_string(),
                                title: head_title.clone(),
                                project: project.clone(),
                                status: raw_st.unwrap_or(AgentStatus::Idle),
                                last_active_ms: mtime_ms,
                                detail: raw_det.clone(),
                                subagent_count: 0,
                                todo: tail_todo.clone(),
                                pid: codex_pid,
                                process_running: codex_running,
                            },
                        },
                    );
                }
                let (st, det) = match raw_st {
                    Some(AgentStatus::Working) if !allow_working => (Some(AgentStatus::Idle), None),
                    other => (other, raw_det),
                };
                (thread_id, project, head_title, st, det, tail_todo)
            };

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
