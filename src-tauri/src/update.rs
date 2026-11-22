use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::Command,
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
}

impl Default for UpdateTracker {
    fn default() -> Self {
        Self {
            last: Mutex::new(None),
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
    Available,
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

/// Henter frisk manifest, downloader, verificerer, starter installer og lukker appen.
pub async fn install(app: &AppHandle) -> Result<(), AppError> {
    if local_build_id().is_none() {
        return Err(AppError::Other(
            "Opdatering er kun tilgængelig i release-builds".into(),
        ));
    }
    let manifest = fetch_manifest(&client()).await?;
    let installer = download_installer(app, &manifest).await?;

    log::info!(
        target: "update",
        "Starter installer {:?} og lukker Open Dock",
        installer
    );
    Command::new(&installer)
        .spawn()
        .map_err(|err| AppError::Other(format!("Kunne ikke starte installationsprogrammet: {err}")))?;
    crate::commands::app::quit_app(app);
    Ok(())
}

/// Kører ét opdateringstjek kort efter opstart (kun release-builds med build-id)
/// og adviserer via event + Windows-notifikation hvis et nyt build findes.
pub fn install_background_check(app: &AppHandle) {
    if local_build_id().is_none() || cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_CHECK_DELAY).await;
        match check().await {
            Ok(status) => {
                if let Some(state) = app.try_state::<UpdateTracker>() {
                    *state.last.lock() = Some(status.clone());
                }
                if let Err(err) = app.emit("update://status", &status) {
                    log::warn!(target: "update", "Kunne ikke sende update://status: {err}");
                }
                if status.state == UpdateCheckState::Available {
                    let remote_version = status
                        .latest
                        .as_ref()
                        .map(|m| m.version.as_str())
                        .unwrap_or("");
                    notify_update(&app, remote_version);
                }
            }
            Err(err) => {
                log::info!(target: "update", "Baggrundstjek for opdatering fejlede: {err:?}");
            }
        }
    });
}

fn notify_update(app: &AppHandle, remote_version: &str) {
    use tauri_plugin_notification::NotificationExt;
    let body = if remote_version.is_empty() {
        "En ny version af Open Dock er klar. Åbn Indstillinger for at installere den.".to_string()
    } else {
        format!("Open Dock {remote_version} er klar. Åbn Indstillinger for at installere den.")
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
