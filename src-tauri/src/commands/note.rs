use std::{fs, io::ErrorKind};

use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

const MAX_NOTE_CHARS: usize = 4000;

#[tauri::command]
pub async fn note_load(app: AppHandle, id: Uuid) -> Result<String, AppError> {
    crate::commands::blocking("note_load", move || {
        let notes_dir = app.state::<AppState>().config.notes_dir();
        let path = notes_dir.join(format!("{id}.txt"));
        match fs::read_to_string(&path) {
            Ok(content) => Ok(truncate_note(&content, id)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(String::new()),
            Err(err) => {
                log::error!(target: "cmd", "note_load fejlede for {id}: {err:?}");
                Err(AppError::Other("Kunne ikke indlæse noten".into()))
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn note_save(app: AppHandle, id: Uuid, text: String) -> Result<(), AppError> {
    crate::commands::blocking("note_save", move || {
        let notes_dir = app.state::<AppState>().config.notes_dir();
        if let Err(err) = fs::create_dir_all(&notes_dir) {
            log::error!(target: "cmd", "note_save create_dir_all fejlede for {id}: {err:?}");
            return Err(AppError::Other("Kunne ikke gemme noten".into()));
        }

        let trimmed = truncate_note(&text, id);
        let tmp = notes_dir.join(format!("{id}.txt.tmp"));
        let target = notes_dir.join(format!("{id}.txt"));

        if let Err(err) = fs::write(&tmp, trimmed.as_bytes()).and_then(|_| fs::rename(&tmp, &target)) {
            log::error!(target: "cmd", "note_save fejlede for {id}: {err:?}");
            return Err(AppError::Other("Kunne ikke gemme noten".into()));
        }
        Ok(())
    })
    .await
}

fn truncate_note(text: &str, id: Uuid) -> String {
    if text.chars().count() > MAX_NOTE_CHARS {
        log::warn!(target: "cmd", "Note {id} oversteg {MAX_NOTE_CHARS} tegn og blev afkortet");
        text.chars().take(MAX_NOTE_CHARS).collect()
    } else {
        text.to_string()
    }
}
