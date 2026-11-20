# Contributing to Open Dock

Thanks for helping improve **Open Dock**. Contributions focused on Win32 desktop hardening, low-overhead system providers, shader/wallpaper engine performance, and accessible UI polish are especially welcome.

## Prerequisites (Windows)

- **Node.js 24+** (`npm`)
- **Rust 1.95+** (`x86_64-pc-windows-msvc` toolchain)
- **Visual Studio Build Tools** with C++ desktop workload & Windows 11 SDK
- **WebView2 Runtime** (pre-installed on Windows 10/11)

## Local Development

```powershell
# Install frontend packages
npm install

# Launch Vite + Tauri desktop app in dev mode
npm run dev:tauri
```

## Verification Checks

Run the full suite before opening a pull request:

```powershell
# Regenerate TypeScript bindings from Rust (ts-rs) when modifying IPC structs/enums
npm run bindings

# Frontend checks
npx tsc --noEmit
npm run lint
npm test

# Rust checks & unit tests
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

## Architectural Rules

1. **Rust owns state:** Runtime config (`config.json`), provider subscriptions, window layering, and wallpaper surfaces are owned in Rust (`src-tauri/src/`). Avoid duplicating authoritative state in frontend `localStorage`.
2. **Push-based IPC — zero polling:** Providers subscribe/unsubscribe via `providers::scheduler::Scheduler` and emit Tauri events only on change or hardware cadence. Do not add idle `requestAnimationFrame` loops in React widgets.
3. **Never fabricate metrics:** If a hardware sensor (CPU package temperature, NVIDIA NVML, etc.) or media field is unavailable on the host machine, report `None`/`null` from Rust and render `—` or omit the field cleanly in the UI.
4. **Type-safe IPC:** Annotate shared Rust payload types with `#[derive(TS)]` and regenerate `src/shared/bindings/` with `npm run bindings` rather than hand-writing duplicate TypeScript interfaces.
5. **Win32 subclassing safety:** Maintain proper `SetWindowSubclass` / `RemoveWindowSubclass` cleanup on `WM_NCDESTROY`, preserve `HWND` lifetime guarantees, and avoid blocking the UI message loop.
6. **Zero runtime panics:** Avoid `.unwrap()` / `.expect()` in Rust runtime paths (return `Result<_, AppError>` and log with context).

## Pull Requests

- Keep PRs focused and well-scoped.
- Describe the Windows version / display setup (single vs multi-monitor, DPI scale) you tested on, especially for `src-tauri/src/desktop/` or `src-tauri/src/wallpaper/` changes.
