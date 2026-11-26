use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use image::{imageops::FilterType, RgbImage};
use windows::Storage::Streams::{DataReader, InputStreamOptions};

use super::color::extract_ambient_color;

const MAX_ART_FILES: usize = 60;
const CONTENT_ART_PREFIX: &str = "c-";

pub(super) fn resolve_artwork(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
    title: &str,
    artist: &str,
    art_dir: &Path,
    color_cache: &mut HashMap<u64, Option<[u8; 3]>>,
) -> (Option<String>, Option<[u8; 3]>) {
    if title.is_empty() && artist.is_empty() {
        return (None, None);
    }

    let Some(bytes) = read_thumbnail_bytes(props) else {
        return (None, None);
    };

    resolve_artwork_from_bytes(&bytes, art_dir, color_cache)
}

fn read_thumbnail_bytes(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<Vec<u8>> {
    let thumb_ref = props.Thumbnail().ok()?;
    let stream = thumb_ref.OpenReadAsync().and_then(|op| op.join()).ok()?;
    let size_u64 = stream.Size().ok()?;
    if size_u64 == 0 || size_u64 > 16 * 1024 * 1024 {
        return None;
    }
    let size = size_u64 as u32;
    let input = stream.GetInputStreamAt(0).ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    let _ = reader.SetInputStreamOptions(InputStreamOptions::ReadAhead);
    let loaded = reader.LoadAsync(size).and_then(|op| op.join()).ok()?;
    if loaded == 0 {
        return None;
    }
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

/// Content-addressed artwork cache keyed by a hash of the raw thumbnail bytes
/// rather than `(title, artist, album)`. Browsers like Firefox/Chrome update
/// GSMTC Title/Artist immediately on track change before downloading the new
/// cover art, so metadata-keyed caches permanently poison track B with track
/// A's thumbnail. Hashing the bytes ensures the file path (and frontend URL)
/// changes the moment the new thumbnail arrives.
fn resolve_artwork_from_bytes(
    bytes: &[u8],
    art_dir: &Path,
    color_cache: &mut HashMap<u64, Option<[u8; 3]>>,
) -> (Option<String>, Option<[u8; 3]>) {
    if bytes.is_empty() {
        return (None, None);
    }

    let key = fnv1a_64_bytes(bytes);
    let file_path = art_dir.join(format!("{CONTENT_ART_PREFIX}{key:016x}.jpg"));
    if file_path.exists() {
        let color = if let Some(cached) = color_cache.get(&key) {
            *cached
        } else {
            let computed = image::open(&file_path)
                .ok()
                .and_then(|img| extract_ambient_color(&img.to_rgb8()));
            color_cache.insert(key, computed);
            computed
        };
        return (Some(file_path.to_string_lossy().into_owned()), color);
    }

    let Ok(decoded) = image::load_from_memory(bytes) else {
        return (None, None);
    };
    let cropped = crop_square_320(&decoded);
    let rgb = cropped.to_rgb8();
    let ambient = extract_ambient_color(&rgb);

    if let Err(err) = save_jpeg_q86(&rgb, &file_path) {
        log::warn!(target: "provider:media", "Kunne ikke gemme cover-cache: {err}");
        return (None, ambient);
    }
    evict_old_art(art_dir, MAX_ART_FILES);
    color_cache.insert(key, ambient);

    (Some(file_path.to_string_lossy().into_owned()), ambient)
}

pub(super) fn purge_legacy_art_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".jpg") && !name.starts_with(CONTENT_ART_PREFIX) {
            let _ = fs::remove_file(path);
        }
    }
}

fn crop_square_320(img: &image::DynamicImage) -> image::DynamicImage {
    let w = img.width();
    let h = img.height();
    let side = w.min(h).max(1);
    let x = (w - side) / 2;
    let y = (h - side) / 2;
    img.crop_imm(x, y, side, side)
        .resize_exact(320, 320, FilterType::Lanczos3)
}

fn save_jpeg_q86(rgb: &RgbImage, target: &Path) -> Result<(), image::ImageError> {
    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let file = fs::File::create(target).map_err(image::ImageError::IoError)?;
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, 86);
    encoder.encode_image(rgb)
}

fn evict_old_art(dir: &Path, max_files: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, SystemTime)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(UNIX_EPOCH);
            Some((path, modified))
        })
        .collect();

    if files.len() <= max_files {
        return;
    }

    files.sort_by_key(|(_, time)| *time);
    let remove_count = files.len() - max_files;
    for (path, _) in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

fn fnv1a_64_bytes(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
