use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

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
};

const MANIFEST_URL: &str =
    "https://github.com/Oscar0912351/open-dock/releases/latest/download/latest.json";
const USER_AGENT: &str = "Open-Dock/0.1";
const STARTUP_CHECK_DELAY: Duration = Duration::from_secs(15);
/// Installeren er lille nok til at downloade på under et sekund — hold
/// download-staten synlig lidt, så indikatoren ikke blinker forbi uset.
const MIN_DOWNLOAD_DISPLAY: Duration = Duration::from_millis(1800);

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

/// Downloads må ikke dø på en samlet request-timeout — en stor fil på en sløv
/// forbindelse bruger længere tid end manifestet. Begræns i stedet connect-tid
/// og stilstand mellem chunks, så fremskridt altid kan nå i mål.
fn download_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
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

/// Verificerer at en allerede-downloadet fil matcher manifestets SHA-256.
fn verify_sha256(path: &Path, expected: &str) -> Result<bool, AppError> {
    let bytes = fs::read(path)
        .map_err(|err| AppError::Other(format!("Kunne ikke læse hentet fil: {err}")))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Ok(digest.eq_ignore_ascii_case(expected))
}

/// Downloader installeren fra manifestet, verificerer SHA-256 og returnerer stien.
async fn download_installer(app: &AppHandle, manifest: &ReleaseManifest) -> Result<PathBuf, AppError> {
    if manifest.url.is_empty() || manifest.sha256.is_empty() {
        return Err(AppError::Other("Manifestet mangler url eller sha256".into()));
    }
    let path = std::env::temp_dir().join(format!("open-dock-update-{}.exe", manifest.build_id));
    // En tidligere session kan have hentet samme build — genbrug hvis hashet matcher.
    if path.exists() {
        if verify_sha256(&path, &manifest.sha256).unwrap_or(false) {
            log::info!(target: "update", "Genbruger hentet installer {:?}", path);
            return Ok(path);
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

    let mut response = download_client()
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
    // fs::rename overskriver ikke en eksisterende fil på Windows.
    if path.exists() {
        let _ = fs::remove_file(&path);
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
    let cached_manifest = tracker
        .as_ref()
        .and_then(|t| t.last.lock().clone())
        .and_then(|s| s.latest);

    let result = async {
        let (installer, manifest) = match staged {
            Some((_, path)) if path.exists() => (path, cached_manifest),
            _ => {
                let manifest = fetch_manifest(&client()).await?;
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

    let temp_dir = std::env::temp_dir();
    let splash_path = temp_dir.join("open-dock-update-splash.ps1");
    let stage_path = temp_dir.join("open-dock-update-stage.txt");
    let script_path = temp_dir.join("open-dock-update.ps1");
    // Stage-filen skal findes før splashen starter, så dens første poll
    // læser "closing" frem for at falde tilbage på default.
    fs::write(&stage_path, "closing")
        .map_err(|err| AppError::Other(format!("Kunne ikke skrive opdateringsstatus: {err}")))?;
    let splash = build_splash_script(&stage_path, meta);
    fs::write(&splash_path, splash)
        .map_err(|err| AppError::Other(format!("Kunne ikke skrive opdateringsscript: {err}")))?;
    let script = update_script(installer, &relaunch, std::process::id(), &stage_path);
    fs::write(&script_path, script)
        .map_err(|err| AppError::Other(format!("Kunne ikke skrive opdateringsscript: {err}")))?;

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
        let _ = fs::remove_file(&stage_path);
        let _ = fs::remove_file(&splash_path);
        let _ = fs::remove_file(&script_path);
        return Err(AppError::Other(format!(
            "Kunne ikke starte installationsprogrammet: {err}"
        )));
    }
    Ok(())
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
) -> String {
    let esc = |p: &Path| ps_escape(&p.display().to_string());
    let body = UPDATE_SCRIPT
        .replace("__APP_PID__", &app_pid.to_string())
        .replace("__INSTALLER__", &esc(installer))
        .replace("__RELAUNCH__", &esc(relaunch))
        .replace("__STAGE_FILE__", &esc(stage));
    format!("\u{feff}{body}")
}

/// Skulpturelt opdateringsvindue i Open Dock's eget designsprog: selve
/// app-ikonets form med Studio dot-grid, tre svævende bento-brikker og én
/// hårfin fremgangslinje. Kører som sin egen powershell-proces med 3-min
/// sikkerhedsnet.
const SPLASH_SCRIPT: &str = r##"$ErrorActionPreference = 'SilentlyContinue'
Add-Type -AssemblyName PresentationFramework,PresentationCore,WindowsBase

$stageFile    = '__STAGE_FILE__'
$theme        = '__THEME__'
$lang         = '__LANG__'
$versionBadge = '__VERSION_BADGE__'
$isLight      = ($theme -eq 'light')
$isEn         = ($lang -eq 'en')
$allowMotion  = $true
try { $allowMotion = [bool][System.Windows.SystemParameters]::ClientAreaAnimation } catch {}

function Make-Color([string]$hex) {
    [System.Windows.Media.ColorConverter]::ConvertFromString($hex)
}
function Make-Brush([string]$hex) {
    $b = New-Object System.Windows.Media.SolidColorBrush (Make-Color $hex)
    $b.Freeze()
    $b
}
function Make-Linear([string]$from, [string]$to, [double]$x1, [double]$y1, [double]$x2, [double]$y2) {
    $g = New-Object System.Windows.Media.LinearGradientBrush
    $g.StartPoint = New-Object System.Windows.Point($x1, $y1)
    $g.EndPoint   = New-Object System.Windows.Point($x2, $y2)
    $null = $g.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color $from), 0.0)))
    $null = $g.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color $to), 1.0)))
    $g.Freeze()
    $g
}

