//! AI-agent-aktivitet for udviklere.
//!
//! Cloud-agenter (Devin i skyen, GitHub Copilot Chat osv.) kan ikke observeres
//! lokalt — vi detekterer kun hvad der rent faktisk kører på maskinen:
//!
//! - **Proces-scan** via sysinfo for kendte agent-eksekverbare (claude, codex,
//!   opencode, cursor, qoder, antigravity, ...).
//! - **Session-filer** for agenter der skriver lokal state: Claude Code
//!   (~/.claude/projects/*​/*.jsonl), Codex CLI (~/.codex/sessions/**​/*.jsonl),
//!   Gemini CLI (~/.gemini/tmp/**/chats/*.json), OpenCode
//!   (~/.local/share/opencode/**), og Cline/Kilo Code i editorens
//!   globalStorage (**/tasks/*​).
//!
//! Status er ærlig heuristik: "arbejder" når filen er ændret for nylig,
//! "inaktiv" når den er ældre men stadig åben/relevant. Der findes ingen reel
//! procent-fremskridt hos agenterne, så vi viser sidste handling i stedet.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tauri::{AppHandle, Emitter};
use ts_rs::TS;

const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Session-filer ældre end dette ignoreres helt (færdige/historiske chats).
const STALE_MS: i64 = 48 * 60 * 60 * 1000;
/// Ændret inden for dette => "arbejder".
const ACTIVE_MS: i64 = 20 * 1000;
/// Maks antal session-filer der parses per scan (pris på at læse haler).
const MAX_SESSION_FILES: usize = 24;
/// Antal bytes der læses bagfra til status-detektion.
const TAIL_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum AgentStatus {
    /// Session-filen er ændret for nylig — agenten arbejder (sandsynligvis).
    Working,
    /// Sessionen findes men er ikke rørt for nylig — afventer input eller idle.
    Idle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AgentSession {
    /// Stabil id (agent + filsti) så UI kan animere ændringer.
    pub id: String,
    /// Spec-id, f.eks. "claude-code".
    pub agent: String,
    /// Visningsnavn, f.eks. "Claude Code".
    pub agent_label: String,
    /// Projektnavn uddraget af stien, eller processens navn.
    pub project: String,
    pub status: AgentStatus,
    /// Epoch ms for filens sidste ændring (eller nu for proces-fund).
    pub last_active_ms: i64,
    /// Kort dansk beskrivelse af seneste handling når vi kan læse det.
    pub detail: Option<String>,
    /// Om selve processen også er fundet kørende.
    pub process_running: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AgentsSnapshot {
    pub sessions: Vec<AgentSession>,
    pub updated_at: i64,
}

/// Hvordan en spec finder sit projektnavn ud af en session-fils sti.
#[derive(Clone, Copy)]
enum ProjectFrom {
    /// <root>/<projekt-slug>/<fil> — brug mappe-navnet (Claude Code).
    ParentDir,
    /// <root>/.../<fil> hvor roden er flad — læs "cwd" fra filens indhold.
    CwdInFile,
    /// <root>/<task-id>/ — Cline/Kilo task-mappe; læs projekt fra task-filen.
    TaskDir,
}

/// Hvordan "sidste handling" læses ud af fil-halen.
#[derive(Clone, Copy)]
enum TailKind {
    /// Claude Code JSONL: {"type":"assistant|user|system", ...}
    ClaudeJsonl,
    /// Codex rollout JSONL: {"type":"...", "payload":{...}}
    CodexJsonl,
    /// Ingen detalje — kun mtime.
    None,
}

struct AgentSpec {
    id: &'static str,
    label: &'static str,
    /// Proces-navne (lowercase, uden .exe) der betragtes som denne agent.
    exes: &'static [&'static str],
    /// Filer under roden med disse endelser tæller som sessioner.
    extensions: &'static [&'static str],
    project_from: ProjectFrom,
    tail: TailKind,
}

struct AgentRoot {
    spec: &'static AgentSpec,
    /// Mappe relativt til hjemme-biblioteket (None = absolut/sti via env).
    rel: &'static str,
}

