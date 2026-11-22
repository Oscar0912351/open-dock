use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, RecvTimeoutError, Sender},
        OnceLock,
    },
    thread,
    time::{Duration, UNIX_EPOCH},
};

use image::{DynamicImage, ImageReader};
use tauri::{AppHandle, Emitter, Manager};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::RECT,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
            COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{
            DesktopWallpaper, IDesktopWallpaper, DESKTOP_WALLPAPER_POSITION, DWPOS_CENTER,
            DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH, DWPOS_TILE,
        },
    },
};

use crate::{
    config::schema::{RectI, SizeF, ThemeId},
    state::AppState,
};

use super::{
    monitors::{self, MonitorEntry, MonitorInfo, TauriMonitorSnapshot},
    plate::{
        compose_glass_plate, compose_plate_image, output_dimensions, plate_hash, prune_old_plates,
        save_glass_jpeg, save_jpeg_82, theme_glass_params, ComposeSpec, LumaGrid, PlateSource,
        RectPx, WallpaperPosition,
    },
};

const DEBOUNCE_MS: u64 = 180;
const BACKUP_POLL_SECS: u64 = 120;

enum WallpaperCmd {
    RefreshSync(Sender<()>),
    RefreshNow,
    RefreshDebounced,
    Shutdown,
}

static WORKER_TX: OnceLock<Sender<WallpaperCmd>> = OnceLock::new();

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopMonitorWallpaper {
    rect: RectI,
    path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopWallpaperSnapshot {
    position: WallpaperPosition,
    bg: [u8; 3],
    monitors: Vec<DesktopMonitorWallpaper>,
}

#[derive(Debug, Clone, PartialEq)]
struct ComposeFingerprint {
    snapshot: DesktopWallpaperSnapshot,
    tauri_monitors: Vec<TauriMonitorSnapshot>,
    mtimes: Vec<(String, u128)>,
    live_overrides: Vec<(String, String, u128)>,
    glass_theme: ThemeId,
    glass_strength_pct: i32,
}

pub fn install(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<WallpaperCmd>();
    if WORKER_TX.set(tx.clone()).is_err() {
        log::warn!("Tapet-overvågning er allerede startet");
        return;
    }

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    if let Err(error) = thread::Builder::new()
        .name("opendock-wallpaper".into())
        .spawn(move || worker_loop(app, rx))
    {
        log::error!("Kunne ikke starte tapet-tråd: {error}");
        return;
    }

    if tx.send(WallpaperCmd::RefreshSync(ready_tx)).is_ok() {
        let _ = ready_rx.recv_timeout(Duration::from_secs(2));
    }
}

pub fn trigger_refresh_debounced() {
    if let Some(tx) = WORKER_TX.get() {
        let _ = tx.send(WallpaperCmd::RefreshDebounced);
    }
}

pub fn trigger_refresh_now() {
    if let Some(tx) = WORKER_TX.get() {
        let _ = tx.send(WallpaperCmd::RefreshNow);
    }
}

pub fn shutdown() {
    if let Some(tx) = WORKER_TX.get() {
        let _ = tx.send(WallpaperCmd::Shutdown);
    }
}

fn worker_loop(app: AppHandle, rx: mpsc::Receiver<WallpaperCmd>) {
    let _com = ComAptGuard::init();
    let mut last_fingerprint: Option<ComposeFingerprint> = None;
    let mut desktop_wp: Option<IDesktopWallpaper> = None;

    loop {
        match rx.recv_timeout(Duration::from_secs(BACKUP_POLL_SECS)) {
            Ok(WallpaperCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(WallpaperCmd::RefreshSync(done)) => {
                compose_if_changed(&app, &mut desktop_wp, &mut last_fingerprint, true);
                let _ = done.send(());
            }
            Ok(WallpaperCmd::RefreshNow) => {
                compose_if_changed(&app, &mut desktop_wp, &mut last_fingerprint, false);
            }
            Ok(WallpaperCmd::RefreshDebounced) => {
                if !drain_debounce(&rx) {
                    break;
                }
                compose_if_changed(&app, &mut desktop_wp, &mut last_fingerprint, false);
            }
            Err(RecvTimeoutError::Timeout) => {
                compose_if_changed(&app, &mut desktop_wp, &mut last_fingerprint, false);
            }
        }
    }
}

fn drain_debounce(rx: &mpsc::Receiver<WallpaperCmd>) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(DEBOUNCE_MS);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return true;
        }
        match rx.recv_timeout(remaining) {
            Ok(WallpaperCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => return false,
            Ok(WallpaperCmd::RefreshNow) => return true,
            Ok(WallpaperCmd::RefreshSync(done)) => {
                let _ = done.send(());
                return true;
            }
            Ok(WallpaperCmd::RefreshDebounced) => continue,
            Err(RecvTimeoutError::Timeout) => return true,
        }
    }
}