$copy = if ($isEn) {
    @{
        Title       = 'Updating Open Dock'
        Closing     = 'Closing…'
        Installing  = 'Installing…'
        Relaunching = 'Relaunching…'
        Done        = 'Starting…'
    }
} else {
    @{
        Title       = 'Opdaterer Open Dock'
        Closing     = 'Lukker…'
        Installing  = 'Installerer…'
        Relaunching = 'Starter igen…'
        Done        = 'Åbner…'
    }
}

$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Width="316" Height="324"
        WindowStyle="None" ResizeMode="NoResize"
        AllowsTransparency="True" Background="Transparent"
        Topmost="True" ShowInTaskbar="False"
        WindowStartupLocation="CenterScreen"
        UseLayoutRounding="True" SnapsToDevicePixels="True"
        TextOptions.TextFormattingMode="Ideal"
        TextOptions.TextRenderingMode="ClearType">
  <Grid x:Name="RootMotion" Margin="28" Opacity="0" RenderTransformOrigin="0.5,0.5">
    <Grid.RenderTransform>
      <TransformGroup>
        <ScaleTransform x:Name="RootScale" ScaleX="0.96" ScaleY="0.96"/>
        <TranslateTransform x:Name="RootTranslate" Y="6"/>
      </TransformGroup>
    </Grid.RenderTransform>

    <!-- Adskilt skygge-plade så al grafik og tekst forbliver knivskarp -->
    <Border x:Name="ShadowPlate" CornerRadius="28" IsHitTestVisible="False">
      <Border.Effect>
        <DropShadowEffect x:Name="CardShadow" Color="#000000" Direction="270"
                          ShadowDepth="12" BlurRadius="34" Opacity="0.65"/>
      </Border.Effect>
    </Border>

    <!-- Hovedflade: samme silhuet som Open Dock app-ikonet -->
    <Border x:Name="Card" CornerRadius="28" BorderThickness="1">
      <Grid x:Name="CardInner">
        <!-- Diskret Open Dock Studio dot-grid bag mærket -->
        <Rectangle x:Name="DotGrid" IsHitTestVisible="False"/>

        <!-- Meget rolig top-refleksion -->
        <Border x:Name="TopGlow" IsHitTestVisible="False"/>

        <!-- 1px specular top-kant -->
        <Rectangle x:Name="TopSpecular" Height="1" VerticalAlignment="Top"
                   Margin="30,0,30,0" IsHitTestVisible="False"/>

        <!-- Ét diskret entrance-sheen -->
        <Rectangle x:Name="Sheen" Width="110" HorizontalAlignment="Left"
                   IsHitTestVisible="False" Opacity="0">
          <Rectangle.RenderTransform>
            <TransformGroup>
              <SkewTransform AngleX="-22"/>
              <TranslateTransform x:Name="SheenX" X="-160"/>
            </TransformGroup>
          </Rectangle.RenderTransform>
        </Rectangle>

        <!-- Centreret indhold -->
        <Grid Margin="28,42,28,22">
          <Grid.RowDefinitions>
            <RowDefinition Height="*"/>
            <RowDefinition Height="Auto"/>
            <RowDefinition Height="Auto"/>
            <RowDefinition Height="Auto"/>
          </Grid.RowDefinitions>

          <!-- Open Dock 3-panel bento-mærke, fritstående og levende -->
          <Canvas Grid.Row="0" Width="68" Height="66"
                  HorizontalAlignment="Center" VerticalAlignment="Center">
            <Rectangle x:Name="TileA" Canvas.Left="0" Canvas.Top="0"
                       Width="24.5" Height="29" RadiusX="7" RadiusY="7">
              <Rectangle.RenderTransform>
                <TranslateTransform x:Name="TileAY" Y="0"/>
              </Rectangle.RenderTransform>
            </Rectangle>
            <Rectangle x:Name="TileB" Canvas.Left="31.5" Canvas.Top="0"
                       Width="36.5" Height="29" RadiusX="7" RadiusY="7">
              <Rectangle.RenderTransform>
                <TranslateTransform x:Name="TileBY" Y="0"/>
              </Rectangle.RenderTransform>
            </Rectangle>
            <Rectangle x:Name="TileC" Canvas.Left="0" Canvas.Top="36"
                       Width="68" Height="30" RadiusX="7" RadiusY="7">
              <Rectangle.RenderTransform>
                <TranslateTransform x:Name="TileCY" Y="0"/>
              </Rectangle.RenderTransform>
            </Rectangle>
          </Canvas>

          <!-- Titel og én rolig statuslinje -->
          <StackPanel Grid.Row="1" Margin="0,24,0,0" HorizontalAlignment="Center">
            <TextBlock x:Name="TitleText"
                       FontFamily="Segoe UI Variable Display, Segoe UI"
                       FontSize="14.5" FontWeight="SemiBold"
                       HorizontalAlignment="Center"/>
            <TextBlock x:Name="StageText" Margin="0,5,0,0"
                       FontFamily="Segoe UI Variable Text, Segoe UI"
                       FontSize="12" HorizontalAlignment="Center"/>
          </StackPanel>

          <!-- Hårfin 2.5px fremgangslinje -->
          <Grid x:Name="TrackHost" Grid.Row="2" Width="148" Height="2.5"
                Margin="0,22,0,0" HorizontalAlignment="Center">
            <Border x:Name="TrackBg" CornerRadius="1.25"/>
            <Border x:Name="TrackFill" HorizontalAlignment="Left" Width="14" CornerRadius="1.25"/>
          </Grid>

          <!-- Ren versionstekst uden ramme eller pill -->
          <TextBlock x:Name="VersionText" Grid.Row="3" Margin="0,14,0,0"
                     FontFamily="Segoe UI Variable Text, Segoe UI"
                     FontSize="10.5" HorizontalAlignment="Center"/>
        </Grid>
      </Grid>
    </Border>
  </Grid>
