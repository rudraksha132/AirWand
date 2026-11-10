//! Just enough DNS wire format for a multicast DNS responder (RFC 6762): parse a query or response,
//! and build the answer for one `.local` name that has one IPv4 address. Standard library only.
use std::net::Ipv4Addr;

pub const TYPE_A: u16 = 1;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_NSEC: u16 = 47;
pub const TYPE_ANY: u16 = 255;

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub name: String,
    pub qtype: u16,
    /// The "unicast response wanted" bit (top bit of the class).
    pub unicast: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub name: String,
    pub rtype: u16,
    pub a: Option<Ipv4Addr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: u16,
    pub response: bool,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

/// Reads a possibly compressed name. Returns it lowercased, dot separated, and the offset after it.
fn read_name(buf: &[u8], start: usize) -> Option<(String, usize)> {
    let mut pos = start;
    let mut end: Option<usize> = None;
    let mut jumps = 0;
    let mut name = String::new();
    loop {
        let len = *buf.get(pos)? as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            let low = *buf.get(pos + 1)? as usize;
            if end.is_none() {
                end = Some(pos + 2);
            }
            pos = ((len & 0x3F) << 8) | low;
            jumps += 1;
            if jumps > 16 {
                return None;
            }
            continue;
        }
        if len & 0xC0 != 0 {
            return None;
        }
        let label = buf.get(pos + 1..pos + 1 + len)?;
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(&String::from_utf8_lossy(label).to_ascii_lowercase());
        if name.len() > 253 {
            return None;
        }
        pos += 1 + len;
    }
    Some((name, end.unwrap_or(pos)))
}

pub fn parse(buf: &[u8]) -> Option<Message> {
    if buf.len() < 12 {
        return None;
    }
    let id = u16_at(buf, 0)?;
    let flags = u16_at(buf, 2)?;
    let (qd, an) = (u16_at(buf, 4)? as usize, u16_at(buf, 6)? as usize);
    if qd > 32 || an > 64 {
        return None;
    }
    let mut pos = 12;
    let mut questions = Vec::new();
    for _ in 0..qd {
        let (name, next) = read_name(buf, pos)?;
        let qtype = u16_at(buf, next)?;
        let qclass = u16_at(buf, next + 2)?;
        pos = next + 4;
        questions.push(Question { name, qtype, unicast: qclass & 0x8000 != 0 });
    }
    let mut answers = Vec::new();
    for _ in 0..an {
        let (name, next) = read_name(buf, pos)?;
        let rtype = u16_at(buf, next)?;
        let rdlen = u16_at(buf, next + 8)? as usize;
        let rdata = buf.get(next + 10..next + 10 + rdlen)?;
        let a = if rtype == TYPE_A && rdlen == 4 { Some(Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3])) } else { None };
        pos = next + 10 + rdlen;
        answers.push(Record { name, rtype, a });
    }
    Some(Message { id, response: flags & 0x8000 != 0, questions, answers })
}

pub fn encode_name(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.split('.').filter(|l| !l.is_empty()) {
        let l = &label.as_bytes()[..label.len().min(63)];
        out.push(l.len() as u8);
        out.extend_from_slice(l);
    }
    out.push(0);
    out
}

fn record(name: &[u8], rtype: u16, ttl: u32, rdata: &[u8]) -> Vec<u8> {
    let mut r = name.to_vec();
    r.extend_from_slice(&rtype.to_be_bytes());
    r.extend_from_slice(&0x8001u16.to_be_bytes()); // class IN with the cache-flush bit
    r.extend_from_slice(&ttl.to_be_bytes());
    r.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
    r.extend_from_slice(rdata);
    r
}

/// The reply for a question about `name`. `legacy` is for a plain DNS client (source port not 5353):
/// it gets the question echoed back, its id, and a short TTL, as RFC 6762 section 6.7 asks.
pub fn build_reply(name: &str, ip: Ipv4Addr, qid: u16, qtype: u16, legacy: bool) -> Vec<u8> {
    let wire = encode_name(name);
    let ttl = if legacy { 10 } else { 120 };
    let a = record(&wire, TYPE_A, ttl, &ip.octets());
    // NSEC says "this name has an A record and nothing else", so a client waiting for AAAA stops waiting.
    let mut nsec_data = wire.clone();
    nsec_data.extend_from_slice(&[0x00, 0x01, 0x40]);
    let nsec = record(&wire, TYPE_NSEC, ttl, &nsec_data);

    let (answers, extra, an, ar): (Vec<u8>, Vec<u8>, u16, u16) = if qtype == TYPE_AAAA { (nsec, Vec::new(), 1, 0) } else { (a, nsec, 1, 1) };
    let mut m = Vec::new();
    m.extend_from_slice(&(if legacy { qid } else { 0 }).to_be_bytes());
    m.extend_from_slice(&0x8400u16.to_be_bytes()); // response, authoritative
    m.extend_from_slice(&(if legacy { 1u16 } else { 0 }).to_be_bytes());
    m.extend_from_slice(&an.to_be_bytes());
    m.extend_from_slice(&0u16.to_be_bytes());
    m.extend_from_slice(&ar.to_be_bytes());
    if legacy {
        m.extend_from_slice(&wire);
        m.extend_from_slice(&qtype.to_be_bytes());
        m.extend_from_slice(&1u16.to_be_bytes());
    }
    m.extend_from_slice(&answers);
    m.extend_from_slice(&extra);
    m
}

