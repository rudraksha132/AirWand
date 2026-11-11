//! Pointing feel: sensitivity, edge push (drift) and trackpad gain. Pure maths, no Windows calls.
//! Offsets are in "screen halves": 1.0 is the screen edge from the centre, in x and in y.

pub const MIN_RANGE: f64 = 8.0;
pub const MAX_RANGE: f64 = 45.0;

/// Degrees of turn that reach the screen edge. Sensitivity 1 = 45 degrees, 100 = 8 degrees, smooth in between.
pub fn range_for(sens: u8) -> f64 {
    let s = (sens.clamp(1, 100) as f64 - 1.0) / 99.0;
    MAX_RANGE * (MIN_RANGE / MAX_RANGE).powf(s)
}

/// Turn from the centre (degrees) to screen halves. Up/down uses the same angle as left/right, so a small
/// tilt reaches the top and bottom as easily as the sides.
pub fn norm(dx_deg: f64, dy_deg: f64, range_deg: f64) -> (f64, f64) {
    let r = range_deg.clamp(5.0, 80.0).to_radians().tan();
    let f = |d: f64| if d.is_finite() { d.clamp(-80.0, 80.0).to_radians().tan() / r } else { 0.0 };
    (f(dx_deg), f(dy_deg))
}

/// Edge push starts past this distance from the centre (screen halves). Stronger push, smaller zone.
pub fn zone(push: u8) -> f64 {
    0.8 - 0.35 * (push.min(100) as f64 / 100.0)
}

/// Top drift speed, screen halves per second.
pub fn max_speed(push: u8) -> f64 {
    0.4 + 2.0 * (push.min(100) as f64 / 100.0)
}

const LIM: f64 = 2.0;

/// The cursor offset added by edge push. It stays where the drift left it when the phone returns to centre.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drift {
    pub ox: f64,
    pub oy: f64,
}

impl Drift {
    /// Chooses the offset so that pointing at (u, v) puts the cursor at (cu, cv): no jump when aiming resumes.
    pub fn anchor(&mut self, u: f64, v: f64, cu: f64, cv: f64) {
        self.ox = (cu - u).clamp(-LIM, LIM);
        self.oy = (cv - v).clamp(-LIM, LIM);
    }
}

impl Drift {
    /// (u, v) = where the phone points now, in screen halves from the centre. Past the zone the cursor
    /// keeps moving that way, slowly at first (speed grows with the square of the distance past the zone).
    pub fn step(&mut self, u: f64, v: f64, dt: f64, push: u8) {
        // (offsets may go to +-2 so a re-anchor after trackpad use can hold a far-off cursor)
        if push == 0 || !u.is_finite() || !v.is_finite() {
            return;
        }
        let m = u.hypot(v);
        let z = zone(push);
        if m <= z {
            return;
        }
        let e = ((m - z) / (1.0 - z)).min(1.5);
        let speed = max_speed(push) * e * e;
        let dt = dt.clamp(0.0, 0.05);
        self.ox = (self.ox + u / m * speed * dt).clamp(-LIM, LIM);
        self.oy = (self.oy + v / m * speed * dt).clamp(-LIM, LIM);
    }
}

/// Trackpad finger-to-cursor gain: the sensitivity sets the base, quick strokes travel further.
pub fn trackpad_gain(sens: u8, magnitude: f64) -> f64 {
    let base = 2f64.powf((sens.clamp(1, 100) as f64 - 50.0) / 50.0);
    base * (1.0 + 0.8 * (magnitude.abs().min(30.0) / 30.0))
}

/// Scroll speed multiplier from the 1..100 slider (50 = unchanged).
pub fn scroll_gain(scroll: u8) -> f64 {
    2f64.powf((scroll.clamp(1, 100) as f64 - 50.0) / 25.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_runs_from_45_to_8_degrees() {
        assert!((range_for(1) - 45.0).abs() < 1e-9);
        assert!((range_for(100) - 8.0).abs() < 1e-9);
        assert!(range_for(50) < range_for(30) && range_for(70) < range_for(50));
    }

    #[test]
    fn range_degrees_reach_the_edge_in_both_directions() {
        let (u, v) = norm(20.0, -20.0, 20.0);
        assert!((u - 1.0).abs() < 1e-9 && (v + 1.0).abs() < 1e-9);
        assert_eq!(norm(0.0, 0.0, 20.0), (0.0, 0.0));
        assert_eq!(norm(f64::NAN, 1.0, 20.0).0, 0.0);
    }

    #[test]
    fn no_drift_inside_the_zone_or_when_off() {
        let mut d = Drift::default();
        d.step(0.5, 0.0, 0.016, 50);
        assert_eq!(d, Drift::default());
        d.step(1.0, 0.0, 0.016, 0);
        assert_eq!(d, Drift::default());
    }

    #[test]
    fn drifts_the_way_you_point_and_stops_when_you_return() {
        let mut d = Drift::default();
        for _ in 0..30 {
            d.step(0.0, -0.9, 0.016, 50); // a little up
        }
        assert!(d.oy < -0.05 && d.ox.abs() < 1e-12, "{d:?}");
        let before = d;
        for _ in 0..30 {
            d.step(0.0, 0.0, 0.016, 50); // back at the starting angle
        }
        assert_eq!(d, before);
    }

    #[test]
    fn diagonal_and_all_four_directions() {
        for (u, v) in [(0.9, 0.0), (-0.9, 0.0), (0.0, 0.9), (0.0, -0.9), (0.7, 0.7), (-0.7, -0.7)] {
            let mut d = Drift::default();
            d.step(u, v, 0.05, 80);
            assert!(d.ox * u >= 0.0 && d.oy * v >= 0.0 && (d.ox != 0.0 || d.oy != 0.0), "{u} {v} {d:?}");
        }
    }

    #[test]
    fn farther_past_the_zone_is_faster_and_it_is_bounded() {
        let (mut a, mut b) = (Drift::default(), Drift::default());
        a.step(0.7, 0.0, 0.05, 50);
        b.step(1.2, 0.0, 0.05, 50);
        assert!(b.ox > a.ox && a.ox > 0.0);
        for _ in 0..10_000 {
            b.step(1.5, 1.5, 0.05, 100);
        }
        assert!(b.ox <= 2.0 && b.oy <= 2.0);
        let mut c = Drift::default();
        c.step(1.0, 0.0, 5.0, 100); // a stalled clock cannot teleport the cursor
        assert!(c.ox < 0.2);
    }

    #[test]
    fn anchor_keeps_the_cursor_where_it_is() {
        let mut d = Drift::default();
        d.anchor(0.2, -0.1, 0.5, 0.4);
        assert!((0.2 + d.ox - 0.5).abs() < 1e-12 && (-0.1 + d.oy - 0.4).abs() < 1e-12);
    }

    #[test]
    fn stronger_push_means_smaller_zone_and_faster_drift() {
        assert!(zone(100) < zone(50) && zone(50) < zone(1));
        assert!(max_speed(100) > max_speed(50));
    }

    #[test]
    fn trackpad_and_scroll_gain() {
        assert!((trackpad_gain(50, 0.0) - 1.0).abs() < 1e-9);
        assert!(trackpad_gain(80, 0.0) > trackpad_gain(20, 0.0));
        assert!(trackpad_gain(50, 30.0) > trackpad_gain(50, 2.0));
        assert!((scroll_gain(50) - 1.0).abs() < 1e-9 && scroll_gain(100) > 3.9 && scroll_gain(1) < 0.3);
    }
}
