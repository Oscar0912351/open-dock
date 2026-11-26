use std::path::Path;
use super::super::common::*;
use super::super::{AgentPaths, CachedFileSession, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{process::ProcessIndex, AgentSession, AgentStatus};

pub(super) struct GeminiCliAdapter;

impl AgentAdapter for GeminiCliAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_gemini_cli_cached(&paths.gemini_tmp, procs, now, cache)
    }
}

// 7. Gemini CLI, OpenCode & Windsurf Cascade + Aider
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn scan_gemini_cli(root: &Path, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    scan_gemini_cli_cached(root, procs, now, None)
}

fn scan_gemini_cli_cached(
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
    let running = procs.has_any(&["gemini"]);
    let pid = procs.first_pid(&["gemini"]);
    let files = collect_recent_files(root, &["json"], 4, MAX_RECENT_MS, now, 8);
    let mut out = Vec::new();
    for (path, mtime_ms, file_len) in files {
        if !path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|n| n.eq_ignore_ascii_case("chats"))
            .unwrap_or(false)
        {
            continue;
        }
        let fp = (mtime_ms, file_len);
        let age = now - mtime_ms;
        let working = age <= WORKING_TIMEOUT_MS && (running || age <= SHORT_ACTIVE_MS);
        let project = if let Some(cached) = cache.as_deref().and_then(|c| c.get_file(&path, fp)) {
            cached.session.project.clone()
        } else {
            let head = read_head(&path, HEAD_BYTES).unwrap_or_default();
            extract_cwd_from_text(&head)
                .as_deref()
                .and_then(project_from_path_str)
        };
        let session = AgentSession {
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
        };
        if let Some(c) = cache.as_deref_mut() {
            c.store_file(
                path,
                CachedFileSession {
                    fingerprint: fp,
                    session: session.clone(),
                    merge_key: None,
                    raw_working_status: None,
                    raw_working_detail: None,
                },
            );
        }
        out.push(session);
    }
    out
}

