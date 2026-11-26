use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use image::ImageReader;
use reqwest::Client;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

use crate::{
    config::schema::{CustomWallpaper, Language, ThemeId, WallpaperKind, WallpaperMood},
    error::AppError,
    state::AppState,
};

use super::{
    catalog::{
        find_photo, find_shader, localized_photo_text, localized_shader_text, photo_full_url,
        photo_thumb_url, BUILTIN_SHADERS, CURATED_PHOTOS,
    },
    shader_compiler::{render_shader_to_png, save_png_atomic},
    video::{cover_resize_rgb, extract_video_poster},
};

use super::catalog::WallpaperItem;

static THUMBS_RUNNING: AtomicBool = AtomicBool::new(false);

pub struct WallpaperDirs {
    pub root: PathBuf,
    pub thumbs: PathBuf,
    pub cache: PathBuf,
    pub posters: PathBuf,
    pub applied: PathBuf,
    pub custom: PathBuf,
}

pub fn wallpaper_dirs(_app: &AppHandle) -> Result<WallpaperDirs, AppError> {
    let root = crate::paths::data_root().join("wallpapers");
    let dirs = WallpaperDirs {
        thumbs: root.join("thumbs"),
        cache: root.join("cache"),
        posters: root.join("posters"),
        applied: root.join("applied"),
        custom: root.join("custom"),
        root,
    };
    fs::create_dir_all(&dirs.thumbs).map_err(AppError::ConfigWrite)?;
    fs::create_dir_all(&dirs.cache).map_err(AppError::ConfigWrite)?;
    fs::create_dir_all(&dirs.posters).map_err(AppError::ConfigWrite)?;
    fs::create_dir_all(&dirs.applied).map_err(AppError::ConfigWrite)?;
    fs::create_dir_all(&dirs.custom).map_err(AppError::ConfigWrite)?;
    Ok(dirs)
}

pub fn build_catalog(app: &AppHandle) -> Result<Vec<WallpaperItem>, AppError> {
    let dirs = wallpaper_dirs(app)?;
    let cfg = app.state::<AppState>().config.get();
    let lang = cfg.general.language;

    let mut items = Vec::with_capacity(
        BUILTIN_SHADERS.len() + CURATED_PHOTOS.len() + cfg.wallpaper.custom_items.len(),
    );

    for spec in BUILTIN_SHADERS {
        let (title, subtitle) = localized_shader_text(spec, lang);
        let thumb_file = dirs.thumbs.join(format!("{}.png", spec.id));
        items.push(WallpaperItem {
            id: spec.id.to_string(),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            kind: WallpaperKind::Shader,
            moods: spec.moods.to_vec(),
            recommended_theme: spec.recommended_theme,
            accent_hex: spec.accent_hex.to_string(),
            estimated_luma: spec.estimated_luma,
            thumb_path: existing_path_str(&thumb_file),
            is_custom: false,
            is_cached: true,
            resolution_label: match lang {
                Language::Da => "Nativ 4K+".into(),
                Language::En => "Native 4K+".into(),
            },
        });
    }

    for spec in CURATED_PHOTOS {
        let (title, subtitle) = localized_photo_text(spec, lang);
        let thumb_file = dirs.thumbs.join(format!("{}.jpg", spec.id));
        let full_file = dirs.cache.join(format!("{}.jpg", spec.id));
        items.push(WallpaperItem {
            id: spec.id.to_string(),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            kind: WallpaperKind::Photo,
            moods: spec.moods.to_vec(),
            recommended_theme: spec.recommended_theme,
            accent_hex: spec.accent_hex.to_string(),
            estimated_luma: spec.estimated_luma,
            thumb_path: existing_path_str(&thumb_file),
            is_custom: false,
            is_cached: is_valid_cached_file(&full_file, 16_384),
            resolution_label: "4K UHD".into(),
        });
    }

    for custom in &cfg.wallpaper.custom_items {
        let thumb_exists = Path::new(&custom.thumb_path).is_file();
        let file_exists = Path::new(&custom.file_path).is_file();
        items.push(WallpaperItem {
            id: custom.id.clone(),
            title: custom.title.clone(),
            subtitle: match (custom.kind, lang) {
                (WallpaperKind::Video, Language::Da) => "Egen video-baggrund".into(),
                (WallpaperKind::Video, Language::En) => "Custom video wallpaper".into(),
                (_, Language::Da) => "Eget billede".into(),
                (_, Language::En) => "Custom image".into(),
            },
            kind: custom.kind,
            moods: vec![WallpaperMood::Minimal],
            recommended_theme: ThemeId::Grafit,
            accent_hex: "#FFFFFF".into(),
            estimated_luma: 0.25,
            thumb_path: if thumb_exists {
                custom.thumb_path.clone()
            } else {
                String::new()
            },
            is_custom: true,
            is_cached: file_exists,
            resolution_label: if custom.width > 0 && custom.height > 0 {
                format!("{}×{}", custom.width, custom.height)
            } else {
                "—".into()
            },
        });
    }

    spawn_thumbnail_hydration(app.clone());
    Ok(items)
}

