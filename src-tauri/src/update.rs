use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::error::AppError;

const MANIFEST_URL: &str =
    "https://github.com/Oscar0912351/open-dock/releases/latest/download/latest.json";
const USER_AGENT: &str = "Open-Dock/0.1";
const STARTUP_CHECK_DELAY: Duration = Duration::from_secs(15);

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
    #[ts(type = "number")]
    pub build_id: i64,
    pub url: String,
    pub sha256: String,
    #[ts(type = "number")]
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
    #[ts(type = "number | null")]
    pub current_build_id: Option<i64>,
    pub version: String,
    pub latest: Option<ReleaseManifest>,
    #[ts(type = "number")]
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct UpdateProgress {
    #[ts(type = "number")]
    pub downloaded: u64,
    #[ts(type = "number")]
    pub total: u64,
}

/// Build-ID indlejret af scripts/build-release.mjs. Mangler i dev-/lokale builds.
fn local_build_id() -> Option<i64> {
    option_env!("OPEN_DOCK_BUILD_ID")
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
}

fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_else(|_| Client::new())
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
            "Ingen udgivelser fundet på GitHub endnu".into(),
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

    let manifest = fetch_manifest(&client()).await?;
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

/// Downloader installeren fra manifestet, verificerer SHA-256 og returnerer stien.
async fn download_installer(app: &AppHandle, manifest: &ReleaseManifest) -> Result<PathBuf, AppError> {
    if manifest.url.is_empty() || manifest.sha256.is_empty() {
        return Err(AppError::Other("Manifestet mangler url eller sha256".into()));
    }
    let mut response = client()
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
    let path = std::env::temp_dir().join(format!("open-dock-update-{}.exe", manifest.build_id));
    let tmp_path = path.with_extension("download");

    let mut file = fs::File::create(&tmp_path)
        .map_err(|err| AppError::Other(format!("Kunne ikke oprette midlertidig fil: {err}")))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;

    while let Some(bytes) = response.chunk().await.map_err(AppError::Network)? {
        file.write_all(&bytes).map_err(|err| {
            AppError::Other(format!("Kunne ikke skrive opdateringsfil: {err}"))
        })?;
        hasher.update(&bytes);
        downloaded += bytes.len() as u64;
        if downloaded - last_emit > 256 * 1024 || downloaded == total {
            last_emit = downloaded;
            let _ = app.emit("update://progress", UpdateProgress { downloaded, total });
        }
    }
    drop(file);

    let digest = format!("{:x}", hasher.finalize());
    if !digest.eq_ignore_ascii_case(&manifest.sha256) {
        let _ = fs::remove_file(&tmp_path);
        log::warn!(target: "update", "SHA-256 mismatch: forventede {}, fik {digest}", manifest.sha256);
        return Err(AppError::Other(
            "Integritetstjek fejlede for den hentede fil".into(),
        ));
    }
    if let Err(err) = fs::rename(&tmp_path, &path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(AppError::Other(format!(
            "Kunne ikke færdiggøre opdateringsfilen: {err}"
        )));
    }
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
    match download_installer(app, &manifest).await {
        Ok(path) => {
            log::info!(target: "update", "Opdatering b{} hentet til {:?}", manifest.build_id, path);
            *tracker.staged.lock() = Some((manifest.build_id, path));
            status.state = UpdateCheckState::Ready;
            status.checked_at = unix_now_s();
            set_status(app, &status);
            notify_ready(&app, &manifest.version);
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

/// Genstarter appen ind i den nye version: kører den verificerede installer
/// silent (/S — ingen "allerede installeret"-dialog ved samme versionsnummer),
/// og lader en frakoblet cmd-kæde vente, installere og starte appen igen.
pub async fn install(app: &AppHandle) -> Result<(), AppError> {
    if local_build_id().is_none() {
        return Err(AppError::Other(
            "Opdatering er kun tilgængelig i release-builds".into(),
        ));
    }
    let tracker = app.try_state::<UpdateTracker>();
    if let Some(t) = &tracker {
        if t.working.swap(true, Ordering::SeqCst) {
            return Err(AppError::Other(
                "Opdateringen downloades stadig — vent et øjeblik".into(),
            ));
        }
    }
    let staged = tracker
        .as_ref()
        .and_then(|t| t.staged.lock().clone());

    let result = async {
        let installer = match staged {
            Some((_, path)) if path.exists() => path,
            _ => {
                let manifest = fetch_manifest(&client()).await?;
                download_installer(app, &manifest).await?
            }
        };
        spawn_silent_install(&installer)?;
        crate::commands::app::quit_app(app);
        Ok(())
    }
    .await;

    if let Some(t) = &tracker {
        t.working.store(false, Ordering::SeqCst);
    }
    result
}

/// Starter en frakoblet cmd-kæde: kort pause (så appen når at lukke), derefter
/// NSIS silent-install og til sidst genstart af det installerede build.
fn spawn_silent_install(installer: &Path) -> Result<(), AppError> {
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

    log::info!(
        target: "update",
        "Silent install {:?} og genstart af {:?}",
        installer,
        relaunch
    );
    Command::new("cmd")
        .arg("/c")
        .raw_arg(format!(
            "ping 127.0.0.1 -n 3 >nul & \"{}\" /S & \"{}\"",
            installer.display(),
            relaunch.display()
        ))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|err| AppError::Other(format!("Kunne ikke starte installationsprogrammet: {err}")))?;
    Ok(())
}

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
        "Opdateringen er hentet og klar — genstart Open Dock for at installere den.".to_string()
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

fn unix_now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
