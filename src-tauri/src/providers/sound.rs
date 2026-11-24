//! Afspilning af timer-lyde med winmm `PlaySoundW`.
//!
//! Lydene ligger som bundlede resources (`sounds/*.wav`) og er 44.1 kHz
//! 16-bit stereo PCM. Volumen skaleres ved at omskrive samples til en cache-fil
//! pr. (lyd, volumen-trin) — filnavnet er indholdsstabilt, så gentagen
//! afspilning af samme fil aldrig overskriver noget der er i brug.
//!
//! Fil-baseret afspilning (SND_FILENAME) bruges bevidst frem for SND_MEMORY:
//! Windows ejer selv fil-læsningen, og vi skal ikke gætte hvornår en buffer
//! må frigives.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use tauri::{AppHandle, Manager};

pub const SOUNDS: [&str; 3] = ["chime", "marimba", "crystal"];

static WARNED_MISSING: AtomicBool = AtomicBool::new(false);

/// Normaliserer et lydnavn mod whitelisten så der aldrig læses vilkårlige filer.
pub fn normalize_name(name: &str) -> &'static str {
    SOUNDS
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
        .unwrap_or("chime")
}

fn cache_dir() -> PathBuf {
    crate::paths::data_root().join("cache").join("timer-sounds")
}

/// Rydder skalerede cache-filer ved opstart (volumen-trin fra tidligere sessioner).
pub fn clean_cache() {
    let dir = cache_dir();
    if dir.is_dir() {
        if let Err(error) = fs::remove_dir_all(&dir) {
            log::warn!(target: "sound", "Kunne ikke rydde lyd-cache {:?}: {error}", dir);
        }
    }
}

/// Finder den originale wav: bundlet resource i release, repo-sti i dev.
fn source_path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("sounds").join(format!("{name}.wav")));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("sounds").join(format!("{name}.wav")));
        }
    }
    // Dev-fallback: repoet ligger der hvor binæren blev bygget.
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("sounds")
            .join(format!("{name}.wav")),
    );
    candidates.into_iter().find(|path| path.is_file())
}

struct WavParts {
    header_end: usize,
    data_start: usize,
    data_len: usize,
    bits_per_sample: u16,
}

/// Minimal RIFF/WAVE-parser: finder data-chunken og validerer PCM16.
/// Filerne er selv-genererede, men parseren er skrevet tolerant alligevel.
fn parse_wav(bytes: &[u8]) -> Option<WavParts> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut bits_per_sample = 0u16;
    let mut audio_format = 0u16;
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        if body + size > bytes.len() {
            break;
        }
        if id == b"fmt " && size >= 16 {
            audio_format = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
            bits_per_sample = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
        }
        if id == b"data" {
            if audio_format != 1 || bits_per_sample != 16 {
                return None;
            }
            return Some(WavParts {
                header_end: offset,
                data_start: body,
                data_len: size,
                bits_per_sample,
            });
        }
        offset = body + size + (size % 2);
    }
    None
}

/// Skalerer PCM16-samples med `gain` og skriver en ny wav pr. (navn, trin).
/// Returnerer stien til den fil der skal afspilles.
fn scaled_copy(name: &str, source: &PathBuf, volume: u8) -> Option<PathBuf> {
    let bytes = match fs::read(source) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!(target: "sound", "Kunne ikke læse {:?}: {error}", source);
            return None;
        }
    };
    if volume >= 100 {
        return Some(source.clone());
    }
    let step = volume / 5 * 5; // trin á 5 holder cache-mappen begrænset
    let target = cache_dir().join(format!("{name}-{step}.wav"));
    if target.is_file() {
        return Some(target);
    }
    let Some(parts) = parse_wav(&bytes) else {
        log::warn!(target: "sound", "{:?} er ikke PCM16 wav — afspiller uskaleret", source);
        return Some(source.clone());
    };
    let _ = parts.bits_per_sample;
    let gain = (volume as f32 / 100.0).powi(2);
    let mut out = bytes.clone();
    let data_end = parts.data_start + parts.data_len;
    for chunk in out[parts.data_start..data_end].chunks_exact_mut(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        let scaled = (sample as f32 * gain).round().clamp(i16::MIN as f32, i16::MAX as f32);
        chunk.copy_from_slice(&(scaled as i16).to_le_bytes());
    }
    let _ = parts.header_end;
    if let Err(error) = fs::create_dir_all(cache_dir()) {
        log::warn!(target: "sound", "Kunne ikke oprette lyd-cache: {error}");
        return Some(source.clone());
    }
    let tmp = cache_dir().join(format!("{name}-{step}.wav.tmp"));
    match fs::write(&tmp, &out).and_then(|_| fs::rename(&tmp, &target)) {
        Ok(()) => Some(target),
        Err(error) => {
            log::warn!(target: "sound", "Kunne ikke skrive skaleret wav: {error}");
            Some(source.clone())
        }
    }
}

/// Afspiller lyden asynkront. volume 0 = helt lydløs (ingen afspilning).
/// Fejl logges med kontekst — lyd må aldrig vælte timer-flowet.
pub fn play(app: &AppHandle, name: &str, volume: u8) {
    if volume == 0 {
        return;
    }
    let name = normalize_name(name);
    let Some(source) = source_path(app, name) else {
        if !WARNED_MISSING.swap(true, Ordering::Relaxed) {
            log::warn!(target: "sound", "Lyden '{name}' blev ikke fundet hverken som resource eller i dev-stien");
        }
        return;
    };
    let Some(path) = scaled_copy(name, &source, volume) else {
        return;
    };

    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};
    let played = unsafe {
        PlaySoundW(
            windows::core::PCWSTR(wide.as_ptr()),
            None,
            SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
        )
    };
    if !played.as_bool() {
        log::warn!(target: "sound", "PlaySoundW kunne ikke afspille {:?}", path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_whitelist() {
        assert_eq!(normalize_name("marimba"), "marimba");
        assert_eq!(normalize_name("../../etc/passwd"), "chime");
        assert_eq!(normalize_name(""), "chime");
    }

    #[test]
    fn parser_accepterer_genererede_filer() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("sounds")
            .join("chime.wav");
        if path.is_file() {
            let bytes = fs::read(&path).unwrap();
            let parts = parse_wav(&bytes).expect("chime.wav skal parse");
            assert_eq!(parts.bits_per_sample, 16);
            assert!(parts.data_len > 1000);
        }
    }
}
