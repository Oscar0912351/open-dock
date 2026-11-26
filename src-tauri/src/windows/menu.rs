use std::sync::atomic::Ordering;

use tauri::{
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
    AppHandle, Manager,
};
use uuid::Uuid;

use crate::{
    commands::widgets::{apply_widget_update, remove_widget_by_id},
    config::schema::{Language, Layer, WidgetKind, WidgetPatch, WidgetSize},
    error::AppError,
    state::AppState,
    windows::widget,
};

struct MenuCopy {
    edit: &'static str,
    size: &'static str,
    narrow: &'static str,
    standard: &'static str,
    wide: &'static str,
    layer: &'static str,
    desktop: &'static str,
    top: &'static str,
    settings: &'static str,
    remove: &'static str,
    devtools: &'static str,
}

fn copy_for(lang: Language) -> MenuCopy {
    match lang {
        Language::Da => MenuCopy {
            edit: "Flyt widgets",
            size: "Størrelse",
            narrow: "Smal (232 px)",
            standard: "Standard (264 px)",
            wide: "Bred (288 px)",
            layer: "Lag",
            desktop: "På skrivebordet",
            top: "Altid øverst",
            settings: "Indstillinger…",
            remove: "Fjern widget",
            devtools: "Åbn DevTools",
        },
        Language::En => MenuCopy {
            edit: "Edit layout",
            size: "Size",
            narrow: "Narrow (232 px)",
            standard: "Standard (264 px)",
            wide: "Wide (288 px)",
            layer: "Layer",
            desktop: "On the desktop",
            top: "Always on top",
            settings: "Widget settings…",
            remove: "Remove widget",
            devtools: "Open DevTools",
        },
    }
}

pub fn show_widget_context_menu(app: &AppHandle, id: &str) -> Result<(), AppError> {
    let uuid = Uuid::parse_str(id).map_err(|_| AppError::WidgetNotFound)?;
    let state = app.state::<AppState>();
    let config = state.config.get();
    let copy = copy_for(config.general.language);
    let instance = config
        .widgets
        .into_iter()
        .find(|w| w.id == uuid)
        .ok_or(AppError::WidgetNotFound)?;

    let window = app
        .get_webview_window(&instance.window_label())
        .ok_or(AppError::WidgetNotFound)?;
    let edit_on = state.edit_mode.load(Ordering::Acquire);

    let edit_item = CheckMenuItemBuilder::with_id(format!("ctx:{uuid}:edit"), copy.edit)
        .checked(edit_on)
        .build(app)?;

    let mut size_menu_builder = SubmenuBuilder::new(app, copy.size);
    if instance.kind == WidgetKind::Cluster {
        let cur_w = instance.cluster.as_ref().map(|c| c.width).unwrap_or(264);
        for (width, label) in [
            (232_u32, copy.narrow),
            (264_u32, copy.standard),
            (288_u32, copy.wide),
        ] {
            let item = CheckMenuItemBuilder::with_id(
                format!("ctx:{uuid}:cwidth:{width}"),
                label,
            )
            .checked(cur_w == width)
            .build(app)?;
            size_menu_builder = size_menu_builder.item(&item);
        }
    } else {
        for &size in instance.kind.allowed_sizes() {
            let item = CheckMenuItemBuilder::with_id(
                format!("ctx:{uuid}:size:{}", size_key(size)),
                size_label(config.general.language, size),
            )
            .checked(instance.size == size)
            .build(app)?;
            size_menu_builder = size_menu_builder.item(&item);
        }
    }
    let size_menu = size_menu_builder.build()?;

    let layer_menu = SubmenuBuilder::new(app, copy.layer)
        .item(
            &CheckMenuItemBuilder::with_id(format!("ctx:{uuid}:layer:desktop"), copy.desktop)
                .checked(instance.layer == Layer::Desktop)
                .build(app)?,
        )
        .item(
            &CheckMenuItemBuilder::with_id(format!("ctx:{uuid}:layer:top"), copy.top)
                .checked(instance.layer == Layer::Top)
                .build(app)?,
        )
        .build()?;

    let settings_item = MenuItemBuilder::with_id(
        format!("ctx:{uuid}:settings"),
        copy.settings,
    )
    .build(app)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let remove_item =
        MenuItemBuilder::with_id(format!("ctx:{uuid}:remove"), copy.remove).build(app)?;
    let devtools_separator = PredefinedMenuItem::separator(app)?;
    let devtools_item =
        MenuItemBuilder::with_id(format!("ctx:{uuid}:devtools"), copy.devtools).build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&edit_item)
        .item(&size_menu)
        .item(&layer_menu)
        .item(&settings_item)
        .item(&separator)
        .item(&remove_item)
        .item(&devtools_separator)
        .item(&devtools_item)
        .build()?;

    window.popup_menu(&menu)?;
    Ok(())
}

