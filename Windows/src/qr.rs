//! QR Code encoder (ISO/IEC 18004), byte mode, error correction L or M, versions 1 to 10.
//! Standard library only. Enough for a link such as `http://192.168.1.20:47821` (version 3 at level M).
//! Structure follows the standard: bit stream, Reed-Solomon blocks, interleave, function patterns,
//! zig-zag placement, the best of the 8 masks, format and version information.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ecc {
    L,
    M,
}

/// (total codewords, ecc codewords per block, [(blocks, data codewords per block); 2]) for versions 1..=10.
type Row = (usize, usize, [(usize, usize); 2]);

const TABLE_L: [Row; 10] = [
    (26, 7, [(1, 19), (0, 0)]),
    (44, 10, [(1, 34), (0, 0)]),
    (70, 15, [(1, 55), (0, 0)]),
    (100, 20, [(1, 80), (0, 0)]),
    (134, 26, [(1, 108), (0, 0)]),
    (172, 18, [(2, 68), (0, 0)]),
    (196, 20, [(2, 78), (0, 0)]),
    (242, 24, [(2, 97), (0, 0)]),
    (292, 30, [(2, 116), (0, 0)]),
    (346, 18, [(2, 68), (2, 69)]),
];

const TABLE_M: [Row; 10] = [
    (26, 10, [(1, 16), (0, 0)]),
    (44, 16, [(1, 28), (0, 0)]),
    (70, 26, [(1, 44), (0, 0)]),
    (100, 18, [(2, 32), (0, 0)]),
    (134, 24, [(2, 43), (0, 0)]),
    (172, 16, [(4, 27), (0, 0)]),
    (196, 18, [(4, 31), (0, 0)]),
    (242, 22, [(2, 38), (2, 39)]),
    (292, 22, [(3, 36), (2, 37)]),
    (346, 26, [(4, 43), (1, 44)]),
];

const ALIGN: [&[usize]; 10] = [&[], &[6, 18], &[6, 22], &[6, 26], &[6, 30], &[6, 34], &[6, 22, 38], &[6, 24, 42], &[6, 26, 46], &[6, 28, 50]];

fn row(ecc: Ecc, version: usize) -> Row {
    match ecc {
        Ecc::L => TABLE_L[version - 1],
        Ecc::M => TABLE_M[version - 1],
    }
}

fn data_codewords(r: &Row) -> usize {
    r.2.iter().map(|(n, d)| n * d).sum()
}

/// Largest payload (bytes) that fits in some version at this level.
pub fn max_bytes(ecc: Ecc) -> usize {
    (data_codewords(&row(ecc, 10)) * 8 - 4 - 16) / 8
}

// ---------- Reed-Solomon over GF(256), polynomial 0x11D ----------

fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u32 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= (((y >> i) & 1) as u32) * x as u32;
    }
    z as u8
}

fn rs_divisor(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree];
    result[degree - 1] = 1;
    let mut root: u8 = 1;
    for _ in 0..degree {
        for j in 0..degree {
            result[j] = gf_mul(result[j], root);
            if j + 1 < degree {
                result[j] ^= result[j + 1];
            }
        }
        root = gf_mul(root, 2);
    }
    result
}

fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; divisor.len()];
    for &b in data {
        let factor = b ^ result.remove(0);
        result.push(0);
        for (r, &c) in result.iter_mut().zip(divisor) {
            *r ^= gf_mul(c, factor);
        }
    }
    result
}

fn push_bits(bits: &mut Vec<bool>, value: u32, count: usize) {
    for i in (0..count).rev() {
        bits.push((value >> i) & 1 == 1);
    }
}

