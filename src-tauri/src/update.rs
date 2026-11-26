use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};
use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::{
    config::schema::{ControlTheme, Language},
    error::AppError,
    state::AppState,
    util::unix_now_s,
};

const MANIFEST_URL: &str =
    "https://github.com/Oscar0912351/open-dock/releases/latest/download/latest.json";
const STARTUP_CHECK_DELAY: Duration = Duration::from_secs(15);
/// Installeren er lille nok til at downloade på under et sekund — hold
/// download-staten synlig lidt, så indikatoren ikke blinker forbi uset.
const MIN_DOWNLOAD_DISPLAY: Duration = Duration::from_millis(1800);

/// Offentlig ed25519-nøgle (hex) som opdateringssignaturer verificeres mod.
/// Indlejret ved compile-time — en angriber der kun kontrollerer GitHub-
/// releasen (manifest + installer) kan ikke fremstille en gyldig signatur
/// uden privatnøglen, som ligger på udviklerens maskine uden for repoet
/// (%USERPROFILE%/.open-dock/update-signing-key.pem). Nøglen genereres med
/// scripts/generate-update-key.mjs; mistes den, kan eksisterende installationer
/// ikke opdateres længere — behold en sikker backup af privatnøglen.
const UPDATE_PUBLIC_KEY_HEX: &str =
    "f8403449d1bc826bdd204d77cdf099e0a7a14069f1089b23f10cfa01266d33dc";

/// Cached result of the last update check, managed as Tauri state.
pub struct UpdateTracker {
    pub last: Mutex<Option<UpdateStatus>>,
    /// Verificeret, downloadet installer klar til silent install: (build_id, sti).
    pub staged: Mutex<Option<(i64, PathBuf)>>,
    /// Sandt mens et tjek eller en download er i gang — forhindrer dobbelt-arbejde.
    pub working: AtomicBool,
}

