use serde::{Deserialize, Serialize};
use ts_rs::TS;
use windows::{
    core::w,
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
            HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_VALUE_TYPE,
        },
    },
};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WindowsQualityStatus {
    pub lossless_enabled: bool,
    pub jpeg_import_quality: Option<u32>,
}

struct RegKeyGuard(HKEY);

impl Drop for RegKeyGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

pub fn read_windows_quality() -> WindowsQualityStatus {
    let mut hkey = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Desktop"),
            None,
            KEY_READ,
            &mut hkey,
        )
    };
    if status != ERROR_SUCCESS {
        return WindowsQualityStatus {
            lossless_enabled: false,
            jpeg_import_quality: None,
        };
    }
    let _guard = RegKeyGuard(hkey);

    let mut value_type = REG_VALUE_TYPE::default();
    let mut buf = [0_u8; 4];
    let mut size = buf.len() as u32;
    let query_status = unsafe {
        RegQueryValueExW(
            hkey,
            w!("JPEGImportQuality"),
            None,
            Some(&mut value_type),
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };

    if query_status != ERROR_SUCCESS || value_type != REG_DWORD || size < 4 {
        return WindowsQualityStatus {
            lossless_enabled: false,
            jpeg_import_quality: None,
        };
    }

    let val = u32::from_le_bytes(buf);
    WindowsQualityStatus {
        lossless_enabled: val >= 100,
        jpeg_import_quality: Some(val),
    }
}

pub fn set_windows_lossless_quality(enabled: bool) -> Result<WindowsQualityStatus, AppError> {
    let mut hkey = HKEY::default();
    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Desktop"),
            None,
            KEY_SET_VALUE,
            &mut hkey,
        )
    };
    if open_status != ERROR_SUCCESS {
        return Err(AppError::Other(format!(
            "Kunne ikke åbne Windows-registreringsdatabasen for baggrundskvalitet (fejl {})",
            open_status.0
        )));
    }
    let _guard = RegKeyGuard(hkey);

    if enabled {
        let quality: u32 = 100;
        let bytes = quality.to_le_bytes();
        let write_status = unsafe {
            RegSetValueExW(
                hkey,
                w!("JPEGImportQuality"),
                None,
                REG_DWORD,
                Some(&bytes),
            )
        };
        if write_status != ERROR_SUCCESS {
            return Err(AppError::Other(format!(
                "Kunne ikke skrive JPEGImportQuality til registreringsdatabasen (fejl {})",
                write_status.0
            )));
        }
        log::info!("Windows JPEGImportQuality sat til 100 (tabsfri baggrundskvalitet)");
    } else {
        let del_status = unsafe { RegDeleteValueW(hkey, w!("JPEGImportQuality")) };
        if del_status != ERROR_SUCCESS && del_status != ERROR_FILE_NOT_FOUND {
            return Err(AppError::Other(format!(
                "Kunne ikke nulstille JPEGImportQuality i registreringsdatabasen (fejl {})",
                del_status.0
            )));
        }
        log::info!("Windows JPEGImportQuality nulstillet til Windows-standard (85 %)");
    }

    Ok(read_windows_quality())
}
