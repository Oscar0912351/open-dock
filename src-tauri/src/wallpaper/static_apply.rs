use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use image::{imageops::FilterType, ImageReader};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
            COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{
            DesktopWallpaper, IDesktopWallpaper, DESKTOP_WALLPAPER_POSITION, DWPOS_CENTER,
            DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH,
        },
    },
};

use crate::{
    config::schema::{RectI, WallpaperFit},
    desktop::monitors::{list_tauri_monitors, MonitorInfo},
    error::AppError,
    state::AppState,
};

struct ComAptGuard(bool);

impl ComAptGuard {
    fn init() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self(hr.is_ok())
    }
}

impl Drop for ComAptGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct CoTaskStr(PWSTR);

impl Drop for CoTaskStr {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                CoTaskMemFree(Some(self.0 .0 as *const _));
            }
        }
    }
}

pub fn apply_static_wallpaper(
    app: &AppHandle,
    target_monitor: Option<&str>,
    image_path: &Path,
    fit: WallpaperFit,
    lossless_enabled: bool,
) -> Result<(), AppError> {
    if !image_path.is_file() {
        return Err(AppError::Other(format!(
            "Billedfilen findes ikke: {}",
            image_path.display()
        )));
    }

    let monitors: Vec<MonitorInfo> = app
        .state::<AppState>()
        .monitors
        .read()
        .iter()
        .map(|m| m.info.clone())
        .collect();

    let all_monitors = matches!(target_monitor, None | Some("*"));
    let target_rect = if all_monitors {
        monitors
            .iter()
            .max_by_key(|m| (m.rect.width.max(1) as u64) * (m.rect.height.max(1) as u64))
            .map(|m| m.rect)
            .or_else(|| {
                list_tauri_monitors(app)
                    .into_iter()
                    .max_by_key(|m| (m.rect.width.max(1) as u64) * (m.rect.height.max(1) as u64))
                    .map(|m| m.rect)
            })
    } else {
        target_monitor.and_then(|name| {
            monitors
                .iter()
                .find(|m| m.name == name)
                .map(|m| m.rect)
                .or_else(|| {
                    list_tauri_monitors(app)
                        .into_iter()
                        .find(|m| m.name == name)
                        .map(|m| m.rect)
                })
        })
    };

    let ext = image_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let final_path = if lossless_enabled || ext == "webp" {
        prepare_lossless_png(app, image_path, target_rect, fit)?
    } else {
        image_path.to_path_buf()
    };

    let _com = ComAptGuard::init();
    let dw: IDesktopWallpaper = unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)? };

    let _ = unsafe { dw.Enable(true) };
    unsafe {
        dw.SetPosition(map_fit_to_dwpos(fit))?;
    }

    let wide_path: Vec<u16> = final_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    if all_monitors {
        unsafe {
            dw.SetWallpaper(PCWSTR::null(), PCWSTR(wide_path.as_ptr()))?;
        }
    } else {
        let count = unsafe { dw.GetMonitorDevicePathCount()? };
        let mut applied_any = false;

        if let Some(target) = target_rect {
            for i in 0..count {
                let Ok(raw_id) = (unsafe { dw.GetMonitorDevicePathAt(i) }) else {
                    continue;
                };
                let _id_guard = CoTaskStr(raw_id);
                let Ok(rect) = (unsafe { dw.GetMonitorRECT(PCWSTR(raw_id.0)) }) else {
                    continue;
                };
                let dw_rect = RectI {
                    x: rect.left,
                    y: rect.top,
                    width: rect.right - rect.left,
                    height: rect.bottom - rect.top,
                };

                if rect_matches_or_overlaps(target, dw_rect) {
                    unsafe {
                        dw.SetWallpaper(PCWSTR(raw_id.0), PCWSTR(wide_path.as_ptr()))?;
                    }
                    applied_any = true;
                    break;
                }
            }
        }

        if !applied_any {
            if count <= 1 {
                unsafe {
                    dw.SetWallpaper(PCWSTR::null(), PCWSTR(wide_path.as_ptr()))?;
                }
            } else {
                return Err(AppError::Other(
                    "Kunne ikke finde den valgte skærm til baggrunden".into(),
                ));
            }
        }
    }

    crate::desktop::wallpaper::trigger_refresh_now();
    Ok(())
}

