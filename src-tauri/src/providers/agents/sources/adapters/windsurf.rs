use std::path::Path;
use super::super::common::*;
use super::super::{AgentPaths, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{process::ProcessIndex, AgentSession, AgentStatus};

pub(super) struct WindsurfAdapter;

impl AgentAdapter for WindsurfAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_windsurf_cascade_cached(&paths.windsurf_cascade, procs, now, cache)
    }
}

#[allow(dead_code)]
pub fn scan_windsurf_cascade(
    cascade_dir: &Path,
    procs: &ProcessIndex,
    now: i64,
) -> Vec<AgentSession> {
    scan_windsurf_cascade_cached(cascade_dir, procs, now, None)
}

fn scan_windsurf_cascade_cached(
    cascade_dir: &Path,
    procs: &ProcessIndex,
    now: i64,
    cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let running = procs.has_any(&["windsurf"]);
    if !running {
        return Vec::new();
    }
    let dir_ok = match cache {
        Some(c) => c.dir_exists(cascade_dir, now),
        None => cascade_dir.is_dir(),
    };
    if !dir_ok {
        return Vec::new();
    }
    let pid = procs.first_pid(&["windsurf"]);
    let files = collect_recent_files(cascade_dir, &["pb"], 2, MAX_RECENT_MS, now, 6);
    files
        .into_iter()
        .map(|(path, mtime_ms, _)| {
            let age = now - mtime_ms;
            let working = age <= SHORT_ACTIVE_MS;
            AgentSession {
                id: format!("windsurf:{}", path.display()),
                agent: "windsurf".to_string(),
                agent_label: "Windsurf".to_string(),
                title: None,
                project: None,
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
                process_running: true,
            }
        })
        .collect()
}
