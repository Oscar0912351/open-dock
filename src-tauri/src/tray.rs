use std::sync::{atomic::Ordering, OnceLock};

use parking_lot::Mutex;
use tauri::{
    image::Image,
    menu::{
        CheckMenuItem, CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem,
        SubmenuBuilder,
    },
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};

use crate::{
    commands,
    config::schema::{Language, WidgetKind},
    desktop::drag,
    error::AppError,
    state::AppState,
    windows::control,
};

const TRAY_ID: &str = "open-dock-tray";
const ICON_SIZE: u32 = 32;
const SUPERSAMPLE: u32 = 4;

#[derive(Clone, PartialEq, Eq)]
struct TraySyncKey {
    language: Language,
    edit_mode: bool,
    visible: bool,
    paused: bool,
}

static LAST_KEY: OnceLock<Mutex<Option<TraySyncKey>>> = OnceLock::new();
static TRAY_ICONS: OnceLock<(Vec<u8>, Vec<u8>)> = OnceLock::new();

pub fn install(app: &AppHandle) -> Result<(), AppError> {
    let key = current_key(app);
    let menu = build_menu(app, &key)?;
    let icon = tray_icon_image(key.paused);
    let tip = tooltip_text(key.language, key.paused);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(tip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                if let Err(error) = control::open(tray.app_handle(), None) {
                    log::warn!(target: "tray", "Kunne ikke åbne kontrolcentret fra tray: {error:?}");
                }
            }
            _ => {}
        })
        .build(app)?;

    *last_key_lock().lock() = Some(key);
    Ok(())
}

pub fn sync(app: &AppHandle) {
    if app.tray_by_id(TRAY_ID).is_none() {
        return;
    }
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        sync_on_main_thread(&handle);
    }) {
        log::warn!(target: "tray", "Kunne ikke planlægge tray-opdatering: {error}");
    }
}

fn sync_on_main_thread(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let next = current_key(app);
    {
        let guard = last_key_lock().lock();
        if guard.as_ref() == Some(&next) {
            return;
        }
    }

    if let Err(error) = tray.set_icon(Some(tray_icon_image(next.paused))) {
        log::warn!(target: "tray", "Kunne ikke opdatere tray-ikon: {error}");
    }
    if let Err(error) = tray.set_tooltip(Some(tooltip_text(next.language, next.paused))) {
        log::warn!(target: "tray", "Kunne ikke opdatere tray-tooltip: {error}");
    }
    match build_menu(app, &next) {
        Ok(menu) => {
            if let Err(error) = tray.set_menu(Some(menu)) {
                log::warn!(target: "tray", "Kunne ikke opdatere tray-menu: {error}");
            }
        }
        Err(error) => log::warn!(target: "tray", "Kunne ikke bygge tray-menu: {error:?}"),
    }

    *last_key_lock().lock() = Some(next);
}

pub fn handle_menu_event(app: &AppHandle, menu_id: &str) {
    match menu_id {
        "tray:open" => {
            if let Err(error) = control::open(app, None) {
                log::warn!(target: "tray", "Kunne ikke åbne kontrolcentret: {error:?}");
            }
        }
        "tray:edit" => {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                let next = !state.edit_mode.load(Ordering::Acquire);
                if next && !state.widgets_visible.load(Ordering::Acquire) {
                    if let Err(error) = commands::app::set_widgets_visible(&handle, true) {
                        log::warn!(target: "tray", "Kunne ikke vise widgets før redigering: {error:?}");
                    }
                }
                if let Err(error) = drag::set_edit_mode(&handle, next) {
                    log::warn!(target: "tray", "Kunne ikke skifte redigeringstilstand: {error:?}");
                }
            });
        }
        "tray:visible" => {
            let visible = app.state::<AppState>().widgets_visible.load(Ordering::Acquire);
            if let Err(error) = commands::app::set_widgets_visible(app, !visible) {
                log::warn!(target: "tray", "Kunne ikke skifte widget-synlighed: {error:?}");
            }
        }
        "tray:pause" => {
            let manual = app.state::<AppState>().manual_paused.load(Ordering::Acquire);
            commands::app::set_manual_paused(app, !manual);
        }
        "tray:add:cluster:allInOne" => {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                ensure_widgets_visible(&handle);
                if let Err(error) =
                    commands::widgets::add_cluster_preset_internal(&handle, "allInOne", None)
                {
                    log::warn!(target: "tray", "Kunne ikke tilføje Alt-i-én Boks: {error:?}");
                }
            });
        }
        "tray:quit" => {
            commands::app::quit_app(app);
        }
        other => {
            if let Some(kind_str) = other.strip_prefix("tray:add:") {
                if let Some(kind) = parse_add_kind(kind_str) {
                    let kind_label = kind_str.to_string();
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        ensure_widgets_visible(&handle);
                        if let Err(error) = commands::widgets::add_widget_internal(
                            &handle,
                            kind,
                            kind.default_size(),
                            None,
                        ) {
                            log::warn!(target: "tray", "Kunne ikke tilføje widget {kind_label}: {error:?}");
                        }
                    });
                }
            }
        }
    }
}

