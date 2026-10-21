//! Existing GameStrike read-only registry and ShellExecute adapter, extracted from launcher.rs.
use gamestrike::stores::{Hive, Registry};
use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS},
    System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        KEY_WOW64_64KEY, REG_DWORD, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
        RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW},
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

/// Read-only access to the Windows registry for the store scanners.
pub struct WindowsRegistry;

struct OpenKey(HKEY);

impl Drop for OpenKey {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

fn open_key(hive: Hive, path: &str) -> Option<OpenKey> {
    let root = match hive {
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        Hive::CurrentUser => HKEY_CURRENT_USER,
    };
    let path = wide(OsStr::new(path));
    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ | KEY_WOW64_64KEY, &mut key)
    };
    (status == ERROR_SUCCESS).then_some(OpenKey(key))
}

impl Registry for WindowsRegistry {
    fn subkeys(&self, hive: Hive, path: &str) -> Vec<String> {
        let Some(key) = open_key(hive, path) else {
            return Vec::new();
        };
        let mut names = Vec::new();
        let mut buffer = [0u16; 256];
        for index in 0..20_000u32 {
            let mut length = buffer.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            if status == ERROR_SUCCESS {
                names.push(String::from_utf16_lossy(&buffer[..length as usize]));
            }
        }
        names
    }

    fn string(&self, hive: Hive, path: &str, name: &str) -> Option<String> {
        let key = open_key(hive, path)?;
        let name = wide(OsStr::new(name));
        let mut kind = 0u32;
        let mut size = 0u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS || size > 64 * 1024 {
            return None;
        }
        let mut data = vec![0u8; size as usize + 2];
        let mut length = size;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut length,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        data.truncate(length as usize);
        match kind {
            REG_SZ | REG_EXPAND_SZ => {
                let units: Vec<u16> = data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes(*pair))
                    .collect();
                let end = units
                    .iter()
                    .position(|unit| *unit == 0)
                    .unwrap_or(units.len());
                Some(String::from_utf16_lossy(&units[..end]))
            }
            REG_DWORD if data.len() >= 4 => {
                Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]).to_string())
            }
            _ => None,
        }
    }
}

/// Opens a store link, a shortcut, or a folder the way Explorer would.
pub fn open(target: &OsStr) -> std::result::Result<(), String> {
    let operation = wide(OsStr::new("open"));
    let file = wide(target);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    let code = result as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("Windows could not open it (code {code})"))
    }
}