impl Default for UpdateTracker {
    fn default() -> Self {
        Self {
            last: Mutex::new(None),
            staged: Mutex::new(None),
            working: AtomicBool::new(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct ReleaseManifest {
    pub version: String,
    pub build_id: i64,
    pub url: String,
    pub sha256: String,
    /// Detached ed25519-signatur (base64) over installer-bytesene, lavet med
    /// privatnøglen ved release-tidspunktet. Manifestet er kun transport —
    /// tilliden kommer fra den indlejrede offentlige nøgle, ikke kanalen.
    #[serde(default)]
    pub signature: String,
    pub size: u64,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum UpdateCheckState {
    /// Lokalt build uden build-id (dev/kilde-build) — opdateringstjek er slået fra.
    Dev,
    UpToDate,
    /// Et nyt build findes — download starter straks automatisk.
    Available,
    /// Installeren downloades i baggrunden (se update://progress for procent).
    Downloading,
    /// Installeren er hentet og verificeret — klar til genstart + silent install.
    Ready,
    /// Download eller verificering slog fejl — prøv igen via et nyt tjek.
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct UpdateStatus {
    pub state: UpdateCheckState,
    pub current_build_id: Option<i64>,
    pub version: String,
    pub latest: Option<ReleaseManifest>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: u64,
}

/// Build-ID indlejret af scripts/build-release.mjs. Mangler i dev-/lokale builds.
fn local_build_id() -> Option<i64> {
    option_env!("OPEN_DOCK_BUILD_ID")
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
}

async fn fetch_manifest(client: &Client) -> Result<ReleaseManifest, AppError> {
    let response = client
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(AppError::Network)?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::Other(
            "Der er ingen versioner at hente endnu".into(),
        ));
    }
    if !status.is_success() {
        return Err(AppError::Other(format!("Manifestet svarede med HTTP {status}")));
    }
    response.json().await.map_err(AppError::Network)
}

/// Henter manifestet og sammenligner remote build-id med det indlejrede.
pub async fn check() -> Result<UpdateStatus, AppError> {
    let current = local_build_id();
    let version = env!("CARGO_PKG_VERSION").to_string();
    let checked_at = unix_now_s();

    let Some(build_id) = current else {
        return Ok(UpdateStatus {
            state: UpdateCheckState::Dev,
            current_build_id: None,
            version,
            latest: None,
            checked_at,
        });
    };

    let manifest = fetch_manifest(crate::util::http_client()).await?;
    let state = if manifest.build_id > build_id {
        UpdateCheckState::Available
    } else {
        UpdateCheckState::UpToDate
    };
    log::info!(
        target: "update",
        "Opdateringstjek: lokal build {build_id}, remote {} -> {state:?}",
        manifest.build_id
    );
    Ok(UpdateStatus {
        state,
        current_build_id: Some(build_id),
        version,
        latest: Some(manifest),
        checked_at,
    })
}

/// Den kanoniske besked der signeres ved release: domæne-separeret og med
/// build-id bundet til exe-hashen. Uden bindingen kunne en angriber på GitHub
/// servere en gammel, legitimt signeret build med et højere build-id i
/// manifestet og dermed nedgradere brugere (rollback-angreb).
fn signed_message(build_id: i64, sha256: &str) -> Vec<u8> {
    format!("open-dock-update-v1\n{build_id}\n{sha256}\n").into_bytes()
}

/// Verificerer en ed25519-signatur (base64, detached) over bytes mod den
/// compile-time indlejrede offentlige nøgle. Det er dét der reelt beskytter
/// opdateringen — SHA-256 i manifestet dækker kun transportfejl, fordi både
/// manifest og installer hostes samme sted.
fn verify_signature(bytes: &[u8], signature_b64: &str) -> bool {
    let Ok(key_bytes) = hex_decode_32(UPDATE_PUBLIC_KEY_HEX) else {
        log::error!(target: "update", "Indlejret opdateringsnøgle er ugyldig hex");
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else {
        log::error!(target: "update", "Indlejret opdateringsnøgle er ikke en ed25519-nøgle");
        return false;
    };
    let Ok(sig_bytes) = B64.decode(signature_b64.trim()) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&sig_bytes) else {
        return false;
    };
    key.verify_strict(bytes, &signature).is_ok()
}

fn hex_decode_32(hex: &str) -> Result<[u8; 32], ()> {
    if hex.len() != 64 {
        return Err(());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| ())?;
    }
    Ok(out)
}

/// Fuld integritetsverificering af en downloadet installer: SHA-256 (transport-
/// fejl) OG ed25519-signatur (ægthed). Begge skal passere.
fn verify_installer_bytes(bytes: &[u8], manifest: &ReleaseManifest) -> Result<(), AppError> {
    let digest = format!("{:x}", Sha256::digest(bytes));
    if !digest.eq_ignore_ascii_case(&manifest.sha256) {
        log::warn!(target: "update", "SHA-256 mismatch: forventede {}, fik {digest}", manifest.sha256);
        return Err(AppError::Other(
            "Integritetstjek fejlede for den hentede fil".into(),
        ));
    }
    if manifest.signature.is_empty() {
        log::warn!(target: "update", "Manifestet har ingen signatur — afviser opdateringen");
        return Err(AppError::Other(
            "Opdateringen kunne ikke bekræftes som ægte, så den bliver ikke installeret".into(),
        ));
    }
    let message = signed_message(manifest.build_id, &digest);
    if !verify_signature(&message, &manifest.signature) {
        log::warn!(target: "update", "ed25519-signatur verificerede ikke for build {}", manifest.build_id);
        return Err(AppError::Other(
            "Opdateringen kunne ikke bekræftes som ægte, så den bliver ikke installeret".into(),
        ));
    }
    Ok(())
}

/// Downloader installeren fra manifestet, verificerer SHA-256 + ed25519-signatur
/// og returnerer stien. Bytesene holdes i memory under download og skrives FØRST
/// til disk efter fuld verificering — der eksisterer aldrig en uverificeret
/// installer på disken under det endelige filnavn.
async fn download_installer(app: &AppHandle, manifest: &ReleaseManifest) -> Result<PathBuf, AppError> {
    if manifest.url.is_empty() || manifest.sha256.is_empty() || manifest.signature.is_empty() {
        return Err(AppError::Other(
            "Manifestet mangler url, sha256 eller signatur".into(),
        ));
    }
    let path = std::env::temp_dir().join(format!("open-dock-update-{}.exe", manifest.build_id));
    // En tidligere session kan have hentet samme build — genbrug hvis den
    // stadig verificerer (hash + signatur) mod manifestet.
    if path.exists() {
        let cached = fs::read(&path).ok();
        if let Some(bytes) = cached {
            if verify_installer_bytes(&bytes, manifest).is_ok() {
                log::info!(target: "update", "Genbruger hentet installer {:?}", path);
                return Ok(path);
            }
        }
        let _ = fs::remove_file(&path);
    }
    // Ryd ældre builds op — filnavnet er per build-id og ellers hobles de op i TEMP.
    if let Ok(entries) = fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if entry.path() != path
                && s.starts_with("open-dock-update-")
                && (s.ends_with(".exe") || s.ends_with(".download"))
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    let mut response = crate::util::http_download_client()
        .get(&manifest.url)
        .send()
        .await
        .map_err(AppError::Network)?;
    if !response.status().is_success() {
        return Err(AppError::Other(format!(
            "Download svarede med HTTP {}",
            response.status()
        )));
    }

    let total = manifest.size.max(response.content_length().unwrap_or(0));
    let mut bytes: Vec<u8> = Vec::with_capacity(total.min(64 * 1024 * 1024) as usize);
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;

    while let Some(chunk) = response.chunk().await.map_err(AppError::Network)? {
        bytes.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;
        if downloaded - last_emit > 256 * 1024 || downloaded == total {
            last_emit = downloaded;
            let _ = app.emit("update://progress", UpdateProgress { downloaded, total });
        }
    }

    verify_installer_bytes(&bytes, manifest)?;
    fs::write(&path, &bytes)
        .map_err(|err| AppError::Other(format!("Kunne ikke færdiggøre opdateringsfilen: {err}")))?;
    Ok(path)
}

/// Gemmer status i trackeren og broadcaster den til frontend.
fn set_status(app: &AppHandle, status: &UpdateStatus) {
    if let Some(tracker) = app.try_state::<UpdateTracker>() {
        *tracker.last.lock() = Some(status.clone());
    }
    if let Err(err) = app.emit("update://status", status) {
        log::warn!(target: "update", "Kunne ikke sende update://status: {err}");
    }
}

/// Henter manifest, sammenligner build-id og downloader automatisk i baggrunden
/// hvis et nyt build findes. Bruges af både update_check-kommandoen og
/// opstartstjekket — resultatet ender i Ready (eller Failed) og spejles i
/// update://status-events hele vejen.
pub async fn check_and_stage(app: &AppHandle) -> Result<UpdateStatus, AppError> {
    let Some(tracker) = app.try_state::<UpdateTracker>() else {
        return check().await;
    };
    if tracker.working.swap(true, Ordering::SeqCst) {
        if let Some(last) = tracker.last.lock().clone() {
            return Ok(last);
        }
        return check().await;
    }
    let result = run_cycle(app).await;
    tracker.working.store(false, Ordering::SeqCst);
    result
}

async fn run_cycle(app: &AppHandle) -> Result<UpdateStatus, AppError> {
    let mut status = check().await?;
    set_status(app, &status);
    if status.state != UpdateCheckState::Available {
        return Ok(status);
    }
    let Some(manifest) = status.latest.clone() else {
        return Ok(status);
    };
    let Some(tracker) = app.try_state::<UpdateTracker>() else {
        return Ok(status);
    };

    // Samme build allerede hentet og verificeret → gå direkte til Ready.
    if let Some((build_id, path)) = tracker.staged.lock().clone() {
        if build_id == manifest.build_id && path.exists() {
            status.state = UpdateCheckState::Ready;
            set_status(app, &status);
            return Ok(status);
        }
    }

    status.state = UpdateCheckState::Downloading;
    set_status(app, &status);
    let download_started = Instant::now();
    let mut downloaded = download_installer(app, &manifest).await;
    if downloaded.is_err() {
        // Transient netfejl dræbte auto-downloaden her (body-timeout midt i
        // streamen) — ét automatisk genforsøg før vi overhovedet melder fejl.
        log::info!(target: "update", "Download fejlede — genforsøger om 3s");
        tokio::time::sleep(Duration::from_secs(3)).await;
        downloaded = download_installer(app, &manifest).await;
    }
    match downloaded {
        Ok(path) => {
            let elapsed = download_started.elapsed();
            if elapsed < MIN_DOWNLOAD_DISPLAY {
                tokio::time::sleep(MIN_DOWNLOAD_DISPLAY - elapsed).await;
            }
            log::info!(target: "update", "Opdatering b{} hentet til {:?}", manifest.build_id, path);
            *tracker.staged.lock() = Some((manifest.build_id, path));
            status.state = UpdateCheckState::Ready;
            status.checked_at = unix_now_s();
            set_status(app, &status);
            notify_ready(app, &manifest.version);
            Ok(status)
        }
        Err(err) => {
            log::warn!(target: "update", "Download af opdatering fejlede: {err:?}");
            status.state = UpdateCheckState::Failed;
            set_status(app, &status);
            Err(err)
        }
    }
}

/// Metadata givet videre til det frakoblede opdaterings-splash, så det
/// matcher brugerens aktive tema, sprog og faktiske målversion.
struct SplashMeta {
    theme: ControlTheme,
    language: Language,
    version_badge: String,
}

fn format_version_badge(target: Option<&ReleaseManifest>) -> String {
    let current_ver = env!("CARGO_PKG_VERSION").trim();
    match target {
        Some(m) => {
            let target_ver = m.version.trim();
            if !target_ver.is_empty() && target_ver != current_ver {
                format!("v{current_ver} → v{target_ver}")
            } else if !target_ver.is_empty() {
                format!("v{target_ver}")
            } else {
                format!("v{current_ver}")
            }
        }
        None => format!("v{current_ver}"),
    }
}

fn collect_splash_meta(app: &AppHandle, target: Option<&ReleaseManifest>) -> SplashMeta {
    let (theme, language) = app
        .try_state::<AppState>()
        .map(|s| {
            let cfg = s.config.get();
            (cfg.general.control_theme, cfg.general.language)
        })
        .unwrap_or((ControlTheme::Dark, Language::Da));
    SplashMeta {
        theme,
        language,
        version_badge: format_version_badge(target),
    }
}

/// Genstarter appen ind i den nye version: kører den verificerede installer
/// silent (/S — ingen "allerede installeret"-dialog ved samme versionsnummer)
/// via et frakoblet script der viser en statusboks undervejs og starter appen
/// igen bagefter.
pub async fn install(app: &AppHandle) -> Result<(), AppError> {
    if local_build_id().is_none() {
        return Err(AppError::Other(
            "Opdateringer virker kun i den installerede app".into(),
        ));
    }
    let tracker = app.try_state::<UpdateTracker>();
    if let Some(t) = &tracker {
        if t.working.swap(true, Ordering::SeqCst) {
            return Err(AppError::Other(
                "Vent, til opdateringen er hentet".into(),
            ));
        }
    }
    let staged = tracker
        .as_ref()
        .and_then(|t| t.staged.lock().clone());
    let cached_manifest = tracker
        .as_ref()
        .and_then(|t| t.last.lock().clone())
        .and_then(|s| s.latest);

    let result = async {
        let (installer, manifest) = match staged {
            Some((_, path)) if path.exists() => (path, cached_manifest),
            _ => {
                let manifest = fetch_manifest(crate::util::http_client()).await?;
                let path = download_installer(app, &manifest).await?;
                (path, Some(manifest))
            }
        };
        let meta = collect_splash_meta(app, manifest.as_ref());
        spawn_silent_install(&installer, &meta)?;
        crate::commands::app::quit_app(app);
        Ok(())
    }
    .await;

    if let Some(t) = &tracker {
        t.working.store(false, Ordering::SeqCst);
    }
    result
}

/// Starter den frakoblede opdateringskæde: status-splashen startes direkte
/// herfra (så den er synlig mens appen lukker — ingen blank pause), og et
/// midlertidigt PowerShell-script venter på at appen lukker, kører NSIS
/// silent (/S — ingen "allerede installeret"-dialog ved samme versionsnummer)
/// og genstarter det installerede build. Splashen læser sin tilstand fra en
/// stage-fil og lukker selv; fejler den, kører installationen stadig som før.
fn spawn_silent_install(installer: &Path, meta: &SplashMeta) -> Result<(), AppError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let app_exe = std::env::current_exe()
        .map_err(|err| AppError::Other(format!("Kunne ikke finde appens exe-sti: {err}")))?;
    let install_dir = crate::paths::data_root()
        .parent()
        .map(|local| local.join("Open Dock"));
    let relaunch = match &install_dir {
        Some(dir) => {
            let candidate = dir.join(app_exe.file_name().unwrap_or_default());
            if app_exe.starts_with(dir) {
                app_exe.clone()
            } else if candidate.exists() {
                candidate
            } else {
                app_exe.clone()
            }
        }
        None => app_exe.clone(),
    };

    let update_root = crate::paths::data_root().join("update");
    cleanup_stale_update_runs(&update_root);
    let run_dir = update_root.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&run_dir).map_err(|err| {
        AppError::Other(format!("Kunne ikke oprette opdateringsmappe: {err}"))
    })?;
    let splash_path = run_dir.join("splash.ps1");
    let stage_path = run_dir.join("stage.txt");
    let script_path = run_dir.join("run.ps1");
    // Stage-filen skal findes før splashen starter, så dens første poll
    // læser "closing" frem for at falde tilbage på default.
    if let Err(err) = write_new_file(&stage_path, "closing") {
        let _ = fs::remove_dir_all(&run_dir);
        return Err(err);
    }
    let splash = build_splash_script(&stage_path, meta);
    if let Err(err) = write_new_file(&splash_path, &splash) {
        let _ = fs::remove_dir_all(&run_dir);
        return Err(err);
    }
    let script = update_script(
        installer,
        &relaunch,
        std::process::id(),
        &stage_path,
        &splash_path,
        &run_dir,
    );
    if let Err(err) = write_new_file(&script_path, &script) {
        let _ = fs::remove_dir_all(&run_dir);
        return Err(err);
    }

    log::info!(
        target: "update",
        "Silent install {:?} og genstart af {:?}",
        installer,
        relaunch
    );

    let ps_args = |script: &Path| -> Command {
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(script)
        .creation_flags(CREATE_NO_WINDOW);
        cmd
    };

    // Splashen startes FØR appen lukker, så den når at boote (PS + WPF ≈ 1s)
    // imens vinduerne forsvinder — den viser "Lukker…" næsten med det samme.
    // Kan den ikke starte, fortsætter installationen bare uden statusboks.
    let mut splash_child = match ps_args(&splash_path).spawn() {
        Ok(child) => Some(child),
        Err(err) => {
            log::warn!(target: "update", "Kunne ikke starte status-splash: {err}");
            None
        }
    };

    if let Err(err) = ps_args(&script_path).spawn() {
        if let Some(child) = splash_child.as_mut() {
            let _ = child.kill();
        }
        let _ = fs::remove_dir_all(&run_dir);
        return Err(AppError::Other(format!(
            "Kunne ikke starte installationsprogrammet: {err}"
        )));
    }
    Ok(())
}

fn cleanup_stale_update_runs(update_root: &Path) {
    let Ok(entries) = fs::read_dir(update_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(24 * 60 * 60));
        if stale {
            if let Err(error) = fs::remove_dir_all(&path) {
                log::warn!(target: "update", "Kunne ikke rydde gammel opdateringsmappe {:?}: {error}", path);
            }
        }
    }
}

