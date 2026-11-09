#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    if let Err(e) = win::run() {
        win::log::write(&format!("fatal: {e}"));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("AirWand's receiver runs on Windows only.");
}
