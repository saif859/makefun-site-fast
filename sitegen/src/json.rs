//! A JSON reader for cases.json. Strings come out as their UTF-8 bytes; a \u escape is taken only below U+0080
//! (main.ts sends any other input the JavaScript way), and anything this build does not read gives None.

use std::borrow::Cow;

pub enum J<'a> {
    Null,
    #[allow(dead_code)]
    Bool(bool),
    Num(f64),
    Str(Cow<'a, [u8]>),
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

    pub fn str(&self) -> Option<&[u8]> {
        match self {
            J::Str(s) => Some(&s[..]),
            _ => None,
        }
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while let Some(&c) = self.s.get(self.i) {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn lit(&mut self, w: &[u8], v: J<'a>) -> Option<J<'a>> {
        if self.s[self.i..].starts_with(w) {
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
        match self.peek()? {
            b'{' => {
                self.i += 1;
                let mut v: Vec<(&'a [u8], J<'a>)> = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Some(J::Obj(v));
                }
                loop {
                    self.ws();
                    if self.peek()? != b'"' {
                        return None;
                    }
                    // Keys are taken as they are written: one holding an escape is not this build's input.
                    let k = match self.string()? {
                        Cow::Borrowed(k) => k,
                        Cow::Owned(_) => return None,
                    };
                    self.ws();
                    if self.peek()? != b':' {
                        return None;
                    }
                    self.i += 1;
                    let x = self.value(depth + 1)?;
                    match v.iter_mut().find(|e| e.0 == k) {
                        Some(e) => e.1 = x,
                        None => v.push((k, x)),
                    }
                    self.ws();
                    match self.peek()? {
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
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Some(J::Arr(v));
                }
                loop {
                    v.push(self.value(depth + 1)?);
                    self.ws();
                    match self.peek()? {
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

    fn string(&mut self) -> Option<Cow<'a, [u8]>> {
        let s = self.s;
        let start = self.i + 1;
        let mut j = crate::text::next_json(s, start);
        if j < s.len() && s[j] == b'"' {
            self.i = j + 1;
            return Some(Cow::Borrowed(&s[start..j]));
        }
        let mut out = Vec::with_capacity(j - start + 64);
        out.extend_from_slice(&s[start..j.min(s.len())]);
        loop {
            // A run up to the next quote, backslash or control character, copied whole.
            let from = j;
            j = crate::text::next_json(s, j);
            out.extend_from_slice(&s[from..j]);
            let c = *s.get(j)?;
            if c == b'"' {
                self.i = j + 1;
                return Some(Cow::Owned(out));
            }
            if c < 0x20 {
                return None;
            }
            let e = *s.get(j + 1)?;
            j += 2;
            match e {
                b'"' => out.push(b'"'),
                b'\\' => out.push(b'\\'),
                b'/' => out.push(b'/'),
                b'b' => out.push(8),
                b'f' => out.push(12),
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'u' => {
                    let h = s.get(j..j + 4)?;
                    if !h.iter().all(|c| c.is_ascii_hexdigit()) {
                        return None;
                    }
                    let v = u32::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
                    if v >= 0x80 {
                        return None;
                    }
                    out.push(v as u8);
                    j += 4;
                }
                _ => return None,
            }
        }
    }

    fn number(&mut self) -> Option<J<'a>> {
        let s = self.s;
        let start = self.i;
        let mut j = start;
        while j < s.len() && (s[j].is_ascii_digit() || matches!(s[j], b'.' | b'e' | b'E' | b'+' | b'-')) {
            j += 1;
        }
        if j == start {
            return None;
        }
        let v: f64 = std::str::from_utf8(&s[start..j]).ok()?.parse().ok()?;
        self.i = j;
        Some(J::Num(v))
    }
}

/// The input of every case in cases.json (a JSON array of {id, input}), or None.
pub fn parse_cases(src: &[u8]) -> Option<Vec<J<'_>>> {
    let mut p = P { s: src, i: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.i != src.len() {
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
