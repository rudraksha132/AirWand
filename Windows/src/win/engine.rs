//! Owns the system cursor (port of PointerEngine). Sources feed target points; a dedicated thread
//! eases the cursor toward the target ~250 times a second, looking a few ms ahead along the aim velocity.
use super::native;
use crate::geometry::{self, Rect, P};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct State {
    target: Option<P>,
    current: Option<P>,
    source: Option<String>,
    history: Vec<(f64, P)>,
    frozen: bool,
    pressed: bool,
    dragging: bool,
    enabled: bool,
    press_point: Option<P>,
    press_id: u32,
    vel: P,
    feed_at: Instant,
    range_deg: f64,
}

struct DisplayCache {
    at: Option<Instant>,
    rects: Vec<Rect>,
    main: Rect,
}

pub struct Engine {
    st: Mutex<State>,
    displays: Mutex<DisplayCache>,
    predict: AtomicBool,
    last_click: Mutex<Option<(Instant, P)>>,
    scroll_rem: Mutex<(f64, f64)>,
}

impl Engine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            st: Mutex::new(State {
                target: None,
                current: None,
                source: None,
                history: Vec::new(),
                frozen: false,
                pressed: false,
                dragging: false,
                enabled: true,
                press_point: None,
                press_id: 0,
                vel: (0.0, 0.0),
                feed_at: Instant::now(),
                range_deg: 28.0,
            }),
            displays: Mutex::new(DisplayCache { at: None, rects: Vec::new(), main: Rect::default() }),
            predict: AtomicBool::new(true),
            last_click: Mutex::new(None),
            scroll_rem: Mutex::new((0.0, 0.0)),
        })
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        let _ = std::thread::Builder::new().name("airwand-cursor".into()).spawn(move || {
            let timer = native::PreciseTimer::new();
            loop {
                me.tick();
                timer.wait(4);
            }
        });
    }

    pub fn set_range(&self, deg: f64) {
        self.st.lock().unwrap().range_deg = deg;
    }

    pub fn set_predict(&self, on: bool) {
        self.predict.store(on, Ordering::Relaxed);
    }

    pub fn set_enabled(self: &Arc<Self>, on: bool) {
        {
            let mut s = self.st.lock().unwrap();
            s.enabled = on;
            s.target = None;
            s.current = None;
            s.history.clear();
        }
        if !on {
            self.cancel_press();
        }
    }

    // geometry

    fn displays(&self) -> (Vec<Rect>, Rect) {
        let mut d = self.displays.lock().unwrap();
        let stale = d.at.map(|t| t.elapsed() > Duration::from_secs(2)).unwrap_or(true);
        if stale || d.rects.is_empty() {
            d.rects = native::displays();
            d.main = native::primary();
            d.at = Some(Instant::now());
        }
        (d.rects.clone(), d.main)
    }

    fn clamp(&self, p: P) -> P {
        let (rects, _) = self.displays();
        geometry::clamp(p, &rects)
    }

    /// Laser-pointer projection onto the main display, clamped to the display layout.
    pub fn project(&self, x_deg: f64, y_deg: f64) -> P {
        let range = self.st.lock().unwrap().range_deg;
        let (rects, main) = self.displays();
        geometry::clamp(geometry::project(x_deg, y_deg, main, range), &rects)
    }

    // loop

    fn tick(&self) {
        let new_c;
        {
            let mut s = self.st.lock().unwrap();
            if !s.enabled || s.frozen {
                return;
            }
            let Some(t) = s.target else { return };
            let c = s.current.unwrap_or(t);
            let mut tp = t;
            if self.predict.load(Ordering::Relaxed) {
                let age = s.feed_at.elapsed().as_secs_f64();
                let sp = ((s.vel.0 as f64).powi(2) + (s.vel.1 as f64).powi(2)).sqrt();
                if age < 0.04 && sp > 100.0 {
                    let g = ((sp - 100.0) / 300.0).min(1.0);
                    let lead = (age + 0.010).min(0.04) * g;
                    tp = self.clamp((t.0 + (s.vel.0 as f64 * lead) as f32, t.1 + (s.vel.1 as f64 * lead) as f32));
                }
            }
            let dx = tp.0 - c.0;
            let dy = tp.1 - c.1;
            if dx.abs() < 0.05 && dy.abs() < 0.05 && s.current.is_some() {
                return;
            }
            let nc = (c.0 + dx * 0.42, c.1 + dy * 0.42);
            s.current = Some(nc);
            new_c = nc;
        }
        native::move_to(new_c);
    }

    // input

    /// A new aim sample. t is on the source's own clock.
    pub fn feed(&self, id: &str, t: f64, point: P) {
        let mut s = self.st.lock().unwrap();
        if s.source.as_deref() != Some(id) {
            s.source = Some(id.to_string());
            s.history.clear();
        }
        s.target = Some(point);
        s.history.push((t, point));
        if s.history.len() > 400 {
            s.history.retain(|h| t - h.0 <= 1.0);
        }
        s.feed_at = Instant::now();
        s.vel = (0.0, 0.0);
        for i in (0..s.history.len().saturating_sub(1)).rev() {
            let dt = t - s.history[i].0;
            if dt < 0.012 {
                continue;
            }
            if dt <= 0.08 {
                let q = s.history[i].1;
                s.vel = (((point.0 - q.0) as f64 / dt) as f32, ((point.1 - q.1) as f64 / dt) as f32);
            }
            break;
        }
    }

    /// Where the pointer was aimed at client time t.
    fn point_at(s: &State, t: Option<f64>) -> P {
        if let Some(tt) = t {
            if let Some(h) = s.history.iter().rev().find(|h| h.0 <= tt) {
                return h.1;
            }
        }
        s.current.or(s.target).or_else(native::cursor_pos).unwrap_or((0.0, 0.0))
    }

    /// Thumb down. The cursor freezes on the point you were aiming at when the touch started,
    /// so the jolt of the tap doesn't move the click.
    pub fn press(self: &Arc<Self>, t: Option<f64>) {
        let id;
        {
            let mut s = self.st.lock().unwrap();
            s.pressed = true;
            s.frozen = true;
            s.press_point = Some(Self::point_at(&s, t));
            s.press_id = s.press_id.wrapping_add(1);
            id = s.press_id;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(330));
            me.begin_drag_if_held(id);
        });
    }

    fn begin_drag_if_held(&self, id: u32) {
        let p;
        {
            let mut s = self.st.lock().unwrap();
            let Some(pp) = s.press_point else { return };
            if !s.pressed || s.press_id != id {
                return;
            }
            p = pp;
            s.dragging = true;
            s.frozen = false;
            s.current = Some(p);
        }
        native::button(p, native::LEFT_DOWN);
    }

    pub fn release(self: &Arc<Self>) {
        let mut p;
        let was_dragging;
        {
            let mut s = self.st.lock().unwrap();
            if !s.pressed {
                return;
            }
            s.pressed = false;
            s.press_id = s.press_id.wrapping_add(1);
            was_dragging = s.dragging;
            s.dragging = false;
            p = (if was_dragging { s.current } else { s.press_point })
                .or(s.press_point)
                .or_else(native::cursor_pos)
                .unwrap_or((0.0, 0.0));
        }
        if was_dragging {
            native::button(p, native::LEFT_UP);
            return;
        }
        // Keep repeat clicks on the same pixel so Windows sees a double-click.
        {
            let mut lc = self.last_click.lock().unwrap();
            if let Some((at, lp)) = *lc {
                if at.elapsed() < Duration::from_millis(native::double_click_ms() as u64)
                    && (p.0 - lp.0).abs() < 8.0
                    && (p.1 - lp.1).abs() < 8.0
                {
                    p = lp;
                }
            }
            *lc = Some((Instant::now(), p));
        }
        native::button(p, native::LEFT_DOWN | native::LEFT_UP);
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            let mut s = me.st.lock().unwrap();
            if !s.pressed {
                s.frozen = false;
                s.current = Some(p);
            }
        });
    }

    fn cancel_press(&self) {
        let p;
        let was_dragging;
        {
            let mut s = self.st.lock().unwrap();
            was_dragging = s.dragging;
            p = s.current.or(s.press_point);
            s.pressed = false;
            s.dragging = false;
            s.frozen = false;
            s.press_id = s.press_id.wrapping_add(1);
        }
        if was_dragging {
            if let Some(p) = p {
                native::button(p, native::LEFT_UP);
            }
        }
    }

    pub fn right_click(&self, t: Option<f64>) {
        let p = {
            let s = self.st.lock().unwrap();
            Self::point_at(&s, t)
        };
        native::button(p, native::RIGHT_DOWN | native::RIGHT_UP);
    }

    /// dy > 0 = content moves down (natural), like the phone's scroll strip.
    pub fn scroll(&self, dx: f64, dy: f64) {
        if !crate::validate::delta(dx, dy) {
            return;
        }
        let (ix, iy);
        {
            let mut r = self.scroll_rem.lock().unwrap();
            r.1 += dy * 3.0; // ~40 px of finger travel per wheel notch (120 units)
            r.0 += -dx * 3.0;
            iy = r.1.trunc() as i32;
            ix = r.0.trunc() as i32;
            r.1 -= iy as f64;
            r.0 -= ix as f64;
        }
        if iy != 0 {
            native::wheel(iy, false);
        }
        if ix != 0 {
            native::wheel(ix, true);
        }
    }

    /// Relative move, used by the phone's trackpad mode.
    pub fn nudge(&self, dx: f64, dy: f64) {
        if !crate::validate::delta(dx, dy) {
            return;
        }
        let p;
        let skip;
        {
            let mut s = self.st.lock().unwrap();
            if !s.enabled {
                return;
            }
            let b = s.current.or_else(native::cursor_pos).unwrap_or((0.0, 0.0));
            p = self.clamp((b.0 + dx as f32, b.1 + dy as f32));
            s.source = Some("pad".into());
            s.history.clear();
            s.target = Some(p);
            s.current = Some(p);
            skip = s.frozen;
        }
        if !skip {
            native::move_to(p);
        }
    }
}