pub fn handle_event(app: &AppHandle, menu_id: &str) {
    let mut parts = menu_id.split(':');
    let Some("ctx") = parts.next() else {
        return;
    };
    let Some(id_str) = parts.next() else {
        return;
    };
    let Some(action) = parts.next() else {
        return;
    };
    let Ok(uuid) = Uuid::parse_str(id_str) else {
        return;
    };

    match action {
        "edit" => {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let next = !handle.state::<AppState>().edit_mode.load(Ordering::Acquire);
                if let Err(error) = crate::desktop::drag::set_edit_mode(&handle, next) {
                    log::warn!("Menu-redigering fejlede: {error:?}");
                }
            });
        }
        "size" => {
            let Some(raw) = parts.next() else { return };
            let Some(size) = parse_size(raw) else { return };
            if let Err(error) = apply_widget_update(
                app,
                uuid,
                WidgetPatch {
                    size: Some(size),
                    ..Default::default()
                },
            ) {
                log::warn!("Menu-størrelse fejlede for {uuid}: {error:?}");
            }
        }
        "cwidth" => {
            let Some(raw) = parts.next() else { return };
            let Ok(width) = raw.parse::<u32>() else { return };
            if !matches!(width, 232 | 264 | 288) {
                return;
            }
            let result = app.state::<AppState>().config.mutate(|cfg| {
                let Some(target) = cfg.widgets.iter_mut().find(|w| w.id == uuid) else {
                    return Err(AppError::WidgetNotFound);
                };
                if let Some(cluster) = target.cluster.as_mut() {
                    cluster.width = width;
                }
                Ok(target.clone())
            });
            match result {
                Ok(updated) => {
                    {
                        let state = app.state::<AppState>();
                        let mut sizes = state.cluster_sizes.write();
                        if let Some(entry) = sizes.get_mut(&uuid) {
                            entry.0 = width as f64;
                        }
                    }
                    widget::apply_instance_live(app, &updated);
                }
                Err(error) => log::warn!("Menu-klyngebredde fejlede for {uuid}: {error:?}"),
            }
        }
        "layer" => {
            let Some(raw) = parts.next() else { return };
            let Some(layer) = parse_layer(raw) else { return };
            if let Err(error) = apply_widget_update(
                app,
                uuid,
                WidgetPatch {
                    layer: Some(layer),
                    ..Default::default()
                },
            ) {
                log::warn!("Menu-lag fejlede for {uuid}: {error:?}");
            }
        }
        "settings" => {
            if let Err(error) =
                crate::windows::control::open(app, Some(format!("/desktop?select={uuid}")))
            {
                log::warn!("Kunne ikke åbne kontrolcentret fra menu: {error:?}");
            }
        }
        "remove" => {
            if let Err(error) = remove_widget_by_id(app, uuid) {
                log::warn!("Menu-fjern fejlede for {uuid}: {error:?}");
            }
        }
        "devtools" => {
            if let Some(window) = app.get_webview_window(&format!("w-{uuid}")) {
                window.open_devtools();
            }
        }
        _ => {}
    }
}

fn size_key(size: WidgetSize) -> &'static str {
    match size {
        WidgetSize::Pill => "pill",
        WidgetSize::S => "s",
        WidgetSize::M => "m",
        WidgetSize::L => "l",
    }
}

fn size_label(lang: Language, size: WidgetSize) -> &'static str {
    match (lang, size) {
        (Language::Da, WidgetSize::Pill) => "Kapsel",
        (Language::Da, WidgetSize::S) => "Lille",
        (Language::Da, WidgetSize::M) => "Mellem",
        (Language::Da, WidgetSize::L) => "Stor",
        (Language::En, WidgetSize::Pill) => "Pill",
        (Language::En, WidgetSize::S) => "Small",
        (Language::En, WidgetSize::M) => "Medium",
        (Language::En, WidgetSize::L) => "Large",
    }
}

fn parse_size(raw: &str) -> Option<WidgetSize> {
    match raw {
        "pill" => Some(WidgetSize::Pill),
        "s" => Some(WidgetSize::S),
        "m" => Some(WidgetSize::M),
        "l" => Some(WidgetSize::L),
        _ => None,
    }
}

fn parse_layer(raw: &str) -> Option<Layer> {
    match raw {
        "desktop" => Some(Layer::Desktop),
        "top" => Some(Layer::Top),
        _ => None,
    }
}
