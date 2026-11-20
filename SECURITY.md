# Security Policy

## Reporting a Vulnerability

If you discover a security issue in **Open Dock** (for example, an IPC/CSP bypass, path traversal in wallpaper/asset loading, privilege-escalation hazard, or Win32 handle issue), please **do not open a public GitHub issue**.

Instead, report it privately via **GitHub Security Advisories** (`Security` → `Advisories` → `Report a vulnerability` on this repository). Please include reproduction steps, affected Windows version, and impact assessment.

## Security Architecture Notes

- **Zero accounts & zero telemetry:** Open Dock never collects analytics, crash dumps, or user identifiers.
- **Strict Rust-only network boundary:** Frontend WebViews enforce a restrictive Tauri CSP (`script-src 'self'`, no external `fetch`/`connect-src`). All outbound HTTP calls execute exclusively in Rust (`reqwest` with Windows native SChannel TLS) for:
  - Weather geocoding & forecasts via [Open-Meteo](https://open-meteo.com/).
  - On-demand caching of user-selected curated 4K wallpapers (`images.unsplash.com`) into `$APPCACHE/wallpapers/`.
- **Optional UAC elevation for hardware sensors:** `src-tauri/src/elevation.rs` supports auto-elevating at startup to enable ring-0 CPU package temperature reading via `OpenDock.CpuSensors.exe` (when the separately installed PawnIO driver is present). If UAC is declined, Open Dock runs unprivileged and cleanly omits CPU temperature.
- **Pinned upstream sensor binaries:** Every third-party DLL in `src-tauri/resources/cpu-sensor/` is verified against its pinned SHA-256 checksum inside `src-tauri/build.rs` against `libraries.json` at compile time.