/// A query for `name` (type ANY, multicast response), used to find out whether another host already owns it.
pub fn build_probe(name: &str) -> Vec<u8> {
    let mut m = vec![0u8; 12];
    m[5] = 1; // one question
    m.extend_from_slice(&encode_name(name));
    m.extend_from_slice(&TYPE_ANY.to_be_bytes());
    m.extend_from_slice(&1u16.to_be_bytes());
    m
}

/// Which question in this query we should answer, if any: `Some(qtype)` for A, ANY or AAAA about `name`.
/// An A or ANY question wins over AAAA when both are present.
pub fn wanted(msg: &Message, name: &str) -> Option<(u16, bool)> {
    let mine = |q: &&Question| q.name == name;
    if let Some(q) = msg.questions.iter().filter(mine).find(|q| q.qtype == TYPE_A || q.qtype == TYPE_ANY) {
        return Some((TYPE_A, q.unicast));
    }
    msg.questions.iter().filter(mine).find(|q| q.qtype == TYPE_AAAA).map(|q| (TYPE_AAAA, q.unicast))
}

/// True when a response says that another host (not `ours`) has `name`.
pub fn is_conflict(msg: &Message, name: &str, ours: Ipv4Addr) -> bool {
    msg.response && msg.answers.iter().any(|r| r.name == name && r.rtype == TYPE_A && r.a.map(|a| a != ours).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(name: &str, qtype: u16, class: u16) -> Vec<u8> {
        let mut m = vec![0x12, 0x34, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        m.extend_from_slice(&encode_name(name));
        m.extend_from_slice(&qtype.to_be_bytes());
        m.extend_from_slice(&class.to_be_bytes());
        m
    }

    #[test]
    fn parses_a_question_case_insensitively() {
        let m = parse(&query("AirWand.LOCAL", TYPE_A, 0x8001)).unwrap();
        assert_eq!(m.id, 0x1234);
        assert!(!m.response);
        assert_eq!(m.questions, vec![Question { name: "airwand.local".into(), qtype: 1, unicast: true }]);
    }

    #[test]
    fn reply_round_trips() {
        let ip = Ipv4Addr::new(192, 168, 1, 20);
        let r = parse(&build_reply("airwand.local", ip, 7, TYPE_A, false)).unwrap();
        assert!(r.response);
        assert_eq!(r.id, 0);
        assert_eq!(r.answers[0], Record { name: "airwand.local".into(), rtype: TYPE_A, a: Some(ip) });
        let legacy = parse(&build_reply("airwand.local", ip, 7, TYPE_A, true)).unwrap();
        assert_eq!(legacy.id, 7);
        assert_eq!(legacy.questions.len(), 1);
        let aaaa = parse(&build_reply("airwand.local", ip, 7, TYPE_AAAA, false)).unwrap();
        assert_eq!(aaaa.answers[0].rtype, TYPE_NSEC);
    }

    #[test]
    fn a_reply_carries_the_expected_bytes() {
        let r = build_reply("a.local", Ipv4Addr::new(10, 0, 0, 5), 0, TYPE_A, false);
        assert_eq!(&r[..12], &[0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 1]);
        let tail = &r[12..];
        assert_eq!(&tail[..9], &[1, b'a', 5, b'l', b'o', b'c', b'a', b'l', 0]);
        assert_eq!(&tail[9..19], &[0, 1, 0x80, 1, 0, 0, 0, 120, 0, 4]);
        assert_eq!(&tail[19..23], &[10, 0, 0, 5]);
    }

    #[test]
    fn picks_the_question_to_answer() {
        let a = parse(&query("airwand.local", TYPE_A, 1)).unwrap();
        assert_eq!(wanted(&a, "airwand.local"), Some((TYPE_A, false)));
        assert_eq!(wanted(&a, "other.local"), None);
        let any = parse(&query("airwand.local", TYPE_ANY, 0x8001)).unwrap();
        assert_eq!(wanted(&any, "airwand.local"), Some((TYPE_A, true)));
        let v6 = parse(&query("airwand.local", TYPE_AAAA, 1)).unwrap();
        assert_eq!(wanted(&v6, "airwand.local"), Some((TYPE_AAAA, false)));
        let ptr = parse(&query("airwand.local", 12, 1)).unwrap();
        assert_eq!(wanted(&ptr, "airwand.local"), None);
    }

    #[test]
    fn detects_a_conflicting_owner() {
        let mine = Ipv4Addr::new(192, 168, 1, 20);
        let other = parse(&build_reply("airwand.local", Ipv4Addr::new(192, 168, 1, 99), 0, TYPE_A, false)).unwrap();
        assert!(is_conflict(&other, "airwand.local", mine));
        let own = parse(&build_reply("airwand.local", mine, 0, TYPE_A, false)).unwrap();
        assert!(!is_conflict(&own, "airwand.local", mine));
        assert!(!is_conflict(&other, "different.local", mine));
        assert!(!is_conflict(&parse(&build_probe("airwand.local")).unwrap(), "airwand.local", mine));
    }

    #[test]
    fn follows_compression_pointers_and_rejects_loops() {
        // question name is a pointer to a name stored later in the packet
        let mut m = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        m.extend_from_slice(&[0xC0, 18, 0, 1, 0, 1]); // pointer to offset 18
        m.extend_from_slice(&[3, b'a', b'b', b'c', 0]); // at 18
        let p = parse(&m).unwrap();
        assert_eq!(p.questions[0].name, "abc");
        let mut bad = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        bad.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]); // points at itself
        assert!(parse(&bad).is_none());
    }

    #[test]
    fn garbage_does_not_panic() {
        for n in 0..40 {
            let junk: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let _ = parse(&junk);
        }
        assert!(parse(&[0u8; 5]).is_none());
        assert!(parse(&[0, 0, 0, 0, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0]).is_none());
    }
}
