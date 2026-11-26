use tauri::{AppHandle, Manager};

use crate::{
    error::AppError,
    providers::{
        cpu_sensor::{self, CpuSensorStatus},
        gpu::SystemCaps,
        scheduler::Scheduler,
        system::SystemSample,
    },
};

#[tauri::command]
pub fn system_history(app: AppHandle) -> Result<Vec<SystemSample>, AppError> {
    Ok(app.state::<Scheduler>().system().history())
}

#[tauri::command]
pub async fn system_caps(app: AppHandle) -> Result<SystemCaps, AppError> {
    crate::commands::blocking("system_caps", move || {
        Ok(app.state::<Scheduler>().system().caps())
    })
    .await
}

#[tauri::command]
pub async fn cpu_sensor_status() -> Result<CpuSensorStatus, AppError> {
    tokio::task::spawn_blocking(cpu_sensor::service_status)
        .await
        .map_err(|e| AppError::Other(format!("CPU-sensorens status kunne ikke læses: {e}")))
}

#[tauri::command]
pub async fn cpu_sensor_install(window: tauri::Window) -> Result<CpuSensorStatus, AppError> {
    crate::commands::ensure_control_window(&window)?;
    tokio::task::spawn_blocking(cpu_sensor::install_service)
        .await
        .map_err(|e| AppError::Other(format!("CPU-sensoren kunne ikke installeres: {e}")))?
        .map_err(AppError::Other)
}

#[tauri::command]
pub async fn cpu_sensor_uninstall(window: tauri::Window) -> Result<CpuSensorStatus, AppError> {
    crate::commands::ensure_control_window(&window)?;
    tokio::task::spawn_blocking(cpu_sensor::uninstall_service)
        .await
        .map_err(|e| AppError::Other(format!("CPU-sensoren kunne ikke fjernes: {e}")))?
        .map_err(AppError::Other)
}