fn ensure_widgets_visible(app: &AppHandle) {
    let visible = app.state::<AppState>().widgets_visible.load(Ordering::Acquire);
    if !visible {
        if let Err(error) = commands::app::set_widgets_visible(app, true) {
            log::warn!(target: "tray", "Kunne ikke vise widgets: {error:?}");
        }
    }
}

fn parse_add_kind(raw: &str) -> Option<WidgetKind> {
    match raw {
        "clock" => Some(WidgetKind::Clock),
        "nowPlaying" => Some(WidgetKind::NowPlaying),
        "system" => Some(WidgetKind::System),
        "weather" => Some(WidgetKind::Weather),
        "note" => Some(WidgetKind::Note),
        "discord" => Some(WidgetKind::Discord),
        _ => None,
    }
}

fn last_key_lock() -> &'static Mutex<Option<TraySyncKey>> {
    LAST_KEY.get_or_init(|| Mutex::new(None))
}

fn current_key(app: &AppHandle) -> TraySyncKey {
    let state = app.state::<AppState>();
    let cfg = state.config.get();
    TraySyncKey {
        language: cfg.general.language,
        edit_mode: state.edit_mode.load(Ordering::Acquire),
        visible: state.widgets_visible.load(Ordering::Acquire),
        paused: state.paused.load(Ordering::Acquire),
    }
}

fn tooltip_text(lang: Language, paused: bool) -> &'static str {
    match (lang, paused) {
        (Language::Da, false) => "Open Dock",
        (Language::Da, true) => "Open Dock – pauset",
        (Language::En, false) => "Open Dock",
        (Language::En, true) => "Open Dock – paused",
    }
}

struct TrayCopy {
    open: &'static str,
    edit: &'static str,
    hide_widgets: &'static str,
    show_widgets: &'static str,
    pause: &'static str,
    add_widget: &'static str,
    clock: &'static str,
    now_playing: &'static str,
    system: &'static str,
    weather: &'static str,
    note: &'static str,
    discord: &'static str,
    all_in_one: &'static str,
    quit: &'static str,
}

fn copy_for(lang: Language) -> TrayCopy {
    match lang {
        Language::Da => TrayCopy {
            open: "Åbn Open Dock",
            edit: "Flyt widgets",
            hide_widgets: "Skjul widgets",
            show_widgets: "Vis widgets",
            pause: "Pause",
            add_widget: "Tilføj widget",
            clock: "Ur",
            now_playing: "Afspiller",
            system: "System",
            weather: "Vejr",
            note: "Note",
            discord: "Discord",
            all_in_one: "Alt-i-én Boks",
            quit: "Afslut",
        },
        Language::En => TrayCopy {
            open: "Show Open Dock",
            edit: "Edit layout",
            hide_widgets: "Hide widgets",
            show_widgets: "Show widgets",
            pause: "Pause",
            add_widget: "Add widget",
            clock: "Clock",
            now_playing: "Player",
            system: "System",
            weather: "Weather",
            note: "Note",
            discord: "Discord",
            all_in_one: "All-in-one Box",
            quit: "Quit",
        },
    }
}

fn build_check_item(
    app: &AppHandle,
    id: &str,
    text: &str,
    checked: bool,
    accelerator: Option<&str>,
) -> Result<CheckMenuItem<Wry>, AppError> {
    let mut builder = CheckMenuItemBuilder::with_id(id, text).checked(checked);
    if let Some(accel) = accelerator.filter(|s| !s.trim().is_empty()) {
        builder = builder.accelerator(accel);
    }
    match builder.build(app) {
        Ok(item) => Ok(item),
        Err(_) => Ok(CheckMenuItemBuilder::with_id(id, text)
            .checked(checked)
            .build(app)?),
    }
}

