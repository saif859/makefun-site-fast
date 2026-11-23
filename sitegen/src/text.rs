//! Text rules on UTF-8 bytes: HTML and JSON escaping, JavaScript's \s, trims, word counts, code-unit order and slugs.

use std::cmp::Ordering;


const fn escapable() -> [bool; 128] {
    let mut t = [false; 128];
    let s = b"\\`*_[]()#+-.!>";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
}
const ESCAPABLE: [bool; 128] = escapable();

/// Whether a backslash before c escapes it (makefun-site's table).
#[inline]
pub fn is_escapable(c: u8) -> bool {
    c < 128 && ESCAPABLE[c as usize]
}

pub fn find_byte(s: &[u8], b: u8) -> Option<usize> {
    s.iter().position(|&c| c == b)
}

pub fn find_bytes(s: &[u8], pat: &[u8], from: usize) -> Option<usize> {
    if from > s.len() || pat.len() > s.len() - from {
        return None;
    }
    (from..=s.len() - pat.len()).find(|&i| &s[i..i + pat.len()] == pat)
}

#[inline]
fn entity(c: u8) -> Option<&'static [u8]> {
    match c {
        b'&' => Some(b"&amp;"),
        b'<' => Some(b"&lt;"),
        b'>' => Some(b"&gt;"),
        b'"' => Some(b"&quot;"),
        b'\'' => Some(b"&#39;"),
        _ => None,
    }
}

const fn class_table() -> [u8; 256] {
    // Bit 1: HTML escaping changes the byte. Bit 2: JSON escaping does. Bit 4: inline Markdown acts on it.
    let mut t = [0u8; 256];
    let mut c = 0;
    while c < 0x20 {
        t[c] |= 2;
        c += 1;
    }
    t[b'"' as usize] |= 2;
    t[b'\\' as usize] |= 2;
    let html = b"&<>\"'";
    let mut i = 0;
    while i < html.len() {
        t[html[i] as usize] |= 1 | 4;
        i += 1;
    }
    let md = b"\\`*[";
    let mut i = 0;
    while i < md.len() {
        t[md[i] as usize] |= 4;
        i += 1;
    }
    t
}
pub const CLASS: [u8; 256] = class_table();

/// s with & < > " and ' escaped, appended to out.
pub fn esc_into(out: &mut Vec<u8>, s: &[u8]) {
    let mut run = 0;
    for (i, &c) in s.iter().enumerate() {
        if CLASS[c as usize] & 1 == 0 {
            continue;
        }
        out.extend_from_slice(&s[run..i]);
        out.extend_from_slice(entity(c).unwrap());
        run = i + 1;
    }
    out.extend_from_slice(&s[run..]);
}

#[inline]
fn json_escape(out: &mut Vec<u8>, c: u8) {
    match c {
        b'"' => out.extend_from_slice(b"\\\""),
        b'\\' => out.extend_from_slice(b"\\\\"),
        b'\n' => out.extend_from_slice(b"\\n"),
        b'\r' => out.extend_from_slice(b"\\r"),
        b'\t' => out.extend_from_slice(b"\\t"),
        8 => out.extend_from_slice(b"\\b"),
        12 => out.extend_from_slice(b"\\f"),
        _ => {
            const H: &[u8; 16] = b"0123456789abcdef";
            out.extend_from_slice(b"\\u00");
            out.push(H[(c >> 4) as usize]);
            out.push(H[(c & 15) as usize]);
        }
    }
}

/// s as the inside of a JSON string, as JSON.stringify writes it, appended to out.
pub fn json_into(out: &mut Vec<u8>, s: &[u8]) {
    out.reserve(s.len());
    let mut run = 0;
    for (i, &c) in s.iter().enumerate() {
        if CLASS[c as usize] & 2 == 0 {
            continue;
        }
        out.extend_from_slice(&s[run..i]);
        json_escape(out, c);
        run = i + 1;
    }
    out.extend_from_slice(&s[run..]);
}

/// s HTML-escaped, then as the inside of a JSON string, appended to out.
pub fn html_json_into(out: &mut Vec<u8>, s: &[u8]) {
    out.reserve(s.len());
    let mut run = 0;
    for (i, &c) in s.iter().enumerate() {
        let k = CLASS[c as usize];
        if k & 3 == 0 {
            continue;
        }
        out.extend_from_slice(&s[run..i]);
        if k & 1 != 0 {
            out.extend_from_slice(entity(c).unwrap());
        } else {
            json_escape(out, c);
        }
        run = i + 1;
    }
    out.extend_from_slice(&s[run..]);
}

/// A fast hash for the build's small maps and sets (FxHash: one multiply per 8 bytes).
#[derive(Default, Clone, Copy)]
pub struct Fx(u64);

