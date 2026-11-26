use crate::providers::agents::{process::ProcessIndex, AgentSession};

use super::{AgentPaths, SourceCache};

pub(super) mod aider;
pub(super) mod antigravity;
pub(super) mod claude_code;
pub(super) mod codex;
pub(super) mod cursor;
pub(super) mod devin;
pub(super) mod editor_tasks;
pub(super) mod gemini_cli;
pub(super) mod grok_bot;
pub(super) mod hermes;
pub(super) mod opencode;
pub(super) mod qoder;
pub(super) mod windsurf;

pub(super) trait AgentAdapter: Sync {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession>;
}

pub(super) const ALL_ADAPTERS: &[&dyn AgentAdapter] = &[
    &antigravity::AntigravityAdapter,
    &devin::DevinAdapter,
    &codex::CodexAdapter,
    &claude_code::ClaudeCodeAdapter,
    &cursor::CursorAdapter,
    &editor_tasks::EditorTasksAdapter,
    &gemini_cli::GeminiCliAdapter,
    &opencode::OpenCodeAdapter,
    &windsurf::WindsurfAdapter,
    &aider::AiderAdapter,
    &hermes::HermesAdapter,
    &grok_bot::GrokBotAdapter,
    &qoder::QoderAdapter,
];
