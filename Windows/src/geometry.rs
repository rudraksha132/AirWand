//! Screen geometry: laser-pointer projection and clamping to the display layout.
pub type P = (f32, f32);

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.r - self.l
    }
    pub fn height(&self) -> f32 {
        self.b - self.t
    }
    pub fn contains(&self, p: P) -> bool {
        p.0 >= self.l && p.0 < self.r && p.1 >= self.t && p.1 < self.b
    }
}

/// The main display is a wall at distance d, chosen so turning `range_deg` reaches its left/right edge.
/// x_deg > 0 = right, y_deg > 0 = down. Not clamped to the display layout.
pub fn project(x_deg: f64, y_deg: f64, main: Rect, range_deg: f64) -> P {
    let cx = main.l as f64 + main.width() as f64 / 2.0;
    let cy = main.t as f64 + main.height() as f64 / 2.0;
    if !x_deg.is_finite() || !y_deg.is_finite() {
        return (cx as f32, cy as f32);
    }
    let r = range_deg.clamp(5.0, 80.0).to_radians();
    let d = main.width() as f64 / 2.0 / r.tan();
    let ax = x_deg.clamp(-80.0, 80.0).to_radians();
    let ay = y_deg.clamp(-80.0, 80.0).to_radians();
    ((cx + d * ax.tan()) as f32, (cy + d * ay.tan()) as f32)
}

/// Nearest point inside any display.
pub fn clamp(p: P, rects: &[Rect]) -> P {
    if rects.is_empty() || rects.iter().any(|r| r.contains(p)) {
        return p;
    }
    let mut best = p;
    let mut best_d = f32::MAX;
    for r in rects {
        let q = (p.0.clamp(r.l, (r.r - 1.0).max(r.l)), p.1.clamp(r.t, (r.b - 1.0).max(r.t)));
        let d = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
        if d < best_d {
            best_d = d;
            best = q;
        }
    }
    best
}

/// Wraps an angle difference into -180..180.
pub fn wrap(a: f64) -> f64 {
    let mut x = a % 360.0;
    if x > 180.0 {
        x -= 360.0;
    }
    if x < -180.0 {
        x += 360.0;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect { l: 0.0, t: 0.0, r: 1920.0, b: 1080.0 };

    #[test]
    fn center_and_edge() {
        assert_eq!(project(0.0, 0.0, MAIN, 28.0), (960.0, 540.0));
        let (x, _) = project(28.0, 0.0, MAIN, 28.0);
        assert!((x - 1920.0).abs() < 1.0);
    }

    #[test]
    fn non_finite_is_center() {
        assert_eq!(project(f64::NAN, 0.0, MAIN, 28.0), (960.0, 540.0));
    }

    #[test]
    fn clamps_to_nearest_display() {
        let p = clamp((5000.0, -20.0), &[MAIN]);
        assert_eq!(p, (1919.0, 0.0));
    }

    #[test]
    fn wraps() {
        assert_eq!(wrap(190.0), -170.0);
        assert_eq!(wrap(-190.0), 170.0);
    }
}
