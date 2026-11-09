//! 1€ filter (Casiez et al.): steady when the hand is still, quick when it moves.
pub struct OneEuro {
    pub min_cutoff: f64,
    pub beta: f64,
    x_prev: Option<f64>,
    dx_prev: f64,
    t_prev: f64,
}

const D_CUTOFF: f64 = 1.0;

fn alpha(cutoff: f64, dt: f64) -> f64 {
    let tau = 1.0 / (2.0 * std::f64::consts::PI * cutoff);
    1.0 / (1.0 + tau / dt)
}

impl OneEuro {
    pub fn new(min_cutoff: f64, beta: f64) -> Self {
        Self { min_cutoff, beta, x_prev: None, dx_prev: 0.0, t_prev: 0.0 }
    }

    pub fn filter(&mut self, x: f64, t: f64) -> f64 {
        let Some(xp) = self.x_prev else {
            self.x_prev = Some(x);
            self.t_prev = t;
            return x;
        };
        let dt = (t - self.t_prev).clamp(0.001, 0.1);
        self.t_prev = t;
        let dx = (x - xp) / dt;
        let ad = alpha(D_CUTOFF, dt);
        self.dx_prev = ad * dx + (1.0 - ad) * self.dx_prev;
        let a = alpha(self.min_cutoff + self.beta * self.dx_prev.abs(), dt);
        let r = a * x + (1.0 - a) * xp;
        self.x_prev = Some(r);
        r
    }

    pub fn reset(&mut self) {
        self.x_prev = None;
        self.dx_prev = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_passes_through() {
        assert_eq!(OneEuro::new(1.2, 0.02).filter(5.0, 0.0), 5.0);
    }

    #[test]
    fn smooths_jitter_when_still() {
        let mut f = OneEuro::new(1.2, 0.02);
        let mut max = 0.0f64;
        for i in 0..200 {
            let v = f.filter(10.0 + if i % 2 == 0 { 0.5 } else { -0.5 }, i as f64 * 0.01);
            if i > 50 {
                max = max.max((v - 10.0).abs());
            }
        }
        assert!(max < 0.5);
    }

    #[test]
    fn follows_fast_motion() {
        let mut f = OneEuro::new(1.2, 0.02);
        let mut y = 0.0;
        for i in 0..100 {
            y = f.filter(i as f64 * 2.0, i as f64 * 0.01);
        }
        assert!(y > 150.0);
    }
}
