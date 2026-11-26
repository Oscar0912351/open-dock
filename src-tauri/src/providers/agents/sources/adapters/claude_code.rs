use std::{collections::HashMap, fs, path::{Path, PathBuf}};

use serde_json::Value;

use super::super::common::*;
use super::super::{AgentPaths, CachedFileSession, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{process::ProcessIndex, AgentSession, AgentStatus, AgentTodoProgress};

pub(super) struct ClaudeCodeAdapter;

impl AgentAdapter for ClaudeCodeAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_claude_code_cached(paths, procs, now, cache)
    }
}

// ---------------------------------------------------------------------------
// 4. Claude Code (~/.claude/projects/*/*.jsonl)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn scan_claude_code(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
) -> Vec<AgentSession> {
    scan_claude_code_cached(paths, procs, now, None)
}

fn scan_claude_code_cached(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let projects_dir = &paths.claude_projects;
    let dir_ok = match cache.as_deref_mut() {
        Some(c) => c.dir_exists(projects_dir, now),
        None => projects_dir.is_dir(),
    };
    if !dir_ok {
        return Vec::new();
    }

    let claude_running = procs.has_any(&["claude", "claude-code"]);
    let claude_pid = procs.first_pid(&["claude", "claude-code"]);

    let mut newest_main_per_dir: HashMap<PathBuf, (PathBuf, i64, u64)> = HashMap::new();
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

            let file_len = meta.len();
            newest_main_per_dir
                .entry(parent)
                .and_modify(|slot| {
                    if mtime_ms > slot.1 {
                        slot.0 = path.clone();
                        slot.1 = mtime_ms;
                        slot.2 = file_len;
                    }
                })
                .or_insert((path, mtime_ms, file_len));
        }
    }

    let mut sorted: Vec<(PathBuf, PathBuf, i64, u64)> = newest_main_per_dir
        .into_iter()
        .map(|(dir, (file, mtime, len))| (dir, file, mtime, len))
        .collect();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.2));
    sorted.truncate(12);

    let mut out = Vec::new();
    for (dir, path, mtime_ms, file_len) in sorted {
        let fp = (mtime_ms, file_len);
        let age = now - mtime_ms;
        let allow_working = age <= WORKING_TIMEOUT_MS && (claude_running || age <= SHORT_ACTIVE_MS);
        let subagent_count = active_subagents_per_dir.get(&dir).copied().unwrap_or(0);

        if let Some(cached) = cache.as_deref().and_then(|c| c.get_file(&path, fp)).cloned() {
            let mut session = cached.session;
            if cached.raw_working_status == Some(AgentStatus::Working) && !allow_working {
                session.status = AgentStatus::Idle;
                session.detail = None;
            }
            session.subagent_count = subagent_count;
            session.pid = claude_pid;
            session.process_running = claude_running;
            out.push(session);
            continue;
        }

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

        let tail = parse_claude_tail(&path, true);
        let todo = read_claude_todo_file(&paths.claude_todos, &path).or(tail.todo);
        let raw_status = tail.status;
        let raw_detail = tail.detail.clone();

        let (eff_status, eff_detail) = if raw_status == AgentStatus::Working && !allow_working {
            (AgentStatus::Idle, None)
        } else {
            (raw_status, raw_detail.clone())
        };

        let session = AgentSession {
            id: format!("claude-code:{}", path.display()),
            agent: "claude-code".to_string(),
            agent_label: "Claude Code".to_string(),
            title: tail.summary.or(head_title),
            project,
            status: eff_status,
            last_active_ms: mtime_ms,
            detail: eff_detail,
            subagent_count,
            todo,
            pid: claude_pid,
            process_running: claude_running,
        };

        if let Some(c) = cache.as_deref_mut() {
            let mut stored = session.clone();
            stored.status = raw_status;
            stored.detail = raw_detail.clone();
            c.store_file(
                path,
                CachedFileSession {
                    fingerprint: fp,
                    session: stored,
                    merge_key: None,
                    raw_working_status: Some(raw_status),
                    raw_working_detail: raw_detail,
                },
            );
        }

        out.push(session);
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
    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
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

pub(super) fn map_claude_tool_name(name: &str) -> String {
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
