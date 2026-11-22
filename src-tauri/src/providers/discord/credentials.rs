//! Krypteret lagring af Discord OAuth2-legitimationsoplysninger uden for `config.json`,
//! så `layout_export` aldrig lækker hemmeligheder eller tokens.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const CREDENTIALS_FILE: &str = "discord_rpc.dat";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordRpcCredentials {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default = "default_redirect_uri")]
    pub redirect_uri: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar_hash: Option<String>,
    #[serde(default)]
    pub discriminator: Option<String>,
    #[serde(default)]
    pub auth_via_central: bool,
    #[serde(default)]
    pub proxy_url: Option<String>,
}

fn default_redirect_uri() -> String {
    DEFAULT_REDIRECT_URI.to_string()
}

pub fn credentials_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CREDENTIALS_FILE)
}

pub fn load_credentials(config_dir: &Path) -> Option<DiscordRpcCredentials> {
    let path = credentials_path(config_dir);
    let raw = fs::read(&path).ok()?;
    let plain = match dpapi_unprotect(&raw) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!(target: "provider:discord", "Kunne ikke dekryptere Discord-legitimation: {err}");
            return None;
        }
    };
    match serde_json::from_slice::<DiscordRpcCredentials>(&plain) {
        Ok(creds) => Some(creds),
        Err(err) => {
            log::warn!(target: "provider:discord", "Ugyldig Discord-legitimation på disk: {err}");
            None
        }
    }
}

pub fn save_credentials(config_dir: &Path, creds: &DiscordRpcCredentials) -> Result<(), String> {
    fs::create_dir_all(config_dir)
        .map_err(|err| format!("Kunne ikke oprette config-mappe: {err}"))?;
    let json = serde_json::to_vec(creds)
        .map_err(|err| format!("Kunne ikke serialisere legitimation: {err}"))?;
    let protected = dpapi_protect(&json)?;

    let target = credentials_path(config_dir);
    let tmp = config_dir.join(format!("{CREDENTIALS_FILE}.tmp"));
    fs::write(&tmp, &protected)
        .map_err(|err| format!("Kunne ikke skrive midlertidig Discord-fil: {err}"))?;
    if let Err(rename_err) = fs::rename(&tmp, &target) {
        let _ = fs::remove_file(&target);
        if let Err(err2) = fs::rename(&tmp, &target) {
            let copy_res = fs::copy(&tmp, &target);
            let _ = fs::remove_file(&tmp);
            copy_res.map_err(|_| {
                format!("Kunne ikke gemme Discord-legitimation: {rename_err} / {err2}")
            })?;
        }
    }
    Ok(())
}

pub fn delete_credentials(config_dir: &Path) -> Result<(), String> {
    let path = credentials_path(config_dir);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("Kunne ikke fjerne Discord-legitimation: {err}")),
    }
}

#[cfg(windows)]
fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr::null_mut;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plain.len()).map_err(|_| "Data er for stor til DPAPI".to_string())?,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    unsafe {
        CryptProtectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(|err| format!("CryptProtectData fejlede: {err}"))?;

    if output.pbData.is_null() {
        return Err("CryptProtectData returnerede tom buffer".into());
    }
    let slice = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let result = slice.to_vec();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
    }
    Ok(result)
}

#[cfg(windows)]
fn dpapi_unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr::null_mut;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(cipher.len()).map_err(|_| "Data er for stor til DPAPI".to_string())?,
        pbData: cipher.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(|err| format!("CryptUnprotectData fejlede: {err}"))?;

    if output.pbData.is_null() {
        return Err("CryptUnprotectData returnerede tom buffer".into());
    }
    let slice = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let result = slice.to_vec();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
    }
    Ok(result)
}

#[cfg(not(windows))]
fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn dpapi_unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
    Ok(cipher.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn dpapi_roundtrip() {
        let sample = br#"{"clientId":"123","secret":"s3cr3t"}"#;
        let encrypted = dpapi_protect(sample).expect("protect");
        assert_ne!(encrypted.as_slice(), sample);
        let decrypted = dpapi_unprotect(&encrypted).expect("unprotect");
        assert_eq!(decrypted.as_slice(), sample);
    }

    #[test]
    fn credentials_disk_roundtrip() {
        let dir = std::env::temp_dir().join(format!("od_disc_cred_{}", Uuid::new_v4()));
        let creds = DiscordRpcCredentials {
            client_id: "987654321".into(),
            client_secret: "my-secret".into(),
            redirect_uri: DEFAULT_REDIRECT_URI.into(),
            access_token: Some("tok-a".into()),
            refresh_token: Some("tok-r".into()),
            expires_at_ms: Some(1_900_000_000_000),
            user_id: Some("1234567890".into()),
            username: Some("oscar".into()),
            display_name: Some("Oscar".into()),
            avatar_hash: Some("abc123".into()),
            discriminator: Some("0".into()),
            auth_via_central: true,
            proxy_url: Some("https://ossidev.dk".into()),
        };

        save_credentials(&dir, &creds).expect("save");
        let loaded = load_credentials(&dir).expect("load");
        assert_eq!(loaded, creds);

        delete_credentials(&dir).expect("delete");
        assert!(load_credentials(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
