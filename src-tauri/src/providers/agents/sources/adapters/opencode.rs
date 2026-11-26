use std::{collections::HashSet, path::PathBuf};

use super::super::common::*;
use super::super::{AgentPaths, CachedFileSession, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{process::ProcessIndex, AgentSession, AgentStatus};

pub(super) struct OpenCodeAdapter;

impl AgentAdapter for OpenCodeAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_opencode_cached(&paths.opencode_dirs, procs, now, cache)
    }
}

#[allow(dead_code)]
pub fn scan_opencode(roots: &[PathBuf], procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    scan_opencode_cached(roots, procs, now, None)
}

fn scan_opencode_cached(
    roots: &[PathBuf],
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let running = procs.has_any(&["opencode"]);
    let pid = procs.first_pid(&["opencode"]);
    let mut out = Vec::new();
    let mut seen_paths = HashSet::new();

    for root in roots {
        let dir_ok = match cache.as_deref_mut() {
            Some(c) => c.dir_exists(root, now),
            None => root.is_dir(),
        };
        if !dir_ok {
            continue;
        }
        for (path, mtime_ms, file_len) in
            collect_recent_files(root, &["json", "jsonl"], 4, MAX_RECENT_MS, now, 8)
        {
            if !seen_paths.insert(path.clone()) {
                continue;
            }
            let fp = (mtime_ms, file_len);
            let age = now - mtime_ms;
            let working = age <= WORKING_TIMEOUT_MS && (running || age <= SHORT_ACTIVE_MS);
            let project = if let Some(cached) = cache.as_deref().and_then(|c| c.get_file(&path, fp))
            {
                cached.session.project.clone()
            } else {
                let head = read_head(&path, HEAD_BYTES).unwrap_or_default();
                extract_cwd_from_text(&head)
                    .as_deref()
                    .and_then(project_from_path_str)
            };
            let session = AgentSession {
                id: format!("opencode:{}", path.display()),
                agent: "opencode".to_string(),
                agent_label: "OpenCode".to_string(),
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
    }
    out
}

