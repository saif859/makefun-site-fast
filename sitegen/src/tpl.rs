//! makefun-site's template dialect: {{ path }}, {{{ path }}}, {% for x in path %}, {% if [not] path %}, {% else %},
//! {% endfor %}, {% endif %} and {% include "name" %}, parsed as its parser does (errors word for word) and rendered as
//! its interpreter does, straight into the inside of a JSON string.

use crate::text::*;

use std::rc::Rc;

pub enum Node {
    /// Text, already JSON-escaped.
    Text(Vec<u8>),
    Var(Vec<Vec<u8>>),
    Raw(Vec<Vec<u8>>),
    If(bool, Vec<Vec<u8>>, Vec<Node>, Option<Vec<Node>>),
    For(Vec<u8>, Vec<Vec<u8>>, Vec<Node>),
    Include(Vec<u8>),
}

/// A value templates see. Null stands for null and undefined alike (no rule tells them apart); Loop(k, n) is the loop
/// record { index: k + 1, first, last, length: n }.
#[derive(Clone)]
pub enum V {
    Null,
    Bool(bool),
    Num(i64),
    Str(Rc<[u8]>),
    Arr(Rc<Vec<V>>),
    Obj(Rc<Vec<(&'static [u8], V)>>),
    Loop(u32, u32),
}

pub fn s(b: &[u8]) -> V {
    V::Str(Rc::from(b))
}

pub fn obj(fields: Vec<(&'static [u8], V)>) -> V {
    V::Obj(Rc::new(fields))
}

pub struct Frame<'a> {
    pub vars: &'a [(&'a [u8], V)],
    pub parent: Option<&'a Frame<'a>>,
}

fn err(name: &[u8], msg: &[u8]) -> Vec<u8> {
    [b"template \"" as &[u8], name, b"\": ", msg].concat()
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn path_of(name: &[u8], text: &[u8]) -> Result<Vec<Vec<u8>>, Vec<u8>> {
    let ok = !text.is_empty()
        && text.split(|&c| c == b'.').all(|seg| !seg.is_empty() && is_ident_start(seg[0]) && seg.iter().all(|&c| is_ident(c)));
    if !ok {
        let mut m = b"\"".to_vec();
        json_into(&mut m, text);
        m.extend_from_slice(b"\" is not a path");
        return Err(err(name, &m));
    }
    Ok(text.split(|&c| c == b'.').map(|seg| seg.to_vec()).collect())
}

/// \S+ to the end.
fn non_space(s: &[u8]) -> bool {
    !s.is_empty() && !has_space(s)
}

/// /^for ([A-Za-z_][A-Za-z0-9_]*) in (\S+)$/
fn for_tag(tag: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = tag.strip_prefix(b"for ")?;
    if rest.is_empty() || !is_ident_start(rest[0]) {
        return None;
    }
    let k = rest.iter().take_while(|&&c| is_ident(c)).count();
    let p = rest[k..].strip_prefix(b" in ")?;
    if non_space(p) {
        Some((&rest[..k], p))
    } else {
        None
    }
}

/// /^if (not )?(\S+)$/
fn if_tag(tag: &[u8]) -> Option<(bool, &[u8])> {
    let rest = tag.strip_prefix(b"if ")?;
    if let Some(p) = rest.strip_prefix(b"not ") {
        if non_space(p) {
            return Some((true, p));
        }
    }
    if non_space(rest) {
        Some((false, rest))
    } else {
        None
    }
}

/// /^include "([a-z0-9-]+)"$/
fn include_tag(tag: &[u8]) -> Option<&[u8]> {
    let inner = tag.strip_prefix(b"include \"")?.strip_suffix(b"\"")?;
    if !inner.is_empty() && inner.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') {
        Some(inner)
    } else {
        None
    }
}

enum Tok {
    Text(Vec<u8>),
    Var(Vec<Vec<u8>>, bool),
    Tag(Vec<u8>),
}

struct Open {
    is_for: bool,
    name: Vec<u8>,
    not: bool,
    path: Vec<Vec<u8>>,
    body: Vec<Node>,
    els: Option<Vec<Node>>,
}

