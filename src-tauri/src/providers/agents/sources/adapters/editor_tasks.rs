use std::path::Path;
use std::fs;

use super::super::common::*;
use super::super::{AgentPaths, CachedFileSession, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{process::ProcessIndex, AgentSession, AgentStatus};

pub(super) struct EditorTasksAdapter;

impl AgentAdapter for EditorTasksAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        let Some(appdata) = paths.appdata.as_deref() else {
            return Vec::new();
        };
        scan_editor_tasks_cached(appdata, procs, now, cache)
    }
}

// ---------------------------------------------------------------------------
// 6. Cline / Roo Code / Kilo Code (ui_messages.json i globalStorage)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn scan_editor_tasks(appdata: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    scan_editor_tasks_cached(appdata, procs, now, None)
}

fn scan_editor_tasks_cached(
    appdata: &Path,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
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
            let dir_ok = match cache.as_deref_mut() {
                Some(c) => c.dir_exists(&tasks_dir, now),
                None => tasks_dir.is_dir(),
            };
            if !dir_ok {
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
                let fp = (mtime_ms, meta.len());
                let age = now - mtime_ms;
                let can_be_working = editor_running && age <= SHORT_ACTIVE_MS;

                if let Some(cached) = cache.as_deref().and_then(|c| c.get_file(&ui_path, fp)).cloned()
                {
                    let mut session = cached.session;
                    if cached.raw_working_status == Some(AgentStatus::Working) && !can_be_working {
                        session.status = AgentStatus::Idle;
                        session.detail = None;
                    }
                    session.pid = editor_pid;
                    session.process_running = editor_running;
                    out.push(session);
                    continue;
                }

                let head = read_head(&ui_path, 2048).unwrap_or_default();
                let title = extract_cline_task_title(&head);
                let tail = read_tail(&ui_path, TAIL_BYTES).unwrap_or_default();
                let (raw_status, raw_detail) = classify_cline_tail(&tail, true);
                let (status, detail) = if raw_status == AgentStatus::Working && !can_be_working {
                    (AgentStatus::Idle, None)
                } else {
                    (raw_status, raw_detail.clone())
                };
                let session = AgentSession {
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
                };
                if let Some(c) = cache.as_deref_mut() {
                    let mut stored = session.clone();
                    stored.status = raw_status;
                    stored.detail = raw_detail.clone();
                    c.store_file(
                        ui_path,
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
