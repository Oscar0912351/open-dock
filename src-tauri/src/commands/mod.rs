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

pub fn ensure_control_window(window: &tauri::Window) -> Result<(), crate::error::AppError> {
    if window.label() != "control" {
        log::warn!(
            target: "security",
            "Afviste kontrolcenter-kommando fra vinduet '{}'",
            window.label()
        );
        return Err(crate::error::AppError::Other(
            "Handlingen kan kun udføres fra kontrolcentret.".into(),
        ));
    }
    Ok(())
}
