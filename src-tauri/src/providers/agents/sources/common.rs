use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde_json::Value;

use crate::providers::agents::AgentTodoProgress;

/// Sessioner ældre end 1 time returneres ikke (UI filtrerer videre efter brugervalg).
pub const MAX_RECENT_MS: i64 = 60 * 60 * 1000;
/// Timeout for at nedgradere stale "arbejder"-signaler uden friske opdateringer.
pub const WORKING_TIMEOUT_MS: i64 = 3 * 60 * 1000;
/// Korte fil-læsninger når processen ikke kan bekræftes (f.eks. hurtig CLI-kørsel).
pub(super) const SHORT_ACTIVE_MS: i64 = 45 * 1000;
/// Subagenter betragtes som aktive i dette vindue.
pub(super) const SUBAGENT_ACTIVE_MS: i64 = 90 * 1000;
pub(super) const HEAD_BYTES: usize = 8 * 1024;
pub(super) const TAIL_BYTES: usize = 16 * 1024;
pub(super) const TODO_TAIL_BYTES: usize = 64 * 1024;

pub(super) fn modified_ms(meta: &fs::Metadata) -> Option<i64> {
    meta.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

pub(super) fn collect_recent_files(
    root: &Path,
    extensions: &[&str],
    max_depth: usize,
    max_age_ms: i64,
    now: i64,
    limit: usize,
) -> Vec<(PathBuf, i64, u64)> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth {
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
            if !extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                continue;
            }
            let Some(mtime_ms) = modified_ms(&meta) else {
                continue;
            };
            if now - mtime_ms <= max_age_ms {
                found.push((path, mtime_ms, meta.len()));
            }
        }
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.1));
    found.truncate(limit);
    found
}

pub(super) fn read_head(path: &Path, max_bytes: usize) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(max_bytes.min(HEAD_BYTES));
    file.take(max_bytes as u64).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).to_string())
}

pub(super) fn read_tail(path: &Path, max_bytes: usize) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes as u64);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = vec![0u8; (len - start) as usize];
    file.read_exact(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    if start > 0 {
        if let Some(pos) = text.find('\n') {
            return Some(text[pos + 1..].to_string());
        }
    }
    Some(text.to_string())
}

pub(super) fn read_tail_lines(path: &Path, max_bytes: usize) -> Option<Vec<String>> {
    let tail = read_tail(path, max_bytes)?;
    let lines: Vec<String> = tail
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .map(str::to_string)
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

pub(super) fn extract_cwd_from_value(val: &Value) -> Option<String> {
    for container in [Some(val), val.get("payload")].into_iter().flatten() {
        for key in ["cwd", "workdir", "workspace", "current_dir"] {
            if let Some(dir) = container.get(key).and_then(Value::as_str) {
                if !dir.is_empty() {
                    return Some(dir.to_string());
                }
            }
        }
    }
    None
}

pub(super) fn extract_cwd_from_text(head: &str) -> Option<String> {
    if let Ok(val) = serde_json::from_str::<Value>(head) {
        if let Some(d) = extract_cwd_from_value(&val) {
            return Some(d);
        }
    }
    for line in head.lines().take(24) {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            if let Some(d) = extract_cwd_from_value(&val) {
                return Some(d);
            }
        }
    }
    None
}

pub(super) fn non_empty_trimmed(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub(super) fn parse_todo_array(items: &[Value]) -> Option<AgentTodoProgress> {
    if items.is_empty() {
        return None;
    }
    let total = items.len() as u32;
    let mut done = 0u32;
    let mut current: Option<String> = None;

    for item in items {
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if matches!(status.as_str(), "completed" | "done") {
            done += 1;
        } else if status == "in_progress" && current.is_none() {
            let text = item
                .get("activeForm")
                .or_else(|| item.get("content"))
                .or_else(|| item.get("step"))
                .and_then(Value::as_str)
                .unwrap_or("");
            current = short_title_len(text, 72);
        }
    }

    Some(AgentTodoProgress {
        done,
        total,
        current,
    })
}

pub fn parse_markdown_todos(md: &str) -> Option<AgentTodoProgress> {
    let mut total = 0u32;
    let mut done = 0u32;
    let mut current: Option<String> = None;

    for line in md.lines() {
        let trimmed = line.trim_start();
        let after_bullet = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "));
        let Some(rest) = after_bullet else {
            continue;
        };
        if rest.starts_with("[x]") || rest.starts_with("[X]") {
            total += 1;
            done += 1;
        } else if let Some(body) = rest.strip_prefix("[/]") {
            total += 1;
            if current.is_none() {
                let without_comment = body.split("<!--").next().unwrap_or(body);
                let cleaned = without_comment.replace("**", "").replace('`', "");
                current = short_title_len(&cleaned, 72);
            }
        } else if rest.starts_with("[ ]") {
            total += 1;
        }
    }

    (total > 0).then_some(AgentTodoProgress {
        done,
        total,
        current,
    })
}

pub(super) fn short_title(raw: &str) -> Option<String> {
    short_title_len(raw, 64)
}

pub(super) fn short_title_len(raw: &str, max_chars: usize) -> Option<String> {
    let first_line = raw.lines().find(|l| !l.trim().is_empty())?.trim();
    if first_line.starts_with('<') || first_line.is_empty() {
        return None;
    }
    let mut out: String = first_line.chars().take(max_chars).collect();
    if first_line.chars().count() > max_chars {
        out.push('…');
    }
    Some(out)
}

pub(super) fn normalize_epoch_ms(ts: i64) -> i64 {
    if ts > 0 && ts < 10_000_000_000 {
        ts * 1000
    } else {
        ts
    }
}

pub(super) fn float_epoch_to_ms(ts: f64) -> i64 {
    if !ts.is_finite() || ts <= 0.0 {
        return 0;
    }
    if ts < 10_000_000_000.0 {
        (ts * 1000.0).round() as i64
    } else {
        ts.round() as i64
    }
}

pub fn project_from_path_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let seg = trimmed
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())?;
    if seg.ends_with(':') {
        return None;
    }
    Some(seg.to_string())
}

pub fn project_from_uri(uri: &str) -> Option<String> {
    let decoded = percent_decode_str(uri.trim());
    let path = decoded
        .strip_prefix("file:///")
        .or_else(|| decoded.strip_prefix("file://"))
        .unwrap_or(&decoded);
    project_from_path_str(path)
}

pub(super) fn percent_decode_str(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub(super) fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + (b - b'a')),
        b'A'..=b'F' => Some(10 + (b - b'A')),
        _ => None,
    }
}

pub(super) fn humanize_slug(slug: &str) -> String {
    let parts: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => slug.to_string(),
        1 => parts[0].to_string(),
        n => parts[n - 2..].join("-"),
    }
}

/// Parser RFC3339 / SQLite-datotidsstempel og epoch-strenge til epoch millisekunder.
pub fn parse_timestamp_ms(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(num) = s.parse::<i64>() {
        return Some(normalize_epoch_ms(num));
    }
    if let Ok(num) = s.parse::<f64>() {
        let ms = float_epoch_to_ms(num);
        return (ms > 0).then_some(ms);
    }

    if let Some(ms) = s
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|ts| ts.as_millisecond())
    {
        return Some(ms);
    }

    s.parse::<jiff::civil::DateTime>()
        .ok()
        .and_then(|dt| jiff::tz::Offset::UTC.to_timestamp(dt).ok())
        .map(|ts| ts.as_millisecond())
}