</Window>
'@

$reader = New-Object System.Xml.XmlNodeReader ([xml]$xaml)
$win = [System.Windows.Markup.XamlReader]::Load($reader)

function El([string]$name) { $win.FindName($name) }

$rootMotion    = El 'RootMotion'
$rootScale     = El 'RootScale'
$rootTranslate = El 'RootTranslate'
$shadowPlate   = El 'ShadowPlate'
$cardShadow    = El 'CardShadow'
$card          = El 'Card'
$cardInner     = El 'CardInner'
$dotGrid       = El 'DotGrid'
$topGlow       = El 'TopGlow'
$topSpecular   = El 'TopSpecular'
$sheen         = El 'Sheen'
$sheenX        = El 'SheenX'
$tileA         = El 'TileA'
$tileB         = El 'TileB'
$tileC         = El 'TileC'
$tileAY        = El 'TileAY'
$tileBY        = El 'TileBY'
$tileCY        = El 'TileCY'
$titleText     = El 'TitleText'
$stageText     = El 'StageText'
$trackHost     = El 'TrackHost'
$trackBg       = El 'TrackBg'
$trackFill     = El 'TrackFill'
$versionText   = El 'VersionText'

if ($isLight) {
    $pal = @{
        CardBg      = Make-Linear '#FDFFFFFF' '#FBF5F5F7' 0 0 0 1
        ShadowBg    = Make-Brush '#FFF8F8FA'
        ShadowOp    = 0.16
        ShadowBlur  = 28.0
        ShadowDepth = 9.0
        CardBorder  = Make-Brush '#1609090B'
        TileFill    = Make-Brush '#FF09090B'
        Ink1        = Make-Brush '#FF09090B'
        Ink2        = Make-Brush '#FF62626C'
        Ink3        = Make-Brush '#FF9E9EA8'
        TrackBg     = Make-Brush '#1609090B'
        TrackFill   = Make-Brush '#FF09090B'
        DotHex      = '#1809090B'
        GlowHex     = '#0809090B'
        SpecHex     = '#E6FFFFFF'
        SheenHex    = '#1AFFFFFF'
    }
} else {
    $pal = @{
        CardBg      = Make-Linear '#FA0D0D11' '#FC070709' 0 0 0 1
        ShadowBg    = Make-Brush '#FF070709'
        ShadowOp    = 0.66
        ShadowBlur  = 34.0
        ShadowDepth = 12.0
        CardBorder  = Make-Brush '#1CFFFFFF'
        TileFill    = Make-Brush '#FFF4F4F6'
        Ink1        = Make-Brush '#FFF4F4F6'
        Ink2        = Make-Brush '#FF868692'
        Ink3        = Make-Brush '#FF4C4C56'
        TrackBg     = Make-Brush '#18FFFFFF'
        TrackFill   = Make-Brush '#FFF4F4F6'
        DotHex      = '#22FFFFFF'
        GlowHex     = '#0CFFFFFF'
        SpecHex     = '#38FFFFFF'
        SheenHex    = '#12FFFFFF'
    }
}

