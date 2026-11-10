//! A small multicast DNS responder, so a phone can open `http://airwand.local` instead of an IP address
//! that changes. It answers A (and AAAA with "none") questions for one `.local` name with this PC's address.
//! Before taking a name it asks the network whether another host already has it (then the next
//! candidate, `airwand-<pc>.local`, is used). Android 12+ and iOS resolve `.local` names; if a phone does
//! not, the panel still shows the plain IP address and its QR code.
use crate::dns;
use crate::server;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
pub const PORT: u16 = 5353;

/// Shared status, read by the control panel.
#[derive(Default)]
pub struct Mdns {
    /// The `.local` name in use, once it is claimed.
    pub name: Mutex<Option<String>>,
    pub error: Mutex<Option<String>>,
}

/// `airwand.local`, then `airwand-<pc name>.local` (letters, digits and hyphens only).
pub fn candidate_names(pc_name: &str) -> Vec<String> {
    let mut out = vec!["airwand.local".to_string()];
    let clean: String = pc_name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40)
        .collect();
    if !clean.is_empty() {
        out.push(format!("airwand-{clean}.local"));
    }
    out
}

/// What to do with one received packet: bytes to send and where. Pure, so it can be tested.
pub fn respond(buf: &[u8], src: SocketAddr, name: &str, ip_for: impl Fn(IpAddr) -> Option<Ipv4Addr>) -> Option<(Vec<u8>, SocketAddr)> {
    let msg = dns::parse(buf)?;
    if msg.response {
        return None;
    }
    let (qtype, unicast) = dns::wanted(&msg, name)?;
    let ip = ip_for(src.ip())?;
    let legacy = src.port() != PORT;
    let dest = if legacy || unicast { src } else { SocketAddr::V4(SocketAddrV4::new(GROUP, PORT)) };
    Some((dns::build_reply(name, ip, msg.id, qtype, legacy), dest))
}

fn open_socket() -> std::io::Result<(UdpSocket, Socket)> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?; // Windows' own DNS client also listens on 5353
    s.bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT)))?;
    s.set_multicast_ttl_v4(255)?;
    s.set_multicast_loop_v4(true)?;
    let _ = s.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED);
    let udp: UdpSocket = s.try_clone()?.into();
    udp.set_read_timeout(Some(Duration::from_millis(500)))?;
    Ok((udp, s))
}

