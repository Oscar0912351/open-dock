use serde::ser::{Serialize, SerializeStruct, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Konfigurationen kunne ikke gemmes")]
    ConfigWrite(#[source] std::io::Error),
    #[error("Widget findes ikke")]
    WidgetNotFound,
    #[error("Størrelsen er ikke tilladt for denne widget")]
    SizeNotAllowed,
    #[error("Genvejen bruges allerede af et andet program")]
    ShortcutTaken,
    #[error("Netværksfejl")]
    Network(#[source] reqwest::Error),
    #[error("Windows-fejl: {0}")]
    Win(#[from] windows::core::Error),
    #[error("Tauri-fejl: {0}")]
    Tauri(#[from] tauri::Error),
    #[error("{0}")]
    Other(String),
}

impl AppError {
    fn code(&self) -> &'static str {
        match self {
            Self::ConfigWrite(_) => "config_write",
            Self::WidgetNotFound => "widget_not_found",
            Self::SizeNotAllowed => "size_not_allowed",
            Self::ShortcutTaken => "shortcut_taken",
            Self::Network(_) => "network",
            Self::Win(_) => "win",
            Self::Tauri(_) => "tauri",
            Self::Other(_) => "other",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn serializes_a_stable_danish_error() {
        let value = serde_json::to_value(AppError::WidgetNotFound).unwrap();
        assert_eq!(value["code"], "widget_not_found");
        assert_eq!(value["message"], "Widget findes ikke");
    }
}
