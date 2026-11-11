//! Calibration + filtering glue between the receiver and the engine (port of AppDelegate's aim handling).
use super::engine::Engine;
use super::settings::Settings;
use super::native;
use airwand::aim::{self, Drift};
use airwand::geometry::wrap;
use airwand::one_euro::OneEuro;
use airwand::protocol::Event;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    drift: Drift,
    last_t: Option<f64>,
    /// Set after trackpad use: the next aim sample re-anchors so the cursor does not jump.
    anchor: bool,
}

#[derive(Clone, Copy)]
struct Tune {
    sens: u8,
    push: u8,
    scroll: u8,
}

impl Calib {
    fn new(idx: usize) -> Self {
        let (mc, beta) = SMOOTH[idx.min(3)];
        Self { az0: None, el0: None, last_az: 0.0, last_el: 0.0, fx: OneEuro::new(mc, beta), fy: OneEuro::new(mc, beta), drift: Drift::default(), last_t: None, anchor: false }
    }

    fn recenter(&mut self) {
        self.az0 = Some(self.last_az);
        self.el0 = Some(self.last_el);
        self.fx.reset();
        self.fy.reset();
        self.drift = Drift::default();
        self.anchor = false;
    }
}

pub struct Controller {
    engine: Arc<Engine>,
    settings: Arc<Mutex<Settings>>,
    paused: Arc<AtomicBool>,
    calib: Mutex<HashMap<String, Calib>>,
    tune: Mutex<Tune>,
    bucket: Mutex<(Instant, f64)>,
}

impl Controller {
    pub fn new(engine: Arc<Engine>, settings: Arc<Mutex<Settings>>, paused: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self { engine, settings, paused, calib: Mutex::new(HashMap::new()),
            tune: Mutex::new(Tune { sens: 50, push: 40, scroll: 50 }),
            bucket: Mutex::new((Instant::now(), 240.0)),
        })
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

    /// Typing budget: 240 units, refilled at 120 per second. A character costs 1, a special key 4.
    fn allow(&self, cost: f64) -> bool {
        let mut b = self.bucket.lock().unwrap();
        let now = Instant::now();
        b.1 = (b.1 + now.duration_since(b.0).as_secs_f64() * 120.0).min(240.0);
        b.0 = now;
        if b.1 >= cost {
            b.1 -= cost;
            true
        } else {
            false
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
                let tune = *self.tune.lock().unwrap();
                let range = aim::range_for(tune.sens);
                let (cu, cv) = self.engine.cursor_norm();
                let pt = {
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
                    let (fx, fy) = (c.fx.filter(dx, t), c.fy.filter(dy, t));
                    let (u, v) = aim::norm(fx, fy, range);
                    if c.anchor {
                        c.anchor = false;
                        c.drift.anchor(u, v, cu, cv);
                    }
                    let dt = c.last_t.map(|l| t - l).unwrap_or(0.0);
                    c.last_t = Some(t);
                    c.drift.step(u, v, dt, tune.push);
                    self.engine.from_norm((u + c.drift.ox).clamp(-1.0, 1.0), (v + c.drift.oy).clamp(-1.0, 1.0))
                };
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
            Event::Scroll(dx, dy) => {
                let g = aim::scroll_gain(self.tune.lock().unwrap().scroll);
                self.engine.scroll(dx, dy, g)
            }
            Event::Move(dx, dy) => {
                // trackpad: no aim and no drift while it is used; aiming re-anchors to the cursor afterwards
                if let Some(c) = self.calib.lock().unwrap().get_mut(key) {
                    c.anchor = true;
                    c.last_t = None;
                }
                let sens = self.tune.lock().unwrap().sens;
                let g = aim::trackpad_gain(sens, dx.hypot(dy));
                self.engine.nudge(dx * g, dy * g)
            }
            Event::Tune { sens, push, scroll } => *self.tune.lock().unwrap() = Tune { sens, push, scroll },
            Event::Text(s) => {
                if peer_paused || !self.settings.lock().unwrap().typing {
                    return;
                }
                if self.allow(s.chars().count() as f64) {
                    native::type_text(&s);
                }
            }
            Event::Key { code, mods } => {
                if peer_paused || !self.settings.lock().unwrap().typing {
                    return;
                }
                if self.allow(4.0) {
                    native::key(code, mods);
                }
            }
        }
    }
}
