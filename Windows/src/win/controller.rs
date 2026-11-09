//! Calibration + filtering glue between the receiver and the engine (port of AppDelegate's aim handling).
use super::engine::Engine;
use super::settings::Settings;
use crate::geometry::wrap;
use crate::one_euro::OneEuro;
use crate::protocol::Event;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const RANGES: [f64; 3] = [18.0, 28.0, 40.0];
pub const RANGE_NAMES: [&str; 3] = ["Fast (small turns)", "Normal", "Precise (big turns)"];
pub const SMOOTH: [(f64, f64); 4] = [(2.5, 0.03), (1.2, 0.02), (0.6, 0.01), (6.0, 0.1)];
pub const SMOOTH_NAMES: [&str; 4] = ["Low", "Medium", "High", "Minimal (lowest latency)"];

struct Calib {
    az0: Option<f64>,
    el0: Option<f64>,
    last_az: f64,
    last_el: f64,
    fx: OneEuro,
    fy: OneEuro,
}

impl Calib {
    fn new(idx: usize) -> Self {
        let (mc, beta) = SMOOTH[idx.min(3)];
        Self { az0: None, el0: None, last_az: 0.0, last_el: 0.0, fx: OneEuro::new(mc, beta), fy: OneEuro::new(mc, beta) }
    }

    fn recenter(&mut self) {
        self.az0 = Some(self.last_az);
        self.el0 = Some(self.last_el);
        self.fx.reset();
        self.fy.reset();
    }
}

pub struct Controller {
    engine: Arc<Engine>,
    settings: Arc<Mutex<Settings>>,
    paused: Arc<AtomicBool>,
    calib: Mutex<HashMap<String, Calib>>,
}

impl Controller {
    pub fn new(engine: Arc<Engine>, settings: Arc<Mutex<Settings>>, paused: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self { engine, settings, paused, calib: Mutex::new(HashMap::new()) })
    }

    pub fn center_all(&self) {
        for c in self.calib.lock().unwrap().values_mut() {
            c.recenter();
        }
    }

    pub fn apply_smooth_all(&self) {
        let idx = self.settings.lock().unwrap().smooth_idx.min(3);
        let (mc, beta) = SMOOTH[idx];
        for c in self.calib.lock().unwrap().values_mut() {
            c.fx.min_cutoff = mc;
            c.fx.beta = beta;
            c.fy.min_cutoff = mc;
            c.fy.beta = beta;
        }
    }

    /// Called from Bluetooth threads for every approved event.
    pub fn on_event(self: &Arc<Self>, key: &str, peer_paused: bool, ev: Event) {
        if self.paused.load(Ordering::Relaxed) {
            return;
        }
        match ev {
            Event::Aim { t, az, el } => {
                if peer_paused {
                    return;
                }
                let (inv_x, inv_y, idx) = {
                    let s = self.settings.lock().unwrap();
                    (s.inv_x, s.inv_y, s.smooth_idx)
                };
                let p = {
                    let mut map = self.calib.lock().unwrap();
                    let c = map.entry(key.to_string()).or_insert_with(|| Calib::new(idx));
                    c.last_az = az;
                    c.last_el = el;
                    if c.az0.is_none() {
                        c.az0 = Some(az);
                        c.el0 = Some(el);
                    }
                    // az grows turning left, el grows tilting up; screen x grows right, y grows down
                    let mut dx = -wrap(az - c.az0.unwrap_or(az));
                    let mut dy = -(el - c.el0.unwrap_or(el));
                    if inv_x {
                        dx = -dx;
                    }
                    if inv_y {
                        dy = -dy;
                    }
                    (c.fx.filter(dx, t), c.fy.filter(dy, t))
                };
                let pt = self.engine.project(p.0, p.1);
                self.engine.feed(key, t, pt);
            }
            Event::Center => {
                if let Some(c) = self.calib.lock().unwrap().get_mut(key) {
                    c.recenter();
                }
            }
            Event::Pause(_) => {}
            Event::Press(t) => self.engine.press(Some(t)),
            Event::Release(_) => self.engine.release(),
            Event::Right(t) => self.engine.right_click(Some(t)),
            Event::Scroll(dx, dy) => self.engine.scroll(dx, dy),
            Event::Move(dx, dy) => self.engine.nudge(dx, dy),
        }
    }
}