fn compose_if_changed(
    app: &AppHandle,
    desktop_wp: &mut Option<IDesktopWallpaper>,
    last_fingerprint: &mut Option<ComposeFingerprint>,
    force: bool,
) {
    let tauri_monitors = monitors::list_tauri_monitors(app);
    if tauri_monitors.is_empty() {
        return;
    }

    let snapshot = query_desktop_wallpaper(desktop_wp);
    let state = app.state::<AppState>();
    let live_overrides = state.live_plate_overrides.read().clone();
    let appearance = state.config.get().appearance;
    let glass_theme = appearance.theme_id;
    let glass_strength = appearance.glass_strength;
    let glass_strength_pct = (glass_strength * 100.0).round() as i32;
    let fingerprint = build_fingerprint(
        &snapshot,
        &tauri_monitors,
        &live_overrides,
        glass_theme,
        glass_strength_pct,
    );
    if !force && last_fingerprint.as_ref() == Some(&fingerprint) {
        return;
    }

    let plates_dir = crate::paths::data_root().join("plates");

    let entries = compose_entries(
        &plates_dir,
        &snapshot,
        &tauri_monitors,
        &live_overrides,
        glass_theme,
        glass_strength,
    );
    let infos: Vec<MonitorInfo> = entries.iter().map(|e| e.info.clone()).collect();

    let geometry_changed = last_fingerprint
        .as_ref()
        .map(|prev| prev.tauri_monitors != fingerprint.tauri_monitors)
        .unwrap_or(true);

    let was_empty = {
        let mut guard = state.monitors.write();
        let empty = guard.is_empty();
        *guard = entries.clone();
        empty
    };
    *last_fingerprint = Some(fingerprint);

    if was_empty && !entries.is_empty() {
        crate::wallpaper::notify_monitors_changed(app);
    }

    if let Err(error) = app.emit("desktop://monitors", &infos) {
        log::warn!("Kunne ikke sende desktop://monitors: {error}");
    }
    if geometry_changed {
        crate::windows::widget::reposition_all(app.clone());
        monitors::emit_all_placements(app, &entries);
    }
    log::debug!("Tapetplader opdateret for {} skærm(e)", infos.len());
}

fn build_fingerprint(
    snapshot: &DesktopWallpaperSnapshot,
    tauri_monitors: &[TauriMonitorSnapshot],
    overrides: &HashMap<String, PathBuf>,
    glass_theme: ThemeId,
    glass_strength_pct: i32,
) -> ComposeFingerprint {
    let mut mtimes = Vec::new();
    for mon in &snapshot.monitors {
        if let Some(path) = &mon.path {
            mtimes.push((path.to_string_lossy().into_owned(), file_mtime_nanos(path)));
        }
    }
    if let Some(transcoded) = transcoded_wallpaper_path() {
        if transcoded.exists() {
            mtimes.push((
                transcoded.to_string_lossy().into_owned(),
                file_mtime_nanos(&transcoded),
            ));
        }
    }
    let mut live_overrides: Vec<(String, String, u128)> = overrides
        .iter()
        .map(|(mon, path)| {
            (
                mon.clone(),
                path.to_string_lossy().into_owned(),
                file_mtime_nanos(path),
            )
        })
        .collect();
    live_overrides.sort_by(|a, b| a.0.cmp(&b.0));

    ComposeFingerprint {
        snapshot: snapshot.clone(),
        tauri_monitors: tauri_monitors.to_vec(),
        mtimes,
        live_overrides,
        glass_theme,
        glass_strength_pct,
    }
}

