use super::super::{AgentPaths, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{process::ProcessIndex, AgentSession, AgentStatus};

pub(super) struct AiderAdapter;

impl AgentAdapter for AiderAdapter {
    fn scan(
        &self,
        _paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        _cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_aider(procs, now)
    }
}

pub fn scan_aider(procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    procs
        .aider_sessions()
        .iter()
        .map(|(pid, project, started_ms)| AgentSession {
            id: format!("aider:{pid}"),
            agent: "aider".to_string(),
            agent_label: "Aider".to_string(),
            title: None,
            project: project.clone(),
            status: AgentStatus::Idle,
            last_active_ms: if *started_ms > 0 { *started_ms } else { now },
            detail: None,
            subagent_count: 0,
            todo: None,
            pid: Some(*pid),
            process_running: true,
        })
        .collect()
}

// ---------------------------------------------------------------------------
