use std::{fs, path::PathBuf};

/// Én samlet mappe til alt Open Dock-brugerdata: %LOCALAPPDATA%\dev.opendock.app.
/// Ligger i Local (ikke Roaming), deles ikke med andre apps, og bevares ved
/// både afinstallation og opdatering (nsis.deleteAppDataOnUninstall = false).
/// På Windows er Tauris app_cache_dir/app_local_data_dir/app_log_dir allerede
/// den samme sti — config er det eneste der historisk lå i Roaming.
pub fn data_root() -> PathBuf {
    local_appdata().join("dev.opendock.app")
}

pub fn logs_dir() -> PathBuf {
    data_root().join("logs")
}

/// Tidligere placering (%APPDATA%\dev.opendock.app) før samlingen i Local.
fn legacy_config_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|home| PathBuf::from(home).join("AppData").join("Roaming"))
        })
        .unwrap_or_else(|| PathBuf::from("."))
        .join("dev.opendock.app")
}

fn local_appdata() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|home| PathBuf::from(home).join("AppData").join("Local"))
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Filnavne der flyttes fra Roaming ind i data_root ved første start efter opdatering.
const MIGRATED_FILES: [&str; 3] = ["config.json", "config.bak.json", "discord_rpc.dat"];
/// Mapper der flyttes fra Roaming ind i data_root.
const MIGRATED_DIRS: [&str; 1] = ["notes"];

/// Flytter brugerdata fra den gamle Roaming-mappe ind i data_root.
/// Idempotent og konservativ: eksisterende filer i data_root overskrives aldrig.
/// Kaldes i setup() før ConfigStore::load og discord-configure læser filerne.
pub fn migrate_storage() {
    let legacy = legacy_config_dir();
    let root = data_root();
    if legacy == root || !legacy.is_dir() {
        return;
    }
    if let Err(error) = fs::create_dir_all(&root) {
        log::warn!(target: "storage", "Kunne ikke oprette datamappe {:?}: {error}", root);
        return;
    }

    let mut moved = 0u32;
    for name in MIGRATED_FILES {
        moved += move_entry(&legacy.join(name), &root.join(name));
    }
    for name in MIGRATED_DIRS {
        moved += move_entry(&legacy.join(name), &root.join(name));
    }

    if moved > 0 {
        log::info!(
            target: "storage",
            "Flyttede {moved} elementer fra {:?} til {:?}",
            legacy,
            root
        );
    }
    // Fjern den gamle mappe hvis den er tom — fejler stille hvis der er rester.
    let _ = fs::remove_dir(&legacy);
}

fn move_entry(from: &PathBuf, to: &PathBuf) -> u32 {
    if !from.exists() {
        return 0;
    }
    if to.exists() {
        log::warn!(
            target: "storage",
            "Springer migration af {:?} over — findes allerede i {:?}",
            from,
            to
        );
        return 0;
    }
    match fs::rename(from, to) {
        Ok(()) => 1,
        Err(error) => {
            log::warn!(target: "storage", "Kunne ikke flytte {:?} -> {:?}: {error}", from, to);
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_root_er_local_og_egen_mappe() {
        let root = data_root();
        let s = root.to_string_lossy();
        assert!(s.contains("dev.opendock.app"), "root var {s}");
        assert!(!s.contains("OSSI"), "må ikke ligge under OSSI: {s}");
    }
}