/// Asks the network whether `name` is taken by someone else. True means it is free.
fn name_is_free(udp: &UdpSocket, name: &str, ip: Ipv4Addr) -> bool {
    let probe = dns::build_probe(name);
    let dest = SocketAddr::V4(SocketAddrV4::new(GROUP, PORT));
    let mut buf = [0u8; 1500];
    for _ in 0..2 {
        let _ = udp.send_to(&probe, dest);
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            if let Ok((n, _)) = udp.recv_from(&mut buf) {
                if let Some(m) = dns::parse(&buf[..n]) {
                    if dns::is_conflict(&m, name, ip) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// Starts the responder on its own thread. `pc_name` seeds the fallback name.
pub fn start(pc_name: &str) -> Arc<Mdns> {
    let state = Arc::new(Mdns::default());
    let names = candidate_names(pc_name);
    let st = state.clone();
    let _ = std::thread::Builder::new().name("airwand-mdns".into()).spawn(move || {
        let (udp, opts) = match open_socket() {
            Ok(x) => x,
            Err(e) => {
                *st.error.lock().unwrap() = Some(format!("Short name unavailable: {e}"));
                return;
            }
        };
        let mut joined: Option<Ipv4Addr> = None;
        let mut checked = Instant::now() - Duration::from_secs(60);
        let mut buf = [0u8; 1500];
        loop {
            // follow the network: join the group on the current LAN interface, claim a name again after a change
            if checked.elapsed() >= Duration::from_secs(5) {
                checked = Instant::now();
                let ip = server::lan_ipv4();
                if ip != joined {
                    if let Some(ip) = ip {
                        let _ = opts.join_multicast_v4(&GROUP, &ip);
                        let _ = opts.set_multicast_if_v4(&ip);
                    }
                    joined = ip;
                    *st.name.lock().unwrap() = None;
                    if let Some(ip) = ip {
                        let free = names.iter().find(|n| name_is_free(&udp, n, ip)).cloned();
                        match &free {
                            Some(n) => {
                                // announce, so caches on the network learn it at once
                                let note = dns::build_reply(n, ip, 0, dns::TYPE_A, false);
                                let _ = udp.send_to(&note, SocketAddr::V4(SocketAddrV4::new(GROUP, PORT)));
                                *st.error.lock().unwrap() = None;
                            }
                            None => *st.error.lock().unwrap() = Some("The short names are taken by other devices on this network.".to_string()),
                        }
                        *st.name.lock().unwrap() = free;
                    }
                }
            }
            let name = st.name.lock().unwrap().clone();
            if let Ok((n, src)) = udp.recv_from(&mut buf) {
                if let Some(name) = name {
                    if let Some((reply, dest)) = respond(&buf[..n], src, &name, server::local_ip_for) {
                        let _ = udp.send_to(&reply, dest);
                    }
                }
            }
        }
    });
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(_: IpAddr) -> Option<Ipv4Addr> {
        Some(Ipv4Addr::new(192, 168, 1, 20))
    }

    fn q(name: &str, qtype: u16, class: u16) -> Vec<u8> {
        let mut m = vec![0xAB, 0xCD, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        m.extend_from_slice(&dns::encode_name(name));
        m.extend_from_slice(&qtype.to_be_bytes());
        m.extend_from_slice(&class.to_be_bytes());
        m
    }

    #[test]
    fn names_are_clean() {
        assert_eq!(candidate_names("DESKTOP-9F3K"), vec!["airwand.local", "airwand-desktop-9f3k.local"]);
        assert_eq!(candidate_names("Ada's PC!"), vec!["airwand.local", "airwand-ada-s-pc.local"]);
        assert_eq!(candidate_names("  "), vec!["airwand.local"]);
        assert!(candidate_names(&"x".repeat(200))[1].len() < 63);
    }

    #[test]
    fn multicast_query_gets_multicast_answer() {
        let src: SocketAddr = "192.168.1.50:5353".parse().unwrap();
        let (reply, dest) = respond(&q("airwand.local", dns::TYPE_A, 1), src, "airwand.local", ip).unwrap();
        assert_eq!(dest, "224.0.0.251:5353".parse::<SocketAddr>().unwrap());
        let m = dns::parse(&reply).unwrap();
        assert_eq!(m.id, 0);
        assert_eq!(m.answers[0].a, Some(Ipv4Addr::new(192, 168, 1, 20)));
    }

    #[test]
    fn unicast_bit_and_legacy_clients_get_unicast_answers() {
        let src: SocketAddr = "192.168.1.50:5353".parse().unwrap();
        let (_, dest) = respond(&q("airwand.local", dns::TYPE_A, 0x8001), src, "airwand.local", ip).unwrap();
        assert_eq!(dest, src);
        let legacy: SocketAddr = "192.168.1.50:49152".parse().unwrap();
        let (reply, dest) = respond(&q("airwand.local", dns::TYPE_A, 1), legacy, "airwand.local", ip).unwrap();
        assert_eq!(dest, legacy);
        assert_eq!(dns::parse(&reply).unwrap().id, 0xABCD);
    }

    #[test]
    fn ignores_other_names_responses_and_junk() {
        let src: SocketAddr = "192.168.1.50:5353".parse().unwrap();
        assert!(respond(&q("printer.local", dns::TYPE_A, 1), src, "airwand.local", ip).is_none());
        assert!(respond(&dns::build_reply("airwand.local", Ipv4Addr::new(1, 2, 3, 4), 0, dns::TYPE_A, false), src, "airwand.local", ip).is_none());
        assert!(respond(&[1, 2, 3], src, "airwand.local", ip).is_none());
        assert!(respond(&q("airwand.local", dns::TYPE_A, 1), src, "airwand.local", |_| None).is_none());
    }
}