fn interleave(data: &[u8], r: &Row) -> Vec<u8> {
    let (total, ecc_len, groups) = *r;
    let div = rs_divisor(ecc_len);
    let mut blocks: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut k = 0;
    for &(n, d) in groups.iter() {
        for _ in 0..n {
            let dat = &data[k..k + d];
            k += d;
            blocks.push((dat.to_vec(), rs_remainder(dat, &div)));
        }
    }
    let longest = blocks.iter().map(|b| b.0.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(total);
    for i in 0..longest {
        for b in &blocks {
            if i < b.0.len() {
                out.push(b.0[i]);
            }
        }
    }
    for i in 0..ecc_len {
        for b in &blocks {
            out.push(b.1[i]);
        }
    }
    out
}

// ---------- the symbol ----------

pub struct Qr {
    size: usize,
    version: usize,
    mask: u8,
    modules: Vec<bool>,
    func: Vec<bool>,
}

impl Qr {
    /// Smallest version that holds `data` at level `ecc`, or None if it does not fit in version 10.
    pub fn encode(data: &[u8], ecc: Ecc) -> Option<Qr> {
        let cc_bits = |v: usize| if v <= 9 { 8 } else { 16 };
        let version = (1..=10).find(|&v| 4 + cc_bits(v) + 8 * data.len() <= 8 * data_codewords(&row(ecc, v)))?;
        let r = row(ecc, version);
        let cap = data_codewords(&r) * 8;

        let mut bits: Vec<bool> = Vec::with_capacity(cap);
        push_bits(&mut bits, 0b0100, 4); // byte mode
        push_bits(&mut bits, data.len() as u32, cc_bits(version));
        for &b in data {
            push_bits(&mut bits, b as u32, 8);
        }
        let term = (cap - bits.len()).min(4);
        push_bits(&mut bits, 0, term);
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        let mut pad = 0xECu32;
        while bits.len() < cap {
            push_bits(&mut bits, pad, 8);
            pad ^= 0xEC ^ 0x11;
        }
        let bytes: Vec<u8> = bits.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | b as u8)).collect();
        let all = interleave(&bytes, &r);

        let size = 17 + 4 * version;
        let mut q = Qr { size, version, mask: 0, modules: vec![false; size * size], func: vec![false; size * size] };
        q.draw_function_patterns();
        q.draw_codewords(&all);

        let mut best = (i32::MAX, 0u8);
        for m in 0..8u8 {
            q.apply_mask(m);
            q.draw_format(ecc, m);
            let p = q.penalty();
            if p < best.0 {
                best = (p, m);
            }
            q.apply_mask(m); // undo
        }
        q.apply_mask(best.1);
        q.draw_format(ecc, best.1);
        q.mask = best.1;
        Some(q)
    }

    /// Level M when it fits (more forgiving of a dirty or glary screen), else level L.
    pub fn encode_best(data: &[u8]) -> Option<Qr> {
        Qr::encode(data, Ecc::M).or_else(|| Qr::encode(data, Ecc::L))
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn version(&self) -> usize {
        self.version
    }

    pub fn mask(&self) -> u8 {
        self.mask
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x]
    }

    fn set_func(&mut self, x: i32, y: i32, dark: bool) {
        if x >= 0 && y >= 0 && (x as usize) < self.size && (y as usize) < self.size {
            let i = y as usize * self.size + x as usize;
            self.modules[i] = dark;
            self.func[i] = true;
        }
    }

    fn draw_function_patterns(&mut self) {
        let n = self.size as i32;
        for i in 0..n {
            self.set_func(6, i, i % 2 == 0);
            self.set_func(i, 6, i % 2 == 0);
        }
        for &(cx, cy) in &[(3, 3), (n - 4, 3), (3, n - 4)] {
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    let dist = dx.abs().max(dy.abs());
                    self.set_func(cx + dx, cy + dy, dist != 2 && dist != 4);
                }
            }
        }
        let pos = ALIGN[self.version - 1];
        let last = pos.len().saturating_sub(1);
        for i in 0..pos.len() {
            for j in 0..pos.len() {
                if (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0) {
                    continue;
                }
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        self.set_func(pos[i] as i32 + dx, pos[j] as i32 + dy, dx.abs().max(dy.abs()) != 1);
                    }
                }
            }
        }
        // reserve the format area (real bits are drawn once the mask is known), then version info
        self.draw_format(Ecc::M, 0);
        if self.version >= 7 {
            let mut rem = self.version as u32;
            for _ in 0..12 {
                rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
            }
            let bits = ((self.version as u32) << 12) | rem;
            for i in 0..18 {
                let dark = (bits >> i) & 1 == 1;
                let a = n - 11 + (i % 3) as i32;
                let b = (i / 3) as i32;
                self.set_func(a, b, dark);
                self.set_func(b, a, dark);
            }
        }
    }

    fn draw_format(&mut self, ecc: Ecc, mask: u8) {
        let n = self.size as i32;
        let bits = format_bits(ecc, mask);
        let bit = |i: u32| (bits >> i) & 1 == 1;
        for i in 0..6 {
            self.set_func(8, i, bit(i as u32));
        }
        self.set_func(8, 7, bit(6));
        self.set_func(8, 8, bit(7));
        self.set_func(7, 8, bit(8));
        for i in 9..15 {
            self.set_func(14 - i, 8, bit(i as u32));
        }
        for i in 0..8 {
            self.set_func(n - 1 - i, 8, bit(i as u32));
        }
        for i in 8..15 {
            self.set_func(8, n - 15 + i, bit(i as u32));
        }
        self.set_func(8, n - 8, true); // the always-dark module
    }

    fn draw_codewords(&mut self, data: &[u8]) {
        let n = self.size as i32;
        let mut i = 0usize;
        let mut right = n - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..n {
                for j in 0..2 {
                    let x = right - j;
                    let upward = ((right + 1) & 2) == 0;
                    let y = if upward { n - 1 - vert } else { vert };
                    let idx = y as usize * self.size + x as usize;
                    if !self.func[idx] && i < data.len() * 8 {
                        self.modules[idx] = (data[i >> 3] >> (7 - (i & 7))) & 1 == 1;
                        i += 1;
                    }
                }
            }
            right -= 2;
        }
    }

    fn apply_mask(&mut self, mask: u8) {
        for y in 0..self.size {
            for x in 0..self.size {
                let (xi, yi) = (x as i64, y as i64);
                let invert = match mask {
                    0 => (xi + yi) % 2 == 0,
                    1 => yi % 2 == 0,
                    2 => xi % 3 == 0,
                    3 => (xi + yi) % 3 == 0,
                    4 => (xi / 3 + yi / 2) % 2 == 0,
                    5 => xi * yi % 2 + xi * yi % 3 == 0,
                    6 => (xi * yi % 2 + xi * yi % 3) % 2 == 0,
                    _ => ((xi + yi) % 2 + xi * yi % 3) % 2 == 0,
                };
                let i = y * self.size + x;
                if invert && !self.func[i] {
                    self.modules[i] = !self.modules[i];
                }
            }
        }
    }

    fn penalty(&self) -> i32 {
        let n = self.size;
        let at = |x: usize, y: usize| self.modules[y * n + x];
        let mut score = 0;
        // N1 (runs of 5 or more) and N3 (finder-like 1:1:3:1:1 with 4 light modules), rows then columns
        for vertical in [false, true] {
            for a in 0..n {
                let line: Vec<bool> = (0..n).map(|b| if vertical { at(a, b) } else { at(b, a) }).collect();
                let mut run = 1;
                for i in 1..n {
                    if line[i] == line[i - 1] {
                        run += 1;
                    } else {
                        if run >= 5 {
                            score += 3 + (run - 5);
                        }
                        run = 1;
                    }
                }
                if run >= 5 {
                    score += 3 + (run - 5);
                }
                const P: [bool; 7] = [true, false, true, true, true, false, true];
                for i in 0..n.saturating_sub(6) {
                    if line[i..i + 7] == P {
                        let before = i >= 4 && line[i - 4..i].iter().all(|b| !b);
                        let after = i + 11 <= n && line[i + 7..i + 11].iter().all(|b| !b);
                        if before || after {
                            score += 40;
                        }
                    }
                }
            }
        }
        // N2 (2x2 blocks of one colour)
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = at(x, y);
                if c == at(x + 1, y) && c == at(x, y + 1) && c == at(x + 1, y + 1) {
                    score += 3;
                }
            }
        }
        // N4 (dark/light balance)
        let dark = self.modules.iter().filter(|&&d| d).count() as i32;
        let total = (n * n) as i32;
        let k = ((dark * 20 - total * 10).abs() + total - 1) / total - 1;
        score + k.max(0) * 10
    }

    /// Black-on-white SVG with a quiet zone of `quiet` modules (the standard asks for 4).
    pub fn to_svg(&self, quiet: usize) -> String {
        let dim = self.size + 2 * quiet;
        let mut path = String::new();
        for y in 0..self.size {
            let mut x = 0;
            while x < self.size {
                if self.get(x, y) {
                    let start = x;
                    while x < self.size && self.get(x, y) {
                        x += 1;
                    }
                    path.push_str(&format!("M{},{}h{}v1h-{}z", start + quiet, y + quiet, x - start, x - start));
                } else {
                    x += 1;
                }
            }
        }
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {dim} {dim}\" shape-rendering=\"crispEdges\"><rect width=\"{dim}\" height=\"{dim}\" fill=\"#fff\"/><path d=\"{path}\" fill=\"#000\"/></svg>"
        )
    }
}

