//! Glue between the control panel / Wi-Fi web server and the rest of the app.
use super::controller::{Controller, RANGES};
use super::engine::Engine;
use super::log;
use super::receiver::Receiver;
use super::settings::Settings;
use airwand::net::jstr;
use airwand::protocol::LinkState;
use airwand::server::{self, Assets, Backend, Sink};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const PORT: u16 = 47821;
pub static GAME_HTML: &[u8] = include_bytes!("../../game.html");

pub struct Control {
    settings: Arc<Mutex<Settings>>,
    receiver: Arc<Receiver>,
    ctrl: Arc<Controller>,
    engine: Arc<Engine>,
    paused: Arc<AtomicBool>,
    wifi_error: Mutex<Option<String>>,
    wifi_listening: AtomicBool,
    autostart: AtomicBool,
    quit: AtomicBool,
    /// (aim count at last sample, when, aims per second)
    rate: Mutex<(u64, Instant, u64)>,
}

impl Control {
    pub fn new(
        settings: Arc<Mutex<Settings>>,
        receiver: Arc<Receiver>,
        ctrl: Arc<Controller>,
        engine: Arc<Engine>,
        paused: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            settings,
            receiver,
            ctrl,
            engine,
            paused,
            wifi_error: Mutex::new(None),
            wifi_listening: AtomicBool::new(false),
            autostart: AtomicBool::new(query_autostart()),
            quit: AtomicBool::new(false),
            rate: Mutex::new((0, Instant::now(), 0)),
        })
    }

    /// The panel's Quit button sets this; the tray's 1 s timer sees it and closes the app.
    pub fn quit_requested(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }

    fn save(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.lock().unwrap();
        f(&mut s);
        s.save();
    }

    fn set_paused(&self, on: bool) {
        self.paused.store(on, Ordering::Relaxed);
        self.receiver.set_paused(on);
        self.engine.set_enabled(!on);
    }

    fn aim_rate(&self) -> u64 {
        let aims = self.receiver.aims();
        let mut r = self.rate.lock().unwrap();
        let dt = r.1.elapsed().as_secs_f64();
        if dt >= 0.5 {
            r.2 = (aims.saturating_sub(r.0) as f64 / dt).round() as u64;
            r.0 = aims;
            r.1 = Instant::now();
        }
        r.2
    }
}

fn log_tail(n: usize) -> Vec<String> {
    std::fs::read_to_string(log::path())
        .map(|t| {
            let lines: Vec<&str> = t.lines().collect();
            lines[lines.len().saturating_sub(n)..].iter().map(|s| s.to_string()).collect()
        })
        .unwrap_or_default()
}

fn query_autostart() -> bool {
    super::autostart::is_on()
}

fn set_autostart(on: bool) -> bool {
    super::autostart::set(on)
}

/// Opens the control panel in its own window (Edge or Chrome app mode), else the default browser.
pub fn open_panel() {
    let url = format!("http://127.0.0.1:{PORT}/ui");
    // Open at the size and place the user left it in.
    let s = Settings::load();
    let (w, h) = if s.win_w >= 480 && s.win_h >= 360 { (s.win_w, s.win_h) } else { (1040, 820) };
    let size = format!("--window-size={w},{h}");
    let pos = if s.win_pos { Some(format!("--window-position={},{}", s.win_x, s.win_y)) } else { None };
    let browsers = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    for b in browsers {
        if std::path::Path::new(b).exists() {
            let mut c = Command::new(b);
            c.arg(format!("--app={url}")).arg(&size);
            if let Some(p) = &pos {
                c.arg(p);
            }
            if c.spawn().is_ok() {
                return;
            }
        }
    }
    let _ = Command::new("explorer").arg(&url).spawn();
}

pub fn open_game() {
    let p = std::env::temp_dir().join("airwand-game.html");
    if std::fs::write(&p, GAME_HTML).is_ok() {
        let _ = Command::new("explorer").arg(&p).spawn();
    }
}

/// Starts the web server (Wi-Fi link + control panel) on its own thread.
pub fn start_server(ctl: Arc<Control>, lan: bool) {
    let _ = std::thread::Builder::new().name("airwand-server".into()).spawn(move || match server::bind(PORT, lan) {
        Ok(listener) => {
            ctl.wifi_listening.store(true, Ordering::Relaxed);
            log::write(&format!("web server on port {PORT} ({})", if lan { "network" } else { "this PC only" }));
            let be: Arc<dyn Backend> = ctl.clone();
            server::serve(
                listener,
                be,
                Assets {
                    phone: include_str!("../../../Web/index.html"),
                    panel: include_str!("../../panel.html"),
                    icon: include_bytes!("../../../Web/icon.png"),
                    font: include_bytes!("../../../Web/Geist.woff2"),
                    font_mono: include_bytes!("../../../Web/GeistMono.woff2"),
                },
            );
        }
        Err(e) => {
            let msg = format!("Could not start the Wi-Fi link and control panel on port {PORT}: {e}");
            log::write(&msg);
            *ctl.wifi_error.lock().unwrap() = Some(msg);
        }
    });
}