fn write_new_file(path: &Path, content: &str) -> Result<(), AppError> {
    use std::io::Write;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| AppError::Other(format!("Kunne ikke skrive opdateringsscript: {err}")))?;
    file.write_all(content.as_bytes())
        .map_err(|err| AppError::Other(format!("Kunne ikke skrive opdateringsscript: {err}")))
}

fn ps_escape(value: &str) -> String {
    value.replace('\'', "''")
}

/// Bygger PowerShell-scriptet til splash-vinduet med UTF-8 BOM, så Windows
/// PowerShell 5.1 læser danske tegn og pilsymboler korrekt.
fn build_splash_script(stage_file: &Path, meta: &SplashMeta) -> String {
    let theme = match meta.theme {
        ControlTheme::Light => "light",
        ControlTheme::Dark => "dark",
    };
    let lang = match meta.language {
        Language::En => "en",
        Language::Da => "da",
    };
    let stage_str = stage_file.display().to_string();
    let body = SPLASH_SCRIPT
        .replace("__STAGE_FILE__", &ps_escape(&stage_str))
        .replace("__THEME__", theme)
        .replace("__LANG__", lang)
        .replace("__VERSION_BADGE__", &ps_escape(&meta.version_badge));
    format!("\u{feff}{body}")
}

/// Bygger PowerShell-scriptet til opdateringskæden. UTF-8 BOM præfikses så
/// Windows PowerShell 5.1 parser danske tegn korrekt.
fn update_script(
    installer: &Path,
    relaunch: &Path,
    app_pid: u32,
    stage: &Path,
    splash: &Path,
    run_dir: &Path,
) -> String {
    let esc = |p: &Path| ps_escape(&p.display().to_string());
    let body = UPDATE_SCRIPT
        .replace("__APP_PID__", &app_pid.to_string())
        .replace("__INSTALLER__", &esc(installer))
        .replace("__RELAUNCH__", &esc(relaunch))
        .replace("__STAGE_FILE__", &esc(stage))
        .replace("__SPLASH_SCRIPT__", &esc(splash))
        .replace("__RUN_DIR__", &esc(run_dir));
    format!("\u{feff}{body}")
}