const SPECS: &[AgentSpec] = &[
    AgentSpec {
        id: "claude-code",
        label: "Claude Code",
        exes: &["claude", "claude-code"],
        extensions: &["jsonl"],
        project_from: ProjectFrom::ParentDir,
        tail: TailKind::ClaudeJsonl,
    },
    AgentSpec {
        id: "codex",
        label: "Codex",
        exes: &["codex", "codex-cli"],
        extensions: &["jsonl"],
        project_from: ProjectFrom::CwdInFile,
        tail: TailKind::CodexJsonl,
    },
    AgentSpec {
        id: "gemini",
        label: "Gemini CLI",
        exes: &["gemini"],
        extensions: &["json"],
        project_from: ProjectFrom::CwdInFile,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "opencode",
        label: "OpenCode",
        exes: &["opencode"],
        extensions: &["json", "jsonl"],
        project_from: ProjectFrom::CwdInFile,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "cline",
        label: "Cline",
        exes: &[],
        extensions: &["json"],
        project_from: ProjectFrom::TaskDir,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "kilo",
        label: "Kilo Code",
        exes: &[],
        extensions: &["json"],
        project_from: ProjectFrom::TaskDir,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "cursor",
        label: "Cursor",
        exes: &["cursor"],
        extensions: &[],
        project_from: ProjectFrom::ParentDir,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "windsurf",
        label: "Windsurf",
        exes: &["windsurf"],
        extensions: &[],
        project_from: ProjectFrom::ParentDir,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "qoder",
        label: "Qoder",
        exes: &["qoder"],
        extensions: &[],
        project_from: ProjectFrom::ParentDir,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "antigravity",
        label: "Antigravity",
        exes: &["antigravity"],
        extensions: &[],
        project_from: ProjectFrom::ParentDir,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "aider",
        label: "Aider",
        exes: &["aider"],
        extensions: &[],
        project_from: ProjectFrom::ParentDir,
        tail: TailKind::None,
    },
    AgentSpec {
        id: "freebuff",
        label: "FreeBuff",
        exes: &["freebuff"],
        extensions: &[],
        project_from: ProjectFrom::ParentDir,
        tail: TailKind::None,
    },
];

/// Relative rødder under hjemmebiblioteket der scannes for session-filer.
fn agent_roots() -> Vec<AgentRoot> {
    let spec = |id| SPECS.iter().find(|s| s.id == id).unwrap();
    vec![
        AgentRoot { spec: spec("claude-code"), rel: ".claude/projects" },
        AgentRoot { spec: spec("codex"), rel: ".codex/sessions" },
        AgentRoot { spec: spec("gemini"), rel: ".gemini/tmp" },
        AgentRoot { spec: spec("opencode"), rel: ".local/share/opencode" },
        AgentRoot { spec: spec("opencode"), rel: ".config/opencode" },
    ]
}

/// Absolutte rødder for editor-baserede agenter (Cline/Kilo task-mapper).
fn task_roots(appdata: &Path) -> Vec<(AgentSpec, PathBuf)> {
    let spec = |id: &str| SPECS.iter().find(|s| s.id == id).unwrap().clone_spec();
    let editors = [
        "Code", "Code - Insiders", "VSCodium", "Cursor", "Windsurf", "Antigravity",
    ];
    let exts: &[(&str, &str)] = &[
        ("saoudrizwan.claude-dev", "cline"),
        ("kilocode.kilo-code", "kilo"),
        ("rooveterinaryinc.roo-cline", "cline"),
    ];
    let mut out = Vec::new();
    for editor in editors {
        for (ext_id, spec_id) in exts {
            let root = appdata
                .join(editor)
                .join("User/globalStorage")
                .join(ext_id)
                .join("tasks");
            if root.is_dir() {
                out.push((spec(spec_id), root));
            }
        }
    }
    out
}

impl AgentSpec {
    fn clone_spec(&self) -> AgentSpec {
        AgentSpec {
            id: self.id,
            label: self.label,
            exes: self.exes,
            extensions: self.extensions,
            project_from: self.project_from,
            tail: self.tail,
        }
    }
}

pub struct AgentsProvider {
    running: AtomicBool,
    worker: Mutex<Option<Worker>>,
    latest: Arc<Mutex<AgentsSnapshot>>,
}

