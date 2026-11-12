//! Anti-wobble stage that runs after the 1€ filter. Platform-free.
//!
//! Hand tremor is zero-mean and oscillating; intent is piecewise constant with ballistic jumps.
//! So the cursor behaves like it has static friction:
//!  * **dead-band leash** (stiction): the output anchor only moves when the aim leaves a small band;
//!    the band shrinks with speed, so fast moves feel free;
//!  * **self-calibrating band**: its size follows the wobble measured about the anchor (tired or shaky hand => wider);
//!  * **held boost**: after ~0.6 s of stillness the band widens a little and the anchor follows more slowly;
//!  * **coherent speed**: a heavily low-passed velocity. Tremor cancels in it, a deliberate slow creep does not,
//!    so slow precise aiming is not held back;
//!  * **tap-jolt freeze**: touching or lifting the thumb jolts the phone. The output is held until the jolt has
//!    died down (or the user clearly moves away) and the jolt is then forgotten (rebased);
//!  * **hidden resync**: leftover offset is paid back only while moving, at most 25% of the distance travelled.
//! Input and output are in degrees (az/el relative to the centre).

#[derive(Clone, Copy, Debug)]
pub struct SteadyOpts {
    pub r_min: f64,
    pub r_max: f64,
    pub r_k: f64,
    pub s0: f64,
    pub floor: f64,
    pub cdz: f64,
    pub cg: f64,
    pub cc: f64,
    pub creep: f64,
    pub hold_t: f64,
    pub freeze_min: f64,
    pub freeze_max: f64,
    pub breakout: f64,
    pub resync: f64,
}

impl SteadyOpts {
    /// Strength 0 = off. 1..=3 = gentle, normal, strong.
    pub fn level(l: u8) -> Option<Self> {
        let r_max = match l {
            0 => return None,
            1 => 0.22,
            2 => 0.35,
            _ => 0.5,
        };
        Some(Self { r_min: 0.03, r_max, r_k: 1.5, s0: 18.0, floor: 3.0, cdz: 0.5, cg: 4.0, cc: 4.0, creep: 0.35, hold_t: 0.6,
            freeze_min: 0.12, freeze_max: 0.45, breakout: 1.5, resync: 0.25 })
    }
}

pub struct Steady {
    o: SteadyOpts,
    ready: bool,
    t_prev: f64,
    raw_prev: (f64, f64),
    dv: (f64, f64),
    cv: (f64, f64),
    a: (f64, f64),
    off: (f64, f64),
    off_fast: bool,
    sig2: f64,
    h: f64,
    pressing: bool,
    frz: Option<(f64, f64)>, // (start time, calm seconds)
}

fn alpha(fc: f64, dt: f64) -> f64 {
    let k = std::f64::consts::TAU * fc * dt;
    k / (1.0 + k)
}

impl Steady {
    pub fn new(o: SteadyOpts) -> Self {
        Self { o, ready: false, t_prev: 0.0, raw_prev: (0.0, 0.0), dv: (0.0, 0.0), cv: (0.0, 0.0), a: (0.0, 0.0), off: (0.0, 0.0),
            off_fast: false, sig2: 0.01, h: 0.0, pressing: false, frz: None }
    }

    pub fn set_opts(&mut self, o: SteadyOpts) {
        self.o = o;
    }

    pub fn reset(&mut self) {
        self.ready = false;
        self.frz = None;
        self.pressing = false;
    }

    /// Thumb down (true) or up (false). Call when the press/release event arrives.
    pub fn touch(&mut self, down: bool) {
        self.pressing = down;
        if self.ready {
            self.frz = Some((self.t_prev, 0.0));
        }
    }

