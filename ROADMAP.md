# Open Dock Roadmap

Open Dock `v0.1.0` ships with 6 freeform widgets, adaptive clusters, 3 themes, frosted glass plate sampling, and a native D3D11 + DirectComposition + Media Foundation wallpaper engine. Planned work focuses on hardening the Win32 layer, adding developer/agent feeds, and expanding widget modularity.

## 1. Win32 Hardening & Multi-Desktop Resilience
- **Harden `SetWindowSubclass` & `Win+D` recovery:** Edge-case validation across virtual desktops (`IVirtualDesktopManager`), Explorer crashes, rapid monitor hotplugs, mixed-DPI coordinate transitions, and Windows 11 24H2/25H2 Raised Desktop transitions.
- **Automated Win32 integration harness:** Deterministic verification scripts for z-order invariants, `MA_NOACTIVATE` focus guarantees, and occlusion culling CPU/GPU overhead.

## 2. Developer & Agent Status Feeds
- **Coding Agent Feed widget & cluster module:** Push-based status streams for autonomous coding sessions (e.g., Devin tasks, GitHub Actions CI pipelines, PR review states) displayed cleanly with strict zero-fake-data semantics.
- **Calendar (ICS) & App Launcher:** Native desktop modules matching the existing Aurora / Lærk / Grafit glass tokens.
- **Dedicated Battery & Disk modules:** Standalone compact widgets alongside System telemetry.

## 3. Wallpaper Engine & Customization
- **Interactive & Audio-Reactive HLSL Scenes:** Low-overhead audio/pointer uniforms for built-in shaders while respecting fullscreen and occlusion pause budgets.
- **Shareable Profiles & Theme Packs (`.opendock`):** One-click switching between Work / Focus / Gaming widget layouts and theme presets.
- **Extensible Widget SDK:** Documented schema for community widgets bound through `ts-rs` and Tauri capabilities.

## 4. Platform Exploration
- **Linux (Wayland layer-shell) & macOS:** Long-term evaluation of equivalent desktop-layer anchoring abstractions outside Windows.
