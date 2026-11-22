use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::providers::{
    discord::{DiscordRpcStatus, DiscordVoiceAction},
    scheduler::Scheduler,
};

#[tauri::command]
pub fn discord_rpc_status(scheduler: State<'_, Scheduler>) -> DiscordRpcStatus {
    scheduler.discord().rpc_status()
}

#[tauri::command]
pub async fn discord_rpc_connect(
    app: AppHandle,
    scheduler: State<'_, Scheduler>,
    client_id: String,
    client_secret: String,
    redirect_uri: Option<String>,
) -> Result<DiscordRpcStatus, AppError> {
    scheduler
        .discord()
        .connect_rpc(&app, client_id, client_secret, redirect_uri)
        .await
}

#[tauri::command]
pub async fn discord_rpc_connect_central(
    app: AppHandle,
    scheduler: State<'_, Scheduler>,
) -> Result<DiscordRpcStatus, AppError> {
    scheduler.discord().connect_rpc_central(&app).await
}

#[tauri::command]
pub fn discord_rpc_disconnect(
    app: AppHandle,
    scheduler: State<'_, Scheduler>,
) -> Result<DiscordRpcStatus, AppError> {
    scheduler.discord().disconnect_rpc(&app)
}

#[tauri::command]
pub fn discord_voice_action(
    scheduler: State<'_, Scheduler>,
    action: DiscordVoiceAction,
) -> Result<(), AppError> {
    scheduler.discord().voice_action(action)
}