struct Worker {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Default for AgentsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentsProvider {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            worker: Mutex::new(None),
            latest: Arc::new(Mutex::new(AgentsSnapshot {
                sessions: Vec::new(),
                updated_at: 0,
            })),
        }
    }

    pub fn snapshot(&self) -> AgentsSnapshot {
        self.latest.lock().clone()
    }

    pub fn start(&self, app: &AppHandle) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        log::info!(target: "provider:agents", "Starter agents-provider (proces + session-filer)");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let latest = Arc::clone(&self.latest);
        let app_handle = app.clone();
        let join = std::thread::Builder::new()
            .name("od-agents".into())
            .spawn(move || run_agents_thread(app_handle, stop_thread, latest))
            .ok();
        if join.is_none() {
            log::warn!(target: "provider:agents", "Kunne ikke starte od-agents-tråd");
            self.running.store(false, Ordering::Release);
            return;
        }
        *self.worker.lock() = Some(Worker { stop, join });
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::AcqRel) {
            return;
        }
        log::info!(target: "provider:agents", "Stopper agents-provider");
        if let Some(mut worker) = self.worker.lock().take() {
            worker.stop.store(true, Ordering::Release);
            if let Some(join) = worker.join.take() {
                let _ = join.join();
            }
        }
    }
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn run_agents_thread(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<AgentsSnapshot>>,
) {
    let home = dirs_home();
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let mut sys = match System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
    ) {
        Ok(sys) => sys,
        Err(err) => {
            log::warn!(target: "provider:agents", "Kunne ikke oprette proces-skanner: {err}");
            return;
        }
    };
    let mut last_emit: Option<AgentsSnapshot> = None;

    while !stop.load(Ordering::Acquire) {
        let now = unix_now_ms();
        let running_exes = scan_processes(&mut sys);
        let sessions = scan_sessions(&home, appdata.as_deref(), &running_exes, now);
        let snapshot = AgentsSnapshot {
            sessions,
            updated_at: now,
        };
        *latest.lock() = snapshot.clone();
        if last_emit.as_ref() != Some(&snapshot) {
            last_emit = Some(snapshot.clone());
            if let Err(err) = app.emit("provider://agents", &snapshot) {
                log::warn!(target: "provider:agents", "Kunne ikke emitte provider://agents: {err}");
            }
        }
        let mut slept = Duration::ZERO;
        while slept < POLL_INTERVAL && !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(100));
            slept += Duration::from_millis(100);
        }
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/"))
}

/// Returnerer mængden af proces-navne (lowercase, uden .exe) der kører.
fn scan_processes(sys: &mut System) -> HashSet<String> {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.processes()
        .values()
        .filter_map(|p| p.name().to_str().map(str::to_owned))
        .map(|name| {
            name.trim_end_matches(".exe")
                .trim_end_matches(".EXE")
                .to_lowercase()
        })
        .collect()
}

fn scan_sessions(
    home: &Path,
    appdata: Option<&Path>,
    running_exes: &HashSet<String>,
    now: i64,
) -> Vec<AgentSession> {
    let mut sessions: Vec<AgentSession> = Vec::new();

    // 1) Fil-baserede sessioner under hjemmerødder.
    let mut roots: Vec<(AgentSpec, PathBuf)> = Vec::new();
    for root in agent_roots() {
        let dir = home.join(root.rel);
        if dir.is_dir() {
            roots.push((root.spec.clone_spec(), dir));
        }
    }
    if let Some(appdata) = appdata {
        roots.extend(task_roots(appdata));
    }

    let mut scanned_files = 0usize;
    for (spec, dir) in roots {
        if scanned_files >= MAX_SESSION_FILES {
            break;
        }
        collect_session_files(&spec, &dir, now, &mut scanned_files, &mut sessions);
    }

    // 2) Proces-fund for agenter uden session-filer (eller supplerende).
    for spec in SPECS {
        let running = spec
            .exes
            .iter()
            .any(|exe| running_exes.contains(*exe));
        for session in sessions.iter_mut().filter(|s| s.agent == spec.id) {
            if running {
                session.process_running = true;
            }
        }
        if running && !sessions.iter().any(|s| s.agent == spec.id) {
            sessions.push(AgentSession {
                id: format!("{}:proc", spec.id),
                agent: spec.id.to_string(),
                agent_label: spec.label.to_string(),
                project: spec.label.to_string(),
                status: AgentStatus::Idle,
                last_active_ms: now,
                detail: None,
                process_running: true,
            });
        }
    }

    sessions.sort_by(|a, b| {
        b.status_rank()
            .cmp(&a.status_rank())
            .then(b.last_active_ms.cmp(&a.last_active_ms))
    });
    sessions
}

impl AgentSession {
    fn status_rank(&self) -> u8 {
        match self.status {
            AgentStatus::Working => 1,
            AgentStatus::Idle => 0,
        }
    }
}

