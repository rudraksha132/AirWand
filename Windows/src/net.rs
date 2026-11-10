//! Minimal HTTP + WebSocket (RFC 6455, server side) helpers. Standard library only.

/// SHA-1 (needed by the WebSocket handshake; not used for anything security related).
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Value of `Sec-WebSocket-Accept` for a given `Sec-WebSocket-Key`.
pub fn ws_accept(key: &str) -> String {
    let mut v = key.trim().as_bytes().to_vec();
    v.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64(&sha1(&v))
}

/// JSON string literal.
pub fn jstr(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// ---------- HTTP ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    /// Header lookup, name in lowercase.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// Index just past the blank line that ends the headers.
pub fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

pub fn parse_head(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let first = lines.next()?;
    let mut it = first.split(' ');
    let method = it.next()?.to_string();
    let target = it.next()?;
    let version = it.next()?;
    if !version.starts_with("HTTP/1.") || it.next().is_some() || method.is_empty() {
        return None;
    }
    let path = target.split('?').next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for l in lines {
        if l.is_empty() {
            continue;
        }
        let (k, v) = l.split_once(':')?;
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        if headers.len() > 64 {
            return None;
        }
    }
    Some(Request { method, path, headers })
}

pub fn http_response(status: &str, ctype: &str, body: &[u8], extra: &[(&str, &str)]) -> Vec<u8> {
    let mut h = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str("\r\n");
    let mut out = h.into_bytes();
    out.extend_from_slice(body);
    out
}

// ---------- WebSocket frames ----------

#[derive(Debug, PartialEq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub enum FrameParse {
    /// Not enough bytes yet.
    Need,
    Bad(&'static str),
    /// A frame and how many bytes it used.
    Done(Frame, usize),
}

/// Parses one client frame (clients must mask). `max` caps the payload size.
pub fn parse_frame(buf: &[u8], max: usize) -> FrameParse {
    if buf.len() < 2 {
        return FrameParse::Need;
    }
    let (b0, b1) = (buf[0], buf[1]);
    if b0 & 0x70 != 0 {
        return FrameParse::Bad("reserved bits");
    }
    if b1 & 0x80 == 0 {
        return FrameParse::Bad("unmasked client frame");
    }
    let fin = b0 & 0x80 != 0;
    let opcode = b0 & 0x0F;
    let mut len = (b1 & 0x7F) as usize;
    let mut i = 2;
    if len == 126 {
        if buf.len() < 4 {
            return FrameParse::Need;
        }
        len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        i = 4;
    } else if len == 127 {
        if buf.len() < 10 {
            return FrameParse::Need;
        }
        let mut a = [0u8; 8];
        a.copy_from_slice(&buf[2..10]);
        let l = u64::from_be_bytes(a);
        if l > max as u64 {
            return FrameParse::Bad("frame too big");
        }
        len = l as usize;
        i = 10;
    }
    if len > max {
        return FrameParse::Bad("frame too big");
    }
    if opcode >= 8 && (len > 125 || !fin) {
        return FrameParse::Bad("bad control frame");
    }
    if buf.len() < i + 4 + len {
        return FrameParse::Need;
    }
    let mask = [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]];
    let payload: Vec<u8> = buf[i + 4..i + 4 + len].iter().enumerate().map(|(k, b)| b ^ mask[k % 4]).collect();
    FrameParse::Done(Frame { fin, opcode, payload }, i + 4 + len)
}

/// Server frame (never masked, always FIN).
pub fn encode_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | (opcode & 0x0F)];
    let n = payload.len();
    if n < 126 {
        f.push(n as u8);
    } else if n <= 0xFFFF {
        f.push(126);
        f.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        f.push(127);
        f.extend_from_slice(&(n as u64).to_be_bytes());
    }
    f.extend_from_slice(payload);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha1_vectors() {
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(hex(&sha1(&vec![b'a'; 1000])), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
    }

    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn rfc6455_accept_example() {
        assert_eq!(ws_accept("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(jstr("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn parses_request() {
        let r = parse_head("GET /ws?x=1 HTTP/1.1\r\nHost: 192.168.1.5:47821\r\nUpgrade: websocket\r\n\r\n").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/ws");
        assert_eq!(r.header("host"), Some("192.168.1.5:47821"));
        assert_eq!(r.header("upgrade"), Some("websocket"));
        assert!(parse_head("garbage").is_none());
        assert!(parse_head("GET / HTTP/1.1 extra\r\n\r\n").is_none());
        assert!(parse_head("GET / FTP\r\n\r\n").is_none());
    }

    #[test]
    fn finds_header_end() {
        assert_eq!(header_end(b"GET / HTTP/1.1\r\n\r\nrest"), Some(18));
        assert_eq!(header_end(b"GET / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn parses_rfc_masked_hello() {
        // RFC 6455 section 5.7: masked "Hello"
        let b = [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58];
        match parse_frame(&b, 100) {
            FrameParse::Done(f, n) => {
                assert_eq!(n, 11);
                assert!(f.fin);
                assert_eq!(f.opcode, 1);
                assert_eq!(f.payload, b"Hello");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(parse_frame(&b[..10], 100), FrameParse::Need);
    }

    #[test]
    fn rejects_unmasked_oversized_and_bad_control() {
        assert!(matches!(parse_frame(&[0x82, 0x01, 0x00], 100), FrameParse::Bad(_)));
        assert!(matches!(parse_frame(&[0x82, 0xFE, 0x10, 0x00], 100), FrameParse::Bad(_)));
        assert!(matches!(parse_frame(&[0x09, 0x80, 0, 0, 0, 0], 100), FrameParse::Bad(_)));
        assert!(matches!(parse_frame(&[0xC2, 0x80, 0, 0, 0, 0], 100), FrameParse::Bad(_)));
    }

    #[test]
    fn encodes_frames() {
        assert_eq!(encode_frame(1, b"Hello"), [0x81, 0x05, b'H', b'e', b'l', b'l', b'o']);
        let big = vec![7u8; 300];
        let f = encode_frame(2, &big);
        assert_eq!(&f[..4], &[0x82, 126, 0x01, 0x2C]);
        assert_eq!(f.len(), 304);
    }

    #[test]
    fn masked_round_trip_with_126_length() {
        let payload: Vec<u8> = (0..200u8).collect();
        let mask = [1u8, 2, 3, 4];
        let mut f = vec![0x82, 0x80 | 126, 0, 200];
        f.extend_from_slice(&mask);
        f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        match parse_frame(&f, 4096) {
            FrameParse::Done(fr, n) => {
                assert_eq!(n, f.len());
                assert_eq!(fr.payload, payload);
            }
            other => panic!("{other:?}"),
        }
    }
}
