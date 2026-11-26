use std::path::Path;
use std::fs;

use serde_json::Value;

use super::super::common::*;
use super::super::{AgentPaths, CachedFileSession, SourceCache};
use super::AgentAdapter;
use crate::providers::agents::{process::ProcessIndex, AgentSession, AgentStatus};

pub(super) struct GrokBotAdapter;

impl AgentAdapter for GrokBotAdapter {
    fn scan(
        &self,
        paths: &AgentPaths,
        procs: &ProcessIndex,
        now: i64,
        cache: Option<&mut SourceCache>,
    ) -> Vec<AgentSession> {
        scan_grok_bot_cached(paths, procs, now, cache)
    }
}

// ---------------------------------------------------------------------------
// 9. Grok Bot (%APPDATA%\Grok Bot\sand-client-persistence + ~/.grokbot daemon)
// ---------------------------------------------------------------------------

const GROK_BLOB_MAX_BYTES: u64 = 2_000_000;

#[allow(dead_code)]
pub fn scan_grok_bot(paths: &AgentPaths, procs: &ProcessIndex, now: i64) -> Vec<AgentSession> {
    scan_grok_bot_cached(paths, procs, now, None)
}

fn scan_grok_bot_cached(
    paths: &AgentPaths,
    procs: &ProcessIndex,
    now: i64,
    mut cache: Option<&mut SourceCache>,
) -> Vec<AgentSession> {
    let running = procs.has_any(&["grok bot"]);
    let pid = procs.first_pid(&["grok bot"]);
    let mut out = Vec::new();

    // Lokal eksekverings-dæmon: inflightCount > 0 betyder en computer-use opgave kører nu.
    if let Ok(meta) = fs::metadata(&paths.grokbot_daemon) {
        if let Some(mtime_ms) = modified_ms(&meta) {
            if now - mtime_ms <= WORKING_TIMEOUT_MS {
                if let Ok(raw) = fs::read_to_string(&paths.grokbot_daemon) {
                    if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                        let inflight = val
                            .get("inflightCount")
                            .and_then(Value::as_i64)
                            .unwrap_or(0);
                        if inflight > 0 && running {
                            out.push(AgentSession {
                                id: "grok-bot:local-exec".to_string(),
                                agent: "grok-bot".to_string(),
                                agent_label: "Grok Bot".to_string(),
                                title: None,
                                project: None,
                                status: AgentStatus::Working,
                                last_active_ms: mtime_ms,
                                detail: Some(if inflight > 1 {
                                    format!("Udfører {inflight} lokale handlinger")
                                } else {
                                    "Udfører lokal handling".to_string()
                                }),
                                subagent_count: 0,
                                todo: None,
                                pid,
                                process_running: true,
                            });
                        }
                    }
                }
            }
        }
    }

    let Some(persist_dir) = paths.grokbot_persistence.as_deref() else {
        return out;
    };
    let dir_ok = match cache.as_deref_mut() {
        Some(c) => c.dir_exists(persist_dir, now),
        None => persist_dir.is_dir(),
    };
    if !dir_ok {
        return out;
    }

    for (path, mtime_ms, file_len) in
        collect_recent_files(persist_dir, &["blob"], 1, MAX_RECENT_MS, now, 10)
    {
        let fp = (mtime_ms, file_len);
        let age = now - mtime_ms;
        let allow_working = age <= WORKING_TIMEOUT_MS && (running || age <= SHORT_ACTIVE_MS);

        if let Some(cached) = cache.as_deref().and_then(|c| c.get_file(&path, fp)).cloned() {
            let mut session = cached.session;
            if cached.raw_working_status == Some(AgentStatus::Working) && !allow_working {
                session.status = AgentStatus::Idle;
                session.detail = None;
            }
            session.pid = pid;
            session.process_running = running;
            out.push(session);
            continue;
        }

        let Some(parsed) = parse_grok_blob(&path, file_len) else {
            continue;
        };

        let (raw_status, raw_detail) = match parsed.turn {
            GrokTurn::InFlight => (AgentStatus::Working, Some("Arbejder…".to_string())),
            GrokTurn::Thinking => (AgentStatus::Working, Some("Tænker…".to_string())),
            GrokTurn::Done => (AgentStatus::Ready, Some("Klar til svar".to_string())),
        };
        let (status, detail) = if raw_status == AgentStatus::Working && !allow_working {
            (AgentStatus::Idle, None)
        } else {
            (raw_status, raw_detail.clone())
        };

        let stem = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let session = AgentSession {
            id: format!("grok-bot:{stem}"),
            agent: "grok-bot".to_string(),
            agent_label: "Grok Bot".to_string(),
            title: parsed.bot_name,
            project: None,
            status,
            last_active_ms: parsed.last_active_ms,
            detail,
            subagent_count: 0,
            todo: None,
            pid,
            process_running: running,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrokTurn {
    /// Sidste hændelse er en afsendt besked/vedhæftning eller streamer stadig.
    InFlight,
    /// Sidste besked er fra brugeren — botten tænker formentlig.
    Thinking,
    /// Sidste besked er et færdigt svar fra botten.
    Done,
}

struct GrokTail {
    last_active_ms: i64,
    turn: GrokTurn,
    bot_name: Option<String>,
}

fn parse_grok_blob(path: &Path, file_len: u64) -> Option<GrokTail> {
    if file_len == 0 || file_len > GROK_BLOB_MAX_BYTES {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    let entries = val.pointer("/value/entries")?.as_array()?;
    if entries.is_empty() {
        return None;
    }

    let last_ts = entries
        .iter()
        .rev()
        .take(8)
        .filter_map(|e| e.get("timestampMs").and_then(Value::as_i64))
        .max()
        .or_else(|| {
            val.pointer("/value/persistedAt")
                .and_then(Value::as_i64)
        })
        .unwrap_or(0);
    if last_ts <= 0 {
        return None;
    }

    let mut turn = GrokTurn::Done;
    for e in entries.iter().rev().take(8) {
        match e.get("kind").and_then(Value::as_str).unwrap_or("") {
            "message" => {
                let streaming = e.get("isStreaming").and_then(Value::as_bool) == Some(true);
                let role = e.get("role").and_then(Value::as_str).unwrap_or("");
                turn = if streaming {
                    GrokTurn::InFlight
                } else if role == "user" {
                    GrokTurn::Thinking
                } else {
                    GrokTurn::Done
                };
                break;
            }
            "send-message" | "user-attachment" => {
                turn = GrokTurn::InFlight;
                break;
            }
            _ => {}
        }
    }

    let mut bot_name = None;
    'outer: for e in entries.iter().rev().take(16) {
        for key in ["toAgent", "fromAgent", "author"] {
            if let Some(name) = e
                .get(key)
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
            {
                bot_name = Some(name.to_string());
                break 'outer;
            }
        }
    }

    Some(GrokTail {
        last_active_ms: last_ts,
        turn,
        bot_name,
    })
}

// ---------------------------------------------------------------------------
