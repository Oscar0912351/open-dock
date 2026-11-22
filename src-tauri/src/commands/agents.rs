use crate::error::AppError;

#[tauri::command]
pub async fn agent_focus(
    agent: String,
    pid: Option<u32>,
    project: Option<String>,
) -> Result<bool, AppError> {
    let focused = tokio::task::spawn_blocking(move || {
        crate::providers::agents::focus_agent_window(&agent, pid, project.as_deref())
    })
    .await
    .map_err(|err| AppError::Other(format!("Kunne ikke fokusere agent-vindue: {err}")))?;
    Ok(focused)
}