fn compose_entries(
    plates_dir: &Path,
    snapshot: &DesktopWallpaperSnapshot,
    tauri_monitors: &[TauriMonitorSnapshot],
    live_overrides: &HashMap<String, PathBuf>,
    glass_theme: ThemeId,
    glass_strength: f32,
) -> Vec<MonitorEntry> {
    let virtual_bounds = compute_virtual_bounds(tauri_monitors);
    let mut decoded_cache: HashMap<PathBuf, DynamicImage> = HashMap::new();
    let mut keep_names = HashSet::new();
    let mut entries = Vec::with_capacity(tauri_monitors.len());
    let (blur_logical, sat) = theme_glass_params(glass_theme, glass_strength);
    let blur_key = (blur_logical * 10.0).round() as u32;
    let sat_key = (sat * 100.0).round() as u32;

    for mon in tauri_monitors {
        let (source, effective_pos) = if let Some(poster) = live_overrides.get(&mon.name).filter(|p| p.is_file()) {
            (PlateSource::File(poster.clone()), WallpaperPosition::Fill)
        } else {
            let matched = match_desktop_monitor(mon.rect, &snapshot.monitors);
            resolve_source_and_position(
                matched.and_then(|m| m.path.as_deref()),
                snapshot.position,
                snapshot.bg,
            )
        };

        let (source_tag, mtime_ns) = match &source {
            PlateSource::File(path) => (path.to_string_lossy().into_owned(), file_mtime_nanos(path)),
            PlateSource::Solid(rgb) => (format!("solid:{},{},{}", rgb[0], rgb[1], rgb[2]), 0),
        };

        let mon_px = RectPx {
            x: mon.rect.x,
            y: mon.rect.y,
            width: mon.rect.width.max(1) as u32,
            height: mon.rect.height.max(1) as u32,
        };
        let hash = plate_hash(
            &source_tag,
            mtime_ns,
            mon_px,
            virtual_bounds,
            effective_pos,
            snapshot.bg,
        );
        let file_name = format!("plate-{hash}.jpg");
        let glass_name = format!("plate-glass-{hash}-b{blur_key}-s{sat_key}.jpg");
        keep_names.insert(file_name.clone());
        keep_names.insert(glass_name.clone());
        let plate_file = plates_dir.join(&file_name);
        let glass_file = plates_dir.join(&glass_name);

        let scale = mon.scale.max(0.5);
        let logical_w = mon.rect.width as f64 / scale;

        let (luma, sharp_img) = if plate_file.exists() {
            if let Some(img) = load_cached_plate_rgb(&plate_file) {
                let luma = LumaGrid::from_rgb_image(&img);
                (luma, Some(img))
            } else {
                compose_and_save(
                    &plate_file,
                    &source,
                    effective_pos,
                    snapshot.bg,
                    mon_px,
                    virtual_bounds,
                    &mut decoded_cache,
                )
            }
        } else {
            compose_and_save(
                &plate_file,
                &source,
                effective_pos,
                snapshot.bg,
                mon_px,
                virtual_bounds,
                &mut decoded_cache,
            )
        };

        if !glass_file.exists() {
            if let Some(ref sharp) = sharp_img {
                let glass_img = compose_glass_plate(sharp, logical_w, blur_logical, sat);
                if let Err(error) = save_glass_jpeg(&glass_img, &glass_file) {
                    log::error!(
                        "Kunne ikke gemme forud-sløret glasplade {}: {error}",
                        glass_file.display()
                    );
                }
            }
        }

        let avg_luma = luma.average();
        entries.push(MonitorEntry {
            info: MonitorInfo {
                name: mon.name.clone(),
                rect: mon.rect,
                work_area: mon.work_area,
                scale,
                logical_size: SizeF {
                    width: logical_w,
                    height: mon.rect.height as f64 / scale,
                },
                plate_path: if plate_file.exists() {
                    plate_file.to_string_lossy().into_owned()
                } else {
                    String::new()
                },
                glass_plate_path: if glass_file.exists() {
                    glass_file.to_string_lossy().into_owned()
                } else {
                    String::new()
                },
                avg_luma,
                is_primary: mon.is_primary,
            },
            luma,
        });
    }

    prune_old_plates(plates_dir, &keep_names);
    entries
}

