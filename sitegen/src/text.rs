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

/// The first b in s: eight bytes at a time (the lowest flagged byte of a word is always a true one).
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn find_byte(s: &[u8], b: u8) -> Option<usize> {
    let mut i = 0;
    while i + 8 <= s.len() {
        let m = bytes_eq(u64::from_le_bytes(s[i..i + 8].try_into().unwrap()), b);
        if m != 0 {
            return Some(i + (m.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    while i < s.len() {
        if s[i] == b {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub fn find_bytes(s: &[u8], pat: &[u8], from: usize) -> Option<usize> {
    if from > s.len() || pat.len() > s.len() - from {
        return None;
    }
    (from..=s.len() - pat.len()).find(|&i| &s[i..i + pat.len()] == pat)
}

#[inline]
pub fn entity(c: u8) -> Option<&'static [u8]> {
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
    // Inline Markdown also stops at control characters (its output is the inside of a JSON string).
    let mut c = 0;
    while c < 0x20 {
        t[c] |= 4;
        c += 1;
    }
    let mut i = 0;
    while i < md.len() {
        t[md[i] as usize] |= 4;
        i += 1;
    }
    t
}
pub const CLASS: [u8; 256] = class_table();

/// Appending bytes: runs of up to 32 bytes (almost all of them: text between escapes, entities, markup) are copied
/// inline with two overlapping loads and stores, up to 256 in 16-byte steps, longer ones by memcpy (musl's costs
/// more to call than a short copy takes).
pub trait Put {
    fn put(&mut self, s: &[u8]);
}

impl Put for Vec<u8> {
    #[inline(always)]
    fn put(&mut self, s: &[u8]) {
        let n = s.len();
        if n > 32 {
            if n > 256 {
                self.extend_from_slice(s);
                return;
            }
            self.reserve(n);
            unsafe {
                let d = self.as_mut_ptr().add(self.len());
                let p = s.as_ptr();
                let mut k = 0;
                while k + 16 < n {
                    (d.add(k) as *mut [u8; 16]).write_unaligned((p.add(k) as *const [u8; 16]).read_unaligned());
                    k += 16;
                }
                (d.add(n - 16) as *mut [u8; 16]).write_unaligned((p.add(n - 16) as *const [u8; 16]).read_unaligned());
                self.set_len(self.len() + n);
            }
            return;
        }
        self.reserve(n);
        unsafe {
            let d = self.as_mut_ptr().add(self.len());
            let p = s.as_ptr();
            if n >= 16 {
                let a = (p as *const [u8; 16]).read_unaligned();
                let b = (p.add(n - 16) as *const [u8; 16]).read_unaligned();
                (d as *mut [u8; 16]).write_unaligned(a);
                (d.add(n - 16) as *mut [u8; 16]).write_unaligned(b);
            } else if n >= 8 {
                let a = (p as *const u64).read_unaligned();
                let b = (p.add(n - 8) as *const u64).read_unaligned();
                (d as *mut u64).write_unaligned(a);
                (d.add(n - 8) as *mut u64).write_unaligned(b);
            } else if n >= 4 {
                let a = (p as *const u32).read_unaligned();
                let b = (p.add(n - 4) as *const u32).read_unaligned();
                (d as *mut u32).write_unaligned(a);
                (d.add(n - 4) as *mut u32).write_unaligned(b);
            } else if n > 0 {
                *d = *p;
                if n > 1 {
                    *d.add(1) = *p.add(1);
                    if n > 2 {
                        *d.add(2) = *p.add(2);
                    }
                }
            }
            self.set_len(self.len() + n);
        }
    }
}

const LO: u64 = 0x0101_0101_0101_0101;
const HI: u64 = 0x8080_8080_8080_8080;

/// A high bit in every byte of w that is c (and maybe in bytes after one: never none when there is one).
#[inline(always)]
fn bytes_eq(w: u64, c: u8) -> u64 {
    let x = w ^ (LO * c as u64);
    x.wrapping_sub(LO) & !x & HI
}

/// The first index from i of a byte in `set` (or below 0x20, with `ctl`), or s.len(): eight bytes at a time while
/// none is there, then byte by byte on CLASS bit `bit`, which marks the same bytes.
#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
pub fn scan<const N: usize>(s: &[u8], mut i: usize, set: [u8; N], ctl: bool, bit: u8) -> usize {
    while i + 8 <= s.len() {
        let w = u64::from_le_bytes(s[i..i + 8].try_into().unwrap());
        let mut m = if ctl { w.wrapping_sub(LO * 0x20) & !w & HI } else { 0 };
        for c in set {
            m |= bytes_eq(w, c);
        }
        if m != 0 {
            // Every test flags its first match exactly (false flags only come after a true one): the lowest is it.
            return i + (m.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while i < s.len() && CLASS[s[i] as usize] & bit == 0 {
        i += 1;
    }
    i
}

#[inline(always)]
fn next_html(s: &[u8], i: usize) -> usize {
    scan(s, i, [b'&', b'<', b'>', b'"', b'\''], false, 1)
}

#[inline(always)]
pub fn next_json(s: &[u8], i: usize) -> usize {
    scan(s, i, [b'"', b'\\'], true, 2)
}

#[inline(always)]
fn next_html_json(s: &[u8], i: usize) -> usize {
    scan(s, i, [b'&', b'<', b'>', b'"', b'\'', b'\\'], true, 3)
}

/// The next byte inline Markdown acts on or escaping changes (& < > " ' \ ` * [ and control characters).
#[inline(always)]
pub fn next_inline(s: &[u8], i: usize) -> usize {
    scan(s, i, [b'&', b'<', b'>', b'"', b'\'', b'\\', b'`', b'*', b'['], true, 4)
}

/// s with & < > " and ' escaped, appended to out.
pub fn esc_into(out: &mut Vec<u8>, s: &[u8]) {
    let mut run = 0;
    let mut i = next_html(s, 0);
    while i < s.len() {
        out.put(&s[run..i]);
        out.put(entity(s[i]).unwrap());
        run = i + 1;
        i = next_html(s, i + 1);
    }
    out.put(&s[run..]);
}

#[inline]
pub fn json_escape(out: &mut Vec<u8>, c: u8) {
    match c {
        b'"' => out.put(b"\\\""),
        b'\\' => out.put(b"\\\\"),
        b'\n' => out.put(b"\\n"),
        b'\r' => out.put(b"\\r"),
        b'\t' => out.put(b"\\t"),
        8 => out.put(b"\\b"),
        12 => out.put(b"\\f"),
        _ => {
            const H: &[u8; 16] = b"0123456789abcdef";
            out.put(b"\\u00");
            out.push(H[(c >> 4) as usize]);
            out.push(H[(c & 15) as usize]);
        }
    }
}

/// s as the inside of a JSON string, as JSON.stringify writes it, appended to out.
pub fn json_into(out: &mut Vec<u8>, s: &[u8]) {
    out.reserve(s.len());
    let mut run = 0;
    let mut i = next_json(s, 0);
    while i < s.len() {
        out.put(&s[run..i]);
        json_escape(out, s[i]);
        run = i + 1;
        i = next_json(s, i + 1);
    }
    out.put(&s[run..]);
}

/// s HTML-escaped, then as the inside of a JSON string, appended to out.
pub fn html_json_into(out: &mut Vec<u8>, s: &[u8]) {
    out.reserve(s.len());
    let mut run = 0;
    let mut i = next_html_json(s, 0);
    while i < s.len() {
        let c = s[i];
        out.put(&s[run..i]);
        if CLASS[c as usize] & 1 != 0 {
            out.put(entity(c).unwrap());
        } else {
            json_escape(out, c);
        }
        run = i + 1;
        i = next_html_json(s, i + 1);
    }
    out.put(&s[run..]);
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

/// A high bit in exactly the bytes of w that are c.
#[inline(always)]
fn eq_exact(w: u64, c: u8) -> u64 {
    let x = w ^ (LO * c as u64);
    !(((x & !HI).wrapping_add(!HI)) | x | !HI)
}

/// A high bit in exactly the bytes of w below c (c <= 0x80).
#[inline(always)]
fn lt_exact(w: u64, c: u8) -> u64 {
    !(((w & !HI).wrapping_add(LO * (0x80 - c as u64))) | w) & HI
}

/// The count of /\S+/g matches: eight bytes at a time (ASCII whitespace by mask, word starts by popcount) where no
/// byte can start a non-ASCII \s character, byte by byte elsewhere.
#[cfg(not(target_arch = "x86_64"))]
pub fn words(s: &[u8]) -> usize {
    let mut n = 0;
    let mut in_word = false;
    let mut i = 0;
    while i < s.len() {
        let mut end = s.len();
        if i + 8 <= s.len() {
            let w = u64::from_le_bytes(s[i..i + 8].try_into().unwrap());
            let lead = bytes_eq(w, 0xc2) | bytes_eq(w, 0xe1) | bytes_eq(w, 0xe2) | bytes_eq(w, 0xe3) | bytes_eq(w, 0xef);
            if lead == 0 {
                let space = eq_exact(w, b' ') | (lt_exact(w, 14) & !lt_exact(w, 9));
                let prev_space = (space << 8) | if in_word { 0 } else { 0x80 };
                n += (!space & HI & prev_space).count_ones() as usize;
                in_word = space >> 63 == 0;
                i += 8;
                continue;
            }
            end = i + 8;
        }
        while i < end {
            let c = s[i];
            if c == b' ' || (9..=13).contains(&c) {
                in_word = false;
                i += 1;
                continue;
            }
            let ws = if c >= 0xc2 { ws_at(s, i) } else { 0 };
            if ws > 0 {
                in_word = false;
                i += ws;
            } else {
                if !in_word {
                    in_word = true;
                    n += 1;
                }
                i += 1;
            }
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

// ---------------------------------------------------------------- x86-64: the same scans sixteen bytes at a time
//
// SSE2 is part of x86-64, so these need no feature test: a compare per byte of a set, the lanes' high bits gathered
// into a mask, and its lowest set bit is the first match.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// The first b in s.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn find_byte(s: &[u8], b: u8) -> Option<usize> {
    let mut i = 0;
    unsafe {
        let p = s.as_ptr();
        let needle = _mm_set1_epi8(b as i8);
        while i + 16 <= s.len() {
            let m = _mm_movemask_epi8(_mm_cmpeq_epi8(_mm_loadu_si128(p.add(i) as *const __m128i), needle)) as u32;
            if m != 0 {
                return Some(i + m.trailing_zeros() as usize);
            }
            i += 16;
        }
    }
    while i < s.len() {
        if s[i] == b {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The first index from i of a byte in `set` (or below 0x20, with `ctl`), or s.len(); CLASS bit `bit` marks the
/// same bytes for the tail.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn scan<const N: usize>(s: &[u8], mut i: usize, set: [u8; N], ctl: bool, bit: u8) -> usize {
    unsafe {
        let p = s.as_ptr();
        let low = _mm_set1_epi8(0x1f);
        while i + 16 <= s.len() {
            let v = _mm_loadu_si128(p.add(i) as *const __m128i);
            let mut m = if ctl { _mm_cmpeq_epi8(_mm_min_epu8(v, low), v) } else { _mm_setzero_si128() };
            for c in set {
                m = _mm_or_si128(m, _mm_cmpeq_epi8(v, _mm_set1_epi8(c as i8)));
            }
            let mask = _mm_movemask_epi8(m) as u32;
            if mask != 0 {
                return i + mask.trailing_zeros() as usize;
            }
            i += 16;
        }
    }
    while i < s.len() && CLASS[s[i] as usize] & bit == 0 {
        i += 1;
    }
    i
}

/// The count of /\S+/g matches: sixteen bytes at a time (ASCII whitespace by mask, word starts by popcount) where no
/// byte can start a non-ASCII \s character, byte by byte elsewhere.
#[cfg(target_arch = "x86_64")]
pub fn words(s: &[u8]) -> usize {
    let mut n = 0;
    let mut in_word = false;
    let mut i = 0;
    unsafe {
        let p = s.as_ptr();
        let (space, nine, four) = (_mm_set1_epi8(b' ' as i8), _mm_set1_epi8(9), _mm_set1_epi8(4));
        let leads = [0xc2u8, 0xe1, 0xe2, 0xe3, 0xef].map(|c| _mm_set1_epi8(c as i8));
        while i < s.len() {
            let mut end = s.len();
            if i + 16 <= s.len() {
                let v = _mm_loadu_si128(p.add(i) as *const __m128i);
                let mut lead = 0;
                if _mm_movemask_epi8(v) != 0 {
                    // A byte that starts a non-ASCII \s character together with the byte after it (C2 A0, E1 9A,
                    // E2 80, E2 81, E3 80, EF BB), so that other characters with those leads (the fi ligature, EF AC
                    // 81, is common) stay on this path. Near the end, where the next block cannot be read, any lead.
                    if i + 17 <= s.len() {
                        let next = _mm_loadu_si128(p.add(i + 1) as *const __m128i);
                        let pair = |a: __m128i, b: u8| _mm_and_si128(_mm_cmpeq_epi8(v, a), _mm_cmpeq_epi8(next, _mm_set1_epi8(b as i8)));
                        let mut m = _mm_or_si128(pair(leads[0], 0xa0), pair(leads[1], 0x9a));
                        m = _mm_or_si128(m, _mm_or_si128(pair(leads[2], 0x80), pair(leads[2], 0x81)));
                        m = _mm_or_si128(m, _mm_or_si128(pair(leads[3], 0x80), pair(leads[4], 0xbb)));
                        lead = _mm_movemask_epi8(m);
                    } else {
                        let mut m = _mm_setzero_si128();
                        for l in leads {
                            m = _mm_or_si128(m, _mm_cmpeq_epi8(v, l));
                        }
                        lead = _mm_movemask_epi8(m);
                    }
                }
                if lead == 0 {
                    // Whitespace: a space, or 9-13 (v - 9 at most 4, unsigned).
                    let t = _mm_sub_epi8(v, nine);
                    let ws = _mm_or_si128(_mm_cmpeq_epi8(v, space), _mm_cmpeq_epi8(_mm_min_epu8(t, four), t));
                    let sp = _mm_movemask_epi8(ws) as u32;
                    let prev = (sp << 1) | if in_word { 0 } else { 1 };
                    n += (!sp & 0xffff & prev).count_ones() as usize;
                    in_word = sp & 0x8000 == 0;
                    i += 16;
                    continue;
                }
                end = i + 16;
            }
            while i < end {
                let c = s[i];
                if c == b' ' || (9..=13).contains(&c) {
                    in_word = false;
                    i += 1;
                    continue;
                }
                let w = if c >= 0xc2 { ws_at(s, i) } else { 0 };
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
        }
    }
    n
}

/// The text whose JSON-string inside is s (json_into undone).
pub fn json_unescape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c != b'\\' || i + 1 >= s.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let e = s[i + 1];
        i += 2;
        out.push(match e {
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'b' => 8,
            b'f' => 12,
            b'u' => {
                let v = s.get(i..i + 4).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(&h[2..], 16).ok()).unwrap_or(0);
                i += 4;
                v
            }
            other => other,
        });
    }
    out
}
