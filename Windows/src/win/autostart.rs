//! Start-with-Windows switch (HKCU Run key), written through the registry API.
//! No helper process is spawned: a hidden reg.exe at every start looks like malware persistence to antivirus heuristics.
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

/// True when the Run entry exists.
pub fn is_on() -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\CurrentVersion\Run"), 0, KEY_QUERY_VALUE, &mut key) != ERROR_SUCCESS {
            return false;
        }
        let mut len = 0u32;
        let r = RegQueryValueExW(key, w!("AirWand"), None, None, None, Some(&mut len));
        let _ = RegCloseKey(key);
        r == ERROR_SUCCESS
    }
}

/// Adds or removes the Run entry for the current user. Returns whether it worked.
pub fn set(on: bool) -> bool {
    unsafe {
        let mut key = HKEY::default();
        let r = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        );
        if r != ERROR_SUCCESS {
            return false;
        }
        let ok = if on {
            match std::env::current_exe() {
                Ok(exe) => {
                    let data: Vec<u16> = format!("\"{}\" --background", exe.display()).encode_utf16().chain(std::iter::once(0)).collect();
                    let bytes = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
                    RegSetValueExW(key, w!("AirWand"), 0, REG_SZ, Some(bytes)) == ERROR_SUCCESS
                }
                Err(_) => false,
            }
        } else {
            let r = RegDeleteValueW(key, w!("AirWand"));
            r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND
        };
        let _ = RegCloseKey(key);
        ok
    }
}
