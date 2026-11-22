//! Proces-indeks og vinduesfokus for AI-agenter (ingen falske sessioner).

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::Path,
};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows::Win32::{
    Foundation::{HWND, LPARAM},
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowLongPtrW, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow,
        ShowWindow, GWL_EXSTYLE, SW_RESTORE, WS_EX_TOOLWINDOW,
    },
};
use windows_core::BOOL;

/// Indekserer kørende eksekverbare og (kun for CLI-agenten `aider`) aktive proces-sessions.
pub struct ProcessIndex {
    exe_names: HashSet<String>,
    pids_by_exe: HashMap<String, Vec<u32>>,
    pid_to_exe: HashMap<u32, String>,
    pid_to_parent: HashMap<u32, u32>,
    aider_sessions: Vec<(u32, Option<String>, i64)>,
}

impl Default for ProcessIndex {
    fn default() -> Self {
        Self::empty()
    }
}

impl ProcessIndex {
    pub fn empty() -> Self {
        Self {
            exe_names: HashSet::new(),
            pids_by_exe: HashMap::new(),
            pid_to_exe: HashMap::new(),
            pid_to_parent: HashMap::new(),
            aider_sessions: Vec::new(),
        }
    }

    pub fn scan(sys: &mut System) -> Self {
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());

        let mut exe_names = HashSet::new();
        let mut pids_by_exe: HashMap<String, Vec<u32>> = HashMap::new();
        let mut pid_to_exe = HashMap::new();
        let mut pid_to_parent = HashMap::new();
        let mut aider_pids = Vec::new();

        for (&pid, proc_) in sys.processes() {
            let pid_u32 = pid.as_u32();
            if let Some(parent) = proc_.parent() {
                pid_to_parent.insert(pid_u32, parent.as_u32());
            }
            let Some(name) = proc_.name().to_str() else {
                continue;
            };
            let norm = normalize_exe_name(name);
            if norm.is_empty() {
                continue;
            }
            if norm == "aider" {
                aider_pids.push(pid);
            }
            exe_names.insert(norm.clone());
            pids_by_exe.entry(norm.clone()).or_default().push(pid_u32);
            pid_to_exe.insert(pid_u32, norm);
        }

        let mut aider_sessions = Vec::new();
        if !aider_pids.is_empty() {
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&aider_pids),
                false,
                ProcessRefreshKind::nothing()
                    .with_cwd(UpdateKind::OnlyIfNotSet)
                    .with_cmd(UpdateKind::OnlyIfNotSet),
            );
            for pid in aider_pids {
                if let Some(proc_) = sys.process(pid) {
                    let project = proc_
                        .cwd()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .or_else(|| project_from_cmd(proc_.cmd()));
                    let started_ms = (proc_.start_time() as i64) * 1000;
                    aider_sessions.push((pid.as_u32(), project, started_ms));
                }
            }
        }

        Self {
            exe_names,
            pids_by_exe,
            pid_to_exe,
            pid_to_parent,
            aider_sessions,
        }
    }

    pub fn has_any(&self, exes: &[&str]) -> bool {
        exes.iter().any(|exe| self.exe_names.contains(*exe))
    }

    pub fn first_pid(&self, exes: &[&str]) -> Option<u32> {
        for exe in exes {
            if let Some(pids) = self.pids_by_exe.get(*exe) {
                if let Some(&pid) = pids.first() {
                    return Some(pid);
                }
            }
        }
        None
    }

    pub fn aider_sessions(&self) -> &[(u32, Option<String>, i64)] {
        &self.aider_sessions
    }

    pub fn exe_for_pid(&self, pid: u32) -> Option<&str> {
        self.pid_to_exe.get(&pid).map(String::as_str)
    }

    pub fn ancestors(&self, mut pid: u32) -> HashSet<u32> {
        let mut out = HashSet::new();
        for _ in 0..10 {
            out.insert(pid);
            let Some(&parent) = self.pid_to_parent.get(&pid) else {
                break;
            };
            if parent == 0 || !out.insert(parent) {
                break;
            }
            pid = parent;
        }
        out
    }

    /// Test-hjælper så unit-tests kan opbygge et syntetisk procesbillede.
    #[cfg(test)]
    pub fn from_exes(exes: &[&str]) -> Self {
        let mut idx = Self::empty();
        for (i, exe) in exes.iter().enumerate() {
            let norm = normalize_exe_name(exe);
            let pid = 10_000 + i as u32;
            idx.exe_names.insert(norm.clone());
            idx.pids_by_exe.entry(norm.clone()).or_default().push(pid);
            idx.pid_to_exe.insert(pid, norm);
        }
        idx
    }
}