$shadowPlate.Background = $pal.ShadowBg
$cardShadow.Opacity     = $pal.ShadowOp
$cardShadow.BlurRadius  = $pal.ShadowBlur
$cardShadow.ShadowDepth = $pal.ShadowDepth

$card.Background  = $pal.CardBg
$card.BorderBrush = $pal.CardBorder

$tileA.Fill = $pal.TileFill
$tileB.Fill = $pal.TileFill
$tileC.Fill = $pal.TileFill

# Studio dot-grid med blød radial maske centreret bag brikkerne
$dotGeom = New-Object System.Windows.Media.EllipseGeometry((New-Object System.Windows.Point(8, 8)), 0.75, 0.75)
$dotDrawing = New-Object System.Windows.Media.GeometryDrawing((Make-Brush $pal.DotHex), $null, $dotGeom)
$dotBrush = New-Object System.Windows.Media.DrawingBrush($dotDrawing)
$dotBrush.Viewbox       = New-Object System.Windows.Rect(0, 0, 16, 16)
$dotBrush.ViewboxUnits  = [System.Windows.Media.BrushMappingMode]::Absolute
$dotBrush.Viewport      = New-Object System.Windows.Rect(0, 0, 16, 16)
$dotBrush.ViewportUnits = [System.Windows.Media.BrushMappingMode]::Absolute
$dotBrush.TileMode      = [System.Windows.Media.TileMode]::Tile
$dotBrush.Freeze()
$dotGrid.Fill = $dotBrush

$dotMask = New-Object System.Windows.Media.RadialGradientBrush
$dotMask.GradientOrigin = New-Object System.Windows.Point(0.5, 0.36)
$dotMask.Center         = New-Object System.Windows.Point(0.5, 0.36)
$dotMask.RadiusX        = 0.52
$dotMask.RadiusY        = 0.46
$null = $dotMask.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#FFFFFFFF'), 0.0)))
$null = $dotMask.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#00FFFFFF'), 1.0)))
$dotMask.Freeze()
$dotGrid.OpacityMask = $dotMask

