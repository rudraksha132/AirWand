//! Tiny local web server (standard library only). It serves
//!   `/`            the phone page (same page as `Web/index.html`)
//!   `/ws`          the Wi-Fi link: WebSocket frames carry the same packets as the Bluetooth link
//!   `/ui`, `/api`  the control panel, reachable from this PC only
//! Everything else is a 404. The panel API refuses requests that did not come from this PC's own
//! loopback address, that carry a wrong Host or Origin header, or that lack `X-AirWand: 1`
//! (which a web page on another site cannot send without a CORS preflight we never answer).
use crate::net::{self, FrameParse, Request};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Sends one binary WebSocket message to a connected client.
pub type Sink = Arc<dyn Fn(&[u8]) + Send + Sync>;

pub trait Backend: Send + Sync + 'static {
    /// JSON for the control panel.
    fn state_json(&self) -> String;
    /// A control-panel action, e.g. ("pause", "1").
    fn action(&self, name: &str, value: &str) -> Result<(), String>;
    /// A Wi-Fi client connected. Return false to refuse it.
    fn ws_open(&self, key: &str, send: Sink) -> bool;
    fn ws_packet(&self, key: &str, data: &[u8]);
    fn ws_close(&self, key: &str);
}

pub struct Assets {
    pub phone: &'static str,
    pub panel: &'static str,
    pub icon: &'static [u8],
}

const MAX_CONNECTIONS: usize = 24;
const MAX_FRAME: usize = 4096;

/// `lan = false` listens on this PC only (Wi-Fi link off, control panel still works).
pub fn bind(port: u16, lan: bool) -> io::Result<TcpListener> {
    let addr = if lan { SocketAddr::from(([0, 0, 0, 0], port)) } else { SocketAddr::from(([127, 0, 0, 1], port)) };
    TcpListener::bind(addr)
}

/// Blocks, serving connections, one thread each.
pub fn serve(listener: TcpListener, backend: Arc<dyn Backend>, assets: Assets) {
    let assets: &'static Assets = Box::leak(Box::new(assets));
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let live = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if live.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            drop(stream);
            continue;
        }
        live.fetch_add(1, Ordering::Relaxed);
        let be = backend.clone();
        let counter = live.clone();
        let spawned = std::thread::Builder::new().name("airwand-http".into()).spawn(move || {
            handle(stream, &be, assets, port);
            counter.fetch_sub(1, Ordering::Relaxed);
        });
        if spawned.is_err() {
            live.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// This PC's address on the local network (the route a packet to the internet would take; nothing is sent).
pub fn lan_ips() -> Vec<String> {
    let probe = || -> Option<String> {
        let s = UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect("8.8.8.8:80").ok()?;
        let ip = s.local_addr().ok()?.ip();
        if ip.is_unspecified() || ip.is_loopback() {
            None
        } else {
            Some(ip.to_string())
        }
    };
    probe().into_iter().collect()
}

fn reply(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8], extra: &[(&str, &str)]) {
    let _ = s.write_all(&net::http_response(status, ctype, body, extra));
}

fn host_is_local(req: &Request, port: u16) -> bool {
    match req.header("host") {
        Some(h) => {
            let h = h.to_ascii_lowercase();
            h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}")
        }
        None => false,
    }
}

/// A browser always sends Origin on cross-site requests; same-origin means it equals http://<Host>.
fn origin_ok(req: &Request) -> bool {
    match (req.header("origin"), req.header("host")) {
        (None, _) => true,
        (Some(o), Some(h)) => o.eq_ignore_ascii_case(&format!("http://{h}")),
        (Some(_), None) => false,
    }
}

const PANEL_HEADERS: [(&str, &str); 2] = [
    ("Content-Security-Policy", "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'"),
    ("X-Frame-Options", "DENY"),
];

