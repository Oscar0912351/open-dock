pub mod control;
pub mod menu;
pub mod overlay;
pub mod webview_opt;
pub mod widget;

pub fn apply_webview_defaults<'a, R: tauri::Runtime, M: tauri::Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
) -> tauri::WebviewWindowBuilder<'a, R, M> {
    #[cfg(target_os = "windows")]
    {
        builder.additional_browser_args(webview_opt::LEAN_WEBVIEW_ARGS)
    }
    #[cfg(not(target_os = "windows"))]
    {
        builder
    }
}
