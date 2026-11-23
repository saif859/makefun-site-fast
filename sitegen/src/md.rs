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

/// makefun-site's inline Markdown of raw text t, its text HTML-escaped, appended to out.
pub fn inline(t: &[u8], out: &mut Vec<u8>, link: &mut Link) {
    let n = t.len();
    let mut run = 0;
    let mut i = 0;
    while i < n {
        let c = t[i];
        if CLASS[c as usize] & 4 == 0 {
            i += 1;
            continue;
        }
        match c {
            b'&' | b'<' | b'>' | b'"' | b'\'' => {
                out.extend_from_slice(&t[run..i]);
                esc_into(out, &t[i..i + 1]);
                i += 1;
                run = i;
            }
            b'\\' => {
                if i + 1 < n && is_escapable(t[i + 1]) {
                    out.extend_from_slice(&t[run..i]);
                    if t[i + 1] == b'>' {
                        out.extend_from_slice(b"&gt;");
                        run = i + 2;
                    } else {
                        run = i + 1;
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            b'`' => match find_byte(&t[i + 1..], b'`') {
                Some(off) if off > 0 => {
                    let j = i + 1 + off;
                    out.extend_from_slice(&t[run..i]);
                    out.extend_from_slice(b"<code>");
                    esc_into(out, &t[i + 1..j]);
                    out.extend_from_slice(b"</code>");
                    i = j + 1;
                    run = i;
                }
                _ => i += 1,
            },
            b'*' => {
                if i + 1 < n && t[i + 1] == b'*' {
                    match find_double(t, i + 2) {
                        Some(j) if j > i + 2 => {
                            out.extend_from_slice(&t[run..i]);
                            out.extend_from_slice(b"<strong>");
                            inline(&t[i + 2..j], out, link);
                            out.extend_from_slice(b"</strong>");
                            i = j + 2;
                            run = i;
                        }
                        _ => i += 2,
                    }
                } else {
                    match find_single_star(t, i + 1) {
                        Some(j) if j > i + 1 => {
                            out.extend_from_slice(&t[run..i]);
                            out.extend_from_slice(b"<em>");
                            inline(&t[i + 1..j], out, link);
                            out.extend_from_slice(b"</em>");
                            i = j + 1;
                            run = i;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'[' => {
                let k = find_close(t, i + 1);
                let m = match k {
                    Some(k) if k + 1 < n && t[k + 1] == b'(' => find_byte(&t[k + 2..], b')').map(|o| k + 2 + o),
                    _ => None,
                };
                if let (Some(k), Some(m)) = (k, m) {
                    if m > k + 2 && !has_space(&t[k + 2..m]) {
                        let href = link(&t[k + 2..m]);
                        out.extend_from_slice(&t[run..i]);
                        out.extend_from_slice(b"<a href=\"");
                        esc_into(out, &href);
                        out.extend_from_slice(b"\">");
                        inline(&t[i + 1..k], out, link);
                        out.extend_from_slice(b"</a>");
                        i = m + 1;
                        run = i;
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    out.extend_from_slice(&t[run..]);
}

/// A source body rendered: its HTML, its h2 and h3 entries and its first paragraph's HTML.
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
                    html.extend_from_slice(b"<pre><code>");
                } else {
                    html.extend_from_slice(b"<pre><code class=\"language-");
                    esc_into(&mut html, lang);
                    html.extend_from_slice(b"\">");
                }
                if i > first {
                    esc_into(&mut html, &src[lines[first].0..lines[i - 1].1]);
                    html.push(b'\n');
                }
                i += 1;
                html.extend_from_slice(b"</code></pre>\n");
                continue;
            }
        }
        if c0 == b'#' {
            if let Some((depth, text)) = heading(ln) {
                let id = unique_slug(&slug(text), &mut ids);
                let mut ih = Vec::new();
                inline(text, &mut ih, link);
                let d = b'0' + depth as u8;
                html.extend_from_slice(b"<h");
                html.push(d);
                html.extend_from_slice(b" id=\"");
                html.extend_from_slice(&id);
                html.extend_from_slice(b"\">");
                html.extend_from_slice(&ih);
                html.extend_from_slice(b"</h");
                html.push(d);
                html.extend_from_slice(b">\n");
                if depth == 2 || depth == 3 {
                    toc.push((depth as u8, id, ih));
                }
                i += 1;
                continue;
            }
        }
        if (c0 == b'-' || c0 == b'*') && hr(ln) {
            html.extend_from_slice(b"<hr>\n");
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
                html.extend_from_slice(b"<blockquote>\n</blockquote>\n");
            } else {
                html.extend_from_slice(b"<blockquote>\n<p>");
                inline(&kept.join(&b'\n'), &mut html, link);
                html.extend_from_slice(b"</p>\n</blockquote>\n");
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
                        html.extend_from_slice(b"<ol start=\"");
                        html.extend_from_slice(start.to_string().as_bytes());
                        html.extend_from_slice(b"\">\n");
                    } else {
                        html.extend_from_slice(b"<ol>\n");
                    }
                } else {
                    html.extend_from_slice(b"<ul>\n");
                }
                let mut cur: Option<Vec<u8>> = None;
                while i < nl {
                    let l = line(i);
                    let m = if is_ord { ordered(l).map(|x| x.1) } else { bullet(l) };
                    if let Some(rest) = m {
                        if let Some(c) = cur.take() {
                            html.extend_from_slice(b"<li>");
                            inline(&c, &mut html, link);
                            html.extend_from_slice(b"</li>\n");
                        }
                        cur = Some(trim_st(rest).to_vec());
                        i += 1;
                    } else if continued(l) {
                        let c = cur.as_mut().unwrap();
                        c.push(b'\n');
                        c.extend_from_slice(trim_st(l));
                        i += 1;
                    } else {
                        break;
                    }
                }
                html.extend_from_slice(b"<li>");
                inline(&cur.unwrap_or_default(), &mut html, link);
                html.extend_from_slice(if is_ord { b"</li>\n</ol>\n" as &[u8] } else { b"</li>\n</ul>\n" });
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
                t.extend_from_slice(trim_st(line(k)));
            }
            joined = t;
            &joined
        };
        let mut ih = Vec::with_capacity(text.len() + 16);
        inline(text, &mut ih, link);
        html.extend_from_slice(b"<p>");
        html.extend_from_slice(&ih);
        html.extend_from_slice(b"</p>\n");
        if excerpt.is_none() {
            excerpt = Some(ih);
        }
    }
    Md { html, toc, excerpt: excerpt.unwrap_or_default() }
}