# Subtil top-glød
$glow = New-Object System.Windows.Media.RadialGradientBrush
$glow.GradientOrigin = New-Object System.Windows.Point(0.5, 0.0)
$glow.Center         = New-Object System.Windows.Point(0.5, 0.0)
$glow.RadiusX        = 0.75
$glow.RadiusY        = 0.55
$null = $glow.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color $pal.GlowHex), 0.0)))
$null = $glow.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#00FFFFFF'), 1.0)))
$glow.Freeze()
$topGlow.Background = $glow

function Make-SpecBrush([string]$midHex) {
    $b = New-Object System.Windows.Media.LinearGradientBrush
    $b.StartPoint = New-Object System.Windows.Point(0, 0.5)
    $b.EndPoint   = New-Object System.Windows.Point(1, 0.5)
    $null = $b.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#00FFFFFF'), 0.0)))
    $null = $b.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color $midHex), 0.5)))
    $null = $b.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#00FFFFFF'), 1.0)))
    $b.Freeze()
    $b
}
$topSpecular.Fill = Make-SpecBrush $pal.SpecHex
$sheen.Fill       = Make-SpecBrush $pal.SheenHex

$titleText.Text         = $copy.Title
$titleText.Foreground   = $pal.Ink1
$stageText.Foreground   = $pal.Ink2
$versionText.Foreground = $pal.Ink3
if ([string]::IsNullOrWhiteSpace($versionBadge)) {
    $versionText.Visibility = [System.Windows.Visibility]::Collapsed
} else {
    $versionText.Text = $versionBadge
}

$trackBg.Background   = $pal.TrackBg
$trackFill.Background = $pal.TrackFill

$cardInner.Add_SizeChanged({
    $rect = New-Object System.Windows.Rect(0, 0, $cardInner.ActualWidth, $cardInner.ActualHeight)
    $cardInner.Clip = New-Object System.Windows.Media.RectangleGeometry($rect, 27, 27)
})
$trackHost.Add_SizeChanged({
    $rect = New-Object System.Windows.Rect(0, 0, $trackHost.ActualWidth, $trackHost.ActualHeight)
    $trackHost.Clip = New-Object System.Windows.Media.RectangleGeometry($rect, 1.25, 1.25)
})

$win.Add_MouseLeftButtonDown({
    try { $win.DragMove() } catch {}
})

$easeOut = New-Object System.Windows.Media.Animation.CubicEase
$easeOut.EasingMode = [System.Windows.Media.Animation.EasingMode]::EaseOut
$easeInOut = New-Object System.Windows.Media.Animation.SineEase
$easeInOut.EasingMode = [System.Windows.Media.Animation.EasingMode]::EaseInOut

$script:currentStage = ''
$script:progressFrac = 0.08
$script:closingOut   = $false

function Animate-Fill([double]$frac, [int]$ms) {
    $script:progressFrac = $frac
    $w = $trackHost.ActualWidth
    if ($w -le 0) { $w = 148.0 }
    $targetW = [Math]::Max(8.0, $w * $frac)
    if (-not $allowMotion -or $ms -le 0) {
        $trackFill.BeginAnimation([System.Windows.FrameworkElement]::WidthProperty, $null)
        $trackFill.Width = $targetW
        return
    }
    $anim = New-Object System.Windows.Media.Animation.DoubleAnimation
    $anim.To = $targetW
    $anim.Duration = [System.Windows.Duration]::new([TimeSpan]::FromMilliseconds($ms))
    $anim.EasingFunction = $easeOut
    $trackFill.BeginAnimation([System.Windows.FrameworkElement]::WidthProperty, $anim)
}