pub fn spawn_thumbnail_hydration(app: AppHandle) {
    if THUMBS_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let result = hydrate_missing_thumbnails(&app).await;
        THUMBS_RUNNING.store(false, Ordering::Release);
        match result {
            Ok(true) => {
                let _ = app.emit("wallpaper://thumbs-ready", ());
            }
            Ok(false) => {}
            Err(error) => log::debug!("Thumbnail-hydrering afbrudt: {error:?}"),
        }
    });
}

async fn hydrate_missing_thumbnails(app: &AppHandle) -> Result<bool, AppError> {
    let dirs = wallpaper_dirs(app)?;
    let mut updated = false;

    // 1. Offline D3D11 thumbnails for built-in HLSL scenes
    let missing_shaders: Vec<&'static str> = BUILTIN_SHADERS
        .iter()
        .filter(|s| !is_valid_cached_file(&dirs.thumbs.join(format!("{}.png", s.id)), 1024))
        .map(|s| s.id)
        .collect();

    if !missing_shaders.is_empty() {
        let thumbs_dir = dirs.thumbs.clone();
        let shader_ok = tokio::task::spawn_blocking(move || {
            let mut any = false;
            for id in missing_shaders {
                let out = thumbs_dir.join(format!("{id}.png"));
                if let Err(error) = render_shader_to_png(id, 640, 360, 12.0, 1.0, &out) {
                    log::warn!("Kunne ikke generere shader-thumbnail for {id}: {error:?}");
                } else {
                    any = true;
                }
            }
            any
        })
        .await
        .unwrap_or(false);
        if shader_ok {
            let _ = app.emit("wallpaper://thumbs-ready", ());
        }
    }

    // 2. Bounded parallel download for missing curated photo thumbnails
    let missing_photos: Vec<_> = CURATED_PHOTOS
        .iter()
        .filter(|p| !is_valid_cached_file(&dirs.thumbs.join(format!("{}.jpg", p.id)), 2048))
        .collect();

    if !missing_photos.is_empty() {
        let client = crate::util::http_client();
        let sem = std::sync::Arc::new(Semaphore::new(4));
        let mut handles = Vec::with_capacity(missing_photos.len());

        for spec in missing_photos {
            let sem = sem.clone();
            let out = dirs.thumbs.join(format!("{}.jpg", spec.id));
            let url = photo_thumb_url(spec);
            handles.push(tokio::spawn(async move {
                let Ok(_permit) = sem.acquire().await else {
                    return false;
                };
                download_image_atomic(client, &url, &out, Duration::from_secs(12)).await.is_ok()
            }));
        }

        for handle in handles {
            if let Ok(true) = handle.await {
                updated = true;
            }
        }
    }

    Ok(updated)
}

pub async fn ensure_photo_downloaded(app: &AppHandle, photo_id: &str) -> Result<PathBuf, AppError> {
    let spec = find_photo(photo_id)
        .ok_or_else(|| AppError::Other(format!("Ukendt kurateret baggrund: {photo_id}")))?;
    let dirs = wallpaper_dirs(app)?;
    let out = dirs.cache.join(format!("{}.jpg", spec.id));
    if is_valid_cached_file(&out, 16_384) {
        return Ok(out);
    }

    let client = crate::util::http_download_client();
    let url = photo_full_url(spec);
    download_image_atomic(client, &url, &out, Duration::from_secs(30)).await?;
    Ok(out)
}

pub fn ensure_shader_poster(
    app: &AppHandle,
    shader_id: &str,
    brightness: f32,
) -> Result<PathBuf, AppError> {
    if find_shader(shader_id).is_none() {
        return Err(AppError::Other(format!("Ukendt shader-scene: {shader_id}")));
    }
    let dirs = wallpaper_dirs(app)?;
    let b_tag = (brightness * 100.0).round() as i32;
    let out = dirs.posters.join(format!("{shader_id}-b{b_tag}.png"));
    if !is_valid_cached_file(&out, 4096) {
        render_shader_to_png(shader_id, 1280, 720, 12.0, brightness, &out)?;
    }
    Ok(out)
}

