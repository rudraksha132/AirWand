//! Tiny rotating log: %APPDATA%\AirWand\log.txt (256 KB, one .old backup).
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static GATE: Mutex<()> = Mutex::new(());

pub fn path() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("AirWand").join("log.txt")
}

pub fn write(message: &str) {
    let _g = GATE.lock();
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&p).map(|m| m.len() > 262_144).unwrap_or(false) {
        let _ = std::fs::rename(&p, p.with_extension("txt.old"));
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{secs} {message}");
    }
}
