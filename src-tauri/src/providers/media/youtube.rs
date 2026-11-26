use std::{
    collections::{HashMap, HashSet},
    sync::{mpsc::Sender, Arc},
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use super::Cmd;

const MAX_DURATION_CACHE_ENTRIES: usize = 128;
const NEGATIVE_DURATION_TTL: Duration = Duration::from_secs(120);

struct CachedDuration {
    duration_ms: Option<u64>,
    stored_at: Instant,
    last_used: u64,
}

#[derive(Default)]
pub(super) struct DurationLookupState {
    resolved: HashMap<String, CachedDuration>,
    in_flight: HashSet<String>,
    access_seq: u64,
}

pub(super) fn resolve_fallback_duration_ms(
    cache: &Arc<Mutex<DurationLookupState>>,
    tx: &Sender<Cmd>,
    title: &str,
    artist: &str,
) -> u64 {
    let cleaned_title = clean_media_title(title);
    if cleaned_title.chars().count() < 2 {
        return 0;
    }
    let cleaned_artist = artist.trim().to_string();
    let cache_key = format!("{}|{}", cleaned_title, cleaned_artist);

    {
        let mut guard = cache.lock();
        guard.access_seq = guard.access_seq.saturating_add(1);
        let access_seq = guard.access_seq;
        let mut expired_negative = false;
        if let Some(entry) = guard.resolved.get_mut(&cache_key) {
            if entry.duration_ms.is_none() && entry.stored_at.elapsed() >= NEGATIVE_DURATION_TTL {
                expired_negative = true;
            } else {
                entry.last_used = access_seq;
                return entry.duration_ms.unwrap_or(0);
            }
        }
        if expired_negative {
            guard.resolved.remove(&cache_key);
        }
        if guard.in_flight.contains(&cache_key) || guard.in_flight.len() >= 4 {
            return 0;
        }
        guard.in_flight.insert(cache_key.clone());
    }

    let cache_clone = Arc::clone(cache);
    let tx_clone = tx.clone();
    tauri::async_runtime::spawn(async move {
        let result = fetch_youtube_duration_ms(&cleaned_title, &cleaned_artist).await;
        {
            let mut guard = cache_clone.lock();
            guard.in_flight.remove(&cache_key);
            while guard.resolved.len() >= MAX_DURATION_CACHE_ENTRIES {
                let Some(oldest_key) = guard
                    .resolved
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                guard.resolved.remove(&oldest_key);
            }
            guard.access_seq = guard.access_seq.saturating_add(1);
            let access_seq = guard.access_seq;
            guard.resolved.insert(
                cache_key,
                CachedDuration {
                    duration_ms: result,
                    stored_at: Instant::now(),
                    last_used: access_seq,
                },
            );
        }
        if result.is_some() {
            let _ = tx_clone.send(Cmd::Refresh);
        }
    });

    0
}

async fn fetch_youtube_duration_ms(title: &str, artist: &str) -> Option<u64> {
    let query = if artist.trim().is_empty() {
        title.to_string()
    } else {
        format!("{} {}", title, artist.trim())
    };

    let client = crate::util::http_client();

    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": "2.20241120.01.00"
            }
        },
        "query": query
    });

    let resp = client
        .post("https://www.youtube.com/youtubei/v1/search?prettyPrint=false")
        .timeout(Duration::from_secs(4))
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) OpenDock/0.1",
        )
        .json(&payload)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    extract_matching_youtube_duration_ms(&json, title, artist)
}

pub fn clean_media_title(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix('(') {
        if let Some((digits, after)) = rest.split_once(')') {
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                s = after.trim_start();
            }
        }
    }
    for suffix in [
        " - YouTube Music",
        " — YouTube Music",
        " - YouTube",
        " — YouTube",
    ] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            s = stripped.trim_end();
            break;
        }
    }
    s.to_string()
}