fn prepare_lossless_png(
    _app: &AppHandle,
    source_path: &Path,
    target_rect: Option<RectI>,
    fit: WallpaperFit,
) -> Result<PathBuf, AppError> {
    let applied_dir = crate::paths::data_root().join("wallpapers").join("applied");
    fs::create_dir_all(&applied_dir).map_err(AppError::ConfigWrite)?;

    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let target_w = target_rect
        .map(|r| r.width.max(1) as u32)
        .unwrap_or(3840);
    let target_h = target_rect
        .map(|r| r.height.max(1) as u32)
        .unwrap_or(2160);

    let mtime = fs::metadata(source_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut hasher = Sha256::new();
    hasher.update(source_path.to_string_lossy().as_bytes());
    hasher.update(mtime.to_le_bytes());
    hasher.update(target_w.to_le_bytes());
    hasher.update(target_h.to_le_bytes());
    hasher.update([fit as u8]);
    let hash_hex = format!("{:x}", hasher.finalize());
    let out_path = applied_dir.join(format!("applied-{}.png", &hash_hex[..16]));
    if out_path.is_file() {
        return Ok(out_path);
    }

    let reader = ImageReader::open(source_path)
        .map_err(|e| AppError::Other(format!("Kunne ikke åbne billede: {e}")))?
        .with_guessed_format()
        .map_err(|e| AppError::Other(format!("Ukendt billedformat: {e}")))?;

    let (sw, sh) = reader
        .into_dimensions()
        .map_err(|e| AppError::Other(format!("Kunne ikke læse billedmål: {e}")))?;

    // If source is already a native-size PNG or smaller PNG, skip re-encoding unnecessarily.
    if ext == "png" && sw <= target_w && sh <= target_h {
        return Ok(source_path.to_path_buf());
    }

    let img = ImageReader::open(source_path)
        .map_err(|e| AppError::Other(format!("Kunne ikke åbne billede: {e}")))?
        .with_guessed_format()
        .map_err(|e| AppError::Other(format!("Ukendt billedformat: {e}")))?
        .decode()
        .map_err(|e| AppError::Other(format!("Kunne ikke afkode billede: {e}")))?;

    let processed = match fit {
        WallpaperFit::Fill if sw > target_w || sh > target_h => {
            let scale = (target_w as f64 / sw as f64).max(target_h as f64 / sh as f64);
            if scale < 1.0 {
                let crop_w = ((target_w as f64 / scale).round() as u32).clamp(1, sw);
                let crop_h = ((target_h as f64 / scale).round() as u32).clamp(1, sh);
                let cx = (sw - crop_w) / 2;
                let cy = (sh - crop_h) / 2;
                img.crop_imm(cx, cy, crop_w, crop_h)
                    .resize_exact(target_w, target_h, FilterType::Lanczos3)
            } else {
                img
            }
        }
        WallpaperFit::Fit if sw > target_w || sh > target_h => {
            img.resize(target_w, target_h, FilterType::Lanczos3)
        }
        _ => img,
    };

    let tmp_path = out_path.with_extension("png.tmp");
    processed
        .save_with_format(&tmp_path, image::ImageFormat::Png)
        .map_err(|e| AppError::Other(format!("Kunne ikke gemme tabsfri PNG-baggrund: {e}")))?;
    fs::rename(&tmp_path, &out_path).map_err(AppError::ConfigWrite)?;
    prune_old_applied(&applied_dir, &out_path);

    Ok(out_path)
}

fn prune_old_applied(dir: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p == keep {
                return None;
            }
            let modified = e.metadata().ok().and_then(|m| m.modified().ok())?;
            Some((modified, p))
        })
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, old_path) in files.into_iter().skip(6) {
        let _ = fs::remove_file(old_path);
    }
}

fn rect_matches_or_overlaps(a: RectI, b: RectI) -> bool {
    if a == b {
        return true;
    }
    let ix0 = a.x.max(b.x);
    let iy0 = a.y.max(b.y);
    let ix1 = (a.x + a.width).min(b.x + b.width);
    let iy1 = (a.y + a.height).min(b.y + b.height);
    ix1 > ix0 && iy1 > iy0
}

fn map_fit_to_dwpos(fit: WallpaperFit) -> DESKTOP_WALLPAPER_POSITION {
    match fit {
        WallpaperFit::Fill => DWPOS_FILL,
        WallpaperFit::Fit => DWPOS_FIT,
        WallpaperFit::Stretch => DWPOS_STRETCH,
        WallpaperFit::Center => DWPOS_CENTER,
        WallpaperFit::Span => DWPOS_SPAN,
    }
}