/// 15 format bits: 2 bits level, 3 bits mask, 10 bits BCH, xor 0x5412.
fn format_bits(ecc: Ecc, mask: u8) -> u32 {
    let level: u32 = match ecc {
        Ecc::L => 1,
        Ecc::M => 0,
    };
    let data = (level << 3) | mask as u32;
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    ((data << 10) | rem) ^ 0x5412
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_consistent() {
        for ecc in [Ecc::L, Ecc::M] {
            for v in 1..=10 {
                let (total, e, g) = row(ecc, v);
                let sum: usize = g.iter().map(|(n, d)| n * (d + e)).sum();
                assert_eq!(sum, total, "version {v} {ecc:?}");
            }
        }
    }

    #[test]
    fn format_info_known_values() {
        assert_eq!(format_bits(Ecc::M, 0), 0x5412);
        assert_eq!(format_bits(Ecc::L, 0), 0x77C4);
        assert_eq!(format_bits(Ecc::M, 5), 0b100000011001110);
    }

    #[test]
    fn reed_solomon_known_vector() {
        // Worked example "HELLO WORLD" at version 1-M (data codewords, then the 10 ecc codewords).
        let hello = [32u8, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17];
        assert_eq!(rs_remainder(&hello, &rs_divisor(10)), vec![196, 35, 39, 119, 235, 215, 231, 226, 93, 23]);
    }

    #[test]
    fn sizes_and_versions() {
        let q = Qr::encode(b"http://192.168.1.20:47821", Ecc::M).unwrap();
        assert_eq!(q.version(), 2);
        assert_eq!(q.size(), 25);
        let q = Qr::encode(b"a", Ecc::L).unwrap();
        assert_eq!((q.version(), q.size()), (1, 21));
        assert!(Qr::encode(&vec![b'x'; max_bytes(Ecc::L) + 1], Ecc::L).is_none());
        assert!(Qr::encode(&vec![b'x'; max_bytes(Ecc::L)], Ecc::L).is_some());
        assert!(Qr::encode(&vec![b'x'; max_bytes(Ecc::M)], Ecc::M).is_some());
    }

    #[test]
    fn finder_and_timing_patterns() {
        let q = Qr::encode(b"hello", Ecc::M).unwrap();
        let n = q.size();
        for (ox, oy) in [(0, 0), (n - 7, 0), (0, n - 7)] {
            for d in 0..7 {
                assert!(q.get(ox + d, oy) && q.get(ox + d, oy + 6) && q.get(ox, oy + d) && q.get(ox + 6, oy + d));
            }
            assert!(q.get(ox + 3, oy + 3));
            assert!(!q.get(ox + 1, oy + 1) && !q.get(ox + 5, oy + 5));
        }
        for i in 8..n - 8 {
            assert_eq!(q.get(i, 6), i % 2 == 0);
            assert_eq!(q.get(6, i), i % 2 == 0);
        }
        assert!(q.get(8, n - 8)); // dark module
    }

    #[test]
    fn svg_has_a_path() {
        let s = Qr::encode_best(b"http://airwand.local").unwrap().to_svg(4);
        assert!(s.starts_with("<svg") && s.contains("<path") && s.ends_with("</svg>"));
    }

    #[test]
    fn every_version_builds() {
        for len in [1usize, 14, 26, 42, 62, 84, 106, 122, 152, 180] {
            let d = vec![b'q'; len];
            let q = Qr::encode(&d, Ecc::M).unwrap();
            assert_eq!(q.size(), 17 + 4 * q.version());
        }
    }
}