    pub fn step(&mut self, t: f64, x: f64, y: f64) -> (f64, f64) {
        if !x.is_finite() || !y.is_finite() || !t.is_finite() {
            return self.a;
        }
        if !self.ready {
            self.ready = true;
            self.t_prev = t;
            self.raw_prev = (x, y);
            self.dv = (0.0, 0.0);
            self.cv = (0.0, 0.0);
            self.a = (x, y);
            self.off = (0.0, 0.0);
            self.off_fast = false;
            self.h = 0.0;
            self.frz = None;
            return self.a;
        }
        let dt = (t - self.t_prev).min(0.05);
        if dt <= 0.0 {
            return self.a;
        }
        self.t_prev = t;
        let o = self.o;
        let (vx, vy) = ((x - self.raw_prev.0) / dt, (y - self.raw_prev.1) / dt);
        self.raw_prev = (x, y);
        let ad = alpha(3.0, dt);
        self.dv = (self.dv.0 + ad * (vx - self.dv.0), self.dv.1 + ad * (vy - self.dv.1));
        let ac = alpha(0.4, dt);
        self.cv = (self.cv.0 + ac * (vx - self.cv.0), self.cv.1 + ac * (vy - self.cv.1));
        let co = (self.cv.0.hypot(self.cv.1) - o.cdz).max(0.0);
        let s = (o.cg * co).max((self.dv.0.hypot(self.dv.1) - o.floor).max(0.0));
        self.h = if s < 1.5 { (self.h + dt / o.hold_t).min(1.0) } else { (self.h - dt / 0.1).max(0.0) };
        let r0 = (o.r_k * self.sig2.sqrt()).clamp(o.r_min, o.r_max);
        let r = r0 * (1.0 + 0.6 * self.h) * if self.pressing { 1.2 } else { 1.0 } * (-s / o.s0).exp();
        // pay back the rebase offset, only while the user is moving
        let om = self.off.0.hypot(self.off.1);
        if om > 0.0 {
            let k = if self.off_fast { (-dt / 0.12).exp() } else { (om - om.min(o.resync * s * dt)) / om };
            self.off = (self.off.0 * k, self.off.1 * k);
        }
        let g = (x + self.off.0, y + self.off.1);
        if let Some((t0, calm)) = self.frz {
            let calm = if s < 3.0 { calm + dt } else { 0.0 };
            let age = t - t0;
            let brk = (g.0 - self.a.0).hypot(g.1 - self.a.1) > o.breakout && s > 20.0;
            if brk || (age > o.freeze_min && calm > 0.06) || age > o.freeze_max {
                self.off = (self.a.0 - x, self.a.1 - y);
                self.off_fast = brk;
                self.frz = None;
            } else {
                self.frz = Some((t0, calm));
                return self.a;
            }
        }
        let (d0, d1) = (g.0 - self.a.0, g.1 - self.a.1);
        let d = d0.hypot(d1);
        if s < 4.0 {
            self.sig2 += dt / 1.5 * (d * d - self.sig2);
        }
        if d > r {
            let k = (d - r) / d;
            self.a = (self.a.0 + d0 * k, self.a.1 + d1 * k);
        } else {
            let c = (dt / (o.creep * (1.0 + self.h) / (1.0 + o.cc * co))).min(1.0);
            self.a = (self.a.0 + d0 * c, self.a.1 + d1 * c);
        }
        self.a
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::one_euro::OneEuro;
    use std::f64::consts::TAU;

    struct Rng(u32);
    impl Rng {
        fn f(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
            self.0 as f64 / 4294967296.0
        }
        fn g(&mut self) -> f64 {
            (-2.0 * (self.f() + 1e-12).ln()).sqrt() * (TAU * self.f()).cos()
        }
    }

    fn tremor(t: f64, amp: f64, p: f64) -> f64 {
        amp * (0.14 * (TAU * 4.5 * t + p).sin() + 0.12 * (TAU * 9.0 * t + 2.0 * p).sin() + 0.05 * (TAU * 13.0 * t + 3.0 * p).sin() + 0.1 * (TAU * 1.2 * t + 4.0 * p).sin())
    }

    fn minjerk(u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        10.0 * u.powi(3) - 15.0 * u.powi(4) + 6.0 * u.powi(5)
    }

    /// Hand model: hold, 10 deg ballistic move, hold; tap jolt at 3.5 (down) and 3.9 (up). Returns (jitter p-p while held, tap shift).
    fn run(mc: f64, beta: f64, steady: Option<SteadyOpts>, amp: f64) -> (f64, f64, f64) {
        let (mut fx, mut fy) = (OneEuro::new(mc, beta), OneEuro::new(mc, beta));
        let mut st = steady.map(Steady::new);
        let mut r = Rng(7);
        let (mut t, mut pp_lo, mut pp_hi) = (0.0f64, (1e9f64, 1e9f64), (-1e9f64, -1e9f64));
        let (mut base, mut shift, mut lag_t) = ((0.0, 0.0), 0.0f64, f64::NAN);
        let mut down = false;
        let mut up = false;
        while t < 6.0 {
            t += 1.0 / 60.0 + (r.f() - 0.5) * 0.004;
            if !down && t >= 3.5 {
                down = true;
                if let Some(s) = st.as_mut() { s.touch(true); }
            }
            if !up && t >= 3.9 {
                up = true;
                if let Some(s) = st.as_mut() { s.touch(false); }
            }
            let u = minjerk((t - 2.0) / 0.35);
            let (ia, ie) = (10.0 * u, 3.0 * u);
            let mut jz = 0.0;
            let mut je = 0.0;
            for (t0, sg) in [(3.5, 1.0), (3.9, -0.8)] {
                let x = t - (t0 + 0.03);
                if x > 0.0 {
                    let e = (-x / 0.09).exp() * (TAU * 7.0 * x).cos();
                    jz += sg * -1.2 * e;
                    je += sg * 0.8 * e;
                }
            }
            let az = ia + tremor(t, amp, 1.3) + jz + r.g() * 0.02;
            let el = ie + tremor(t, amp, 2.1) * 0.9 + je + r.g() * 0.02;
            let (mut ox, mut oy) = (fx.filter(az, t), fy.filter(el, t));
            if let Some(s) = st.as_mut() {
                let o = s.step(t, ox, oy);
                ox = o.0;
                oy = o.1;
            }
            if t > 0.6 && t < 1.9 {
                pp_lo = (pp_lo.0.min(ox), pp_lo.1.min(oy));
                pp_hi = (pp_hi.0.max(ox), pp_hi.1.max(oy));
            }
            if (t - 3.48).abs() < 0.009 { base = (ox, oy); }
            if t > 3.48 && t < 4.4 { shift = shift.max((ox - base.0).hypot(oy - base.1)); }
            if lag_t.is_nan() && t > 2.0 && ox >= 5.0 { lag_t = t - 2.175; }
        }
        ((pp_hi.0 - pp_lo.0).hypot(pp_hi.1 - pp_lo.1), shift, lag_t * 1000.0)
    }

    #[test]
    fn steadier_than_one_euro_for_every_preset_and_tap_does_not_move_the_cursor() {
        for (name, (mc, beta)) in [("Low", (2.5, 0.03)), ("Medium", (1.2, 0.02)), ("High", (0.6, 0.01))] {
            for amp in [1.0, 2.0] {
                let (pp0, sh0, lag0) = run(mc, beta, None, amp);
                let (pp1, sh1, lag1) = run(mc, beta, SteadyOpts::level(2), amp);
                println!("{name:6} x{amp}: hold p-p {pp0:.3} -> {pp1:.3} deg | tap shift {sh0:.2} -> {sh1:.2} deg | lag {lag0:.0} -> {lag1:.0} ms");
                assert!(pp1 < pp0 * 0.9 + 0.02, "{name} x{amp}: {pp0} -> {pp1}");
                assert!(sh1 < 0.25 && sh1 <= sh0 + 0.02, "{name} x{amp}: tap {sh0} -> {sh1}");
                assert!(lag1 - lag0 < 40.0, "{name} x{amp}: lag {lag0} -> {lag1}");
            }
        }
    }

    #[test]
    fn slow_deliberate_creep_is_followed() {
        let mut s = Steady::new(SteadyOpts::level(2).unwrap());
        let (mut t, mut worst) = (0.0f64, 0.0f64);
        let mut r = Rng(3);
        while t < 3.0 {
            t += 1.0 / 60.0;
            let ia = (t - 0.5).clamp(0.0, 2.0) * 1.0; // 1 deg/s
            let o = s.step(t, ia + tremor(t, 0.3, 1.3) + r.g() * 0.01, 0.2 * ia);
            if t > 1.5 { worst = worst.max((ia - o.0).abs()); }
        }
        assert!(worst < 1.0, "lags {worst} deg behind a 1 deg/s creep");
    }

    #[test]
    fn follows_fast_moves_and_survives_bad_input() {
        let mut s = Steady::new(SteadyOpts::level(3).unwrap());
        let mut y = (0.0, 0.0);
        for i in 0..120 {
            y = s.step(i as f64 / 60.0, i as f64 * 0.5, 0.0);
        }
        assert!(y.0 > 55.0, "{y:?}");
        let a = s.step(2.1, f64::NAN, 1.0);
        assert!(a.0.is_finite());
        s.touch(true);
        s.reset();
        assert_eq!(s.step(0.0, 1.0, 2.0), (1.0, 2.0));
    }

    #[test]
    fn off_is_none() {
        assert!(SteadyOpts::level(0).is_none());
    }
}
