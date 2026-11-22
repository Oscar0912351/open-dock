use tauri::WebviewWindow;

/// Ensartede letvægts Chromium/WebView2-argumenter for alle vinduer i Open Dock.
/// Alle vinduer skal dele præcis samme streng for at dele samme WebView2-miljø uden deadlock.
pub const LEAN_WEBVIEW_ARGS: &str = concat!(
    "--disable-background-networking ",
    "--disable-component-update ",
    "--disable-sync ",
    "--disable-default-apps ",
    "--disable-extensions ",
    "--no-first-run ",
    "--no-default-browser-check ",
    "--disable-domain-reliability ",
    "--disk-cache-size=8388608 ",
    "--gpu-disk-cache-size-kb=16384 ",
    "--renderer-process-limit=2 ",
    "--disable-features=msSmartScreenProtection,msEdgeSpellcheck,OptimizationHints,MediaRouter,Translate,BackForwardCache,InterestFeedContentSuggestions,AutofillServerCommunication,HeavyAdIntervention,PrivacySandboxSettings4"
);

#[cfg(target_os = "windows")]
pub fn configure_widget_webview(window: &WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Settings5, ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
    };
    use windows::core::Interface;

    let _ = window.with_webview(|wv| unsafe {
        let Ok(core) = wv.controller().CoreWebView2() else {
            return;
        };
        if let Ok(mem) = core.cast::<ICoreWebView2_19>() {
            let _ = mem.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
        }
        if let Ok(settings) = core.Settings() {
            let _ = settings.SetAreDefaultContextMenusEnabled(false);
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetIsZoomControlEnabled(false);
            let _ = settings.SetIsBuiltInErrorPageEnabled(false);
            if let Ok(settings5) = settings.cast::<ICoreWebView2Settings5>() {
                let _ = settings5.SetIsPinchZoomEnabled(false);
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn configure_widget_webview(_window: &WebviewWindow) {}

#[cfg(target_os = "windows")]
pub fn set_webview_suspended(window: &WebviewWindow, suspended: bool) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::ICoreWebView2_3, TrySuspendCompletedHandler,
    };
    use windows::core::Interface;

    let _ = window.with_webview(move |wv| unsafe {
        let Ok(core) = wv.controller().CoreWebView2() else {
            return;
        };
        let Ok(core3) = core.cast::<ICoreWebView2_3>() else {
            return;
        };
        if suspended {
            let handler = TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())));
            let _ = core3.TrySuspend(&handler);
        } else {
            let _ = core3.Resume();
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn set_webview_suspended(_window: &WebviewWindow, _suspended: bool) {}
