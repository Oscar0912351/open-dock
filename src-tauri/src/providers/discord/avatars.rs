//! Lokal disk-cache af Discords deltager-avatars under `$APPCACHE/discord-avatars/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_AVATAR_FILES: usize = 128;
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

fn sanitize_token(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

fn default_avatar_index(user_id: &str, discriminator: Option<&str>) -> u64 {
    let disc = discriminator.map(str::trim).unwrap_or("");
    if disc.is_empty() || disc == "0" {
        (user_id.parse::<u64>().unwrap_or(0) >> 22) % 6
    } else {
        disc.parse::<u64>().unwrap_or(0) % 5
    }
}

pub fn avatar_cdn_url(
    user_id: &str,
    avatar_hash: Option<&str>,
    discriminator: Option<&str>,
) -> String {
    let clean_uid = sanitize_token(user_id);
    if let Some(hash) = avatar_hash.map(str::trim).filter(|s| !s.is_empty()) {
        let clean_hash = sanitize_token(hash);
        format!("https://cdn.discordapp.com/avatars/{clean_uid}/{clean_hash}.png?size=128")
    } else {
        let index = default_avatar_index(user_id, discriminator);
        format!("https://cdn.discordapp.com/embed/avatars/{index}.png")
    }
}

pub fn avatar_cache_key(
    user_id: &str,
    avatar_hash: Option<&str>,
    discriminator: Option<&str>,
) -> String {
    if let Some(hash) = avatar_hash.map(str::trim).filter(|s| !s.is_empty()) {
        format!("{}_{}", sanitize_token(user_id), sanitize_token(hash))
    } else {
        let idx = default_avatar_index(user_id, discriminator);
        format!("{}_def{idx}", sanitize_token(user_id))
    }
}

fn avatar_file_path(
    cache_dir: &Path,
    user_id: &str,
    avatar_hash: Option<&str>,
    discriminator: Option<&str>,
) -> PathBuf {
    let key = avatar_cache_key(user_id, avatar_hash, discriminator);
    cache_dir.join(format!("{key}.png"))
}

pub fn cached_avatar_path(
    cache_dir: &Path,
    user_id: &str,
    avatar_hash: Option<&str>,
    discriminator: Option<&str>,
) -> Option<String> {
    let path = avatar_file_path(cache_dir, user_id, avatar_hash, discriminator);
    let meta = fs::metadata(&path).ok()?;
    if meta.len() > 0 {
        Some(path.to_string_lossy().into_owned())
    } else {
        None
    }
}

pub async fn fetch_and_cache_avatar(
    cache_dir: PathBuf,
    user_id: String,
    avatar_hash: Option<String>,
    discriminator: Option<String>,
) -> Option<String> {
    if let Some(existing) = cached_avatar_path(
        &cache_dir,
        &user_id,
        avatar_hash.as_deref(),
        discriminator.as_deref(),
    ) {
        return Some(existing);
    }

    let url = avatar_cdn_url(
        &user_id,
        avatar_hash.as_deref(),
        discriminator.as_deref(),
    );
    let client = crate::util::http_client();
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    if resp.content_length().is_some_and(|len| len > MAX_AVATAR_BYTES as u64) {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
        return None;
    }

    fs::create_dir_all(&cache_dir).ok()?;
    let target = avatar_file_path(
        &cache_dir,
        &user_id,
        avatar_hash.as_deref(),
        discriminator.as_deref(),
    );
    let tmp = cache_dir.join(format!(
        "{}.tmp",
        avatar_cache_key(&user_id, avatar_hash.as_deref(), discriminator.as_deref())
    ));
    fs::write(&tmp, &bytes).ok()?;
    if fs::rename(&tmp, &target).is_err() {
        let _ = fs::remove_file(&target);
        if fs::rename(&tmp, &target).is_err() {
            let copy_ok = fs::copy(&tmp, &target).is_ok();
            let _ = fs::remove_file(&tmp);
            if !copy_ok {
                return None;
            }
        }
    }
    evict_old_avatars(&cache_dir, MAX_AVATAR_FILES);
    Some(target.to_string_lossy().into_owned())
}

fn evict_old_avatars(dir: &Path, max_files: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("png") {
                return None;
            }
            let modified = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            Some((path, modified))
        })
        .collect();
    if files.len() <= max_files {
        return;
    }
    files.sort_by_key(|(_, modified)| *modified);
    let remove_count = files.len() - max_files;
    for (path, _) in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}
