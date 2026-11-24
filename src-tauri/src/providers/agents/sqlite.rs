//! Sikker read-only adgang til lokale SQLite-databaser (WAL-venlig, aldrig låsende).

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags};

/// Åbn en SQLite-database read-only uden at holde fil-låse mellem scans.
pub fn open_readonly(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = match Connection::open_with_flags(path, flags) {
        Ok(c) => c,
        Err(_) => {
            // Fallback til immutable URI når f.eks. WAL mangler -shm eller rettigheder blokerer shm.
            let uri = to_immutable_uri(path)?;
            let uri_flags = OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            Connection::open_with_flags(&uri, uri_flags).ok()?
        }
    };

    let _ = conn.busy_timeout(Duration::from_millis(150));
    let _ = conn.execute_batch("PRAGMA query_only = ON;");
    Some(conn)
}

/// Fingeraftryk af DB + evt. `-wal` fil: `(max_mtime_ms, samlet_bytes)`.
/// `-shm` tages ikke med: read-only læsere skriver read-marks i shared-memory
/// filen, så vores egen scan ville ellers invalidere cachen hver gang.
pub fn db_fingerprint(path: &Path) -> Option<(i64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let mut max_mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut total_len = meta.len();

    for suffix in ["-wal"] {
        let sidecar = sidecar_path(path, suffix);
        if let Ok(sm) = fs::metadata(&sidecar) {
            total_len = total_len.saturating_add(sm.len());
            if let Ok(modified) = sm.modified() {
                if let Ok(dur) = modified.duration_since(UNIX_EPOCH) {
                    max_mtime = max_mtime.max(dur.as_millis() as i64);
                }
            }
        }
    }

    Some((max_mtime, total_len))
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut os = OsString::from(path.as_os_str());
    os.push(suffix);
    PathBuf::from(os)
}

fn to_immutable_uri(path: &Path) -> Option<String> {
    let raw = path.to_str()?;
    let slash = raw.replace('\\', "/");
    let trimmed = slash.trim_start_matches('/');
    let mut encoded = String::with_capacity(trimmed.len() + 32);
    encoded.push_str("file:///");
    for byte in trimmed.bytes() {
        match byte {
            b'%' => encoded.push_str("%25"),
            b'?' => encoded.push_str("%3F"),
            b'#' => encoded.push_str("%23"),
            b' ' => encoded.push_str("%20"),
            _ => encoded.push(byte as char),
        }
    }
    encoded.push_str("?mode=ro&immutable=1");
    Some(encoded)
}