/// Find session-filer under en rod (maks dybde 6), nyeste pr. "projekt-mappe".
fn collect_session_files(
    spec: &AgentSpec,
    root: &Path,
    now: i64,
    budget: &mut usize,
    out: &mut Vec<AgentSession>,
) {
    // (mappe, nyeste fil, mtime) — én række pr. mappe = én chat.
    let mut newest_per_dir: Vec<(PathBuf, PathBuf, i64)> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 6 {
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
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !spec.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                continue;
            }
            let Ok(modified) = meta.modified() else {
                continue;
            };
            let mtime_ms = modified
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if now - mtime_ms > STALE_MS {
                continue;
            }
            // Nyeste fil i mappen vinder — det er "chatten".
            let dir_key = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            if let Some(slot) = newest_per_dir.iter_mut().find(|(d, _, _)| *d == dir_key) {
                if mtime_ms > slot.2 {
                    slot.1 = path;
                    slot.2 = mtime_ms;
                }
            } else {
                newest_per_dir.push((dir_key, path, mtime_ms));
            }
        }
    }

    newest_per_dir.sort_by(|a, b| b.2.cmp(&a.2));
    for (_, path, mtime_ms) in newest_per_dir {
        if *budget >= MAX_SESSION_FILES {
            break;
        }
        *budget += 1;
        let status = if now - mtime_ms <= ACTIVE_MS {
            AgentStatus::Working
        } else {
            AgentStatus::Idle
        };
        let project = extract_project(spec, &path, root);
        let detail = tail_detail(spec, &path);
        out.push(AgentSession {
            id: format!("{}:{}", spec.id, path.display()),
            agent: spec.id.to_string(),
            agent_label: spec.label.to_string(),
            project,
            status,
            last_active_ms: mtime_ms,
            detail,
            process_running: false,
        });
    }
}

fn extract_project(spec: &AgentSpec, path: &Path, root: &Path) -> String {
    match spec.project_from {
        ProjectFrom::ParentDir => {
            // .claude/projects/<slug>/<fil>.jsonl — slug er stien med - som separator.
            let dir = path.parent().unwrap_or(root);
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| spec.label.to_string());
            humanize_slug(&name)
        }
        ProjectFrom::TaskDir => {
            // tasks/<id>/ — task-mappenavnet er ikke sigende; brug rodens
            // editor-mappe eller filnavnet som fallback.
            root.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| spec.label.to_string())
        }
        ProjectFrom::CwdInFile => {
            // Læs "cwd" fra filens første linjer (Codex/Gemini/OpenCode).
            if let Some(cwd) = read_cwd(path) {
                Path::new(&cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or(cwd)
            } else {
                humanize_slug(
                    &path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
            }
        }
    }
}

/// "C--Users-oscar-open-dock" -> "open-dock" (sidste 1-2 segmenter).
fn humanize_slug(slug: &str) -> String {
    let parts: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => slug.to_string(),
        1 => parts[0].to_string(),
        n => parts[n - 2..].join("-"),
    }
}

fn read_cwd(path: &Path) -> Option<String> {
    // JSONL-filer: tjek hver linje som JSON og led efter cwd/workdir-felter
    // på top-niveau eller i "payload" (Codex turn_context).
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines().take(40) {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        for container in [Some(&value), value.get("payload")].into_iter().flatten() {
            for key in ["cwd", "workdir", "workspace", "current_dir"] {
                if let Some(dir) = container.get(key).and_then(|v| v.as_str()) {
                    if !dir.is_empty() {
                        return Some(dir.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Læs fil-halen og udled en kort status-tekst.
fn tail_detail(spec: &AgentSpec, path: &Path) -> Option<String> {
    match spec.tail {
        TailKind::None => None,
        TailKind::ClaudeJsonl | TailKind::CodexJsonl => {
            let tail = read_tail(path, TAIL_BYTES)?;
            for line in tail.lines().rev().take(12) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                    continue;
                };
                if let Some(detail) = classify_line(spec.tail, &value) {
                    return Some(detail);
                }
            }
            None
        }
    }
}

fn classify_line(kind: TailKind, value: &serde_json::Value) -> Option<String> {
    match kind {
        TailKind::ClaudeJsonl => {
            let ty = value.get("type")?.as_str()?;
            match ty {
                "assistant" => {
                    let has_tool = value
                        .pointer("/message/content")
                        .and_then(|c| c.as_array())
                        .map(|items| {
                            items.iter().any(|i| {
                                i.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                            })
                        })
                        .unwrap_or(false);
                    Some(if has_tool {
                        "Bruger værktøj".to_string()
                    } else {
                        "Skriver svar".to_string()
                    })
                }
                "user" => Some("Afventer bruger".to_string()),
                _ => None,
            }
        }
        TailKind::CodexJsonl => {
            let ty = value.get("type")?.as_str()?;
            match ty {
                "response_item" | "function_call" | "function_call_output" => {
                    Some("Kører kommando".to_string())
                }
                "turn_context" | "user_message" => Some("Afventer bruger".to_string()),
                _ => None,
            }
        }
        TailKind::None => None,
    }
}

fn read_tail(path: &Path, bytes: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(bytes as u64);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = vec![0u8; (len - start) as usize];
    file.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).to_string())
}
