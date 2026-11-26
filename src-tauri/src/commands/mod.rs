pub mod agents;
pub mod app;
pub mod config;
pub mod discord;
pub mod edit;
pub mod media;
pub mod note;
pub mod system;
pub mod timer;
pub mod update;
pub mod wallpaper;
pub mod weather;
pub mod widgets;

pub(crate) async fn blocking<T, F>(context: &'static str, f: F) -> Result<T, crate::error::AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, crate::error::AppError> + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|error| {
        log::error!(target: "cmd", "{context}: baggrundsopgaven fejlede: {error}");
        crate::error::AppError::Other("Handlingen kunne ikke gennemføres. Prøv igen".into())
    })?
}

pub fn ensure_control_window(window: &tauri::Window) -> Result<(), crate::error::AppError> {
    if window.label() != "control" {
        log::warn!(
            target: "security",
            "Afviste kontrolcenter-kommando fra vinduet '{}'",
            window.label()
        );
        return Err(crate::error::AppError::Other(
            "Det kan kun gøres inde i appen.".into(),
        ));
    }
    Ok(())
}