fn compose_and_save(
    plate_file: &Path,
    source: &PlateSource,
    position: WallpaperPosition,
    bg: [u8; 3],
    monitor: RectPx,
    virtual_bounds: RectPx,
    decoded_cache: &mut HashMap<PathBuf, DynamicImage>,
) -> (LumaGrid, Option<image::RgbImage>) {
    let (ow, oh) = output_dimensions(monitor.width, monitor.height);
    let spec = ComposeSpec {
        monitor,
        virtual_bounds,
        position,
        bg,
        output_width: ow,
        output_height: oh,
    };

    let source_img = match source {
        PlateSource::File(path) => {
            if !decoded_cache.contains_key(path) {
                match decode_wallpaper_file(path) {
                    Ok(img) => {
                        decoded_cache.insert(path.clone(), img);
                    }
                    Err(error) => {
                        log::warn!("Kunne ikke åbne tapetfil {}: {error}", path.display());
                    }
                }
            }
            decoded_cache.get(path)
        }
        PlateSource::Solid(_) => None,
    };

    let img = compose_plate_image(source_img, &spec);
    if let Err(error) = save_jpeg_82(&img, plate_file) {
        log::error!("Kunne ikke gemme tapetplade {}: {error}", plate_file.display());
    }
    let luma = LumaGrid::from_rgb_image(&img);
    (luma, Some(img))
}

fn load_cached_plate_rgb(path: &Path) -> Option<image::RgbImage> {
    let reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    Some(reader.decode().ok()?.to_rgb8())
}

fn decode_wallpaper_file(path: &Path) -> Result<DynamicImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    reader.decode().map_err(|e| e.to_string())
}

fn resolve_source_and_position(
    path: Option<&Path>,
    position: WallpaperPosition,
    bg: [u8; 3],
) -> (PlateSource, WallpaperPosition) {
    if let Some(p) = path {
        if !p.as_os_str().is_empty() && p.is_file() {
            return (PlateSource::File(p.to_path_buf()), position);
        }
    }
    if let Some(transcoded) = transcoded_wallpaper_path() {
        if transcoded.is_file() {
            return (PlateSource::File(transcoded), WallpaperPosition::Fill);
        }
    }
    (PlateSource::Solid(bg), position)
}

fn transcoded_wallpaper_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| {
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Themes")
            .join("TranscodedWallpaper")
    })
}