fn normalize_for_match(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn parse_duration_text(text: &str) -> Option<u64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split([':', '.']).collect();
    let total_secs = match parts.as_slice() {
        [mm, ss] => {
            let m: u64 = mm.trim().parse().ok()?;
            let s: u64 = ss.trim().parse().ok()?;
            if s >= 60 {
                return None;
            }
            m.checked_mul(60)?.checked_add(s)?
        }
        [hh, mm, ss] => {
            let h: u64 = hh.trim().parse().ok()?;
            let m: u64 = mm.trim().parse().ok()?;
            let s: u64 = ss.trim().parse().ok()?;
            if m >= 60 || s >= 60 {
                return None;
            }
            h.checked_mul(3600)?
                .checked_add(m.checked_mul(60)?)?
                .checked_add(s)?
        }
        _ => return None,
    };
    (total_secs > 0).then(|| total_secs.saturating_mul(1000))
}

fn extract_yt_text(node: &serde_json::Value) -> String {
    if let Some(simple) = node.get("simpleText").and_then(|v| v.as_str()) {
        return simple.to_string();
    }
    if let Some(runs) = node.get("runs").and_then(|v| v.as_array()) {
        let mut out = String::new();
        for run in runs {
            if let Some(t) = run.get("text").and_then(|v| v.as_str()) {
                out.push_str(t);
            }
        }
        return out;
    }
    String::new()
}

fn collect_video_renderers<'a>(
    node: &'a serde_json::Value,
    out: &mut Vec<&'a serde_json::Value>,
) {
    if out.len() >= 15 {
        return;
    }
    match node {
        serde_json::Value::Object(map) => {
            if let Some(vr) = map.get("videoRenderer") {
                out.push(vr);
            }
            for val in map.values() {
                collect_video_renderers(val, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for val in arr {
                collect_video_renderers(val, out);
            }
        }
        _ => {}
    }
}

pub fn extract_matching_youtube_duration_ms(
    json: &serde_json::Value,
    target_title: &str,
    target_artist: &str,
) -> Option<u64> {
    let norm_title = normalize_for_match(&clean_media_title(target_title));
    if norm_title.is_empty() {
        return None;
    }
    let norm_artist = normalize_for_match(target_artist);

    let mut renderers = Vec::new();
    collect_video_renderers(json, &mut renderers);

    struct Candidate {
        title: String,
        author: String,
        duration_ms: u64,
    }

    let mut candidates = Vec::new();
    for vr in renderers {
        let Some(len_node) = vr.get("lengthText") else {
            continue;
        };
        let Some(duration_ms) = parse_duration_text(&extract_yt_text(len_node)) else {
            continue;
        };
        let title = vr
            .get("title")
            .map(extract_yt_text)
            .map(|t| normalize_for_match(&t))
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let author = vr
            .get("ownerText")
            .or_else(|| vr.get("longBylineText"))
            .or_else(|| vr.get("shortBylineText"))
            .map(extract_yt_text)
            .map(|a| normalize_for_match(&a))
            .unwrap_or_default();
        candidates.push(Candidate {
            title,
            author,
            duration_ms,
        });
    }

    let artist_matches = |c: &Candidate| -> bool {
        norm_artist.is_empty()
            || (!c.author.is_empty()
                && (c.author == norm_artist
                    || c.author.contains(&norm_artist)
                    || norm_artist.contains(&c.author)))
            || c.title.contains(&norm_artist)
    };

    // Pass 1: exact title + artist match.
    for c in &candidates {
        if c.title == norm_title && artist_matches(c) {
            return Some(c.duration_ms);
        }
    }
    // Pass 2: exact title match.
    for c in &candidates {
        if c.title == norm_title {
            return Some(c.duration_ms);
        }
    }
    // Pass 3: substring title match when artist also matches.
    for c in &candidates {
        if (c.title.contains(&norm_title) || norm_title.contains(&c.title)) && artist_matches(c) {
            return Some(c.duration_ms);
        }
    }

    None
}