function Start-AmbientAnimations {
    if (-not $allowMotion) { return }

    # De 3 brikker løfter sig og ånder stille i sekvens
    $tiles  = @($tileA, $tileB, $tileC)
    $shifts = @($tileAY, $tileBY, $tileCY)
    for ($i = 0; $i -lt 3; $i++) {
        $op = New-Object System.Windows.Media.Animation.DoubleAnimation
        $op.From = 1.0
        $op.To   = 0.54
        $op.BeginTime = [TimeSpan]::FromMilliseconds($i * 250)
        $op.Duration  = [System.Windows.Duration]::new([TimeSpan]::FromMilliseconds(950))
        $op.AutoReverse = $true
        $op.RepeatBehavior = [System.Windows.Media.Animation.RepeatBehavior]::Forever
        $op.EasingFunction = $easeInOut
        $tiles[$i].BeginAnimation([System.Windows.UIElement]::OpacityProperty, $op)

        $dy = New-Object System.Windows.Media.Animation.DoubleAnimation
        $dy.From = 0.0
        $dy.To   = -3.0
        $dy.BeginTime = [TimeSpan]::FromMilliseconds($i * 250)
        $dy.Duration  = [System.Windows.Duration]::new([TimeSpan]::FromMilliseconds(950))
        $dy.AutoReverse = $true
        $dy.RepeatBehavior = [System.Windows.Media.Animation.RepeatBehavior]::Forever
        $dy.EasingFunction = $easeInOut
        $shifts[$i].BeginAnimation([System.Windows.Media.TranslateTransform]::YProperty, $dy)
    }

    # Ét enkelt stille lys-sheen ved entrance
    $sheen.Opacity = 1.0
    $sh = New-Object System.Windows.Media.Animation.DoubleAnimation
    $sh.From = -160.0
    $sh.To   = 420.0
    $sh.BeginTime = [TimeSpan]::FromMilliseconds(160)
    $sh.Duration  = [System.Windows.Duration]::new([TimeSpan]::FromMilliseconds(850))
    $sh.EasingFunction = $easeOut
    $sheenX.BeginAnimation([System.Windows.Media.TranslateTransform]::XProperty, $sh)
}

function Apply-Stage([string]$stage) {
    $norm = ($stage -replace '\s+', '').ToLowerInvariant()
    if ([string]::IsNullOrEmpty($norm)) { $norm = 'closing' }
    if ($norm -eq $script:currentStage) { return }
    $script:currentStage = $norm

    switch ($norm) {
        'closing' {
            $stageText.Text = $copy.Closing
            Animate-Fill 0.22 460
        }
        'installing' {
            $stageText.Text = $copy.Installing
            Animate-Fill 0.62 900
        }
        'relaunching' {
            $stageText.Text = $copy.Relaunching
            Animate-Fill 0.90 480
        }
        'done' {
            $stageText.Text = $copy.Done
            Animate-Fill 1.0 200

            foreach ($t in @($tileA, $tileB, $tileC)) {
                $t.BeginAnimation([System.Windows.UIElement]::OpacityProperty, $null)
                $t.Opacity = 1.0
            }
            foreach ($s in @($tileAY, $tileBY, $tileCY)) {
                $s.BeginAnimation([System.Windows.Media.TranslateTransform]::YProperty, $null)
                $s.Y = 0.0
            }

            if (-not $script:closingOut) {
                $script:closingOut = $true
                $closeDelay = New-Object System.Windows.Threading.DispatcherTimer
                $closeDelay.Interval = [TimeSpan]::FromMilliseconds(320)
                $closeDelay.Add_Tick({
                    $closeDelay.Stop()
                    if (-not $allowMotion) {
                        $win.Close()
                        return
                    }
                    $fadeOut = New-Object System.Windows.Media.Animation.DoubleAnimation(1.0, 0.0, [TimeSpan]::FromMilliseconds(220))
                    $fadeOut.EasingFunction = $easeOut
                    $fadeOut.Add_Completed({ $win.Close() })
                    $scaleOut = New-Object System.Windows.Media.Animation.DoubleAnimation(1.0, 0.96, [TimeSpan]::FromMilliseconds(220))
                    $scaleOut.EasingFunction = $easeOut
                    $rootMotion.BeginAnimation([System.Windows.UIElement]::OpacityProperty, $fadeOut)
                    $rootScale.BeginAnimation([System.Windows.Media.ScaleTransform]::ScaleXProperty, $scaleOut)
                    $rootScale.BeginAnimation([System.Windows.Media.ScaleTransform]::ScaleYProperty, $scaleOut)
                })
                $closeDelay.Start()
            }
        }
        default {
            $stageText.Text = $copy.Installing
            Animate-Fill 0.55 700
        }
    }
}