fn build_menu(app: &AppHandle, key: &TraySyncKey) -> Result<Menu<Wry>, AppError> {
    let copy = copy_for(key.language);

    let open_item = MenuItemBuilder::with_id("tray:open", copy.open).build(app)?;
    let edit_item = build_check_item(app, "tray:edit", copy.edit, key.edit_mode, None)?;
    let (visible_text, visible_checked) = if key.visible {
        (copy.hide_widgets, false)
    } else {
        (copy.show_widgets, true)
    };
    let visible_item = build_check_item(app, "tray:visible", visible_text, visible_checked, None)?;
    let pause_item = build_check_item(app, "tray:pause", copy.pause, key.paused, None)?;

    let add_clock = MenuItemBuilder::with_id("tray:add:clock", copy.clock).build(app)?;
    let add_np = MenuItemBuilder::with_id("tray:add:nowPlaying", copy.now_playing).build(app)?;
    let add_sys = MenuItemBuilder::with_id("tray:add:system", copy.system).build(app)?;
    let add_wx = MenuItemBuilder::with_id("tray:add:weather", copy.weather).build(app)?;
    let add_note = MenuItemBuilder::with_id("tray:add:note", copy.note).build(app)?;
    let add_discord = MenuItemBuilder::with_id("tray:add:discord", copy.discord).build(app)?;
    let add_sep = PredefinedMenuItem::separator(app)?;
    let add_aio =
        MenuItemBuilder::with_id("tray:add:cluster:allInOne", copy.all_in_one).build(app)?;

    let add_submenu = SubmenuBuilder::new(app, copy.add_widget)
        .item(&add_clock)
        .item(&add_np)
        .item(&add_sys)
        .item(&add_wx)
        .item(&add_note)
        .item(&add_discord)
        .item(&add_sep)
        .item(&add_aio)
        .build()?;

    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItemBuilder::with_id("tray:quit", copy.quit).build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&open_item)
        .item(&edit_item)
        .item(&visible_item)
        .item(&pause_item)
        .item(&sep1)
        .item(&add_submenu)
        .item(&sep2)
        .item(&quit_item)
        .build()?;

    Ok(menu)
}

fn tray_icon_image(paused: bool) -> Image<'static> {
    let (normal, dimmed) = TRAY_ICONS.get_or_init(render_tray_icons);
    let rgba = if paused { dimmed.clone() } else { normal.clone() };
    Image::new_owned(rgba, ICON_SIZE, ICON_SIZE)
}

fn sd_round_box(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn tray_mark_coverage(x: f32, y: f32) -> f32 {
    // Three rounded bento tiles matching the Open Dock 512x512 mark
    let tl_sd = sd_round_box(x, y, 164.0, 180.0, 48.0, 64.0, 18.0);
    let tr_sd = sd_round_box(x, y, 316.0, 180.0, 80.0, 64.0, 18.0);
    let bot_sd = sd_round_box(x, y, 256.0, 332.0, 140.0, 64.0, 18.0);
    if tl_sd.min(tr_sd).min(bot_sd) <= 0.0 {
        1.0
    } else {
        0.0
    }
}

fn render_tray_icons() -> (Vec<u8>, Vec<u8>) {
    let mut normal = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    let mut dimmed = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);

    // Map 32x32 tray canvas onto the inner 280x280 bento block centered at (256, 256)
    let view_scale = 280.0_f32 / 28.0_f32;
    let samples = (SUPERSAMPLE * SUPERSAMPLE) as f32;

    for py in 0..ICON_SIZE {
        for px in 0..ICON_SIZE {
            let mut coverage_sum = 0.0_f32;
            for sy in 0..SUPERSAMPLE {
                let y = py as f32 + (sy as f32 + 0.5) / SUPERSAMPLE as f32;
                let vy = 256.0 + (y - 16.0) * view_scale;
                for sx in 0..SUPERSAMPLE {
                    let x = px as f32 + (sx as f32 + 0.5) / SUPERSAMPLE as f32;
                    let vx = 256.0 + (x - 16.0) * view_scale;
                    coverage_sum += tray_mark_coverage(vx, vy);
                }
            }

            let alpha = ((coverage_sum / samples) * 255.0).round().clamp(0.0, 255.0) as u8;
            let dim_alpha = ((alpha as f32) * 0.40).round().clamp(0.0, 255.0) as u8;
            normal.extend_from_slice(&[255, 255, 255, alpha]);
            dimmed.extend_from_slice(&[255, 255, 255, dim_alpha]);
        }
    }

    (normal, dimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_open_dock_tray_icons() {
        let (normal, dimmed) = render_tray_icons();
        let expected_len = (ICON_SIZE * ICON_SIZE * 4) as usize;
        assert_eq!(normal.len(), expected_len);
        assert_eq!(dimmed.len(), expected_len);

        let has_opaque_white = normal
            .chunks_exact(4)
            .any(|px| px[0] == 255 && px[1] == 255 && px[2] == 255 && px[3] == 255);
        assert!(has_opaque_white);

        for (n_px, d_px) in normal.chunks_exact(4).zip(dimmed.chunks_exact(4)) {
            assert!(d_px[3] <= n_px[3]);
        }
    }
}
