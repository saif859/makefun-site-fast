//! makefun-site's Markdown subset: blocks by line, then inline Markdown with every piece of text HTML-escaped.

use crate::text::*;


pub struct Md {
    pub html: Vec<u8>,
    /// (level, id, html) of every h2 and h3.
    pub toc: Vec<(u8, Vec<u8>, Vec<u8>)>,
    pub excerpt: Vec<u8>,
}

pub type Link<'a> = dyn FnMut(&[u8]) -> Vec<u8> + 'a;
pub type Slug<'a> = dyn FnMut(&[u8]) -> Vec<u8> + 'a;

/// /^```([A-Za-z0-9_+-]*)[ \t]*$/: the language.
fn fence_open(l: &[u8]) -> Option<&[u8]> {
    if !l.starts_with(b"```") {
        return None;
    }
    let mut j = 3;
    while j < l.len() && (l[j].is_ascii_alphanumeric() || matches!(l[j], b'_' | b'+' | b'-')) {
        j += 1;
    }
    if all_st(&l[j..]) {
        Some(&l[3..j])
    } else {
        None
    }
}

/// /^```[ \t]*$/
fn fence_close(l: &[u8]) -> bool {
    l.starts_with(b"```") && all_st(&l[3..])
}

/// /^(#{1,6})[ \t]+(.*?)[ \t]*$/ with a non-empty text: (depth, text).
fn heading(l: &[u8]) -> Option<(usize, &[u8])> {
    let n = l.iter().take_while(|&&c| c == b'#').count();
    if n == 0 || n > 6 || n >= l.len() || !is_st(l[n]) {
        return None;
    }
    let text = trim_st(&l[n..]);
    if text.is_empty() {
        None
    } else {
        Some((n, text))
    }
}

/// /^(?:---|\*\*\*)[ \t]*$/
fn hr(l: &[u8]) -> bool {
    (l.starts_with(b"---") || l.starts_with(b"***")) && all_st(&l[3..])
}

/// /^[-*] (.*)$/: the rest.
fn bullet(l: &[u8]) -> Option<&[u8]> {
    if l.len() >= 2 && (l[0] == b'-' || l[0] == b'*') && l[1] == b' ' {
        Some(&l[2..])
    } else {
        None
    }
}

/// /^(\d{1,9})\. (.*)$/: (digits, rest).
fn ordered(l: &[u8]) -> Option<(&[u8], &[u8])> {
    let d = l.iter().take_while(|c| c.is_ascii_digit()).count();
    if d == 0 || d > 9 || l.len() < d + 2 || l[d] != b'.' || l[d + 1] != b' ' {
        return None;
    }
    Some((&l[..d], &l[d + 2..]))
}

/// /^ {2,}\S/
fn continued(l: &[u8]) -> bool {
    let k = l.iter().take_while(|&&c| c == b' ').count();
    k >= 2 && k < l.len() && ws_at(l, k) == 0
}

fn blank(l: &[u8]) -> bool {
    all_st(l)
}

/// Whether a line ends a paragraph: blank, a fence, a heading or a thematic break.
fn ends(l: &[u8]) -> bool {
    if l.is_empty() {
        return true;
    }
    match l[0] {
        b' ' | b'\t' => blank(l),
        b'`' => fence_open(l).is_some(),
        b'#' => heading(l).is_some(),
        b'-' | b'*' => hr(l),
        _ => false,
    }
}