$win.Add_Loaded({
    if ($allowMotion) {
        $op = New-Object System.Windows.Media.Animation.DoubleAnimation(0.0, 1.0, [TimeSpan]::FromMilliseconds(260))
        $op.EasingFunction = $easeOut
        $ty = New-Object System.Windows.Media.Animation.DoubleAnimation(6.0, 0.0, [TimeSpan]::FromMilliseconds(320))
        $ty.EasingFunction = $easeOut
        $sc = New-Object System.Windows.Media.Animation.DoubleAnimation(0.96, 1.0, [TimeSpan]::FromMilliseconds(320))
        $sc.EasingFunction = $easeOut
        $rootMotion.BeginAnimation([System.Windows.UIElement]::OpacityProperty, $op)
        $rootTranslate.BeginAnimation([System.Windows.Media.TranslateTransform]::YProperty, $ty)
        $rootScale.BeginAnimation([System.Windows.Media.ScaleTransform]::ScaleXProperty, $sc)
        $rootScale.BeginAnimation([System.Windows.Media.ScaleTransform]::ScaleYProperty, $sc)
    } else {
        $rootMotion.Opacity = 1.0
        $rootTranslate.Y    = 0.0
        $rootScale.ScaleX   = 1.0
        $rootScale.ScaleY   = 1.0
    }

    Start-AmbientAnimations

    $initial = 'closing'
    if (Test-Path -LiteralPath $stageFile) {
        try { $initial = [System.IO.File]::ReadAllText($stageFile) } catch {}
    }
    Apply-Stage $initial
})

$poll = New-Object System.Windows.Threading.DispatcherTimer
$poll.Interval = [TimeSpan]::FromMilliseconds(180)
$poll.Add_Tick({
    if (Test-Path -LiteralPath $stageFile) {
        try {
            $txt = [System.IO.File]::ReadAllText($stageFile)
            Apply-Stage $txt
        } catch {}
    }
    if ($script:currentStage -eq 'installing' -and $script:progressFrac -lt 0.84) {
        $next = [Math]::Min(0.84, $script:progressFrac + 0.008)
        Animate-Fill $next 220
    }
})
$poll.Start()

$life = New-Object System.Windows.Threading.DispatcherTimer
$life.Interval = [TimeSpan]::FromMinutes(3)
$life.Add_Tick({ $win.Close() })
$life.Start()

$null = $win.ShowDialog()
"##;

const UPDATE_SCRIPT: &str = r#"$ErrorActionPreference = 'SilentlyContinue'
$installer = '__INSTALLER__'
$relaunch  = '__RELAUNCH__'
$splashPs1 = Join-Path $env:TEMP 'open-dock-update-splash.ps1'
$stageFile = '__STAGE_FILE__'
$appPid    = __APP_PID__

function Set-Stage([string]$name) {
    try { [System.IO.File]::WriteAllText($stageFile, $name) } catch {}
}

try { Wait-Process -Id $appPid -Timeout 15 } catch {}
Start-Sleep -Milliseconds 450

Set-Stage 'installing'
$iproc = $null
try { $iproc = Start-Process -FilePath $installer -ArgumentList '/S' -PassThru } catch {}
if ($iproc) {
    # En hængt installer må aldrig efterlade statusboksen evigt — dræb efter 3 min.
    try { if (-not $iproc.WaitForExit(180000)) { Stop-Process -Id $iproc.Id -Force } } catch {}
}

Set-Stage 'relaunching'
Start-Sleep -Milliseconds 250
try { Start-Process -FilePath $relaunch } catch {}

Set-Stage 'done'
# Splashen lukker sig selv på 'done' (plus 3 min selvdestruktion som net) —
# giv dens 180 ms-poll tid til at nå at læse stadiet før filen fjernes.
Start-Sleep -Milliseconds 800
try { Remove-Item -LiteralPath $stageFile -Force } catch {}
try { Remove-Item -LiteralPath $splashPs1 -Force } catch {}
try { Remove-Item -LiteralPath $PSCommandPath -Force } catch {}
"#;

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