pub async fn import_local_file(app: &AppHandle, source: &Path) -> Result<CustomWallpaper, AppError> {
    if !source.is_file() {
        return Err(AppError::Other("Den valgte fil findes ikke".into()));
    }
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| AppError::Other("Filen mangler en gyldig filendelse".into()))?;

    let kind = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" => WallpaperKind::Photo,
        "mp4" | "webm" | "mkv" | "mov" | "wmv" => WallpaperKind::Video,
        _ => {
            return Err(AppError::Other(
                "Den filtype virker ikke. Vælg et billede (PNG, JPG, WebP) eller en video (MP4, WebM, MKV).".into(),
            ));
        }
    };

    let dirs = wallpaper_dirs(app)?;
    let id = format!("custom-{}", uuid::Uuid::new_v4());
    let dest = dirs.custom.join(format!("{id}.{ext}"));
    let thumb = dirs.thumbs.join(format!("{id}.png"));
    let poster = dirs.posters.join(format!("{id}.png"));

    let src_clone = source.to_path_buf();
    let dest_clone = dest.clone();
    let thumb_clone = thumb.clone();
    let poster_clone = poster.clone();

    let (width, height, file_size_bytes) = tokio::task::spawn_blocking(move || {
        fs::copy(&src_clone, &dest_clone).map_err(AppError::ConfigWrite)?;
        let size_bytes = fs::metadata(&dest_clone).map(|m| m.len()).unwrap_or(0);

        let (w, h) = match kind {
            WallpaperKind::Photo => {
                let img = ImageReader::open(&dest_clone)
                    .map_err(|e| AppError::Other(format!("Kunne ikke åbne billede: {e}")))?
                    .with_guessed_format()
                    .map_err(|e| AppError::Other(format!("Ukendt billedformat: {e}")))?
                    .decode()
                    .map_err(|e| AppError::Other(format!("Kunne ikke afkode billede: {e}")))?;
                let rgb = img.to_rgb8();
                let (w, h) = rgb.dimensions();
                let thumb_rgb = cover_resize_rgb(&rgb, 640, 360);
                save_png_atomic(&thumb_rgb, &thumb_clone)?;
                (w, h)
            }
            WallpaperKind::Video => {
                extract_video_poster(&dest_clone, &thumb_clone, Some(&poster_clone))?
            }
            WallpaperKind::Shader => (0, 0),
        };
        Ok::<_, AppError>((w, h, size_bytes))
    })
    .await
    .map_err(|e| AppError::Other(format!("Import-opgave afbrudt: {e}")))??;

    let title = source
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Egen baggrund")
        .to_string();

    Ok(CustomWallpaper {
        id,
        title,
        kind,
        file_path: dest.to_string_lossy().into_owned(),
        thumb_path: thumb.to_string_lossy().into_owned(),
        width,
        height,
        file_size_bytes,
        added_at: crate::config::defaults::unix_now(),
    })
}

pub fn delete_custom_assets(app: &AppHandle, item: &CustomWallpaper) {
    let _ = fs::remove_file(&item.file_path);
    let _ = fs::remove_file(&item.thumb_path);
    if let Ok(dirs) = wallpaper_dirs(app) {
        let _ = fs::remove_file(dirs.posters.join(format!("{}.png", item.id)));
    }
}

async fn download_image_atomic(
    client: &Client,
    url: &str,
    out_path: &Path,
    timeout: Duration,
) -> Result<(), AppError> {
    let resp = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(AppError::Network)?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "Billedserver svarede med status {}",
            resp.status()
        )));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.is_empty() && !content_type.starts_with("image/") {
        return Err(AppError::Other(format!(
            "Uventet indholdstype fra billedserver: {content_type}"
        )));
    }
    let bytes = resp.bytes().await.map_err(AppError::Network)?;
    if bytes.len() < 1024 {
        return Err(AppError::Other("Billedfilen er tom eller ufuldstændig. Prøv igen".into()));
    }
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(AppError::ConfigWrite)?;
    }
    let tmp = out_path.with_extension("download.tmp");
    fs::write(&tmp, &bytes).map_err(AppError::ConfigWrite)?;
    fs::rename(&tmp, out_path).map_err(AppError::ConfigWrite)?;
    Ok(())
}

fn existing_path_str(path: &Path) -> String {
    if path.is_file() {
        path.to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

fn is_valid_cached_file(path: &Path, min_bytes: u64) -> bool {
    fs::metadata(path)
        .map(|m| m.is_file() && m.len() >= min_bytes)
        .unwrap_or(false)
}