/// First ']' from `from` outside a backslash escape.
fn find_close(s: &[u8], from: usize) -> Option<usize> {
    let n = s.len();
    let mut j = from;
    while j < n {
        let c = s[j];
        if c == b'\\' && j + 1 < n && is_escapable(s[j + 1]) {
            j += 2;
            continue;
        }
        if c == b']' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// First '**' from `from` outside a backslash escape.
fn find_double(s: &[u8], from: usize) -> Option<usize> {
    let n = s.len();
    let mut j = from;
    while j + 2 <= n {
        let c = s[j];
        if c == b'\\' {
            if is_escapable(s[j + 1]) {
                j += 1;
            }
            j += 1;
            continue;
        }
        if c == b'*' && s[j + 1] == b'*' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// First single '*' from `from` (its run not '**'), outside an escape.
fn find_single_star(s: &[u8], from: usize) -> Option<usize> {
    let n = s.len();
    let mut j = from;
    while j < n {
        let c = s[j];
        if c == b'\\' {
            if j + 1 < n && is_escapable(s[j + 1]) {
                j += 1;
            }
            j += 1;
            continue;
        }
        if c != b'*' {
            j += 1;
            continue;
        }
        if j + 1 < n && s[j + 1] == b'*' {
            j += 2;
            continue;
        }
        return Some(j);
    }
    None
}

/// Text from i up to the next character inline Markdown acts on (\ ` * [), appended to out HTML- and JSON-escaped:
/// that character's index, or t.len().
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn esc_run(t: &[u8], mut i: usize, out: &mut Vec<u8>) -> usize {
    use std::arch::x86_64::*;
    let n = t.len();
    let mut run = i;
    unsafe {
        let p = t.as_ptr();
        let low = _mm_set1_epi8(0x1f);
        let set = [b'&', b'<', b'>', b'"', b'\'', b'\\', b'`', b'*', b'['].map(|c| _mm_set1_epi8(c as i8));
        while i + 16 <= n {
            let v = _mm_loadu_si128(p.add(i) as *const __m128i);
            let mut m = _mm_cmpeq_epi8(_mm_min_epu8(v, low), v);
            for s in set {
                m = _mm_or_si128(m, _mm_cmpeq_epi8(v, s));
            }
            let mut mask = _mm_movemask_epi8(m) as u32;
            // Every flagged byte of the block in turn: escapes written here, Markdown's own characters returned.
            while mask != 0 {
                let k = i + mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let c = *p.add(k);
                out.put(&t[run..k]);
                if let Some(e) = entity(c) {
                    out.put(e);
                } else if c < 0x20 {
                    json_escape(out, c);
                } else {
                    return k;
                }
                run = k + 1;
            }
            i += 16;
        }
    }
    while i < n {
        let c = t[i];
        if CLASS[c as usize] & 4 != 0 {
            out.put(&t[run..i]);
            if let Some(e) = entity(c) {
                out.put(e);
            } else if c < 0x20 {
                json_escape(out, c);
            } else {
                return i;
            }
            run = i + 1;
        }
        i += 1;
    }
    out.put(&t[run..n]);
    n
}

/// Text from i up to the next character inline Markdown acts on (\ ` * [), appended to out HTML- and JSON-escaped:
/// that character's index, or t.len().
#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
fn esc_run(t: &[u8], mut i: usize, out: &mut Vec<u8>) -> usize {
    let n = t.len();
    let mut run = i;
    loop {
        i = next_inline(t, i);
        out.put(&t[run..i]);
        if i >= n {
            return n;
        }
        let c = t[i];
        if let Some(e) = entity(c) {
            out.put(e);
        } else if c < 0x20 {
            json_escape(out, c);
        } else {
            return i;
        }
        i += 1;
        run = i;
    }
}

/// makefun-site's inline Markdown of raw text t, its text HTML-escaped, appended to out as the inside of a
/// JSON string. Everything of t before i is written when the loop comes round.
pub fn inline(t: &[u8], out: &mut Vec<u8>, link: &mut Link) {
    let n = t.len();
    let mut i = 0;
    loop {
        i = esc_run(t, i, out);
        if i >= n {
            return;
        }
        match t[i] {
            b'\\' => {
                if i + 1 < n && is_escapable(t[i + 1]) {
                    // The backslash goes; the character it escapes stays as text (">" as its entity, a backslash as
                    // JSON writes one).
                    match t[i + 1] {
                        b'>' => out.put(b"&gt;"),
                        b'\\' => out.put(b"\\\\"),
                        d => out.push(d),
                    }
                    i += 2;
                } else {
                    out.put(b"\\\\");
                    i += 1;
                }
            }
            b'`' => match find_byte(&t[i + 1..], b'`') {
                Some(off) if off > 0 => {
                    let j = i + 1 + off;
                    out.put(b"<code>");
                    html_json_into(out, &t[i + 1..j]);
                    out.put(b"</code>");
                    i = j + 1;
                }
                _ => {
                    out.push(b'`');
                    i += 1;
                }
            },
            b'*' => {
                if i + 1 < n && t[i + 1] == b'*' {
                    match find_double(t, i + 2) {
                        Some(j) if j > i + 2 => {
                            out.put(b"<strong>");
                            inline(&t[i + 2..j], out, link);
                            out.put(b"</strong>");
                            i = j + 2;
                        }
                        _ => {
                            out.put(b"**");
                            i += 2;
                        }
                    }
                } else {
                    match find_single_star(t, i + 1) {
                        Some(j) if j > i + 1 => {
                            out.put(b"<em>");
                            inline(&t[i + 1..j], out, link);
                            out.put(b"</em>");
                            i = j + 1;
                        }
                        _ => {
                            out.push(b'*');
                            i += 1;
                        }
                    }
                }
            }
            _ => {
                // "[": a link when "](target)" follows, the target non-empty with no \s.
                let k = find_close(t, i + 1);
                let m = match k {
                    Some(k) if k + 1 < n && t[k + 1] == b'(' => find_byte(&t[k + 2..], b')').map(|o| k + 2 + o),
                    _ => None,
                };
                if let (Some(k), Some(m)) = (k, m) {
                    if m > k + 2 && !has_space(&t[k + 2..m]) {
                        let href = link(&t[k + 2..m]);
                        out.put(b"<a href=\\\"");
                        html_json_into(out, &href);
                        out.put(b"\\\">");
                        inline(&t[i + 1..k], out, link);
                        out.put(b"</a>");
                        i = m + 1;
                        continue;
                    }
                }
                out.push(b'[');
                i += 1;
            }
        }
    }
}

/// A source body rendered: its HTML, its h2 and h3 entries and its first paragraph's HTML, each as the inside of
/// a JSON string (what the answer holds).
pub fn render(src: &[u8], link: &mut Link, slug: &mut Slug) -> Md {
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut p = 0;
    loop {
        match find_byte(&src[p..], b'\n') {
            Some(o) => {
                lines.push((p, p + o));
                p += o + 1;
            }
            None => {
                lines.push((p, src.len()));
                break;
            }
        }
    }
    let line = |k: usize| &src[lines[k].0..lines[k].1];
    let nl = lines.len();
    let mut ids: Set<Vec<u8>> = Set::default();
    let mut toc = Vec::new();
    let mut excerpt: Option<Vec<u8>> = None;
    let mut html: Vec<u8> = Vec::with_capacity(src.len() + src.len() / 4);
    let mut i = 0;
    while i < nl {
        let ln = line(i);
        if blank(ln) {
            i += 1;
            continue;
        }
        let c0 = ln[0];
        if c0 == b'`' {
            if let Some(lang) = fence_open(ln) {
                i += 1;
                let first = i;
                while i < nl {
                    let l = line(i);
                    if !l.is_empty() && l[0] == b'`' && fence_close(l) {
                        break;
                    }
                    i += 1;
                }
                if lang.is_empty() {
                    html.put(b"<pre><code>");
                } else {
                    html.put(b"<pre><code class=\\\"language-");
                    html_json_into(&mut html, lang);
                    html.put(b"\\\">");
                }
                if i > first {
                    html_json_into(&mut html, &src[lines[first].0..lines[i - 1].1]);
                    html.put(b"\\n");
                }
                i += 1;
                html.put(b"</code></pre>\\n");
                continue;
            }
        }
        if c0 == b'#' {
            if let Some((depth, text)) = heading(ln) {
                let id = unique_slug(&slug(text), &mut ids);
                let d = b'0' + depth as u8;
                html.put(b"<h");
                html.push(d);
                html.put(b" id=\\\"");
                html.put(&id);
                html.put(b"\\\">");
                let at = html.len();
                inline(text, &mut html, link);
                if depth == 2 || depth == 3 {
                    toc.push((depth as u8, id, html[at..].to_vec()));
                }
                html.put(b"</h");
                html.push(d);
                html.put(b">\\n");
                i += 1;
                continue;
            }
        }
        if (c0 == b'-' || c0 == b'*') && hr(ln) {
            html.put(b"<hr>\\n");
            i += 1;
            continue;
        }
        if c0 == b'>' {
            let mut kept: Vec<&[u8]> = Vec::new();
            while i < nl {
                let l = line(i);
                if l.is_empty() || l[0] != b'>' {
                    break;
                }
                let mut a = 1;
                if a < l.len() && l[a] == b' ' {
                    a += 1;
                }
                let t = trim_st(&l[a..]);
                if !t.is_empty() {
                    kept.push(t);
                }
                i += 1;
            }
            if kept.is_empty() {
                html.put(b"<blockquote>\\n</blockquote>\\n");
            } else {
                html.put(b"<blockquote>\\n<p>");
                inline(&kept.join(&b'\n'), &mut html, link);
                html.put(b"</p>\\n</blockquote>\\n");
            }
            continue;
        }
        if c0 == b'-' || c0 == b'*' || c0.is_ascii_digit() {
            let b = if c0 == b'-' || c0 == b'*' { bullet(ln) } else { None };
            let o = if b.is_none() && c0.is_ascii_digit() { ordered(ln) } else { None };
            if b.is_some() || o.is_some() {
                let is_ord = o.is_some();
                if let Some((digits, _)) = o {
                    let start = digits.iter().fold(0u64, |a, &c| a * 10 + (c - b'0') as u64);
                    if start != 1 {
                        html.put(b"<ol start=\\\"");
                        html.put(start.to_string().as_bytes());
                        html.put(b"\\\">\\n");
                    } else {
                        html.put(b"<ol>\\n");
                    }
                } else {
                    html.put(b"<ul>\\n");
                }
                let mut cur: Option<Vec<u8>> = None;
                while i < nl {
                    let l = line(i);
                    let m = if is_ord { ordered(l).map(|x| x.1) } else { bullet(l) };
                    if let Some(rest) = m {
                        if let Some(c) = cur.take() {
                            html.put(b"<li>");
                            inline(&c, &mut html, link);
                            html.put(b"</li>\\n");
                        }
                        cur = Some(trim_st(rest).to_vec());
                        i += 1;
                    } else if continued(l) {
                        let c = cur.as_mut().unwrap();
                        c.push(b'\n');
                        c.put(trim_st(l));
                        i += 1;
                    } else {
                        break;
                    }
                }
                html.put(b"<li>");
                inline(&cur.unwrap_or_default(), &mut html, link);
                html.put(if is_ord { b"</li>\\n</ol>\\n" as &[u8] } else { b"</li>\\n</ul>\\n" });
                continue;
            }
        }
        // A paragraph: this line and the next ones up to one that ends it, each trimmed, joined by "\n".
        let first = i;
        loop {
            i += 1;
            if i >= nl || ends(line(i)) {
                break;
            }
        }
        // Lines with no space or tab at either end are the source's own run of bytes, joined by its own "\n".
        let clean = (first..i).all(|k| {
            let l = line(k);
            !l.is_empty() && !is_st(l[0]) && !is_st(l[l.len() - 1])
        });
        let joined;
        let text: &[u8] = if clean {
            &src[lines[first].0..lines[i - 1].1]
        } else {
            let mut t = Vec::new();
            for k in first..i {
                if k > first {
                    t.push(b'\n');
                }
                t.put(trim_st(line(k)));
            }
            joined = t;
            &joined
        };
        html.put(b"<p>");
        let at = html.len();
        inline(text, &mut html, link);
        if excerpt.is_none() {
            excerpt = Some(html[at..].to_vec());
        }
        html.put(b"</p>\\n");
    }
    Md { html, toc, excerpt: excerpt.unwrap_or_default() }
}
