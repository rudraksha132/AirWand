//! Wire protocol (identical to Shared/Protocol.swift). Little endian, first byte = kind.
//!
//!   hello   01  uuid[16] kind:u8 name:utf8
//!   aim     02  t:f64 az:f32 el:f32      degrees; az grows turning left, el grows tilting up
//!   press   03  t:f64
//!   release 04  t:f64
//!   right   05  t:f64
//!   scroll  06  dx:f32 dy:f32
//!   center  07
//!   pause   08  on:u8
//!   info    09  battery:u8 (255 = unknown) model:utf8
//!   move    0A  dx:f32 dy:f32
//!   aimpair 0B  t0:u32 (ms) az0:f32 el0:f32 dt:u8 (ms) daz:i16 (0.01 deg) del:i16 (0.01 deg)
//! Status (PC -> client, read + notify): state:u8 (0 waiting, 1 approved, 2 denied) paused:u8 pc_name:utf8
use crate::validate;

pub const SERVICE: u128 = 0x7E1A0001_3C2B_4F7D_9B61_2A9E5D0C4F10;
pub const INPUT: u128 = 0x7E1A0002_3C2B_4F7D_9B61_2A9E5D0C4F10;
pub const STATUS: u128 = 0x7E1A0003_3C2B_4F7D_9B61_2A9E5D0C4F10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Waiting = 0,
    Approved = 1,
    Denied = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Aim { t: f64, az: f64, el: f64 },
    Press(f64),
    Release(f64),
    Right(f64),
    Scroll(f64, f64),
    Move(f64, f64),
    Center,
    Pause(bool),
}

#[derive(Debug, PartialEq)]
pub enum Packet {
    Hello { device_id: String, kind: u8, name: String },
    Info { battery: Option<u8>, model: String },
    Input(Vec<Event>),
    Reject(&'static str),
}

pub struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let s = self.b.get(self.i..self.i + N)?;
        self.i += N;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Some(a)
    }

    pub fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|a| a[0])
    }
    pub fn i16(&mut self) -> Option<i16> {
        self.take::<2>().map(i16::from_le_bytes)
    }
    pub fn u32(&mut self) -> Option<u32> {
        self.take::<4>().map(u32::from_le_bytes)
    }
    pub fn f32(&mut self) -> Option<f32> {
        self.take::<4>().map(f32::from_le_bytes)
    }
    pub fn f64(&mut self) -> Option<f64> {
        self.take::<8>().map(f64::from_le_bytes)
    }

    /// 16 raw bytes in textual order, same as Swift's UUID tuple.
    pub fn uuid(&mut self) -> Option<String> {
        let a = self.take::<16>()?;
        let mut s = String::with_capacity(36);
        for (i, b) in a.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                s.push('-');
            }
            s.push_str(&format!("{b:02x}"));
        }
        Some(s)
    }

    pub fn rest(&mut self) -> String {
        let s = String::from_utf8_lossy(&self.b[self.i.min(self.b.len())..]).into_owned();
        self.i = self.b.len();
        s
    }
}

