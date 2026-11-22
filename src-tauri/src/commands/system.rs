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
pub fn system_caps(app: AppHandle) -> Result<SystemCaps, AppError> {
    Ok(app.state::<Scheduler>().system().caps())
}

#[tauri::command]
pub async fn cpu_sensor_status() -> Result<CpuSensorStatus, AppError> {
    tokio::task::spawn_blocking(cpu_sensor::service_status)
        .await
        .map_err(|e| AppError::Other(format!("Kunne ikke aflæse sensorstatus: {e}")))
}

#[tauri::command]
pub async fn cpu_sensor_install() -> Result<CpuSensorStatus, AppError> {
    tokio::task::spawn_blocking(cpu_sensor::install_service)
        .await
        .map_err(|e| AppError::Other(format!("Kunne ikke installere sensortjeneste: {e}")))?
        .map_err(AppError::Other)
}

#[tauri::command]
pub async fn cpu_sensor_uninstall() -> Result<CpuSensorStatus, AppError> {
    tokio::task::spawn_blocking(cpu_sensor::uninstall_service)
        .await
        .map_err(|e| AppError::Other(format!("Kunne ikke fjerne sensortjeneste: {e}")))?
        .map_err(AppError::Other)
}