impl std::hash::Hasher for Fx {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        let mut h = self.0;
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            h = (h.rotate_left(5) ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(K);
        }
        for &b in chunks.remainder() {
            h = (h.rotate_left(5) ^ b as u64).wrapping_mul(K);
        }
        self.0 = h;
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.0 = (self.0.rotate_left(5) ^ n as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

pub type FxBuild = std::hash::BuildHasherDefault<Fx>;
pub type Map<K, V> = std::collections::HashMap<K, V, FxBuild>;
pub type Set<K> = std::collections::HashSet<K, FxBuild>;

/// The byte length of the JavaScript \s character whose UTF-8 starts at s[i], or 0.
#[inline]
pub fn ws_at(s: &[u8], i: usize) -> usize {
    let c = s[i];
    if c == b' ' || (9..=13).contains(&c) {
        return 1;
    }
    if c < 0xc2 {
        return 0;
    }
    let g = |k: usize| -> u8 { *s.get(i + k).unwrap_or(&0) };
    match c {
        0xc2 => (g(1) == 0xa0) as usize * 2,
        0xe1 => (g(1) == 0x9a && g(2) == 0x80) as usize * 3,
        0xe2 => {
            let (d, e) = (g(1), g(2));
            ((d == 0x80 && ((0x80..=0x8a).contains(&e) || e == 0xa8 || e == 0xa9 || e == 0xaf)) || (d == 0x81 && e == 0x9f)) as usize * 3
        }
        0xe3 => (g(1) == 0x80 && g(2) == 0x80) as usize * 3,
        0xef => (g(1) == 0xbb && g(2) == 0xbf) as usize * 3,
        _ => 0,
    }
}

pub fn has_space(s: &[u8]) -> bool {
    (0..s.len()).any(|i| ws_at(s, i) != 0)
}

/// String.prototype.trim.
pub fn trim_u(s: &[u8]) -> &[u8] {
    let mut a = 0;
    let mut z = s.len();
    while a < z {
        let w = ws_at(s, a);
        if w == 0 {
            break;
        }
        a += w;
    }
    while z > a {
        let c = s[z - 1];
        if c == b' ' || (9..=13).contains(&c) {
            z -= 1;
        } else if z >= a + 2 && ws_at(s, z - 2) == 2 {
            z -= 2;
        } else if z >= a + 3 && ws_at(s, z - 3) == 3 {
            z -= 3;
        } else {
            break;
        }
    }
    &s[a..z]
}

/// JavaScript's trim on ASCII text.
pub fn trim_ascii_js(s: &[u8]) -> &[u8] {
    let w = |c: u8| c == b' ' || (9..=13).contains(&c);
    let mut a = 0;
    let mut z = s.len();
    while a < z && w(s[a]) {
        a += 1;
    }
    while z > a && w(s[z - 1]) {
        z -= 1;
    }
    &s[a..z]
}

#[inline]
pub fn is_st(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

pub fn trim_st(s: &[u8]) -> &[u8] {
    let mut a = 0;
    let mut z = s.len();
    while a < z && is_st(s[a]) {
        a += 1;
    }
    while z > a && is_st(s[z - 1]) {
        z -= 1;
    }
    &s[a..z]
}

pub fn trim_end_st(s: &[u8]) -> &[u8] {
    let mut z = s.len();
    while z > 0 && is_st(s[z - 1]) {
        z -= 1;
    }
    &s[..z]
}

pub fn all_st(s: &[u8]) -> bool {
    s.iter().all(|&c| is_st(c))
}

/// The count of /\S+/g matches.
pub fn words(s: &[u8]) -> usize {
    let mut n = 0;
    let mut in_word = false;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c > 32 && c < 0xc2 {
            if !in_word {
                in_word = true;
                n += 1;
            }
            i += 1;
            continue;
        }
        let w = ws_at(s, i);
        if w > 0 {
            in_word = false;
            i += w;
        } else {
            if !in_word {
                in_word = true;
                n += 1;
            }
            i += 1;
        }
    }
    n
}

/// UTF-16 code-unit order of two UTF-8 strings: byte order, except U+E000-U+FFFF after U+10000 up.
pub fn cmp_units(a: &[u8], b: &[u8]) -> Ordering {
    let n = a.len().min(b.len());
    let mut i = 0;
    while i < n && a[i] == b[i] {
        i += 1;
    }
    if i == n {
        return a.len().cmp(&b.len());
    }
    let (x, y) = (a[i], b[i]);
    let hi = |c: u8| c == 0xee || c == 0xef;
    if hi(x) && y >= 0xf0 {
        return Ordering::Greater;
    }
    if hi(y) && x >= 0xf0 {
        return Ordering::Less;
    }
    x.cmp(&y)
}

/// makefun-site's slug of ASCII text, or None for text with a non-ASCII character (its NFKD is JavaScript's).
pub fn slug_ascii(t: &[u8]) -> Option<Vec<u8>> {
    if !t.is_ascii() {
        return None;
    }
    let mut out = Vec::with_capacity(t.len().min(64));
    let mut gap = false;
    for &c0 in t {
        let c = c0.to_ascii_lowercase();
        if !(c.is_ascii_lowercase() || c.is_ascii_digit()) {
            gap = !out.is_empty();
            continue;
        }
        if gap {
            out.push(b'-');
        }
        gap = false;
        out.push(c);
    }
    if out.len() > 64 {
        out.truncate(64);
        if out[63] == b'-' {
            out.truncate(63);
        }
    }
    Some(if out.is_empty() { b"untitled".to_vec() } else { out })
}

pub fn unique_slug(slug: &[u8], taken: &mut Set<Vec<u8>>) -> Vec<u8> {
    let mut out = slug.to_vec();
    let mut n = 2;
    while taken.contains(&out) {
        out = [slug, b"-", n.to_string().as_bytes()].concat();
        n += 1;
    }
    taken.insert(out.clone());
    out
}

const MONTHS: [&[u8]; 12] = [
    b"January", b"February", b"March", b"April", b"May", b"June", b"July", b"August", b"September", b"October", b"November",
    b"December",
];

/// "D Month YYYY" of a checked YYYY-MM-DD date.
pub fn long_date(d: &[u8]) -> Vec<u8> {
    let num = |s: &[u8]| s.iter().fold(0usize, |a, &c| a * 10 + (c - b'0') as usize);
    let day = num(&d[8..10]);
    let month = num(&d[5..7]);
    [day.to_string().as_bytes(), b" ", MONTHS[month - 1], b" ", &d[0..4]].concat()
}