fn file_mtime_nanos(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn compute_virtual_bounds(monitors: &[TauriMonitorSnapshot]) -> RectPx {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;

    for m in monitors {
        min_x = min_x.min(m.rect.x);
        min_y = min_y.min(m.rect.y);
        max_x = max_x.max(m.rect.x + m.rect.width);
        max_y = max_y.max(m.rect.y + m.rect.height);
    }

    if min_x >= max_x || min_y >= max_y {
        return RectPx {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
    }

    RectPx {
        x: min_x,
        y: min_y,
        width: (max_x - min_x).max(1) as u32,
        height: (max_y - min_y).max(1) as u32,
    }
}

fn match_desktop_monitor(
    target: RectI,
    desktop_monitors: &[DesktopMonitorWallpaper],
) -> Option<&DesktopMonitorWallpaper> {
    desktop_monitors
        .iter()
        .find(|m| m.rect == target)
        .or_else(|| {
            desktop_monitors.iter().max_by_key(|m| {
                let ix0 = m.rect.x.max(target.x);
                let iy0 = m.rect.y.max(target.y);
                let ix1 = (m.rect.x + m.rect.width).min(target.x + target.width);
                let iy1 = (m.rect.y + m.rect.height).min(target.y + target.height);
                ((ix1 - ix0).max(0) as i64) * ((iy1 - iy0).max(0) as i64)
            })
        })
}

fn query_desktop_wallpaper(cached: &mut Option<IDesktopWallpaper>) -> DesktopWallpaperSnapshot {
    let fallback = DesktopWallpaperSnapshot {
        position: WallpaperPosition::Fill,
        bg: [11, 26, 31],
        monitors: Vec::new(),
    };

    if let Some(dw) = cached.as_ref() {
        if let Some(snap) = read_desktop_wallpaper_from(dw) {
            return snap;
        }
        // Explorer kan være genstartet; nulstil COM-proxyen og prøv én gang mere.
        *cached = None;
    }

    let dw: IDesktopWallpaper =
        match unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL) } {
            Ok(instance) => instance,
            Err(error) => {
                log::warn!("Kunne ikke åbne IDesktopWallpaper: {error}");
                return fallback;
            }
        };

    let snap = read_desktop_wallpaper_from(&dw).unwrap_or(fallback);
    *cached = Some(dw);
    snap
}

fn read_desktop_wallpaper_from(dw: &IDesktopWallpaper) -> Option<DesktopWallpaperSnapshot> {
    let position = unsafe { dw.GetPosition() }.ok().map(map_position)?;

    let bg = unsafe { dw.GetBackgroundColor() }
        .map(|c| {
            let v = c.0;
            [(v & 0xff) as u8, ((v >> 8) & 0xff) as u8, ((v >> 16) & 0xff) as u8]
        })
        .unwrap_or([11, 26, 31]);

    let count = unsafe { dw.GetMonitorDevicePathCount() }.ok()?;
    let mut monitors = Vec::new();

    for i in 0..count {
        let Ok(raw_id) = (unsafe { dw.GetMonitorDevicePathAt(i) }) else {
            continue;
        };
        let _id_guard = CoTaskStr(raw_id);
        let Ok(rect) = (unsafe { dw.GetMonitorRECT(PCWSTR(raw_id.0)) }) else {
            continue;
        };

        let path = match unsafe { dw.GetWallpaper(PCWSTR(raw_id.0)) } {
            Ok(raw_path) => {
                let _path_guard = CoTaskStr(raw_path);
                let s = unsafe { raw_path.to_string() }.unwrap_or_default();
                if s.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(s))
                }
            }
            Err(_) => None,
        };

        monitors.push(DesktopMonitorWallpaper {
            rect: rect_to_rect_i(rect),
            path,
        });
    }

    Some(DesktopWallpaperSnapshot {
        position,
        bg,
        monitors,
    })
}

fn rect_to_rect_i(r: RECT) -> RectI {
    RectI {
        x: r.left,
        y: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
    }
}

fn map_position(pos: DESKTOP_WALLPAPER_POSITION) -> WallpaperPosition {
    match pos {
        DWPOS_CENTER => WallpaperPosition::Center,
        DWPOS_TILE => WallpaperPosition::Tile,
        DWPOS_STRETCH => WallpaperPosition::Stretch,
        DWPOS_FIT => WallpaperPosition::Fit,
        DWPOS_FILL => WallpaperPosition::Fill,
        DWPOS_SPAN => WallpaperPosition::Span,
        _ => WallpaperPosition::Fill,
    }
}