/// Skulpturelt opdateringsvindue i Open Dock's eget designsprog: selve
/// app-ikonets form med Studio dot-grid, tre svævende bento-brikker og én
/// hårfin fremgangslinje. Kører som sin egen powershell-proces med 3-min
/// sikkerhedsnet.
const SPLASH_SCRIPT: &str = include_str!("update_splash.ps1");

const UPDATE_SCRIPT: &str = include_str!("update_runner.ps1");

/// Kører ét opdateringstjek kort efter opstart (kun release-builds med build-id).
/// Nye builds downloades automatisk; notifikationen kommer først når filen er
/// verificeret og klar — så er genstart-knappen i toppen allerede synlig.
pub fn install_background_check(app: &AppHandle) {
    if local_build_id().is_none() || cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_CHECK_DELAY).await;
        if let Err(err) = check_and_stage(&app).await {
            log::info!(target: "update", "Baggrundstjek for opdatering fejlede: {err:?}");
        }
    });
}

fn notify_ready(app: &AppHandle, remote_version: &str) {
    use tauri_plugin_notification::NotificationExt;
    let body = if remote_version.is_empty() {
        "Opdateringen er klar. Genstart Open Dock for at få den.".to_string()
    } else {
        format!(
            "Open Dock {remote_version} er hentet og klar — genstart for at installere opdateringen."
        )
    };
    let result = app
        .notification()
        .builder()
        .title("Open Dock")
        .body(body)
        .show();
    if let Err(err) = result {
        log::warn!(target: "update", "Kunne ikke vise opdateringsnotifikation: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cross-kompatibilitetsvektor genereret med node:crypto (throwaway-nøgle)
    // over præcis den kanoniske besked signed_message() bygger — samme kodevej
    // som scripts/build-release.mjs bruger ved rigtige releases. Hvis Node-
    // signeringen og Rust-verificeringen nogensinde uenige om formatet
    // (eller RFC 8032), fanger denne test det FØR en release.
    const VECTOR_PUB_HEX: &str =
        "5e4ba36135b4464aec5a92a805d1b990aec3eb3b0f0ba06360399a5b8db2c0bd";
    const VECTOR_SIG_B64: &str =
        "zffcu5K4dZE4GA+KWCDAKJ7t9jekAR+gxcUSsG1suBjhpB8PVtiqgePtPJQpbRFeNUrSrfuryNYaIqO/aI84CA==";
    const VECTOR_BUILD_ID: i64 = 1732483200;

    fn vector_sha256() -> String {
        "a".repeat(64)
    }

    fn vector_message() -> Vec<u8> {
        signed_message(VECTOR_BUILD_ID, &vector_sha256())
    }

    fn verify_with_key(bytes: &[u8], sig_b64: &str, pub_hex: &str) -> bool {
        let key_bytes = hex_decode_32(pub_hex).expect("testvektor-nøgle skal være gyldig hex");
        let key = VerifyingKey::from_bytes(&key_bytes).expect("testvektor skal være ed25519-nøgle");
        let sig_bytes = B64.decode(sig_b64).expect("testvektor-signatur skal være base64");
        let sig = Signature::from_slice(&sig_bytes).expect("testvektor skal være 64 bytes");
        key.verify_strict(bytes, &sig).is_ok()
    }

    #[test]
    fn node_signatur_verificerer_i_rust() {
        assert!(
            verify_with_key(&vector_message(), VECTOR_SIG_B64, VECTOR_PUB_HEX),
            "Node-ed25519-signatur skal verificere i Rust"
        );
    }

    #[test]
    fn beskadigede_bytes_fejler() {
        let mut bytes = vector_message();
        bytes[0] ^= 0xff;
        assert!(!verify_with_key(&bytes, VECTOR_SIG_B64, VECTOR_PUB_HEX));
    }

    #[test]
    fn forkert_noegle_fejler() {
        // Den reelle indlejrede nøgle er en ANDEN end testvektorens — verificering
        // mod den skal fejle, ellers binder nøglen ikke til signaturen.
        assert!(!verify_with_key(
            &vector_message(),
            VECTOR_SIG_B64,
            UPDATE_PUBLIC_KEY_HEX
        ));
    }

    #[test]
    fn ugyldig_signatur_afvises() {
        assert!(!verify_signature(&vector_message(), "ikke-base64!!!"));
        assert!(!verify_signature(&vector_message(), ""));
        // Gyldig base64 men forkert længde til en ed25519-signatur.
        assert!(!verify_signature(&vector_message(), "aGVsbG8="));
    }

    #[test]
    fn hex_decode_validerer_laengde_og_indhold() {
        assert!(hex_decode_32(UPDATE_PUBLIC_KEY_HEX).is_ok());
        assert!(hex_decode_32("abcd").is_err());
        assert!(hex_decode_32(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn manifest_uden_signatur_afvises() {
        let manifest = ReleaseManifest {
            version: "0.1.0".into(),
            build_id: 1,
            url: "https://example.invalid/x.exe".into(),
            sha256: format!("{:x}", Sha256::digest(b"test")),
            signature: String::new(),
            size: 4,
            notes: None,
        };
        let err = verify_installer_bytes(b"test", &manifest);
        assert!(err.is_err(), "usigneret manifest skal afvises");
    }

    #[test]
    fn manifest_med_forkert_hash_afvises() {
        let manifest = ReleaseManifest {
            version: "0.1.0".into(),
            build_id: 1,
            url: "https://example.invalid/x.exe".into(),
            sha256: "0".repeat(64),
            signature: VECTOR_SIG_B64.into(),
            size: 4,
            notes: None,
        };
        assert!(verify_installer_bytes(b"test", &manifest).is_err());
    }

    // Fuld-flow-vektor signet med den RIGTIGE privatnøgle over harmløse
    // fake-bytes. Kan ikke misbruges: signaturen binder sig til sha256 af
    // netop disse bytes, og ingen kan fremstille en exe med samme hash.
    const REAL_VEC_BYTES: &[u8] = b"open-dock fake installer bytes for test";
    const REAL_VEC_SHA256: &str =
        "329b7d42821cee5fdf98afb85d63998f3c2b6221dfc58045e7b9102254fa01b3";
    const REAL_VEC_BUILD_ID: i64 = 9999999999;
    const REAL_VEC_SIG: &str =
        "S+1wyF5PlKAvv1EXban41V6TjbpGMtzvapVUnvAyeUYJ8AImqw3vlKLRH6UDITi5t+ALeNZqWPmO7towQPfMCQ==";

    fn real_vec_manifest() -> ReleaseManifest {
        ReleaseManifest {
            version: "0.1.0".into(),
            build_id: REAL_VEC_BUILD_ID,
            url: "https://example.invalid/x.exe".into(),
            sha256: REAL_VEC_SHA256.into(),
            signature: REAL_VEC_SIG.into(),
            size: REAL_VEC_BYTES.len() as u64,
            notes: None,
        }
    }

    #[test]
    fn fuld_flow_accepterer_korrekt_signeret_manifest() {
        assert!(
            verify_installer_bytes(REAL_VEC_BYTES, &real_vec_manifest()).is_ok(),
            "korrekt signet manifest skal passere fuld verificering"
        );
    }

    #[test]
    fn rollback_med_hoejere_build_id_afvises() {
        // Rollback-angrebet: manifestet lyver om build-id men genbruger en
        // legitim signatur. Da build-id indgår i den signede besked, bryder
        // verificeringen — ellers kunne gamle builds serveres som "nye".
        let mut manifest = real_vec_manifest();
        manifest.build_id = REAL_VEC_BUILD_ID + 1;
        assert!(verify_installer_bytes(REAL_VEC_BYTES, &manifest).is_err());
    }

    #[test]
    fn fuld_flow_afviser_beskadigede_bytes() {
        let mut bytes = REAL_VEC_BYTES.to_vec();
        bytes[0] ^= 0xff;
        assert!(verify_installer_bytes(&bytes, &real_vec_manifest()).is_err());
    }
}
