//! Stored in %APPDATA%\AirWand\settings.json.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub range_idx: usize,
    pub smooth_idx: usize,
    pub inv_x: bool,
    pub inv_y: bool,
    /// Extrapolate the aim a few ms ahead to hide Bluetooth gaps.
    pub predict: bool,
    /// Require an encrypted (OS-paired) Bluetooth link. Needs a restart.
    pub require_encryption: bool,
    /// Let phones connect over Wi-Fi (the web server listens on the network, not just this PC). Needs a restart.
    pub wifi_enabled: bool,
    /// Let an approved phone type text and press keys on this PC. Off until the user turns it on.
    pub typing: bool,
    pub trusted: Vec<String>,
    /// Control-panel window size and position, saved whenever the user changes them (0 = not saved yet).
    pub win_w: u32,
    pub win_h: u32,
    pub win_x: i32,
    pub win_y: i32,
    pub win_pos: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { range_idx: 1, smooth_idx: 0, inv_x: false, inv_y: false, predict: true, require_encryption: false, wifi_enabled: true, typing: false, trusted: vec![], win_w: 0, win_h: 0, win_x: 0, win_y: 0, win_pos: false }
    }
}

fn path() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("AirWand").join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Settings = std::fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if s.range_idx > 2 {
            s.range_idx = 1;
        }
        if s.smooth_idx > 3 {
            s.smooth_idx = 0;
        }
        s
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(t) = serde_json::to_string(self) {
            let _ = std::fs::write(p, t);
        }
    }
}
