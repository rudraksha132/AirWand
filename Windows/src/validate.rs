//! Strict checks for everything that arrives over Bluetooth.
use sha2::{Digest, Sha256};

/// Control-panel window bounds "w,h,x,y" (CSS pixels from the panel page).
/// Returns (width, height, position). The position is None when it is nonsense,
/// e.g. the (-32000, -32000) Windows reports for a minimised window.
pub fn window(value: &str) -> Option<(u32, u32, Option<(i32, i32)>)> {
    let v: Vec<i64> = value.split(',').map(|p| p.trim().parse().ok()).collect::<Option<Vec<i64>>>()?;
    if v.len() != 4 || !(480..=10000).contains(&v[0]) || !(360..=10000).contains(&v[1]) {
        return None;
    }
    let sane = |n: i64| (-4000..=20000).contains(&n);
    let pos = if sane(v[2]) && sane(v[3]) { Some((v[2] as i32, v[3] as i32)) } else { None };
    Some((v[0] as u32, v[1] as u32, pos))
}

pub fn ok(v: f64, limit: f64) -> bool {
    v.is_finite() && v.abs() <= limit
}

/// t = seconds on the client clock; az/el in degrees.
pub fn aim(t: f64, az: f64, el: f64) -> bool {
    ok(t, 1e9) && ok(az, 1e5) && ok(el, 100.0)
}

pub fn time(t: f64) -> bool {
    ok(t, 1e9)
}

/// scroll / move deltas, pixels per packet
pub fn delta(dx: f64, dy: f64) -> bool {
    ok(dx, 2000.0) && ok(dy, 2000.0)
}

fn is_bad(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}')
}

/// Strips control and bidi/format characters and caps the length, so a device cannot spoof the Allow dialog.
pub fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !is_bad(*c)).take(max).collect::<String>().trim().to_string()
}

/// 6-digit code both sides derive from the device ID (SHA-256 of the lowercase id, first 4 bytes big endian, mod 1e6).
/// The phone shows it, the PC shows it in the Allow dialog; the user checks they match.
pub fn code(device_id: &str) -> String {
    let h = Sha256::digest(device_id.to_lowercase().as_bytes());
    let n = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    format!("{n:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_bounds() {
        assert_eq!(window("1040,820,100,50"), Some((1040, 820, Some((100, 50)))));
        assert_eq!(window(" 900 , 700 , -20 , 5 "), Some((900, 700, Some((-20, 5)))));
        assert_eq!(window("900,700,-32000,-32000"), Some((900, 700, None))); // minimised
        assert_eq!(window("100,700,0,0"), None); // too small
        assert_eq!(window("900,700,0"), None);
        assert_eq!(window("a,b,c,d"), None);
        assert_eq!(window("99999,700,0,0"), None);
    }

    #[test]
    fn rejects_bad_floats() {
        for v in [f64::NAN, f64::INFINITY, 1e30] {
            assert!(!aim(1.0, v, 0.0));
            assert!(!aim(1.0, 0.0, v));
            assert!(!delta(v, 0.0));
            assert!(!time(v));
        }
    }

    #[test]
    fn accepts_normal_values() {
        assert!(aim(123456.7, -359.9, 45.0));
        assert!(delta(-40.0, 12.5));
    }

    #[test]
    fn clean_strips_bidi_and_control() {
        assert_eq!(clean("\u{202E}Pixel\u{0} 8", 24), "Pixel 8");
        assert_eq!(clean(&"a".repeat(100), 24).len(), 24);
    }

    #[test]
    fn pairing_code_matches_web_page() {
        // Same vector computed with Node (crypto) and Python (hashlib).
        assert_eq!(code("123e4567-e89b-12d3-a456-426614174000"), "218249");
        assert_eq!(code("ABC"), code("abc"));
    }
}