pub fn parse(data: &[u8]) -> Packet {
    if data.is_empty() {
        return Packet::Reject("empty");
    }
    let mut r = Reader::new(&data[1..]);
    match data[0] {
        1 => {
            let Some(device_id) = r.uuid() else { return Packet::Reject("short hello") };
            let kind = r.u8().unwrap_or(0);
            let name = validate::clean(&r.rest(), 24);
            Packet::Hello { device_id, kind, name }
        }
        9 => {
            let b = r.u8().unwrap_or(255);
            let model = validate::clean(&r.rest(), 24);
            Packet::Info { battery: if b == 255 { None } else { Some(b) }, model }
        }
        2 => match (r.f64(), r.f32(), r.f32()) {
            (Some(t), Some(az), Some(el)) => {
                let (az, el) = (az as f64, el as f64);
                if validate::aim(t, az, el) {
                    Packet::Input(vec![Event::Aim { t, az, el }])
                } else {
                    Packet::Reject("aim out of range")
                }
            }
            _ => Packet::Reject("short aim"),
        },
        11 => match (r.u32(), r.f32(), r.f32(), r.u8(), r.i16(), r.i16()) {
            (Some(ms), Some(az0), Some(el0), Some(dt), Some(daz), Some(del)) => {
                let t0 = ms as f64 / 1000.0;
                let t1 = t0 + dt as f64 / 1000.0;
                let (az0, el0) = (az0 as f64, el0 as f64);
                let (az1, el1) = (az0 + daz as f64 / 100.0, el0 + del as f64 / 100.0);
                if validate::aim(t0, az0, el0) && validate::aim(t1, az1, el1) {
                    Packet::Input(vec![
                        Event::Aim { t: t0, az: az0, el: el0 },
                        Event::Aim { t: t1, az: az1, el: el1 },
                    ])
                } else {
                    Packet::Reject("aimpair out of range")
                }
            }
            _ => Packet::Reject("short aimpair"),
        },
        3 | 4 | 5 => match r.f64() {
            Some(t) if validate::time(t) => Packet::Input(vec![match data[0] {
                3 => Event::Press(t),
                4 => Event::Release(t),
                _ => Event::Right(t),
            }]),
            Some(_) => Packet::Reject("bad time"),
            None => Packet::Reject("short timed packet"),
        },
        6 | 10 => match (r.f32(), r.f32()) {
            (Some(x), Some(y)) if validate::delta(x as f64, y as f64) => {
                let (x, y) = (x as f64, y as f64);
                Packet::Input(vec![if data[0] == 6 { Event::Scroll(x, y) } else { Event::Move(x, y) }])
            }
            (Some(_), Some(_)) => Packet::Reject("delta out of range"),
            _ => Packet::Reject("short delta"),
        },
        7 => Packet::Input(vec![Event::Center]),
        8 => Packet::Input(vec![Event::Pause(r.u8().unwrap_or(0) != 0)]),
        _ => Packet::Reject("unknown kind"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
    }

    fn aim_packet(t: f64, az: f32, el: f32) -> Vec<u8> {
        let mut b = vec![2u8];
        b.extend(t.to_le_bytes());
        b.extend(az.to_le_bytes());
        b.extend(el.to_le_bytes());
        b
    }

    #[test]
    fn parses_aim() {
        assert_eq!(parse(&aim_packet(12.5, -30.25, 7.5)), Packet::Input(vec![Event::Aim { t: 12.5, az: -30.25, el: 7.5 }]));
    }

    #[test]
    fn rejects_truncated_and_unknown() {
        assert!(matches!(parse(&[2, 1, 2, 3]), Packet::Reject(_)));
        assert!(matches!(parse(&[0x7F]), Packet::Reject(_)));
        assert!(matches!(parse(&[]), Packet::Reject(_)));
    }

    #[test]
    fn rejects_nan_aim() {
        assert!(matches!(parse(&aim_packet(1.0, f32::NAN, 0.0)), Packet::Reject(_)));
    }

    #[test]
    fn parses_aimpair_from_web_page() {
        // Produced by buildAim() in Web/index.html for samples (1000.4 ms, 10.5, -3.25) and (1016.4 ms, 10.62, -3.2)
        match parse(&hex("0be803000000002841000050c0100c000500")) {
            Packet::Input(ev) => {
                assert_eq!(ev.len(), 2);
                match (&ev[0], &ev[1]) {
                    (Event::Aim { t: t0, az: a0, el: e0 }, Event::Aim { t: t1, az: a1, el: e1 }) => {
                        assert!((t0 - 1.0).abs() < 1e-9 && (t1 - 1.016).abs() < 1e-9);
                        assert!((a0 - 10.5).abs() < 1e-6 && (e0 + 3.25).abs() < 1e-6);
                        assert!((a1 - 10.62).abs() < 1e-6 && (e1 + 3.2).abs() < 1e-6);
                    }
                    _ => panic!("wrong events"),
                }
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn uuid_keeps_textual_order() {
        let mut b = vec![1u8];
        b.extend(hex("123e4567e89b12d3a456426614174000"));
        b.push(1);
        match parse(&b) {
            Packet::Hello { device_id, kind, name } => {
                assert_eq!(device_id, "123e4567-e89b-12d3-a456-426614174000");
                assert_eq!(kind, 1);
                assert_eq!(name, "");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn info_battery_unknown() {
        assert_eq!(parse(&[9, 255, b'P', b'i']), Packet::Info { battery: None, model: "Pi".into() });
    }
}