fn cur<'a>(root: &'a mut Vec<Node>, stack: &'a mut [Open]) -> &'a mut Vec<Node> {
    match stack.last_mut() {
        None => root,
        Some(o) => match o.els.as_mut() {
            Some(e) => e,
            None => &mut o.body,
        },
    }
}

/// A template parsed, or its error as makefun-site words it.
pub fn parse(name: &[u8], src: &[u8]) -> Result<Vec<Node>, Vec<u8>> {
    let mut toks = Vec::new();
    let mut text: Vec<u8> = Vec::new();
    let n = src.len();
    let mut i = 0;
    while i < n {
        let open = match find_byte(&src[i..], b'{') {
            Some(o) => i + o,
            None => {
                text.extend_from_slice(&src[i..]);
                break;
            }
        };
        if open == n - 1 {
            text.extend_from_slice(&src[i..]);
            break;
        }
        let next = src[open + 1];
        if next != b'{' && next != b'%' {
            text.extend_from_slice(&src[i..open + 1]);
            i = open + 1;
            continue;
        }
        text.extend_from_slice(&src[i..open]);
        if next == b'{' {
            let raw = src.get(open + 2) == Some(&b'{');
            let from = open + if raw { 3 } else { 2 };
            let pat: &[u8] = if raw { b"}}}" } else { b"}}" };
            let close = match find_bytes(src, pat, from) {
                Some(c) => c,
                None => return Err(err(name, if raw { b"a {{{ is not closed" } else { b"a {{ is not closed" })),
            };
            if !text.is_empty() {
                toks.push(Tok::Text(std::mem::take(&mut text)));
            }
            toks.push(Tok::Var(path_of(name, trim_u(&src[from..close]))?, raw));
            i = close + pat.len();
            continue;
        }
        let close = match find_bytes(src, b"%}", open + 2) {
            Some(c) => c,
            None => return Err(err(name, b"a {% is not closed")),
        };
        let end = close + 2;
        let line_start = src[..open].iter().rposition(|&c| c == b'\n').map(|p| p + 1).unwrap_or(0);
        let nl = find_byte(&src[end..], b'\n').map(|o| end + o);
        let line_end = nl.unwrap_or(n);
        let standalone = all_st(&src[line_start..open]) && all_st(&src[end..line_end]);
        if standalone {
            let keep = text.len().saturating_sub(open - line_start);
            text.truncate(keep);
        }
        if !text.is_empty() {
            toks.push(Tok::Text(std::mem::take(&mut text)));
        }
        toks.push(Tok::Tag(trim_u(&src[open + 2..close]).to_vec()));
        i = if standalone { nl.map(|p| p + 1).unwrap_or(n) } else { end };
    }
    if !text.is_empty() {
        toks.push(Tok::Text(text));
    }

    let mut root: Vec<Node> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    for t in toks {
        match t {
            Tok::Text(x) => {
                let mut e = Vec::with_capacity(x.len() + 8);
                json_into(&mut e, &x);
                cur(&mut root, &mut stack).push(Node::Text(e));
            }
            Tok::Var(p, raw) => cur(&mut root, &mut stack).push(if raw { Node::Raw(p) } else { Node::Var(p) }),
            Tok::Tag(tag) => {
                if let Some((nm, p)) = for_tag(&tag) {
                    let path = path_of(name, p)?;
                    stack.push(Open { is_for: true, name: nm.to_vec(), not: false, path, body: Vec::new(), els: None });
                } else if let Some((not, p)) = if_tag(&tag) {
                    let path = path_of(name, p)?;
                    stack.push(Open { is_for: false, name: Vec::new(), not, path, body: Vec::new(), els: None });
                } else if tag == b"else" {
                    match stack.last_mut() {
                        Some(top) if !top.is_for && top.els.is_none() => top.els = Some(Vec::new()),
                        _ => return Err(err(name, b"an else outside an if")),
                    }
                } else if tag == b"endfor" || tag == b"endif" {
                    let want_for = tag == b"endfor";
                    match stack.last() {
                        Some(top) if top.is_for == want_for => {}
                        _ => return Err(err(name, &[b"an " as &[u8], &tag, b" with no open ", &tag[3..]].concat())),
                    }
                    let o = stack.pop().unwrap();
                    let node = if o.is_for { Node::For(o.name, o.path, o.body) } else { Node::If(o.not, o.path, o.body, o.els) };
                    cur(&mut root, &mut stack).push(node);
                } else if let Some(inc) = include_tag(&tag) {
                    cur(&mut root, &mut stack).push(Node::Include(inc.to_vec()));
                } else {
                    return Err(err(name, &[b"{% " as &[u8], &tag, b" %} is not a tag"].concat()));
                }
            }
        }
    }
    if let Some(top) = stack.last() {
        return Err(err(name, if top.is_for { b"a for is not closed" } else { b"a if is not closed" }));
    }
    Ok(root)
}

