//! A JSON reader for cases.json, over a buffer it may write: every string is unescaped in place (its text is never
//! longer than its JSON) and comes out as a slice of the buffer, so nothing of the input is copied. A \u escape is
//! taken only below U+0080 (main.js sends any other input the JavaScript way), and anything this build does not read
//! gives None.

use crate::text::next_json;
use std::marker::PhantomData;

pub enum J<'a> {
    Null,
    #[allow(dead_code)]
    Bool(bool),
    Num(f64),
    Str(&'a [u8]),
    Arr(Vec<J<'a>>),
    Obj(Vec<(&'a [u8], J<'a>)>),
}

impl<'a> J<'a> {
    /// An object's member (the last one given, as JSON.parse keeps it).
    pub fn get(&self, k: &[u8]) -> Option<&J<'a>> {
        match self {
            J::Obj(v) => v.iter().find(|e| e.0 == k).map(|e| &e.1),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&'a [u8]> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// The reader. Bytes are read at `i` and up and a string's text is written back at or below where it was read, so
/// what has been handed out (behind) and what is still to read (ahead) never meet.
struct P<'a> {
    s: *mut u8,
    n: usize,
    i: usize,
    _buf: PhantomData<&'a mut [u8]>,
}

impl<'a> P<'a> {
    #[inline]
    fn at(&self, k: usize) -> Option<u8> {
        if k < self.n {
            Some(unsafe { *self.s.add(k) })
        } else {
            None
        }
    }

    /// The bytes from `from` on (not yet read, never written).
    #[inline]
    fn ahead(&self, from: usize) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.s.add(from), self.n - from) }
    }

    fn ws(&mut self) {
        while let Some(c) = self.at(self.i) {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn lit(&mut self, w: &[u8], v: J<'a>) -> Option<J<'a>> {
        if self.ahead(self.i).starts_with(w) {
            self.i += w.len();
            Some(v)
        } else {
            None
        }
    }

    fn value(&mut self, depth: u32) -> Option<J<'a>> {
        if depth > 512 {
            return None;
        }
        self.ws();
        match self.at(self.i)? {
            b'{' => {
                self.i += 1;
                let mut v: Vec<(&'a [u8], J<'a>)> = Vec::new();
                self.ws();
                if self.at(self.i) == Some(b'}') {
                    self.i += 1;
                    return Some(J::Obj(v));
                }
                loop {
                    self.ws();
                    if self.at(self.i)? != b'"' {
                        return None;
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.at(self.i)? != b':' {
                        return None;
                    }
                    self.i += 1;
                    let x = self.value(depth + 1)?;
                    match v.iter_mut().find(|e| e.0 == k) {
                        Some(e) => e.1 = x,
                        None => v.push((k, x)),
                    }
                    self.ws();
                    match self.at(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(J::Obj(v));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.at(self.i) == Some(b']') {
                    self.i += 1;
                    return Some(J::Arr(v));
                }
                loop {
                    v.push(self.value(depth + 1)?);
                    self.ws();
                    match self.at(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(J::Arr(v));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => Some(J::Str(self.string()?)),
            b't' => self.lit(b"true", J::Bool(true)),
            b'f' => self.lit(b"false", J::Bool(false)),
            b'n' => self.lit(b"null", J::Null),
            _ => self.number(),
        }
    }

    /// The string at i (its opening quote), unescaped in place: the slice of the buffer it now fills.
    fn string(&mut self) -> Option<&'a [u8]> {
        let start = self.i + 1;
        if start > self.n {
            return None;
        }
        let mut j = start + next_json(self.ahead(start), 0);
        let mut w = j;
        loop {
            let c = self.at(j)?;
            if c == b'"' {
                self.i = j + 1;
                return Some(unsafe { std::slice::from_raw_parts(self.s.add(start), w - start) });
            }
            if c < 0x20 {
                return None;
            }
            let e = self.at(j + 1)?;
            let b = match e {
                b'"' => b'"',
                b'\\' => b'\\',
                b'/' => b'/',
                b'b' => 8,
                b'f' => 12,
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'u' => {
                    let h = self.ahead(j + 2).get(..4)?;
                    if !h.iter().all(|c| c.is_ascii_hexdigit()) {
                        return None;
                    }
                    let v = u32::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
                    if v >= 0x80 {
                        return None;
                    }
                    j += 4;
                    v as u8
                }
                _ => return None,
            };
            j += 2;
            unsafe { *self.s.add(w) = b };
            w += 1;
            // The run up to the next quote, backslash or control character, moved down whole.
            let k = next_json(self.ahead(j), 0);
            unsafe { move_down(self.s, w, j, k) };
            w += k;
            j += k;
        }
    }

    fn number(&mut self) -> Option<J<'a>> {
        let s = self.ahead(self.i);
        let mut j = 0;
        while j < s.len() && (s[j].is_ascii_digit() || matches!(s[j], b'.' | b'e' | b'E' | b'+' | b'-')) {
            j += 1;
        }
        if j == 0 {
            return None;
        }
        let v: f64 = std::str::from_utf8(&s[..j]).ok()?.parse().ok()?;
        self.i += j;
        Some(J::Num(v))
    }
}

/// The k bytes at s+j moved down to s+w (w <= j, so they may overlap): up to 32 with every load made before any
/// store (two overlapping 16-, 8-, 4-byte or single moves), more by memmove.
#[inline(always)]
unsafe fn move_down(s: *mut u8, w: usize, j: usize, k: usize) {
    if w == j || k == 0 {
        return;
    }
    let (d, p) = (s.add(w), s.add(j) as *const u8);
    if k >= 16 && k <= 32 {
        let a = (p as *const [u8; 16]).read_unaligned();
        let b = (p.add(k - 16) as *const [u8; 16]).read_unaligned();
        (d as *mut [u8; 16]).write_unaligned(a);
        (d.add(k - 16) as *mut [u8; 16]).write_unaligned(b);
    } else if k >= 8 && k < 16 {
        let a = (p as *const u64).read_unaligned();
        let b = (p.add(k - 8) as *const u64).read_unaligned();
        (d as *mut u64).write_unaligned(a);
        (d.add(k - 8) as *mut u64).write_unaligned(b);
    } else if k >= 4 && k < 8 {
        let a = (p as *const u32).read_unaligned();
        let b = (p.add(k - 4) as *const u32).read_unaligned();
        (d as *mut u32).write_unaligned(a);
        (d.add(k - 4) as *mut u32).write_unaligned(b);
    } else if k < 4 {
        let (a, b, c) = (*p, *p.add(k / 2), *p.add(k - 1));
        *d = a;
        *d.add(k / 2) = b;
        *d.add(k - 1) = c;
    } else {
        std::ptr::copy(p, d, k);
    }
}

/// The input of every case in cases.json (a JSON array of {id, input}), or None. The buffer is unescaped in place.
pub fn parse_cases(src: &mut [u8]) -> Option<Vec<J<'_>>> {
    let n = src.len();
    let mut p = P { s: src.as_mut_ptr(), n, i: 0, _buf: PhantomData };
    let v = p.value(0)?;
    p.ws();
    if p.i != n {
        return None;
    }
    let J::Arr(items) = v else { return None };
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let J::Obj(fields) = it else { return None };
        let mut input = None;
        for (k, v) in fields {
            if k == b"input" {
                input = Some(v);
            }
        }
        out.push(input?);
    }
    Some(out)
}
