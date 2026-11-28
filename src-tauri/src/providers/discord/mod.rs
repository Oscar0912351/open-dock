//! Discord-provider: lokal Discord RPC IPC (deltagere, profilbilleder, tale-ring,
//! kanalnavn & mute/deafen) med WASAPI mikrofon-fallback når RPC ikke er forbundet.

pub mod avatars;
pub mod call_timer;
pub mod credentials;
pub mod ipc;
pub mod wasapi;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc,
};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::{error::AppError, util::unix_now_ms};
use super::worker::{StopToken, Worker};
use call_timer::CallTimer;
use credentials::{DiscordRpcCredentials, DEFAULT_REDIRECT_URI};
use ipc::{DiscordIpcPipe, PipeError, RawParticipant, SelfUserProfile, OP_PONG};
use wasapi::{AudioSessionMonitor, CallDetector};

const IPC_IN_CALL_WAIT: Duration = Duration::from_millis(150);
const IPC_IDLE_WAIT: Duration = Duration::from_millis(500);
const AUDIO_POLL_RUNNING: Duration = Duration::from_secs(2);
const AUDIO_POLL_CLOSED: Duration = Duration::from_secs(5);
const IPC_RECONNECT_BACKOFF: Duration = Duration::from_secs(6);
const IPC_AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const RPC_CHANNEL_EVENTS: [&str; 5] = [
    "VOICE_STATE_CREATE",
    "VOICE_STATE_UPDATE",
    "VOICE_STATE_DELETE",
    "SPEAKING_START",
    "SPEAKING_STOP",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum DiscordConnectionMode {
    Rpc,
    Audio,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct DiscordParticipant {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub avatar_path: Option<String>,
    pub speaking: bool,
    pub muted: bool,
    pub deafened: bool,
    pub is_self: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct DiscordSnapshot {
    pub running: bool,
    pub in_call: bool,
    pub mode: DiscordConnectionMode,
    pub channel_name: Option<String>,
    pub participants: Vec<DiscordParticipant>,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub can_control_voice: bool,
    pub call_started_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct DiscordRpcStatus {
    pub configured: bool,
    pub connected: bool,
    pub via_central: bool,
    pub client_id: Option<String>,
    pub redirect_uri: String,
    pub has_secret: bool,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_path: Option<String>,
    pub can_control_voice: bool,
    pub last_error: Option<String>,
}

impl Default for DiscordRpcStatus {
    fn default() -> Self {
        Self {
            configured: false,
            connected: false,
            via_central: false,
            client_id: None,
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            has_secret: false,
            user_id: None,
            username: None,
            display_name: None,
            avatar_path: None,
            can_control_voice: false,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum DiscordVoiceAction {
    ToggleMute,
    ToggleDeafen,
}

#[derive(Debug)]
enum WorkerMsg {
    PauseForAuth,
    AttachPipe {
        pipe: DiscordIpcPipe,
        can_write: bool,
        profile: SelfUserProfile,
    },
    ReloadCredentials,
    VoiceAction(DiscordVoiceAction),
    AvatarReady,
    Stop,
}

#[derive(Debug, Clone)]
enum AuthPipeError {
    /// Discord returnerede eksplicit en RPC-fejl (fx ugyldig/udløbet access token).
    TokenRejected(String),
    /// Pipe lukket, optaget eller timeout under handshake/authenticate.
    Transport(String),
}

impl AuthPipeError {
    fn into_message(self) -> String {
        match self {
            Self::TokenRejected(m) | Self::Transport(m) => m,
        }
    }
}

struct AuthInProgressGuard {
    flag: Arc<AtomicBool>,
    cmd_tx: Arc<Mutex<Option<Sender<WorkerMsg>>>>,
    attached: bool,
}

impl AuthInProgressGuard {
    fn acquire(
        flag: &Arc<AtomicBool>,
        cmd_tx: &Arc<Mutex<Option<Sender<WorkerMsg>>>>,
    ) -> Result<Self, AppError> {
        if flag.swap(true, Ordering::AcqRel) {
            return Err(AppError::Other(
                "Er allerede ved at forbinde. Godkend eller luk vinduet i Discord først".into(),
            ));
        }
        if let Some(tx) = cmd_tx.lock().as_ref() {
            let _ = tx.send(WorkerMsg::PauseForAuth);
        }
        Ok(Self {
            flag: Arc::clone(flag),
            cmd_tx: Arc::clone(cmd_tx),
            attached: false,
        })
    }

    fn finish_attached(mut self) {
        self.attached = true;
        self.flag.store(false, Ordering::Release);
    }
}

impl Drop for AuthInProgressGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
        if !self.attached {
            if let Some(tx) = self.cmd_tx.lock().as_ref() {
                let _ = tx.send(WorkerMsg::ReloadCredentials);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SnapshotKey {
    running: bool,
    in_call: bool,
    mode: DiscordConnectionMode,
    channel_name: Option<String>,
    participants: Vec<DiscordParticipant>,
    self_mute: bool,
    self_deaf: bool,
    can_control_voice: bool,
    call_started_at: Option<i64>,
}

impl SnapshotKey {
    fn from_snapshot(s: &DiscordSnapshot) -> Self {
        Self {
            running: s.running,
            in_call: s.in_call,
            mode: s.mode,
            channel_name: s.channel_name.clone(),
            participants: s.participants.clone(),
            self_mute: s.self_mute,
            self_deaf: s.self_deaf,
            can_control_voice: s.can_control_voice,
            call_started_at: s.call_started_at,
        }
    }
}

pub struct DiscordProvider {
    worker: Mutex<Option<Worker>>,
    latest: Arc<Mutex<Option<DiscordSnapshot>>>,
    config_dir: Mutex<Option<PathBuf>>,
    avatar_cache_dir: Mutex<Option<PathBuf>>,
    rpc_status: Arc<Mutex<DiscordRpcStatus>>,
    cmd_tx: Arc<Mutex<Option<Sender<WorkerMsg>>>>,
    auth_in_progress: Arc<AtomicBool>,
}

struct ClearCmdTxOnExit {
    stop: StopToken,
    cmd_tx: Arc<Mutex<Option<Sender<WorkerMsg>>>>,
}

impl Drop for ClearCmdTxOnExit {
    fn drop(&mut self) {
        if !self.stop.is_stopped() {
            *self.cmd_tx.lock() = None;
        }
    }
}

impl Default for DiscordProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordProvider {
    pub fn new() -> Self {
        Self {
            worker: Mutex::new(None),
            latest: Arc::new(Mutex::new(None)),
            config_dir: Mutex::new(None),
            avatar_cache_dir: Mutex::new(None),
            rpc_status: Arc::new(Mutex::new(DiscordRpcStatus::default())),
            cmd_tx: Arc::new(Mutex::new(None)),
            auth_in_progress: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn configure(&self, app: &AppHandle) {
        let cfg_dir = crate::paths::data_root();
        let cache_dir = cfg_dir.join("discord-avatars");
        *self.config_dir.lock() = Some(cfg_dir.clone());
        *self.avatar_cache_dir.lock() = Some(cache_dir.clone());

        if let Some(creds) = credentials::load_credentials(&cfg_dir) {
            let profile = profile_from_creds(&creds);
            let avatar_path =
                ensure_self_avatar_cached(app, &self.rpc_status, &cache_dir, &profile);
            let mut st = self.rpc_status.lock();
            st.configured = !creds.client_id.trim().is_empty() && creds.access_token.is_some();
            st.connected = false;
            st.via_central = creds.auth_via_central;
            st.client_id = Some(creds.client_id);
            st.redirect_uri = if creds.redirect_uri.trim().is_empty() {
                DEFAULT_REDIRECT_URI.to_string()
            } else {
                creds.redirect_uri
            };
            st.has_secret = !creds.client_secret.trim().is_empty();
            st.user_id = profile.user_id;
            st.username = profile.username;
            st.display_name = profile.display_name;
            st.avatar_path = avatar_path;
            st.can_control_voice = false;
            st.last_error = None;
        }
    }

    pub fn snapshot(&self) -> Option<DiscordSnapshot> {
        self.latest.lock().clone()
    }

    pub fn rpc_status(&self) -> DiscordRpcStatus {
        self.rpc_status.lock().clone()
    }

    pub fn start(&self, app: &AppHandle) {
        let mut slot = self.worker.lock();
        if slot.as_ref().is_some_and(|worker| !worker.is_finished()) {
            return;
        }
        let cfg_dir = {
            let guard = self.config_dir.lock();
            guard.clone()
        }
        .unwrap_or_else(crate::paths::data_root);
        let cache_dir = {
            let guard = self.avatar_cache_dir.lock();
            guard.clone()
        }
        .unwrap_or_else(|| cfg_dir.join("discord-avatars"));

        log::info!(target: "provider:discord", "Starter discord-provider (RPC + WASAPI fallback)");
        let latest = Arc::clone(&self.latest);
        let rpc_status = Arc::clone(&self.rpc_status);
        let auth_in_progress = Arc::clone(&self.auth_in_progress);
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        *self.cmd_tx.lock() = Some(tx.clone());
        let cmd_tx_ref = Arc::clone(&self.cmd_tx);
        let app_handle = app.clone();

        let spawned = Worker::spawn("od-discord", "provider:discord", move |stop| {
            let _clear_cmd_tx_on_exit = ClearCmdTxOnExit {
                stop: stop.clone(),
                cmd_tx: cmd_tx_ref,
            };
            run_discord_thread(
                app_handle,
                stop,
                latest,
                rpc_status,
                auth_in_progress,
                cfg_dir,
                cache_dir,
                tx,
                rx,
            );
        });
        if spawned.is_none() {
            *self.cmd_tx.lock() = None;
        }
        *slot = spawned;
    }

    pub fn stop(&self) {
        let Some(worker) = self.worker.lock().take() else {
            return;
        };
        log::info!(target: "provider:discord", "Stopper discord-provider");
        if let Some(tx) = self.cmd_tx.lock().take() {
            let _ = tx.send(WorkerMsg::Stop);
        }
        drop(worker);
    }

    pub async fn connect_rpc(
        &self,
        app: &AppHandle,
        client_id: String,
        client_secret: String,
        redirect_uri: Option<String>,
    ) -> Result<DiscordRpcStatus, AppError> {
        let cfg_dir = self
            .config_dir
            .lock()
            .clone()
            .unwrap_or_else(crate::paths::data_root);

        let clean_id = client_id.trim().to_string();
        if clean_id.is_empty() {
            return Err(AppError::Other("Indsæt dit Client ID".into()));
        }

        let stored = credentials::load_credentials(&cfg_dir);
        let trimmed_secret = client_secret.trim().to_string();
        let clean_secret = if !trimmed_secret.is_empty() {
            trimmed_secret
        } else if let Some(prev) = stored.as_ref().filter(|s| s.client_id == clean_id) {
            if prev.client_secret.trim().is_empty() {
                return Err(AppError::Other("Client Secret mangler".into()));
            }
            prev.client_secret.clone()
        } else {
            return Err(AppError::Other("Client Secret mangler".into()));
        };

        let effective_redirect = redirect_uri
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| stored.as_ref().map(|s| s.redirect_uri.clone()))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_REDIRECT_URI.to_string());

        let auth_guard = AuthInProgressGuard::acquire(&self.auth_in_progress, &self.cmd_tx)?;
        if self.cmd_tx.lock().is_some() {
            tokio::time::sleep(Duration::from_millis(60)).await;
        }

        let cid_auth = clean_id.clone();
        let (code, _, initial_profile, mut pipe) =
            match tokio::task::spawn_blocking(move || ipc::request_authorize_code(&cid_auth))
                .await
            {
                Ok(Ok(res)) => res,
                Ok(Err(msg)) => {
                    self.record_rpc_error(app, Some(clean_id), effective_redirect, msg.clone());
                    return Err(AppError::Other(msg));
                }
                Err(join_err) => {
                    let msg = format!("Intern opgavefejl under Discord-godkendelse: {join_err}");
                    self.record_rpc_error(app, Some(clean_id), effective_redirect, msg.clone());
                    return Err(AppError::Other(msg));
                }
            };

        let token = match ipc::exchange_code_for_token(
            &clean_id,
            &clean_secret,
            &effective_redirect,
            &code,
        )
        .await
        {
            Ok(t) => t,
            Err(msg) => {
                pipe.close();
                self.record_rpc_error(app, Some(clean_id), effective_redirect, msg.clone());
                return Err(AppError::Other(msg));
            }
        };

        let cid_verify = clean_id.clone();
        let access_tok_verify = token.access_token.clone();
        let (mut pipe, can_write, auth_profile) = match tokio::task::spawn_blocking(move || {
            authenticate_or_reopen_pipe(pipe, &cid_verify, &access_tok_verify)
        })
        .await
        {
            Ok(Ok(res)) => res,
            Ok(Err(msg)) => {
                self.record_rpc_error(app, Some(clean_id), effective_redirect, msg.clone());
                return Err(AppError::Other(msg));
            }
            Err(join_err) => {
                let msg = format!("Intern opgavefejl under Discord-bekræftelse: {join_err}");
                self.record_rpc_error(app, Some(clean_id), effective_redirect, msg.clone());
                return Err(AppError::Other(msg));
            }
        };

        let profile = auth_profile.merged_with(&initial_profile);
        let now = unix_now_ms();
        let expires_at_ms = if token.expires_in > 0 {
            Some(now.saturating_add(token.expires_in.saturating_mul(1000)))
        } else {
            None
        };

        let creds = DiscordRpcCredentials {
            client_id: clean_id.clone(),
            client_secret: clean_secret,
            redirect_uri: effective_redirect.clone(),
            access_token: Some(token.access_token),
            refresh_token: token.refresh_token,
            expires_at_ms,
            user_id: profile.user_id.clone(),
            username: profile.username.clone(),
            display_name: profile.display_name.clone(),
            avatar_hash: profile.avatar_hash.clone(),
            discriminator: profile.discriminator.clone(),
            auth_via_central: false,
            proxy_url: None,
        };

        if let Err(err) = credentials::save_credentials(&cfg_dir, &creds) {
            pipe.close();
            return Err(AppError::Other(err));
        }

        auth_guard.finish_attached();

        let cache_dir = self
            .avatar_cache_dir
            .lock()
            .clone()
            .unwrap_or_else(|| cfg_dir.join("discord-avatars"));
        let avatar_path = ensure_self_avatar_cached(app, &self.rpc_status, &cache_dir, &profile);

        let status = {
            let mut st = self.rpc_status.lock();
            st.configured = true;
            st.connected = true;
            st.via_central = false;
            st.client_id = Some(clean_id);
            st.redirect_uri = effective_redirect;
            st.has_secret = true;
            st.user_id = profile.user_id.clone();
            st.username = profile.username.clone();
            st.display_name = profile.display_name.clone();
            st.avatar_path = avatar_path;
            st.can_control_voice = can_write;
            st.last_error = None;
            st.clone()
        };

        let handed_off = if let Some(tx) = self.cmd_tx.lock().as_ref() {
            tx.send(WorkerMsg::AttachPipe {
                pipe,
                can_write,
                profile,
            })
            .is_ok()
        } else {
            pipe.close();
            false
        };
        log::info!(
            target: "provider:discord",
            "Discord RPC forbundet (egen app, handoff={handed_off})"
        );
        emit_rpc_status(app, &status);
        Ok(status)
    }

    pub async fn connect_rpc_central(&self, app: &AppHandle) -> Result<DiscordRpcStatus, AppError> {
        let cfg_dir = self
            .config_dir
            .lock()
            .clone()
            .unwrap_or_else(crate::paths::data_root);
        let base = ipc::central_base_url();

        let auth_guard = AuthInProgressGuard::acquire(&self.auth_in_progress, &self.cmd_tx)?;

        let (client_id, redirect_uri) = match ipc::fetch_central_rpc_config(&base).await {
            Ok(pair) => pair,
            Err(msg) => {
                self.record_rpc_error(app, None, DEFAULT_REDIRECT_URI.to_string(), msg.clone());
                return Err(AppError::Other(msg));
            }
        };

        let cid_auth = client_id.clone();
        let (code, _, initial_profile, mut pipe) =
            match tokio::task::spawn_blocking(move || ipc::request_authorize_code(&cid_auth))
                .await
            {
                Ok(Ok(res)) => res,
                Ok(Err(msg)) => {
                    self.record_rpc_error(app, Some(client_id), redirect_uri, msg.clone());
                    return Err(AppError::Other(msg));
                }
                Err(join_err) => {
                    let msg = format!("Intern opgavefejl under Discord-godkendelse: {join_err}");
                    self.record_rpc_error(app, Some(client_id), redirect_uri, msg.clone());
                    return Err(AppError::Other(msg));
                }
            };

        let token = match ipc::exchange_code_via_central(&base, &code).await {
            Ok(t) => t,
            Err(msg) => {
                pipe.close();
                self.record_rpc_error(app, Some(client_id), redirect_uri, msg.clone());
                return Err(AppError::Other(msg));
            }
        };

        let cid_verify = client_id.clone();
        let access_tok_verify = token.access_token.clone();
        let (mut pipe, can_write, auth_profile) = match tokio::task::spawn_blocking(move || {
            authenticate_or_reopen_pipe(pipe, &cid_verify, &access_tok_verify)
        })
        .await
        {
            Ok(Ok(res)) => res,
            Ok(Err(msg)) => {
                self.record_rpc_error(app, Some(client_id), redirect_uri, msg.clone());
                return Err(AppError::Other(msg));
            }
            Err(join_err) => {
                let msg = format!("Intern opgavefejl under Discord-bekræftelse: {join_err}");
                self.record_rpc_error(app, Some(client_id), redirect_uri, msg.clone());
                return Err(AppError::Other(msg));
            }
        };

        let profile = auth_profile.merged_with(&initial_profile);
        let now = unix_now_ms();
        let expires_at_ms = if token.expires_in > 0 {
            Some(now.saturating_add(token.expires_in.saturating_mul(1000)))
        } else {
            None
        };

        let creds = DiscordRpcCredentials {
            client_id: client_id.clone(),
            client_secret: String::new(),
            redirect_uri: redirect_uri.clone(),
            access_token: Some(token.access_token),
            refresh_token: token.refresh_token,
            expires_at_ms,
            user_id: profile.user_id.clone(),
            username: profile.username.clone(),
            display_name: profile.display_name.clone(),
            avatar_hash: profile.avatar_hash.clone(),
            discriminator: profile.discriminator.clone(),
            auth_via_central: true,
            proxy_url: Some(base),
        };

        if let Err(err) = credentials::save_credentials(&cfg_dir, &creds) {
            pipe.close();
            return Err(AppError::Other(err));
        }

        auth_guard.finish_attached();

        let cache_dir = self
            .avatar_cache_dir
            .lock()
            .clone()
            .unwrap_or_else(|| cfg_dir.join("discord-avatars"));
        let avatar_path = ensure_self_avatar_cached(app, &self.rpc_status, &cache_dir, &profile);

        let status = {
            let mut st = self.rpc_status.lock();
            st.configured = true;
            st.connected = true;
            st.via_central = true;
            st.client_id = Some(client_id);
            st.redirect_uri = redirect_uri;
            st.has_secret = false;
            st.user_id = profile.user_id.clone();
            st.username = profile.username.clone();
            st.display_name = profile.display_name.clone();
            st.avatar_path = avatar_path;
            st.can_control_voice = can_write;
            st.last_error = None;
            st.clone()
        };

        let handed_off = if let Some(tx) = self.cmd_tx.lock().as_ref() {
            tx.send(WorkerMsg::AttachPipe {
                pipe,
                can_write,
                profile,
            })
            .is_ok()
        } else {
            pipe.close();
            false
        };
        log::info!(
            target: "provider:discord",
            "Discord RPC forbundet via OSSI Central (handoff={handed_off})"
        );
        emit_rpc_status(app, &status);
        Ok(status)
    }

    pub fn disconnect_rpc(&self, app: &AppHandle) -> Result<DiscordRpcStatus, AppError> {
        let cfg_dir = self
            .config_dir
            .lock()
            .clone()
            .unwrap_or_else(crate::paths::data_root);
        credentials::delete_credentials(&cfg_dir).map_err(AppError::Other)?;

        let status = {
            let mut st = self.rpc_status.lock();
            *st = DiscordRpcStatus::default();
            st.clone()
        };
        if let Some(tx) = self.cmd_tx.lock().as_ref() {
            let _ = tx.send(WorkerMsg::ReloadCredentials);
        }
        emit_rpc_status(app, &status);
        Ok(status)
    }

    pub fn voice_action(&self, action: DiscordVoiceAction) -> Result<(), AppError> {
        let st = self.rpc_status.lock().clone();
        if !st.connected {
            return Err(AppError::Other(
                "Discord er ikke forbundet lige nu".into(),
            ));
        }
        let guard = self.cmd_tx.lock();
        let Some(tx) = guard.as_ref() else {
            return Err(AppError::Other("Forbindelsen til Discord er afbrudt".into()));
        };
        tx.send(WorkerMsg::VoiceAction(action))
            .map_err(|_| AppError::Other("Kunne ikke sende kommandoen til Discord. Prøv igen".into()))
    }

    fn record_rpc_error(
        &self,
        app: &AppHandle,
        client_id: Option<String>,
        redirect_uri: String,
        message: String,
    ) {
        let status = {
            let mut st = self.rpc_status.lock();
            if client_id.is_some() {
                st.client_id = client_id;
            }
            st.redirect_uri = redirect_uri;
            st.last_error = Some(message);
            st.clone()
        };
        emit_rpc_status(app, &status);
    }
}

fn emit_rpc_status(app: &AppHandle, status: &DiscordRpcStatus) {
    if app.get_webview_window("control").is_some() {
        let _ = app.emit_to("control", "discord://rpc-status", status);
    }
}


fn authenticate_pipe(
    pipe: &mut DiscordIpcPipe,
    access_token: &str,
) -> Result<(bool, serde_json::Value), AuthPipeError> {
    let nonce = pipe
        .send_command("AUTHENTICATE", json!({ "access_token": access_token }), None)
        .map_err(|e| AuthPipeError::Transport(e.to_string()))?;
    let resp = pipe
        .wait_for_nonce(&nonce, IPC_AUTH_TIMEOUT)
        .map_err(|e| AuthPipeError::Transport(e.to_string()))?;
    if let Some(err_msg) = ipc::extract_rpc_error(&resp) {
        return Err(AuthPipeError::TokenRejected(err_msg));
    }
    let data = resp.get("data").cloned().unwrap_or(serde_json::Value::Null);
    let can_write = data
        .get("scopes")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().any(|s| s.as_str() == Some("rpc.voice.write")))
        .unwrap_or(true);
    Ok((can_write, data))
}

fn profile_from_creds(creds: &DiscordRpcCredentials) -> SelfUserProfile {
    SelfUserProfile {
        user_id: creds.user_id.clone(),
        username: creds.username.clone(),
        display_name: creds
            .display_name
            .clone()
            .or_else(|| creds.username.clone()),
        avatar_hash: creds.avatar_hash.clone(),
        discriminator: creds.discriminator.clone(),
    }
}

fn apply_profile_to_creds(creds: &mut DiscordRpcCredentials, profile: &SelfUserProfile) -> bool {
    let mut changed = false;
    if profile.user_id.is_some() && creds.user_id != profile.user_id {
        creds.user_id = profile.user_id.clone();
        changed = true;
    }
    if profile.username.is_some() && creds.username != profile.username {
        creds.username = profile.username.clone();
        changed = true;
    }
    if profile.display_name.is_some() && creds.display_name != profile.display_name {
        creds.display_name = profile.display_name.clone();
        changed = true;
    }
    if profile.avatar_hash.is_some() && creds.avatar_hash != profile.avatar_hash {
        creds.avatar_hash = profile.avatar_hash.clone();
        changed = true;
    }
    if profile.discriminator.is_some() && creds.discriminator != profile.discriminator {
        creds.discriminator = profile.discriminator.clone();
        changed = true;
    }
    changed
}

fn ensure_self_avatar_cached(
    app: &AppHandle,
    rpc_status: &Arc<Mutex<DiscordRpcStatus>>,
    cache_dir: &Path,
    profile: &SelfUserProfile,
) -> Option<String> {
    let uid = profile.user_id.as_deref()?;
    if let Some(existing) = avatars::cached_avatar_path(
        cache_dir,
        uid,
        profile.avatar_hash.as_deref(),
        profile.discriminator.as_deref(),
    ) {
        return Some(existing);
    }

    let dir = cache_dir.to_path_buf();
    let uid_owned = uid.to_string();
    let hash = profile.avatar_hash.clone();
    let disc = profile.discriminator.clone();
    let status_ref = Arc::clone(rpc_status);
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(path) =
            avatars::fetch_and_cache_avatar(dir, uid_owned.clone(), hash, disc).await
        {
            let updated = {
                let mut st = status_ref.lock();
                if st.user_id.as_deref() == Some(uid_owned.as_str())
                    && st.avatar_path.as_deref() != Some(path.as_str())
                {
                    st.avatar_path = Some(path);
                    Some(st.clone())
                } else {
                    None
                }
            };
            if let Some(st) = updated {
                emit_rpc_status(&app_handle, &st);
            }
        }
    });
    None
}

fn apply_auth_user_to_pipe(pipe: &mut DiscordIpcPipe, data: &serde_json::Value) -> SelfUserProfile {
    if let Some(user) = data.get("user") {
        let parsed = SelfUserProfile::from_user_value(user);
        pipe.self_profile.merge_from(&parsed);
    }
    pipe.self_profile.clone()
}

/// Autentificerer direkte på den eksisterende `AUTHORIZE`-pipe for at undgå pipe-churn og
/// race conditions mod Discord. Hvis Discord lukkede pipen under token-udvekslingen, åbnes
/// én frisk pipe som fallback efter en kort pause.
fn authenticate_or_reopen_pipe(
    mut pipe: DiscordIpcPipe,
    client_id: &str,
    access_token: &str,
) -> Result<(DiscordIpcPipe, bool, SelfUserProfile), String> {
    match authenticate_pipe(&mut pipe, access_token) {
        Ok((can_write, data)) => {
            let profile = apply_auth_user_to_pipe(&mut pipe, &data);
            Ok((pipe, can_write, profile))
        }
        Err(AuthPipeError::TokenRejected(msg)) => {
            pipe.close();
            Err(msg)
        }
        Err(AuthPipeError::Transport(first_err)) => {
            log::info!(
                target: "provider:discord",
                "Genbrugt AUTHORIZE-pipe afbrød under AUTHENTICATE ({first_err}); prøver frisk pipe"
            );
            pipe.close();
            drop(pipe);
            std::thread::sleep(Duration::from_millis(250));
            let mut fresh = ipc::connect_and_handshake(client_id)?;
            match authenticate_pipe(&mut fresh, access_token) {
                Ok((can_write, data)) => {
                    let profile = apply_auth_user_to_pipe(&mut fresh, &data);
                    Ok((fresh, can_write, profile))
                }
                Err(err) => {
                    fresh.close();
                    Err(err.into_message())
                }
            }
        }
    }
}

fn subscribe_initial_events(pipe: &mut DiscordIpcPipe) {
    let _ = pipe.send_command("SUBSCRIBE", json!({}), Some("VOICE_CHANNEL_SELECT"));
    let _ = pipe.send_command("SUBSCRIBE", json!({}), Some("VOICE_SETTINGS_UPDATE"));
    let _ = pipe.send_command("GET_VOICE_SETTINGS", json!({}), None);
    let _ = pipe.send_command("GET_SELECTED_VOICE_CHANNEL", json!({}), None);
}

fn close_active_pipe(active: &mut Option<(DiscordIpcPipe, RpcSessionState)>) {
    if let Some((mut pipe, state)) = active.take() {
        if let Some(ch_id) = state.subscribed_channel_id.as_deref() {
            unsubscribe_channel(&mut pipe, ch_id);
        }
        pipe.close();
    }
}

struct RpcSessionState {
    channel_id: Option<String>,
    channel_name: Option<String>,
    participants: Vec<RawParticipant>,
    speaking_ids: HashSet<String>,
    self_mute: bool,
    self_deaf: bool,
    can_control_voice: bool,
    call_started_at: Option<i64>,
    self_session_id: Option<String>,
    subscribed_channel_id: Option<String>,
    self_user_id: Option<String>,
    requested_avatars: HashSet<String>,
}

impl RpcSessionState {
    fn new(can_control_voice: bool, self_user_id: Option<String>) -> Self {
        Self {
            channel_id: None,
            channel_name: None,
            participants: Vec::new(),
            speaking_ids: HashSet::new(),
            self_mute: false,
            self_deaf: false,
            can_control_voice,
            call_started_at: None,
            self_session_id: None,
            subscribed_channel_id: None,
            self_user_id,
            requested_avatars: HashSet::new(),
        }
    }

    /// Slår call-timerens start-tidspunkt op i `timer`. RPC `session_id` er
    /// en autoritativ sessions-nøgle: den kan kun matche et gemt record hvis
    /// voice-sessionen faktisk fortsatte på tværs af app-genstart.
    fn call_timer_value(&self, timer: &mut CallTimer, now_ms: i64) -> i64 {
        match self.self_session_id.as_deref() {
            Some(sid) => timer.observe(&format!("rpc:{sid}"), true, now_ms),
            None => timer.observe_unknown(now_ms),
        }
    }

    fn apply_voice_channel(
        &mut self,
        pipe: &mut DiscordIpcPipe,
        parsed: Option<ipc::ParsedVoiceChannel>,
        timer: &mut CallTimer,
        now_ms: i64,
    ) {
        match parsed {
            None => {
                if let Some(old_id) = self.subscribed_channel_id.take() {
                    unsubscribe_channel(pipe, &old_id);
                }
                self.channel_id = None;
                self.channel_name = None;
                self.participants.clear();
                self.speaking_ids.clear();
                self.call_started_at = None;
                self.self_session_id = None;
                timer.clear();
            }
            Some(ch) => {
                let changed = self.subscribed_channel_id.as_deref() != Some(ch.id.as_str());
                if changed {
                    if let Some(old_id) = self.subscribed_channel_id.take() {
                        unsubscribe_channel(pipe, &old_id);
                    }
                    subscribe_channel(pipe, &ch.id);
                    self.subscribed_channel_id = Some(ch.id.clone());
                }
                self.self_session_id = ch
                    .participants
                    .iter()
                    .find(|p| p.is_self)
                    .and_then(|p| p.session_id.clone());
                self.channel_id = Some(ch.id);
                self.channel_name = ch.name;
                self.participants = ch.participants;
                self.call_started_at = Some(self.call_timer_value(timer, now_ms));
                let active_ids: HashSet<&str> =
                    self.participants.iter().map(|p| p.id.as_str()).collect();
                self.speaking_ids.retain(|id| active_ids.contains(id.as_str()));
            }
        }
    }

    fn upsert_participant(&mut self, participant: RawParticipant) {
        if let Some(existing) = self.participants.iter_mut().find(|p| p.id == participant.id) {
            *existing = participant;
        } else {
            self.participants.push(participant);
        }
    }

    fn remove_participant(&mut self, user_id: &str) {
        self.participants.retain(|p| p.id != user_id);
        self.speaking_ids.remove(user_id);
    }

    fn build_snapshot(
        &mut self,
        cache_dir: &Path,
        tx: &Sender<WorkerMsg>,
        now_ms: i64,
    ) -> DiscordSnapshot {
        let mut out: Vec<DiscordParticipant> = self
            .participants
            .iter()
            .map(|raw| {
                let avatar_path = avatars::cached_avatar_path(
                    cache_dir,
                    &raw.id,
                    raw.avatar.as_deref(),
                    raw.discriminator.as_deref(),
                );
                if avatar_path.is_none() {
                    let key = avatars::avatar_cache_key(
                        &raw.id,
                        raw.avatar.as_deref(),
                        raw.discriminator.as_deref(),
                    );
                    if self.requested_avatars.insert(key) {
                        let dir = cache_dir.to_path_buf();
                        let uid = raw.id.clone();
                        let hash = raw.avatar.clone();
                        let disc = raw.discriminator.clone();
                        let notify = tx.clone();
                        tauri::async_runtime::spawn(async move {
                            if avatars::fetch_and_cache_avatar(dir, uid, hash, disc)
                                .await
                                .is_some()
                            {
                                let _ = notify.send(WorkerMsg::AvatarReady);
                            }
                        });
                    }
                }
                let speaking = self.speaking_ids.contains(&raw.id) && !raw.muted && !raw.deafened;
                DiscordParticipant {
                    id: raw.id.clone(),
                    username: raw.username.clone(),
                    display_name: raw.display_name.clone(),
                    avatar_path,
                    speaking,
                    muted: raw.muted,
                    deafened: raw.deafened,
                    is_self: raw.is_self,
                }
            })
            .collect();

        out.sort_by(|a, b| {
            let group_a = participant_sort_group(a);
            let group_b = participant_sort_group(b);
            group_a.cmp(&group_b).then_with(|| {
                a.display_name
                    .to_ascii_lowercase()
                    .cmp(&b.display_name.to_ascii_lowercase())
            })
        });

        let in_call = self.channel_id.is_some();
        DiscordSnapshot {
            running: true,
            in_call,
            mode: DiscordConnectionMode::Rpc,
            channel_name: if in_call { self.channel_name.clone() } else { None },
            participants: if in_call { out } else { Vec::new() },
            self_mute: self.self_mute,
            self_deaf: self.self_deaf,
            can_control_voice: self.can_control_voice,
            call_started_at: if in_call { self.call_started_at } else { None },
            updated_at: now_ms,
        }
    }
}

fn participant_sort_group(p: &DiscordParticipant) -> u8 {
    match (p.is_self, p.speaking) {
        (false, true) => 0,
        (false, false) => 1,
        (true, true) => 2,
        (true, false) => 3,
    }
}

fn subscribe_channel(pipe: &mut DiscordIpcPipe, channel_id: &str) {
    for evt in RPC_CHANNEL_EVENTS {
        let _ = pipe.send_command("SUBSCRIBE", json!({ "channel_id": channel_id }), Some(evt));
    }
}

fn unsubscribe_channel(pipe: &mut DiscordIpcPipe, channel_id: &str) {
    for evt in RPC_CHANNEL_EVENTS {
        let _ = pipe.send_command("UNSUBSCRIBE", json!({ "channel_id": channel_id }), Some(evt));
    }
}

fn refresh_credentials_token(
    creds: &DiscordRpcCredentials,
    refresh_tok: &str,
) -> Result<ipc::TokenResponse, String> {
    if creds.auth_via_central {
        let base = creds
            .proxy_url
            .as_deref()
            .unwrap_or(ipc::DEFAULT_CENTRAL_BASE);
        tauri::async_runtime::block_on(ipc::refresh_token_via_central(base, refresh_tok))
    } else {
        tauri::async_runtime::block_on(ipc::refresh_access_token(
            &creds.client_id,
            &creds.client_secret,
            refresh_tok,
        ))
    }
}

fn try_open_and_auth_rpc(
    config_dir: &Path,
    creds: &mut DiscordRpcCredentials,
) -> Result<(DiscordIpcPipe, bool, SelfUserProfile), AuthPipeError> {
    let now = unix_now_ms();
    if creds
        .expires_at_ms
        .is_some_and(|exp| exp <= now + 5 * 60 * 1000)
    {
        if let Some(refresh_tok) = creds.refresh_token.clone() {
            match refresh_credentials_token(creds, &refresh_tok) {
                Ok(new_tok) => {
                    creds.access_token = Some(new_tok.access_token);
                    if let Some(rt) = new_tok.refresh_token {
                        creds.refresh_token = Some(rt);
                    }
                    if new_tok.expires_in > 0 {
                        creds.expires_at_ms =
                            Some(unix_now_ms().saturating_add(new_tok.expires_in.saturating_mul(1000)));
                    }
                    let _ = credentials::save_credentials(config_dir, creds);
                }
                Err(err) => {
                    log::warn!(target: "provider:discord", "Token-fornyelse fejlede: {err}");
                }
            }
        }
    }

    let token = creds
        .access_token
        .clone()
        .ok_or_else(|| AuthPipeError::TokenRejected("Mangler access token til Discord RPC".into()))?;

    let mut pipe = ipc::connect_and_handshake(&creds.client_id).map_err(AuthPipeError::Transport)?;
    match authenticate_pipe(&mut pipe, &token) {
        Ok((can_write, data)) => {
            let auth_profile = apply_auth_user_to_pipe(&mut pipe, &data);
            let final_profile = auth_profile.merged_with(&profile_from_creds(creds));
            if apply_profile_to_creds(creds, &final_profile) {
                let _ = credentials::save_credentials(config_dir, creds);
            }
            Ok((pipe, can_write, final_profile))
        }
        Err(AuthPipeError::Transport(err)) => {
            // Transport-fejl (fx timeout eller afbrudt pipe): forny aldrig token og genbrug aldrig pipen.
            pipe.close();
            Err(AuthPipeError::Transport(err))
        }
        Err(AuthPipeError::TokenRejected(first_err)) => {
            let Some(refresh_tok) = creds.refresh_token.clone() else {
                pipe.close();
                return Err(AuthPipeError::TokenRejected(first_err));
            };
            let new_tok = refresh_credentials_token(creds, &refresh_tok)
                .map_err(AuthPipeError::TokenRejected)?;
            creds.access_token = Some(new_tok.access_token.clone());
            if let Some(rt) = new_tok.refresh_token {
                creds.refresh_token = Some(rt);
            }
            if new_tok.expires_in > 0 {
                creds.expires_at_ms =
                    Some(unix_now_ms().saturating_add(new_tok.expires_in.saturating_mul(1000)));
            }
            let _ = credentials::save_credentials(config_dir, creds);

            let (pipe, can_write, auth_profile) =
                authenticate_or_reopen_pipe(pipe, &creds.client_id, &new_tok.access_token)
                    .map_err(AuthPipeError::TokenRejected)?;
            let final_profile = auth_profile.merged_with(&profile_from_creds(creds));
            if apply_profile_to_creds(creds, &final_profile) {
                let _ = credentials::save_credentials(config_dir, creds);
            }
            Ok((pipe, can_write, final_profile))
        }
    }
}




#[allow(clippy::too_many_arguments)]
fn run_discord_thread(
    app: AppHandle,
    stop: StopToken,
    latest: Arc<Mutex<Option<DiscordSnapshot>>>,
    rpc_status: Arc<Mutex<DiscordRpcStatus>>,
    auth_in_progress: Arc<AtomicBool>,
    config_dir: PathBuf,
    cache_dir: PathBuf,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
) {
    let _com = crate::util::ComMtaGuard::init();
    let mut detector = CallDetector::default();
    let mut audio_monitor = AudioSessionMonitor::default();
    let mut call_timer = CallTimer::load(&config_dir);
    let mut last_key: Option<SnapshotKey> = None;
    let mut active_pipe: Option<(DiscordIpcPipe, RpcSessionState)> = None;
    let mut cached_creds: Option<DiscordRpcCredentials> = credentials::load_credentials(&config_dir);
    let mut next_rpc_attempt = Instant::now();
    let mut next_audio_poll = Instant::now();
    let mut consecutive_transport_errors: u32 = 0;
    let mut last_error_is_transport = false;

    let emit_snapshot = |snapshot: DiscordSnapshot, last_key: &mut Option<SnapshotKey>| {
        *latest.lock() = Some(snapshot.clone());
        let key = SnapshotKey::from_snapshot(&snapshot);
        if last_key.as_ref() != Some(&key) {
            *last_key = Some(key);
            super::scheduler::emit_provider(
                &app,
                super::scheduler::ProviderKind::Discord,
                None,
                "provider://discord",
                &snapshot,
            );
        }
    };

    let set_rpc_connected = |connected: bool,
                             can_control_voice: bool,
                             profile: Option<&SelfUserProfile>,
                             last_error: Option<String>| {
        let avatar_path = profile
            .and_then(|p| ensure_self_avatar_cached(&app, &rpc_status, &cache_dir, p));
        let mut st = rpc_status.lock();
        let next_uid = profile
            .and_then(|p| p.user_id.clone())
            .or_else(|| st.user_id.clone());
        let next_user = profile
            .and_then(|p| p.username.clone())
            .or_else(|| st.username.clone());
        let next_display = profile
            .and_then(|p| p.display_name.clone())
            .or_else(|| st.display_name.clone());
        let next_avatar = avatar_path.or_else(|| st.avatar_path.clone());
        let changed = st.connected != connected
            || st.can_control_voice != can_control_voice
            || st.user_id != next_uid
            || st.username != next_user
            || st.display_name != next_display
            || st.avatar_path != next_avatar
            || st.last_error != last_error;
        st.connected = connected;
        st.can_control_voice = can_control_voice;
        st.user_id = next_uid;
        st.username = next_user;
        st.display_name = next_display;
        st.avatar_path = next_avatar;
        st.last_error = last_error;
        if changed {
            let clone = st.clone();
            drop(st);
            emit_rpc_status(&app, &clone);
        }
    };

    while !stop.is_stopped() {
        // 1. Dræn eventuelle beskeder (pause/attach/reload, voice-mute/deafen, avatar-ready).
        let mut pause_for_auth = false;
        let mut attached_pipe: Option<(DiscordIpcPipe, bool, SelfUserProfile)> = None;
        let mut reload_requested = false;
        let mut avatar_updated = false;
        let mut pending_actions: Vec<DiscordVoiceAction> = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            match msg {
                WorkerMsg::PauseForAuth => {
                    pause_for_auth = true;
                    if let Some((mut prev, _, _)) = attached_pipe.take() {
                        prev.close();
                    }
                }
                WorkerMsg::AttachPipe {
                    pipe,
                    can_write,
                    profile,
                } => {
                    if let Some((mut prev, _, _)) = attached_pipe.replace((pipe, can_write, profile))
                    {
                        prev.close();
                    }
                    pause_for_auth = false;
                    reload_requested = false;
                }
                WorkerMsg::ReloadCredentials => reload_requested = true,
                WorkerMsg::AvatarReady => avatar_updated = true,
                WorkerMsg::VoiceAction(action) => pending_actions.push(action),
                WorkerMsg::Stop => {
                    close_active_pipe(&mut active_pipe);
                    if let Some((mut p, _, _)) = attached_pipe.take() {
                        p.close();
                    }
                    return;
                }
            }
        }

        if pause_for_auth {
            close_active_pipe(&mut active_pipe);
            consecutive_transport_errors = 0;
            last_error_is_transport = false;
        }

        if let Some((mut pipe, can_write, profile)) = attached_pipe {
            cached_creds = credentials::load_credentials(&config_dir);
            close_active_pipe(&mut active_pipe);
            subscribe_initial_events(&mut pipe);
            let self_uid = pipe
                .self_profile
                .user_id
                .clone()
                .or_else(|| profile.user_id.clone());
            let state = RpcSessionState::new(can_write, self_uid);
            active_pipe = Some((pipe, state));
            detector.reset();
            consecutive_transport_errors = 0;
            last_error_is_transport = false;
            set_rpc_connected(true, can_write, Some(&profile), None);
        } else if reload_requested {
            cached_creds = credentials::load_credentials(&config_dir);
            close_active_pipe(&mut active_pipe);
            detector.reset();
            consecutive_transport_errors = 0;
            last_error_is_transport = false;
            next_rpc_attempt = Instant::now();
            next_audio_poll = Instant::now();
            if cached_creds.is_none() {
                set_rpc_connected(false, false, None, None);
            }
        }

        // 2. Hvis RPC er konfigureret og ingen aktiv pipe, prøv at forbinde når Discord kører
        //    og der ikke samtidig er en interaktiv OAuth-forbindelse i gang.
        let mut cached_pids: Option<Vec<u32>> = None;
        let auth_busy = auth_in_progress.load(Ordering::Acquire);
        if !auth_busy && active_pipe.is_none() && Instant::now() >= next_rpc_attempt {
            if let Some(ref mut creds) = cached_creds {
                if creds.access_token.is_some() {
                    let pids = wasapi::discord_process_ids();
                    if !pids.is_empty() {
                        match try_open_and_auth_rpc(&config_dir, creds) {
                            Ok((mut pipe, can_write, profile)) => {
                                subscribe_initial_events(&mut pipe);
                                let self_uid = pipe
                                    .self_profile
                                    .user_id
                                    .clone()
                                    .or_else(|| profile.user_id.clone());
                                let state = RpcSessionState::new(can_write, self_uid);
                                active_pipe = Some((pipe, state));
                                detector.reset();
                                consecutive_transport_errors = 0;
                                last_error_is_transport = false;
                                log::info!(target: "provider:discord", "Discord RPC forbundet");
                                set_rpc_connected(true, can_write, Some(&profile), None);
                            }
                            Err(AuthPipeError::Transport(err)) => {
                                consecutive_transport_errors =
                                    consecutive_transport_errors.saturating_add(1);
                                let backoff = match consecutive_transport_errors {
                                    1 => Duration::from_millis(1500),
                                    2 => Duration::from_secs(3),
                                    _ => IPC_RECONNECT_BACKOFF,
                                };
                                log::info!(
                                    target: "provider:discord",
                                    "Discord RPC transport-forsøg #{consecutive_transport_errors} fejlede: {err}"
                                );
                                // Skjul enkeltstående forbigående transport-fejl i UI (fx under opstart/genforbindelse).
                                let ui_err = if consecutive_transport_errors >= 2 {
                                    Some(err)
                                } else {
                                    None
                                };
                                last_error_is_transport = ui_err.is_some();
                                set_rpc_connected(false, false, None, ui_err);
                                next_rpc_attempt = Instant::now() + backoff;
                            }
                            Err(AuthPipeError::TokenRejected(err)) => {
                                log::warn!(
                                    target: "provider:discord",
                                    "Discord RPC afviste token: {err}"
                                );
                                consecutive_transport_errors = 0;
                                last_error_is_transport = false;
                                set_rpc_connected(false, false, None, Some(err));
                                next_rpc_attempt = Instant::now() + IPC_RECONNECT_BACKOFF;
                            }
                        }
                    } else {
                        if last_error_is_transport {
                            last_error_is_transport = false;
                            set_rpc_connected(false, false, None, None);
                        }
                        consecutive_transport_errors = 0;
                        next_rpc_attempt = Instant::now() + IPC_RECONNECT_BACKOFF;
                    }
                    cached_pids = Some(pids);
                }
            }
        }

        // 3. Aktiv RPC-session: dræn alle IPC-frames + udfør mute/deafen-handlinger.
        if let Some((mut pipe, mut state)) = active_pipe.take() {
            let mut changed = avatar_updated;
            let mut broken = false;

            for action in pending_actions {
                if state.can_control_voice {
                    match action {
                        DiscordVoiceAction::ToggleMute => {
                            state.self_mute = !state.self_mute;
                            if pipe
                                .send_command(
                                    "SET_VOICE_SETTINGS",
                                    json!({ "mute": state.self_mute }),
                                    None,
                                )
                                .is_err()
                            {
                                broken = true;
                            }
                            changed = true;
                        }
                        DiscordVoiceAction::ToggleDeafen => {
                            state.self_deaf = !state.self_deaf;
                            if pipe
                                .send_command(
                                    "SET_VOICE_SETTINGS",
                                    json!({ "deaf": state.self_deaf }),
                                    None,
                                )
                                .is_err()
                            {
                                broken = true;
                            }
                            changed = true;
                        }
                    }
                }
            }

            if !broken {
                loop {
                    match pipe.try_read_frame() {
                        Ok(None) => break,
                        Ok(Some((op, val))) => {
                            if op == ipc::OP_PING {
                                if pipe.write_frame(OP_PONG, &val).is_err() {
                                    broken = true;
                                    break;
                                }
                                continue;
                            }
                            if op == ipc::OP_CLOSE {
                                broken = true;
                                break;
                            }
                            if process_rpc_frame(&mut pipe, &mut state, &mut call_timer, &val) {
                                changed = true;
                            }
                        }
                        Err(PipeError::Broken(_)) => {
                            broken = true;
                            break;
                        }
                        Err(PipeError::Protocol(err)) => {
                            log::warn!(target: "provider:discord", "IPC-protokolfejl: {err}");
                            broken = true;
                            break;
                        }
                        Err(PipeError::NotFound) => {
                            broken = true;
                            break;
                        }
                    }
                }
            }

            if broken {
                log::info!(target: "provider:discord", "Discord RPC-forbindelse afbrudt; planlægger genforbindelse");
                set_rpc_connected(false, false, None, None);
                next_rpc_attempt = Instant::now() + Duration::from_secs(2);
                next_audio_poll = Instant::now();
            } else {
                if changed || last_key.is_none() {
                    let snap = state.build_snapshot(&cache_dir, &tx, unix_now_ms());
                    emit_snapshot(snap, &mut last_key);
                }
                active_pipe = Some((pipe, state));
            }
        } else if Instant::now() >= next_audio_poll {
            // 4. WASAPI mikrofon-fallback når RPC ikke er aktiv.
            let pids = cached_pids.take().unwrap_or_else(wasapi::discord_process_ids);
            let running = !pids.is_empty();
            next_audio_poll = Instant::now()
                + if running {
                    AUDIO_POLL_RUNNING
                } else {
                    AUDIO_POLL_CLOSED
                };
            let now = unix_now_ms();
            let session_id = if running {
                audio_monitor.discord_mic_session(&pids)
            } else {
                None
            };
            let in_call = if running {
                detector.tick(session_id.is_some())
            } else {
                detector.reset();
                false
            };
            let call_started_at = if in_call {
                Some(match session_id.as_deref() {
                    // WASAPI-nøglen er ikke autoritativ: genoptagelse kræver
                    // friskt heartbeat (se CallTimer::observe).
                    Some(id) => call_timer.observe(&format!("wasapi:{id}"), false, now),
                    None => call_timer.observe_unknown(now),
                })
            } else {
                call_timer.clear();
                None
            };
            let snap = DiscordSnapshot {
                running,
                in_call,
                mode: if running {
                    DiscordConnectionMode::Audio
                } else {
                    DiscordConnectionMode::Disconnected
                },
                channel_name: None,
                participants: Vec::new(),
                self_mute: false,
                self_deaf: false,
                can_control_voice: false,
                call_started_at,
                updated_at: now,
            };
            emit_snapshot(snap, &mut last_key);
        }

        // Sov helt til næste planlagte hændelse og vågn øjeblikkeligt på worker-besked (mute/deafen/reload/stop).
        let wait_dur = if let Some((_, ref state)) = active_pipe {
            if state.channel_id.is_some() {
                IPC_IN_CALL_WAIT
            } else {
                IPC_IDLE_WAIT
            }
        } else {
            let now_inst = Instant::now();
            let mut next_wake = next_audio_poll;
            if !auth_in_progress.load(Ordering::Acquire)
                && cached_creds
                    .as_ref()
                    .is_some_and(|c| c.access_token.is_some())
            {
                next_wake = next_wake.min(next_rpc_attempt);
            }
            next_wake
                .saturating_duration_since(now_inst)
                .clamp(Duration::from_millis(50), AUDIO_POLL_CLOSED)
        };
        if let Ok(msg) = rx.recv_timeout(wait_dur) {
            // Re-indsend i køen så top-dræn samler alt ensartet.
            let _ = tx.send(msg);
        }
    }
}

fn process_rpc_frame(
    pipe: &mut DiscordIpcPipe,
    state: &mut RpcSessionState,
    timer: &mut CallTimer,
    val: &serde_json::Value,
) -> bool {
    let cmd = val.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
    let evt = val.get("evt").and_then(|v| v.as_str()).unwrap_or("");
    let Some(data) = val.get("data") else {
        return false;
    };

    if evt == "ERROR" {
        return false;
    }

    match (cmd, evt) {
        ("GET_SELECTED_VOICE_CHANNEL", _) => {
            let parsed = ipc::parse_voice_channel(data, state.self_user_id.as_deref());
            state.apply_voice_channel(pipe, parsed, timer, unix_now_ms());
            true
        }
        ("GET_VOICE_SETTINGS", _) | ("SET_VOICE_SETTINGS", _) | ("DISPATCH", "VOICE_SETTINGS_UPDATE") => {
            if let Some((mute, deaf)) = ipc::parse_voice_settings(data) {
                state.self_mute = mute;
                state.self_deaf = deaf;
                return true;
            }
            false
        }
        ("DISPATCH", "VOICE_CHANNEL_SELECT") => {
            let ch_id = ipc::non_empty_str(data.get("channel_id"));
            if ch_id.is_none() {
                state.apply_voice_channel(pipe, None, timer, unix_now_ms());
                return true;
            }
            let _ = pipe.send_command("GET_SELECTED_VOICE_CHANNEL", json!({}), None);
            false
        }
        ("DISPATCH", "VOICE_STATE_CREATE") | ("DISPATCH", "VOICE_STATE_UPDATE") => {
            if let Some(p) = ipc::parse_voice_state_entry(data, state.self_user_id.as_deref()) {
                if p.is_self {
                    if let Some(vs) = data.get("voice_state") {
                        state.self_mute = vs
                            .get("self_mute")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(state.self_mute);
                        state.self_deaf = vs
                            .get("self_deaf")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(state.self_deaf);
                    }
                    if p.session_id.is_some() {
                        state.self_session_id = p.session_id.clone();
                        state.call_started_at =
                            Some(state.call_timer_value(timer, unix_now_ms()));
                    }
                }
                state.upsert_participant(p);
                return true;
            }
            false
        }
        ("DISPATCH", "VOICE_STATE_DELETE") => {
            if let Some(uid) = data
                .get("user")
                .and_then(|u| ipc::non_empty_str(u.get("id")))
            {
                state.remove_participant(&uid);
                return true;
            }
            false
        }
        ("DISPATCH", "SPEAKING_START") => {
            if let Some(uid) = ipc::non_empty_str(data.get("user_id")) {
                return state.speaking_ids.insert(uid);
            }
            false
        }
        ("DISPATCH", "SPEAKING_STOP") => {
            if let Some(uid) = ipc::non_empty_str(data.get("user_id")) {
                return state.speaking_ids.remove(&uid);
            }
            false
        }
        _ => false,
    }
}