impl Backend for Control {
    fn state_json(&self) -> String {
        let s = self.settings.lock().unwrap().clone();
        let ble_error = self.receiver.error.lock().unwrap().clone();
        let wifi_error = self.wifi_error.lock().unwrap().clone();
        let urls: Vec<String> = if s.wifi_enabled {
            server::lan_ips().iter().map(|ip| jstr(&format!("http://{ip}:{PORT}"))).collect()
        } else {
            Vec::new()
        };
        let peers: Vec<String> = self
            .receiver
            .peer_infos()
            .iter()
            .map(|p| {
                let battery = match p.battery {
                    Some(b) => b.to_string(),
                    None => "null".to_string(),
                };
                let state = match p.state {
                    LinkState::Approved => "approved",
                    LinkState::Waiting => "waiting",
                    LinkState::Denied => "denied",
                };
                format!(
                    "{{\"name\":{},\"model\":{},\"battery\":{},\"state\":\"{}\",\"transport\":\"{}\",\"seenMs\":{},\"code\":{},\"paused\":{}}}",
                    jstr(&p.name),
                    jstr(&p.model),
                    battery,
                    state,
                    match (p.wifi, p.ble) {
                        (true, true) => "wifi+bluetooth",
                        (true, false) => "wifi",
                        _ => "bluetooth",
                    },
                    p.seen_ms,
                    jstr(&p.code),
                    p.paused
                )
            })
            .collect();
        let log_lines: Vec<String> = log_tail(40).iter().map(|l| jstr(l)).collect();
        let opt = |o: &Option<String>| match o {
            Some(t) => jstr(t),
            None => "null".to_string(),
        };
        format!(
            "{{\"version\":{},\"pcName\":{},\"paused\":{},\"bluetooth\":{{\"advertising\":{},\"error\":{}}},\"wifi\":{{\"enabled\":{},\"listening\":{},\"port\":{},\"urls\":[{}],\"error\":{}}},\"peers\":[{}],\"trusted\":{},\"aimRate\":{},\"rejected\":{},\"settings\":{{\"range\":{},\"smooth\":{},\"invX\":{},\"invY\":{},\"predict\":{},\"encrypt\":{},\"wifi\":{},\"autostart\":{}}},\"window\":[{},{},{},{},{}],\"log\":[{}]}}",
            jstr(env!("CARGO_PKG_VERSION")),
            jstr(&self.receiver.pc_name),
            self.paused.load(Ordering::Relaxed),
            self.receiver.advertising.load(Ordering::Relaxed),
            opt(&ble_error),
            s.wifi_enabled,
            self.wifi_listening.load(Ordering::Relaxed),
            PORT,
            urls.join(","),
            opt(&wifi_error),
            peers.join(","),
            self.receiver.trusted_count(),
            self.aim_rate(),
            self.receiver.rejected(),
            s.range_idx,
            s.smooth_idx,
            s.inv_x,
            s.inv_y,
            s.predict,
            s.require_encryption,
            s.wifi_enabled,
            self.autostart.load(Ordering::Relaxed),
            s.win_w,
            s.win_h,
            s.win_x,
            s.win_y,
            s.win_pos,
            log_lines.join(",")
        )
    }

    fn action(&self, name: &str, value: &str) -> Result<(), String> {
        let on = value == "1" || value == "true";
        let idx = value.parse::<usize>().ok();
        match name {
            "pause" => {
                let now = if value == "toggle" { !self.paused.load(Ordering::Relaxed) } else { on };
                self.set_paused(now);
            }
            "center" => self.ctrl.center_all(),
            "range" => {
                let i = idx.filter(|i| *i < 3).ok_or("bad value")?;
                self.save(|s| s.range_idx = i);
                self.engine.set_range(RANGES[i]);
            }
            "smooth" => {
                let i = idx.filter(|i| *i < 4).ok_or("bad value")?;
                self.save(|s| s.smooth_idx = i);
                self.ctrl.apply_smooth_all();
            }
            "predict" => {
                self.save(|s| s.predict = on);
                self.engine.set_predict(on);
            }
            "invx" => self.save(|s| s.inv_x = on),
            "invy" => self.save(|s| s.inv_y = on),
            "encrypt" => self.save(|s| s.require_encryption = on),
            "wifi" => self.save(|s| s.wifi_enabled = on),
            "autostart" => {
                if set_autostart(on) {
                    self.autostart.store(on, Ordering::Relaxed);
                } else {
                    return Err("Could not change the startup setting.".to_string());
                }
            }
            "window" => {
                let (w, h, pos) = airwand::validate::window(value).ok_or("bad window size")?;
                self.save(|s| {
                    s.win_w = w;
                    s.win_h = h;
                    if let Some((x, y)) = pos {
                        s.win_x = x;
                        s.win_y = y;
                        s.win_pos = true;
                    }
                });
            }
            "forget" => self.receiver.forget_all(),
            "game" => open_game(),
            "quit" => self.quit.store(true, Ordering::Relaxed),
            _ => return Err(format!("unknown action {name}")),
        }
        Ok(())
    }

    fn ws_open(&self, key: &str, send: Sink) -> bool {
        self.receiver.wifi_connect(key, send)
    }

    fn ws_packet(&self, key: &str, data: &[u8]) {
        self.receiver.wifi_packet(key, data);
    }

    fn ws_close(&self, key: &str) {
        self.receiver.wifi_disconnect(key);
    }
}
