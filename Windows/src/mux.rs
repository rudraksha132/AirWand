//! Several links (Wi-Fi, Bluetooth) can carry the same device at the same time. This module is the
//! device-level view that sits between the links and the cursor engine:
//!
//!   * packets are merged per DEVICE, not per link, so a link switch never resets calibration,
//!     filter or press history (no cursor jump when Wi-Fi drops and Bluetooth carries on);
//!   * the page sends every sample on every live link; the first copy to arrive wins and late or
//!     duplicate copies are dropped by their timestamps (no hand-over delay at all);
//!   * clicks sent on both links fire once;
//!   * if the last link of a device dies while its button is held, a release is synthesised so
//!     the mouse button is never left stuck down.
//!
//! Platform free, so it is unit-tested on any OS.
use crate::protocol::Event;
use std::collections::HashMap;

#[derive(Default)]
struct Dev {
    last_aim_t: Option<f64>,
    last_press_t: Option<f64>,
    last_release_t: Option<f64>,
    last_right_t: Option<f64>,
    paused: bool,
    pressed: bool,
}

#[derive(Default)]
pub struct Mux {
    devs: HashMap<String, Dev>,
    /// link key -> device id, for every link that has delivered something.
    links: HashMap<String, String>,
}

impl Mux {
    pub fn new() -> Self {
        Self::default()
    }

    /// The device said hello (new page load or a link came up): its clock may have restarted.
    pub fn hello(&mut self, dev: &str, link: &str) {
        let d = self.devs.entry(dev.to_string()).or_default();
        d.last_aim_t = None;
        d.last_press_t = None;
        d.last_release_t = None;
        d.last_right_t = None;
        self.links.insert(link.to_string(), dev.to_string());
    }

    /// Remember that `link` belongs to `dev`.
    pub fn link_seen(&mut self, dev: &str, link: &str) {
        if self.links.get(link).map(|d| d != dev).unwrap_or(true) {
            self.links.insert(link.to_string(), dev.to_string());
        }
        self.devs.entry(dev.to_string()).or_default();
    }

    pub fn paused(&self, dev: &str) -> bool {
        self.devs.get(dev).map(|d| d.paused).unwrap_or(false)
    }

    pub fn live_links(&self, dev: &str) -> usize {
        self.links.values().filter(|d| d.as_str() == dev).count()
    }

    /// Returns the event if it is new, None if another link already delivered it.
    pub fn filter(&mut self, dev: &str, ev: Event) -> Option<Event> {
        let d = self.devs.entry(dev.to_string()).or_default();
        match ev {
            Event::Aim { t, .. } => {
                // newest wins; a jump back of 5 s or more means the device's clock restarted
                if let Some(last) = d.last_aim_t {
                    if t <= last && last - t < 5.0 {
                        return None;
                    }
                }
                d.last_aim_t = Some(t);
            }
            Event::Press(t) => {
                if d.last_press_t == Some(t) {
                    return None;
                }
                d.last_press_t = Some(t);
                d.pressed = true;
            }
            Event::Release(t) => {
                if d.last_release_t == Some(t) {
                    return None;
                }
                d.last_release_t = Some(t);
                d.pressed = false;
            }
            Event::Right(t) => {
                if d.last_right_t == Some(t) {
                    return None;
                }
                d.last_right_t = Some(t);
            }
            Event::Pause(on) => d.paused = on,
            _ => {}
        }
        Some(ev)
    }

    /// A link closed. If it was the device's last one and a button is still held, returns the
    /// device id and a release to send to the engine.
    pub fn link_gone(&mut self, link: &str) -> Option<(String, Event)> {
        let dev = self.links.remove(link)?;
        if self.live_links(&dev) > 0 {
            return None;
        }
        let d = self.devs.get_mut(&dev)?;
        if d.pressed {
            d.pressed = false;
            return Some((dev, Event::Release(0.0)));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aim(t: f64) -> Event {
        Event::Aim { t, az: 0.0, el: 0.0 }
    }

    #[test]
    fn first_copy_wins_across_links() {
        let mut m = Mux::new();
        m.link_seen("d", "wifi:1");
        m.link_seen("d", "ble");
        assert!(m.filter("d", aim(1.000)).is_some()); // wifi
        assert!(m.filter("d", aim(1.000)).is_none()); // same sample over Bluetooth
        assert!(m.filter("d", aim(0.990)).is_none()); // older sample arriving late
        assert!(m.filter("d", aim(1.010)).is_some());
    }

    #[test]
    fn clock_restart_is_accepted() {
        let mut m = Mux::new();
        assert!(m.filter("d", aim(500.0)).is_some());
        assert!(m.filter("d", aim(2.0)).is_some()); // page reloaded
        m.hello("d", "ble");
        assert!(m.filter("d", aim(1.0)).is_some()); // hello resets the clock memory
    }

    #[test]
    fn click_on_both_links_fires_once() {
        let mut m = Mux::new();
        assert!(m.filter("d", Event::Press(7.5)).is_some());
        assert!(m.filter("d", Event::Press(7.5)).is_none());
        assert!(m.filter("d", Event::Release(7.6)).is_some());
        assert!(m.filter("d", Event::Release(7.6)).is_none());
        assert!(m.filter("d", Event::Press(9.0)).is_some()); // a new click still works
        assert!(m.filter("d", Event::Right(9.5)).is_some());
        assert!(m.filter("d", Event::Right(9.5)).is_none());
    }

    #[test]
    fn scroll_and_move_always_pass() {
        let mut m = Mux::new();
        assert!(m.filter("d", Event::Scroll(1.0, 2.0)).is_some());
        assert!(m.filter("d", Event::Scroll(1.0, 2.0)).is_some());
        assert!(m.filter("d", Event::Move(1.0, 2.0)).is_some());
    }

    #[test]
    fn pause_is_per_device_not_per_link() {
        let mut m = Mux::new();
        m.filter("d", Event::Pause(true));
        assert!(m.paused("d"));
        assert!(!m.paused("other"));
        m.filter("d", Event::Pause(false));
        assert!(!m.paused("d"));
    }

    #[test]
    fn last_link_dying_mid_press_releases() {
        let mut m = Mux::new();
        m.link_seen("d", "wifi:1");
        m.link_seen("d", "ble");
        m.filter("d", Event::Press(1.0));
        assert!(m.link_gone("wifi:1").is_none()); // Bluetooth still carries the device
        let (dev, ev) = m.link_gone("ble").expect("must release");
        assert_eq!(dev, "d");
        assert_eq!(ev, Event::Release(0.0));
        assert!(m.link_gone("ble").is_none());
    }

    #[test]
    fn link_dying_when_idle_does_nothing() {
        let mut m = Mux::new();
        m.link_seen("d", "wifi:1");
        assert!(m.link_gone("wifi:1").is_none());
    }

    #[test]
    fn reconnect_over_new_socket_keeps_the_hold() {
        let mut m = Mux::new();
        m.link_seen("d", "wifi:1");
        m.filter("d", Event::Press(1.0));
        m.link_seen("d", "wifi:2"); // phone reconnected before the old socket timed out
        assert!(m.link_gone("wifi:1").is_none());
        assert!(m.filter("d", Event::Release(2.0)).is_some());
    }
}