fn normalize_exe_name(name: &str) -> String {
    name.trim()
        .strip_suffix(".exe")
        .or_else(|| name.trim().strip_suffix(".EXE"))
        .unwrap_or(name.trim())
        .to_lowercase()
}

fn project_from_cmd(cmd: &[OsString]) -> Option<String> {
    for arg in cmd.iter().skip(1) {
        let s = arg.to_string_lossy();
        if s.starts_with('-') {
            continue;
        }
        let p = Path::new(s.as_ref());
        if p.exists() {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn target_exes_for_agent(agent: &str) -> (&'static [&'static str], &str) {
    match agent {
        "antigravity" => (&["antigravity"], "antigravity"),
        "devin" => (&["devin"], "devin"),
        "cursor" => (&["cursor"], "cursor"),
        "windsurf" => (&["windsurf"], "windsurf"),
        "claude-code" | "codex" | "gemini" | "opencode" | "aider" => (
            &[
                "windowsterminal",
                "wt",
                "wezterm-gui",
                "alacritty",
                "mintty",
                "code",
                "cursor",
                "antigravity",
                "devin",
                "powershell",
                "pwsh",
                "cmd",
            ],
            match agent {
                "claude-code" => "claude",
                "codex" => "codex",
                "gemini" => "gemini",
                "opencode" => "opencode",
                _ => "aider",
            },
        ),
        "cline" | "kilo" => (
            &["code", "code - insiders", "vscodium", "cursor", "windsurf", "antigravity"],
            agent,
        ),
        "qoder" => (&["qoder"], "qoder"),
        "freebuff" => (&["freebuff"], "freebuff"),
        _ => (&[], agent),
    }
}

struct WindowCandidate {
    hwnd: HWND,
    pid: u32,
    title: String,
}

unsafe extern "system" fn collect_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<WindowCandidate>);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
        return BOOL(1);
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return BOOL(1);
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let read = GetWindowTextW(hwnd, &mut buf);
    if read <= 0 {
        return BOOL(1);
    }
    let title = String::from_utf16_lossy(&buf[..read as usize]);
    if title.trim().is_empty() {
        return BOOL(1);
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    list.push(WindowCandidate { hwnd, pid, title });
    BOOL(1)
}

/// Finder og bringer agentens mest relevante synlige desktop-vindue i forgrunden.
pub fn focus_agent_window(agent: &str, pid: Option<u32>, project: Option<&str>) -> bool {
    let Ok(mut sys) = System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
    ) else {
        return false;
    };
    let index = ProcessIndex::scan(&mut sys);
    let ancestors = pid.map(|p| index.ancestors(p));

    let mut windows: Vec<WindowCandidate> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect_window_proc),
            LPARAM(&mut windows as *mut Vec<WindowCandidate> as isize),
        );
    }

    let (expected_exes, agent_kw) = target_exes_for_agent(agent);
    let project_lc = project
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let label_lc = agent_kw.to_lowercase();

    let mut best: Option<(i32, HWND)> = None;
    for win in windows {
        let win_exe = index.exe_for_pid(win.pid);
        let exe_match = win_exe
            .map(|exe| expected_exes.contains(&exe))
            .unwrap_or(false);
        let pid_match = pid == Some(win.pid);
        let ancestor_match = ancestors
            .as_ref()
            .map(|set| set.contains(&win.pid))
            .unwrap_or(false);
        let title_lc = win.title.to_lowercase();
        let proj_match = project_lc
            .as_deref()
            .map(|p| title_lc.contains(p))
            .unwrap_or(false);
        let agent_match = title_lc.contains(&label_lc);

        if !exe_match && !pid_match && !ancestor_match {
            continue;
        }

        let mut score = 0i32;
        if pid_match {
            score += 100;
        } else if ancestor_match {
            score += 80;
        }
        if exe_match {
            score += 50;
        }
        if proj_match {
            score += 35;
        }
        if agent_match {
            score += 15;
        }

        if best.map(|(s, _)| score > s).unwrap_or(true) {
            best = Some((score, win.hwnd));
        }
    }

    let Some((_, hwnd)) = best else {
        return false;
    };

    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }

        // Hvis widgetten ikke ejer forgrundslåsen, kobl midlertidigt til aktiv tråd.
        let fg = GetForegroundWindow();
        let fg_thread = if !fg.0.is_null() {
            GetWindowThreadProcessId(fg, None)
        } else {
            0
        };
        let cur_thread = GetCurrentThreadId();
        if fg_thread != 0 && fg_thread != cur_thread {
            let _ = AttachThreadInput(cur_thread, fg_thread, true);
            let ok = SetForegroundWindow(hwnd).as_bool();
            let _ = AttachThreadInput(cur_thread, fg_thread, false);
            return ok;
        }
        false
    }
}