fn handle(mut s: TcpStream, be: &Arc<dyn Backend>, assets: &'static Assets, port: u16) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = s.set_nodelay(true);
    let Ok(peer) = s.peer_addr() else { return };
    let local = peer.ip().is_loopback();

    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut tmp = [0u8; 2048];
    let end = loop {
        if let Some(e) = net::header_end(&buf) {
            break e;
        }
        if buf.len() > 8192 {
            return;
        }
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    };
    let Ok(head) = std::str::from_utf8(&buf[..end]) else { return };
    let Some(req) = net::parse_head(head) else {
        reply(&mut s, "400 Bad Request", "text/plain", b"bad request", &[]);
        return;
    };
    let mut rest: Vec<u8> = buf[end..].to_vec();

    let is_ws = req.header("upgrade").map(|v| v.eq_ignore_ascii_case("websocket")).unwrap_or(false);
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/ws") if is_ws => websocket(s, &req, rest, peer, be),
        ("GET", "/") | ("GET", "/index.html") => {
            reply(&mut s, "200 OK", "text/html; charset=utf-8", assets.phone.as_bytes(), &[]);
        }
        ("GET", "/icon.png") => reply(&mut s, "200 OK", "image/png", assets.icon, &[]),
        ("GET", "/ui") => {
            if local && host_is_local(&req, port) {
                reply(&mut s, "200 OK", "text/html; charset=utf-8", assets.panel.as_bytes(), &PANEL_HEADERS);
            } else {
                reply(&mut s, "403 Forbidden", "text/plain", b"The control panel is only available on the PC running AirWand.", &[]);
            }
        }
        ("GET", "/api/state") => {
            if local && host_is_local(&req, port) {
                reply(&mut s, "200 OK", "application/json", be.state_json().as_bytes(), &[]);
            } else {
                reply(&mut s, "403 Forbidden", "text/plain", b"forbidden", &[]);
            }
        }
        ("POST", p) if p.starts_with("/api/") => {
            let allowed = local && host_is_local(&req, port) && origin_ok(&req) && req.header("x-airwand") == Some("1");
            if !allowed {
                reply(&mut s, "403 Forbidden", "text/plain", b"forbidden", &[]);
                return;
            }
            let want: usize = req.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
            if want > 256 {
                reply(&mut s, "413 Payload Too Large", "text/plain", b"too large", &[]);
                return;
            }
            while rest.len() < want {
                match s.read(&mut tmp) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => rest.extend_from_slice(&tmp[..n]),
                }
            }
            let value = String::from_utf8_lossy(&rest[..want.min(rest.len())]).trim().to_string();
            match be.action(&p[5..], &value) {
                Ok(()) => reply(&mut s, "200 OK", "application/json", b"{\"ok\":true}", &[]),
                Err(e) => {
                    let body = format!("{{\"ok\":false,\"error\":{}}}", net::jstr(&e));
                    reply(&mut s, "400 Bad Request", "application/json", body.as_bytes(), &[]);
                }
            }
        }
        _ => reply(&mut s, "404 Not Found", "text/plain", b"not found", &[]),
    }
}

fn write_frame(w: &Mutex<TcpStream>, opcode: u8, payload: &[u8]) {
    if let Ok(mut g) = w.lock() {
        let _ = g.write_all(&net::encode_frame(opcode, payload));
    }
}

