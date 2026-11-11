//! Tiny rotating log: %APPDATA%\AirWand\log.txt (256 KB, one .old backup).
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST};

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

/// Messages already shown, so a problem that repeats every second pops up once, not every second.
static SHOWN: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());

fn show_box(body: &str) {
    let text: Vec<u16> = body.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(None, PCWSTR(text.as_ptr()), w!("AirWand problem"), MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST);
    }
}

fn body(text: &str) -> String {
    format!("{text}\n\nMore detail is in the log:\n{}", path().display())
}

/// Shows a Windows message box about a problem (and logs it). Does not block the caller, and shows
/// the same text at most once every 10 minutes.
pub fn alert(text: &str) {
    {
        let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
        let ten = Duration::from_secs(600);
        shown.retain(|(_, at)| at.elapsed() < ten);
        if shown.iter().any(|(t, _)| t == text) {
            return;
        }
        shown.push((text.to_string(), Instant::now()));
    }
    write(&format!("alert: {text}"));
    let b = body(text);
    let _ = std::thread::Builder::new().name("airwand-alert".into()).spawn(move || show_box(&b));
}

/// Like `alert`, but waits until the box is closed. For a crash, where the process ends right after.
pub fn alert_and_wait(text: &str) {
    write(&format!("alert: {text}"));
    show_box(&body(text));
}

/// Any crash shows a message instead of the app silently vanishing.
pub fn install_panic_alert() {
    std::panic::set_hook(Box::new(|info| {
        alert_and_wait(&format!("AirWand hit an unexpected error and has to close.\n\n{info}"));
    }));
}
