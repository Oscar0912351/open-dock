use std::collections::HashMap;

use windows::{
    core::PCWSTR,
    Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{
            FOLDERID_AppsFolder, IShellItem, SHCreateItemInKnownFolder, KNOWN_FOLDER_FLAG,
            SIGDN_NORMALDISPLAY,
        },
    },
};

fn match_known_source_name(aumid: &str) -> Option<&'static str> {
    let lower = aumid.to_ascii_lowercase();
    if lower.contains("spotify") {
        return Some("Spotify");
    }
    if lower.contains("msedge") || lower.contains("microsoftedge") {
        return Some("Microsoft Edge");
    }
    if lower.contains("chrome") {
        return Some("Google Chrome");
    }
    if lower.contains("firefox")
        || lower.contains("308046b0af4a39cb")
        || lower.contains("e7cf176e110c211b")
        || lower.contains("f0dc299d809b9700")
        || lower.contains("6f193ccc56814779")
    {
        return Some("Firefox");
    }
    if lower.contains("vlc") {
        return Some("VLC");
    }
    if lower.contains("wmplayer") || lower.contains("zune") || lower.contains("mediaplayer") {
        return Some("Medieafspiller");
    }
    None
}

fn resolve_aumid_display_name(aumid: &str) -> Option<String> {
    let trimmed = aumid.trim();
    if trimmed.is_empty() {
        return None;
    }
    let wide: Vec<u16> = trimmed.encode_utf16().chain(std::iter::once(0)).collect();
    let item: IShellItem = unsafe {
        SHCreateItemInKnownFolder(
            &FOLDERID_AppsFolder,
            KNOWN_FOLDER_FLAG(0),
            PCWSTR(wide.as_ptr()),
        )
        .ok()?
    };
    let pwstr = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()? };
    let text = unsafe { pwstr.to_string().ok() };
    unsafe {
        CoTaskMemFree(Some(pwstr.0 as *const _));
    }
    text.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub(super) fn resolve_source_name(aumid: &str, cache: &mut HashMap<String, String>) -> String {
    let trimmed = aumid.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(cached) = cache.get(trimmed) {
        return cached.clone();
    }
    let resolved = if let Some(known) = match_known_source_name(trimmed) {
        known.to_string()
    } else if let Some(display_name) = resolve_aumid_display_name(trimmed) {
        display_name
    } else {
        format_source_name(trimmed)
    };
    cache.insert(trimmed.to_string(), resolved.clone());
    resolved
}

pub fn format_source_name(aumid: &str) -> String {
    if let Some(known) = match_known_source_name(aumid) {
        return known.into();
    }

    let base = aumid
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(aumid)
        .split('!')
        .next()
        .unwrap_or(aumid)
        .trim();
    let stripped = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base);
    let mut chars = stripped.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}