fn websocket(mut s: TcpStream, req: &Request, mut buf: Vec<u8>, peer: SocketAddr, be: &Arc<dyn Backend>) {
    let key = match req.header("sec-websocket-key") {
        Some(k) if req.header("sec-websocket-version") == Some("13") && origin_ok(req) => k.to_string(),
        _ => {
            reply(&mut s, "400 Bad Request", "text/plain", b"bad websocket request", &[]);
            return;
        }
    };
    let accept = net::ws_accept(&key);
    let hs = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if s.write_all(hs.as_bytes()).is_err() {
        return;
    }
    let Ok(wclone) = s.try_clone() else { return };
    let writer = Arc::new(Mutex::new(wclone));
    let id = format!("wifi:{peer}");

    let w2 = writer.clone();
    let sink: Sink = Arc::new(move |data: &[u8]| write_frame(&w2, 2, data));
    if !be.ws_open(&id, sink) {
        write_frame(&writer, 8, &[0x03, 0xE8]);
        let _ = s.shutdown(std::net::Shutdown::Both);
        return;
    }

    let _ = s.set_read_timeout(Some(Duration::from_secs(12))); // the page pings every second
    let mut tmp = [0u8; 2048];
    'conn: loop {
        loop {
            match net::parse_frame(&buf, MAX_FRAME) {
                FrameParse::Need => break,
                FrameParse::Bad(_) => break 'conn,
                FrameParse::Done(f, used) => {
                    buf.drain(..used);
                    match f.opcode {
                        2 if f.fin => be.ws_packet(&id, &f.payload),
                        1 if f.fin => write_frame(&writer, 1, b"k"), // keep-alive: the page pings every second and treats silence as a dead link
                        9 => write_frame(&writer, 10, &f.payload),
                        10 => {}
                        8 => {
                            write_frame(&writer, 8, &[0x03, 0xE8]);
                            break 'conn;
                        }
                        _ => break 'conn,
                    }
                }
            }
        }
        if buf.len() > 16 * 1024 {
            break;
        }
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    be.ws_close(&id);
    let _ = s.shutdown(std::net::Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[derive(Default)]
    struct Mock {
        packets: Mutex<Vec<Vec<u8>>>,
        actions: Mutex<Vec<(String, String)>>,
        opened: AtomicUsize,
        closed: AtomicUsize,
    }

    impl Backend for Mock {
        fn state_json(&self) -> String {
            "{\"ok\":1}".to_string()
        }
        fn action(&self, name: &str, value: &str) -> Result<(), String> {
            if name == "bad" {
                return Err("nope".to_string());
            }
            self.actions.lock().unwrap().push((name.to_string(), value.to_string()));
            Ok(())
        }
        fn ws_open(&self, _key: &str, send: Sink) -> bool {
            self.opened.fetch_add(1, Ordering::SeqCst);
            send(&[1, 0, b'P', b'C']);
            true
        }
        fn ws_packet(&self, _key: &str, data: &[u8]) {
            self.packets.lock().unwrap().push(data.to_vec());
        }
        fn ws_close(&self, _key: &str) {
            self.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn start() -> (u16, Arc<Mock>) {
        let l = bind(0, false).unwrap();
        let port = l.local_addr().unwrap().port();
        let m = Arc::new(Mock::default());
        let be: Arc<dyn Backend> = m.clone();
        std::thread::spawn(move || serve(l, be, Assets { phone: "PHONE", panel: "PANEL", icon: b"png" }));
        (port, m)
    }

    fn http(port: u16, raw: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    fn masked(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [9u8, 8, 7, 6];
        let mut f = vec![0x80 | opcode, 0x80 | payload.len() as u8];
        f.extend_from_slice(&mask);
        f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        f
    }

    fn wait_for(mut ok: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) {
            if ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn serves_pages_and_guards_the_panel() {
        let (p, _m) = start();
        let host = format!("127.0.0.1:{p}");
        assert!(http(p, &format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n")).ends_with("PHONE"));
        assert!(http(p, &format!("GET /ui HTTP/1.1\r\nHost: {host}\r\n\r\n")).ends_with("PANEL"));
        assert!(http(p, "GET /ui HTTP/1.1\r\nHost: evil.example:80\r\n\r\n").starts_with("HTTP/1.1 403"));
        assert!(http(p, "GET /api/state HTTP/1.1\r\nHost: evil.example\r\n\r\n").starts_with("HTTP/1.1 403"));
        assert!(http(p, &format!("GET /api/state HTTP/1.1\r\nHost: {host}\r\n\r\n")).ends_with("{\"ok\":1}"));
        assert!(http(p, &format!("GET /nothing HTTP/1.1\r\nHost: {host}\r\n\r\n")).starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn api_posts_need_header_and_same_origin() {
        let (p, m) = start();
        let host = format!("127.0.0.1:{p}");
        let post = |extra: &str, path: &str| {
            http(p, &format!("POST {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Content-Length: 1\r\n\r\n1"))
        };
        assert!(post("", "/api/pause").starts_with("HTTP/1.1 403"));
        assert!(post("X-AirWand: 1\r\nOrigin: http://evil.example\r\n", "/api/pause").starts_with("HTTP/1.1 403"));
        assert!(m.actions.lock().unwrap().is_empty());
        assert!(post("X-AirWand: 1\r\n", "/api/pause").contains("\"ok\":true"));
        assert!(post(&format!("X-AirWand: 1\r\nOrigin: http://{host}\r\n"), "/api/range").contains("\"ok\":true"));
        assert!(post("X-AirWand: 1\r\n", "/api/bad").starts_with("HTTP/1.1 400"));
        assert_eq!(
            *m.actions.lock().unwrap(),
            vec![("pause".to_string(), "1".to_string()), ("range".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn websocket_round_trip() {
        let (p, m) = start();
        let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        s.write_all(
            format!(
                "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
        // read the handshake response and the status frame the backend sends on open
        let mut got: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 512];
        while !(net::header_end(&got).map(|e| got.len() >= e + 6).unwrap_or(false)) {
            let n = s.read(&mut tmp).unwrap();
            assert!(n > 0);
            got.extend_from_slice(&tmp[..n]);
        }
        let end = net::header_end(&got).unwrap();
        let head = String::from_utf8_lossy(&got[..end]).to_string();
        assert!(head.starts_with("HTTP/1.1 101"));
        assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
        assert_eq!(&got[end..end + 6], &[0x82, 4, 1, 0, b'P', b'C']);

        s.write_all(&masked(2, &[7])).unwrap();
        s.write_all(&masked(2, &[3, 1, 2])).unwrap();
        assert!(wait_for(|| m.packets.lock().unwrap().len() == 2));
        assert_eq!(*m.packets.lock().unwrap(), vec![vec![7u8], vec![3u8, 1, 2]]);

        s.write_all(&masked(8, &[])).unwrap();
        assert!(wait_for(|| m.closed.load(Ordering::SeqCst) == 1));
        assert_eq!(m.opened.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn websocket_keepalive_is_echoed() {
        let (p, _m) = start();
        let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.write_all(
            format!("GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").as_bytes(),
        )
        .unwrap();
        let mut got: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 512];
        while !net::header_end(&got).map(|e| got.len() >= e + 6).unwrap_or(false) {
            let n = s.read(&mut tmp).unwrap();
            assert!(n > 0);
            got.extend_from_slice(&tmp[..n]);
        }
        let end = net::header_end(&got).unwrap();
        let mut rest = got[end + 6..].to_vec();
        s.write_all(&masked(1, b"k")).unwrap();
        while rest.len() < 3 {
            let n = s.read(&mut tmp).unwrap();
            assert!(n > 0);
            rest.extend_from_slice(&tmp[..n]);
        }
        assert_eq!(&rest[..3], &[0x81, 1, b'k']); // text frame "k" back, so the page can tell the link is alive
    }

    #[test]
    fn websocket_rejects_unmasked_frames_and_foreign_origins() {
        let (p, m) = start();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let hs = |origin: &str| {
            format!("GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n{origin}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n")
        };
        assert!(http(p, &hs("Origin: http://evil.example\r\n")).starts_with("HTTP/1.1 400"));
        assert_eq!(m.opened.load(Ordering::SeqCst), 0);

        let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
        s.write_all(hs("").as_bytes()).unwrap();
        assert!(wait_for(|| m.opened.load(Ordering::SeqCst) == 1));
        s.write_all(&[0x82, 0x01, 0x07]).unwrap(); // unmasked: protocol error
        assert!(wait_for(|| m.closed.load(Ordering::SeqCst) == 1));
        assert!(m.packets.lock().unwrap().is_empty());
    }
}