fn walk(v: &V, keys: &[Vec<u8>]) -> V {
    let mut v = v.clone();
    for k in keys {
        let k = k.as_slice();
        v = match &v {
            V::Obj(o) => match o.iter().find(|(kk, _)| *kk == k) {
                Some((_, x)) => x.clone(),
                None => return V::Null,
            },
            V::Arr(a) if k == b"length" => V::Num(a.len() as i64),
            V::Loop(i, n) => match k {
                b"index" => V::Num(*i as i64 + 1),
                b"first" => V::Bool(*i == 0),
                b"last" => V::Bool(*i + 1 == *n),
                b"length" => V::Num(*n as i64),
                _ => return V::Null,
            },
            _ => return V::Null,
        };
    }
    v
}

fn lookup(f: &Frame, path: &[Vec<u8>]) -> V {
    let head = path[0].as_slice();
    let mut cur = Some(f);
    while let Some(fr) = cur {
        if let Some((_, v)) = fr.vars.iter().find(|(k, _)| *k == head) {
            return walk(v, &path[1..]);
        }
        cur = fr.parent;
    }
    V::Null
}

fn truthy(v: &V) -> bool {
    match v {
        V::Null => false,
        V::Bool(b) => *b,
        V::Num(n) => *n != 0,
        V::Str(s) => !s.is_empty(),
        V::Arr(a) => !a.is_empty(),
        V::Obj(_) | V::Loop(..) => true,
    }
}

/// textOf(v), HTML-escaped when `html`, as the inside of a JSON string.
fn emit(v: &V, out: &mut Vec<u8>, html: bool) {
    match v {
        V::Str(s) => {
            if html {
                html_json_into(out, s)
            } else {
                json_into(out, s)
            }
        }
        V::Num(n) => out.extend_from_slice(n.to_string().as_bytes()),
        V::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        V::Arr(a) => {
            for (k, x) in a.iter().enumerate() {
                if k > 0 {
                    out.extend_from_slice(b", ");
                }
                emit(x, out, html);
            }
        }
        _ => {}
    }
}

pub const INCLUDE_DEPTH: usize = 8;

pub fn render(nodes: &[Node], f: &Frame, tpls: &Map<Vec<u8>, Vec<Node>>, depth: usize, out: &mut Vec<u8>) {
    for n in nodes {
        match n {
            Node::Text(t) => out.extend_from_slice(t),
            Node::Var(p) => emit(&lookup(f, p), out, true),
            Node::Raw(p) => emit(&lookup(f, p), out, false),
            Node::If(not, path, then, els) => {
                if truthy(&lookup(f, path)) != *not {
                    render(then, f, tpls, depth, out);
                } else if let Some(e) = els {
                    render(e, f, tpls, depth, out);
                }
            }
            Node::For(name, path, body) => {
                if let V::Arr(xs) = lookup(f, path) {
                    let len = xs.len() as u32;
                    for (k, x) in xs.iter().enumerate() {
                        let rec = V::Loop(k as u32, len);
                        let pair;
                        let one;
                        let vars: &[(&[u8], V)] = if name.as_slice() == b"loop" {
                            one = [(b"loop" as &[u8], rec)];
                            &one
                        } else {
                            pair = [(name.as_slice(), x.clone()), (b"loop" as &[u8], rec)];
                            &pair
                        };
                        let fr = Frame { vars, parent: Some(f) };
                        render(body, &fr, tpls, depth, out);
                    }
                }
            }
            Node::Include(name) => {
                if depth < INCLUDE_DEPTH {
                    if let Some(t) = tpls.get(name) {
                        render(t, f, tpls, depth + 1, out);
                    }
                }
            }
        }
    }
}
