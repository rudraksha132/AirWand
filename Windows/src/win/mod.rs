//! Windows shell: Bluetooth GATT server, cursor control, tray icon.
pub mod app;
pub mod controller;
pub mod engine;
pub mod log;
pub mod native;
pub mod receiver;
pub mod settings;

pub use app::run;
