//! One site's build, as makefun-site 1.0.0's: its output tree written as the JSON object main.ts writes for it.

use crate::json::J;
use crate::md;
use crate::text::*;
use crate::tpl::{self, obj, s, st, st_json, Frame, Node, Str, V};

use std::rc::Rc;

const REPORT_FILE: &[u8] = b"build-report.txt";
const RESERVED: [&[u8]; 3] = [b"posts", b"tags", b"page"];
const KEYS: [&[u8]; 7] = [b"title", b"date", b"tags", b"draft", b"layout", b"slug", b"summary"];

pub struct Input<'j> {
    title: &'j [u8],
    base_url: &'j [u8],
    per_page: usize,
    recent: usize,
    files: Vec<(&'j [u8], &'j [u8])>,
    templates: Vec<(&'j [u8], &'j [u8])>,
}

impl<'j> Input<'j> {
    /// The input this build takes exactly, or None (main.ts then builds the case in JavaScript).
    pub fn from_json(j: &'j J<'j>) -> Option<Input<'j>> {
        let site = j.get(b"site")?;
        let title = site.get(b"title")?.str()?;
        let base_url = site.get(b"base_url")?.str()?;
        let int = |v: Option<&J>, min: f64| -> Option<usize> {
            match v? {
                J::Num(x) if x.fract() == 0.0 && *x >= min && *x < 1e9 => Some(*x as usize),
                _ => None,
            }
        };
        let per_page = int(site.get(b"per_page"), 1.0)?;
        let recent = int(site.get(b"recent"), 0.0)?;
        let J::Arr(fs) = j.get(b"files")? else { return None };
        let mut files = Vec::with_capacity(fs.len());
        for f in fs {
            files.push((f.get(b"path")?.str()?, f.get(b"content")?.str()?));
        }
        let J::Obj(ts) = j.get(b"templates")? else { return None };
        let mut templates = Vec::with_capacity(ts.len());
        for (k, v) in ts {
            templates.push((*k, v.str()?));
        }
        Some(Input { title, base_url, per_page, recent, files, templates })
    }
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
extern "C" {
    fn js_slug(p: *const u8, n: usize, out: *mut u8) -> usize;
}

/// Off WebAssembly (the Linux program), the slug of non-ASCII text as JavaScript makes it: NFKD, every mark
/// (General_Category M, /\p{M}/u) dropped, then the ASCII rules; written at `out`, its length returned.
#[cfg(not(target_arch = "wasm32"))]
unsafe fn js_slug(p: *const u8, n: usize, out: *mut u8) -> usize {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    let text = std::str::from_utf8(std::slice::from_raw_parts(p, n)).unwrap_or("");
    let mut slug: Vec<u8> = Vec::with_capacity(64);
    let mut gap = false;
    for ch in text.nfkd() {
        if is_combining_mark(ch) {
            continue;
        }
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap {
                slug.push(b'-');
            }
            gap = false;
            slug.push(c as u8);
        } else {
            gap = !slug.is_empty();
        }
    }
    if slug.len() > 64 {
        slug.truncate(64);
        if slug[63] == b'-' {
            slug.truncate(63);
        }
    }
    if slug.is_empty() {
        slug.put(b"untitled");
    }
    std::ptr::copy_nonoverlapping(slug.as_ptr(), out, slug.len());
    slug.len()
}

/// makefun-site's slug: ASCII here, other text by JavaScript's NFKD (main.ts's js_slug).
fn slugify_raw(t: &[u8]) -> Vec<u8> {
    if let Some(x) = slug_ascii(t) {
        return x;
    }
    let mut buf = [0u8; 128];
    let n = unsafe { js_slug(t.as_ptr(), t.len(), buf.as_mut_ptr()) };
    buf[..n.min(128)].to_vec()
}

#[derive(Default)]
struct Slugs(Map<Vec<u8>, Vec<u8>>);

impl Slugs {
    fn get(&mut self, t: &[u8]) -> Vec<u8> {
        if let Some(x) = self.0.get(t) {
            return x.clone();
        }
        let x = slugify_raw(t);
        self.0.insert(t.to_vec(), x.clone());
        x
    }
}

#[derive(Default)]
struct Fm {
    title: Option<Vec<u8>>,
    date: Option<Vec<u8>>,
    tags: Option<Vec<Vec<u8>>>,
    draft: Option<bool>,
    layout: Option<Vec<u8>>,
    slug: Option<Vec<u8>>,
    summary: Option<Vec<u8>>,
}

fn msg(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn string_of(raw: &[u8]) -> Result<Vec<u8>, &'static [u8]> {
    if raw.starts_with(b"\"") {
        if raw.len() < 2 || !raw.ends_with(b"\"") {
            return Err(b"a quoted string is not closed");
        }
        let mut out = Vec::with_capacity(raw.len());
        let mut i = 1;
        while i < raw.len() - 1 {
            let c = raw[i];
            if c == b'\\' {
                let n = raw[i + 1];
                if (n != b'"' && n != b'\\') || i + 1 == raw.len() - 1 {
                    return Err(b"a quoted string holds an escape other than \\\" or \\\\");
                }
                out.push(n);
                i += 2;
            } else if c == b'"' {
                return Err(b"a quoted string holds a stray quote");
            } else {
                out.push(c);
                i += 1;
            }
        }
        return Ok(out);
    }
    if raw.starts_with(b"[") {
        return Err(b"a bare string opens on [");
    }
    Ok(raw.to_vec())
}

fn days_in(y: u32, m: u32) -> u32 {
    if m == 2 {
        return if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 };
    }
    if m == 4 || m == 6 || m == 9 || m == 11 {
        30
    } else {
        31
    }
}

fn is_date(raw: &[u8]) -> bool {
    if raw.len() != 10 || raw[4] != b'-' || raw[7] != b'-' {
        return false;
    }
    let digits = |a: usize, b: usize| raw[a..b].iter().all(|c| c.is_ascii_digit());
    if !digits(0, 4) || !digits(5, 7) || !digits(8, 10) {
        return false;
    }
    let num = |a: usize, b: usize| raw[a..b].iter().fold(0u32, |x, &c| x * 10 + (c - b'0') as u32);
    let (y, mo, d) = (num(0, 4), num(5, 7), num(8, 10));
    (1970..=2099).contains(&y) && (1..=12).contains(&mo) && d >= 1 && d <= days_in(y, mo)
}

fn set_value(fm: &mut Fm, key: &[u8], raw: &[u8]) -> Result<(), Vec<u8>> {
    match key {
        b"title" | b"summary" => {
            let v = string_of(raw).map_err(|e| msg(&[b"key \"", key, b"\": ", e]))?;
            if key == b"title" {
                fm.title = Some(v);
            } else {
                fm.summary = Some(v);
            }
        }
        b"date" => {
            if !is_date(raw) {
                return Err(b"key \"date\": not a date from 1970-01-01 to 2099-12-31 (YYYY-MM-DD)".to_vec());
            }
            fm.date = Some(raw.to_vec());
        }
        b"tags" => {
            if !raw.starts_with(b"[") || !raw.ends_with(b"]") {
                return Err(b"key \"tags\": not a list [a, b]".to_vec());
            }
            let inner = &raw[1..raw.len() - 1];
            if trim_u(inner).is_empty() {
                fm.tags = Some(Vec::new());
                return Ok(());
            }
            let tags: Vec<&[u8]> = inner.split(|&c| c == b',').map(trim_u).collect();
            if tags.iter().any(|t| t.is_empty() || t.iter().any(|&c| c == b'"' || c == b'[' || c == b']')) {
                return Err(b"key \"tags\": an empty tag, or one holding \" [ or ]".to_vec());
            }
            fm.tags = Some(tags.into_iter().map(|t| t.to_vec()).collect());
        }
        b"draft" => {
            fm.draft = Some(match raw {
                b"true" => true,
                b"false" => false,
                _ => return Err(b"key \"draft\": true or false".to_vec()),
            });
        }
        b"layout" => {
            if raw.is_empty() || !raw.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') {
                return Err(b"key \"layout\": not a template name".to_vec());
            }
            fm.layout = Some(raw.to_vec());
        }
        b"slug" => {
            let ok = raw.len() <= 64
                && raw.split(|&c| c == b'-').all(|p| !p.is_empty() && p.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit()));
            if !ok {
                return Err(b"key \"slug\": not a slug".to_vec());
            }
            fm.slug = Some(raw.to_vec());
        }
        _ => return Err(msg(&[b"unknown key \"", key, b"\""])),
    }
    Ok(())
}

/// A source's front matter and where its body starts, or its error.
fn parse_fm(text: &[u8]) -> Result<(Fm, usize), Vec<u8>> {
    let n = text.len();
    if !text.starts_with(b"---") || (n > 3 && text[3] != b'\n') {
        return Err(b"no front matter".to_vec());
    }
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut pos = 4;
    let mut body_at = None;
    while pos <= n && n > 3 {
        let nl = find_byte(&text[pos..], b'\n').map(|o| pos + o).unwrap_or(n);
        let line = &text[pos..nl];
        if line == b"---" {
            body_at = Some(nl + 1);
            break;
        }
        lines.push(line);
        pos = nl + 1;
    }
    let Some(body_at) = body_at else { return Err(b"the front matter is not closed".to_vec()) };
    let mut fm = Fm::default();
    let mut seen: Vec<&[u8]> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if all_st(line) || line.starts_with(b"#") {
            continue;
        }
        // /^([a-z_]+):(?: (.*))?$/
        let k = line.iter().take_while(|&&c| c.is_ascii_lowercase() || c == b'_').count();
        let parsed = if k > 0 && line.get(k) == Some(&b':') {
            if line.len() == k + 1 {
                Some(&line[k + 1..])
            } else if line[k + 1] == b' ' {
                Some(&line[k + 2..])
            } else {
                None
            }
        } else {
            None
        };
        let Some(rest) = parsed else {
            return Err(msg(&[b"line ", (i + 2).to_string().as_bytes(), b" is not \"key: value\""]));
        };
        let key = &line[..k];
        let raw = trim_end_st(rest);
        if !KEYS.contains(&key) {
            return Err(msg(&[b"unknown key \"", key, b"\""]));
        }
        if seen.contains(&key) {
            return Err(msg(&[b"key \"", key, b"\" is given twice"]));
        }
        if raw.is_empty() {
            return Err(msg(&[b"key \"", key, b"\" has no value"]));
        }
        set_value(&mut fm, key, raw)?;
        seen.push(key);
    }
    if fm.title.is_none() {
        return Err(b"no title".to_vec());
    }
    Ok((fm, body_at.min(n)))
}

fn dir_of(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|&c| c == b'/') {
        Some(k) => &path[..k],
        None => b"",
    }
}

/// `rel` from directory `dir`, or None when it climbs out of the root.
fn resolve_path(dir: &[u8], rel: &[u8]) -> Option<Vec<u8>> {
    let mut parts: Vec<&[u8]> = if dir.is_empty() { Vec::new() } else { dir.split(|&c| c == b'/').collect() };
    for seg in rel.split(|&c| c == b'/') {
        if seg.is_empty() || seg == b"." {
            continue;
        }
        if seg == b".." {
            parts.pop()?;
        } else {
            parts.push(seg);
        }
    }
    Some(parts.join(&b'/'))
}

/// /^[A-Za-z][A-Za-z0-9+.-]*:/
fn has_scheme(t: &[u8]) -> bool {
    if t.is_empty() || !t[0].is_ascii_alphabetic() {
        return false;
    }
    let k = 1 + t[1..].iter().take_while(|&&c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-')).count();
    t.get(k) == Some(&b':')
}

struct Src<'a> {
    path: &'a [u8],
    text: std::borrow::Cow<'a, [u8]>,
    body_at: usize,
    fm: Fm,
    post: bool,
    layout: Vec<u8>,
    slug: Vec<u8>,
    out_dir: Vec<u8>,
    url: Str,
}

impl<'a> Src<'a> {
    fn body(&self) -> &[u8] {
        &self.text[self.body_at..]
    }
}

enum TagRef {
    Tag(usize),
    Own(Str, Str),
}

struct Tag {
    name: Str,
    slug: Str,
    url: Str,
    posts: Vec<usize>,
}

/// The output object's writer: `{"path":"text",...}`, each text JSON-escaped.
struct Out<'o> {
    out: &'o mut Vec<u8>,
    first: bool,
    keys: Set<Vec<u8>>,
}

impl<'o> Out<'o> {
    fn open(&mut self, key: &[u8]) {
        self.out.push(if self.first { b'{' } else { b',' });
        self.first = false;
        self.out.push(b'"');
        json_into(self.out, key);
        self.out.put(b"\":\"");
        self.keys.insert(key.to_vec());
    }
    fn close(&mut self) {
        self.out.push(b'"');
    }
}

/// Builds one site into `out`; false when it is input this build does not take exactly (out is then left to be cut).
#[cfg(feature = "timing")]
thread_local! { pub static PHASES: std::cell::RefCell<Vec<(&'static str, std::time::Duration)>> = const { std::cell::RefCell::new(Vec::new()) }; }

/// Under the timing feature, the time since the last phase, kept as `name`'s.
macro_rules! phase {
    ($t:ident, $name:expr) => {
        #[cfg(feature = "timing")]
        {
            let now = std::time::Instant::now();
            PHASES.with(|p| p.borrow_mut().push(($name, now - $t)));
            $t = now;
        }
    };
}

pub fn run(inp: &Input, out: &mut Vec<u8>) -> bool {
    #[cfg(feature = "timing")]
    #[allow(unused_mut)]
    let mut t0 = std::time::Instant::now();
    let mut slugs = Slugs::default();
    let mut report: Vec<Vec<u8>> = Vec::new();

    // Templates, parsed in name order; one that does not parse is reported.
    let mut names: Vec<&(&[u8], &[u8])> = inp.templates.iter().collect();
    names.sort_by(|a, b| cmp_units(&a.0, &b.0));
    let mut keys = tpl::Keys::new();
    let mut templates: Map<Vec<u8>, Vec<Node>> = Map::default();
    for (name, src) in names {
        match tpl::parse(name, src, &mut keys) {
            Ok(n) => {
                templates.insert(name.to_vec(), n);
            }
            Err(e) => report.push(e),
        }
    }
    let mut base: &[u8] = inp.base_url;
    while base.last() == Some(&b'/') {
        base = &base[..base.len() - 1];
    }

    // 1. Sources in code-unit order of path; .md are sources, the rest static.
    let mut files: Vec<&(&[u8], &[u8])> = inp.files.iter().collect();
    files.sort_by(|a, b| cmp_units(&a.0, &b.0));
    let mut statics: Vec<&(&[u8], &[u8])> = Vec::new();
    let mut kept: Vec<Src> = Vec::new();
    for f in files {
        if !f.0.ends_with(b".md") {
            statics.push(f);
            continue;
        }
        let path = f.0;
        let c = f.1;
        let text: std::borrow::Cow<[u8]> = if find_byte(c, b'\r').is_none() {
            std::borrow::Cow::Borrowed(c)
        } else {
            let mut t = Vec::with_capacity(c.len());
            let mut i = 0;
            while i < c.len() {
                if c[i] == b'\r' {
                    t.push(b'\n');
                    if c.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                } else {
                    t.push(c[i]);
                }
                i += 1;
            }
            std::borrow::Cow::Owned(t)
        };
        // 2. Front matter.
        let (fm, body_at) = match parse_fm(&text) {
            Ok(x) => x,
            Err(e) => {
                report.push(msg(&[b"refused ", path, b": ", &e]));
                continue;
            }
        };
        let post = path.starts_with(b"posts/");
        if path.starts_with(b"tags/") {
            report.push(msg(&[b"refused ", path, b": tags/ is the generator's own"]));
            continue;
        }
        if path.starts_with(b"page/") {
            report.push(msg(&[b"refused ", path, b": page/ is the generator's own"]));
            continue;
        }
        if post && fm.date.is_none() {
            report.push(msg(&[b"refused ", path, b": a post needs a date"]));
            continue;
        }
        let layout = fm.layout.clone().unwrap_or_else(|| if post { b"post".to_vec() } else { b"page".to_vec() });
        if !templates.contains_key(&layout) {
            report.push(msg(&[b"refused ", path, b": layout \"", &layout, b"\" is no template that parses"]));
            continue;
        }
        if fm.draft == Some(true) {
            report.push(msg(&[b"draft ", path]));
            continue;
        }
        kept.push(Src { path, text, body_at, fm, post, layout, slug: Vec::new(), out_dir: Vec::new(), url: st(b"") });
    }

    phase!(t0, "sources+fm");
    // 3. Permalinks.
    let mut taken: Map<Vec<u8>, Set<Vec<u8>>> = Map::default();
    let mut by_source: Map<&[u8], usize> = Map::default();
    for (k, s) in kept.iter_mut().enumerate() {
        let dir: &[u8] = if s.post { b"posts" } else { dir_of(s.path) };
        let set = taken.entry(dir.to_vec()).or_insert_with(|| if dir.is_empty() { RESERVED.iter().map(|r| r.to_vec()).collect() } else { Set::default() });
        let wanted = match &s.fm.slug {
            Some(x) => x.clone(),
            None => slugs.get(s.fm.title.as_ref().unwrap()),
        };
        s.slug = unique_slug(&wanted, set);
        s.out_dir = if dir.is_empty() { s.slug.clone() } else { [dir, b"/", &s.slug].concat() };
        s.url = st(&[base, b"/", &s.out_dir, b"/"].concat());
        by_source.insert(s.path, k);
    }

    phase!(t0, "permalinks");
    // 4. Markdown, links resolved.
    let static_paths: Set<&[u8]> = statics.iter().map(|f| f.0).collect();
    let mut broken: Vec<Vec<u8>> = Vec::new();
    let mut broken_seen: Set<Vec<u8>> = Set::default();
    let mut mds: Vec<md::Md> = Vec::with_capacity(kept.len());
    for s in &kept {
        let dir = dir_of(s.path);
        let mut link = |target: &[u8]| -> Vec<u8> {
            let c = target[0];
            if c == b'/' || c == b'#' || has_scheme(target) {
                return target.to_vec();
            }
            let (rel, frag) = match find_byte(target, b'#') {
                Some(h) => (&target[..h], &target[h..]),
                None => (target, &b""[..]),
            };
            if !rel.is_empty() {
                if let Some(path) = resolve_path(dir, rel) {
                    if let Some(&hit) = by_source.get(path.as_slice()) {
                        return [&kept[hit].url.b[..], frag].concat();
                    }
                    if static_paths.contains(path.as_slice()) {
                        return [base, b"/", &path, frag].concat();
                    }
                }
            }
            let line = msg(&[b"broken-link ", s.path, b" -> ", target]);
            if broken_seen.insert(line.clone()) {
                broken.push(line);
            }
            target.to_vec()
        };
        let mut slug = |t: &[u8]| slugify_raw(t);
        mds.push(md::render(s.body(), &mut link, &mut slug));
    }

    phase!(t0, "markdown");
    // Each body's HTML and excerpt as the strings templates see, moved there (no copy).
    let md_strs: Vec<(Str, Str)> = mds.iter_mut().map(|m| (st_json(std::mem::take(&mut m.html)), st_json(std::mem::take(&mut m.excerpt)))).collect();

    // 5. Posts, newest first then by slug.
    let mut posts: Vec<usize> = (0..kept.len()).filter(|&k| kept[k].post).collect();
    posts.sort_by(|&x, &y| {
        cmp_units(kept[y].fm.date.as_ref().unwrap(), kept[x].fm.date.as_ref().unwrap()).then_with(|| cmp_units(&kept[x].slug, &kept[y].slug))
    });
    let mut newer: Vec<Option<usize>> = vec![None; kept.len()];
    let mut older: Vec<Option<usize>> = vec![None; kept.len()];
    for (k, &p) in posts.iter().enumerate() {
        newer[p] = if k == 0 { None } else { Some(posts[k - 1]) };
        older[p] = posts.get(k + 1).copied();
    }

    // 6. Tags.
    let mut own_tags = |s: &Src| -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut seen: Set<Vec<u8>> = Set::default();
        let mut out = Vec::new();
        if let Some(list) = &s.fm.tags {
            for name in list {
                let slug = slugs.get(name);
                if seen.insert(slug.clone()) {
                    out.push((name.clone(), slug));
                }
            }
        }
        out
    };
    let mut tags: Vec<Tag> = Vec::new();
    let mut by_slug: Map<Vec<u8>, usize> = Map::default();
    let mut tag_refs: Vec<Vec<TagRef>> = (0..kept.len()).map(|_| Vec::new()).collect();
    for &p in &posts {
        for (name, slug) in own_tags(&kept[p]) {
            let t = match by_slug.get(&slug) {
                Some(&t) => t,
                None => {
                    tags.push(Tag {
                        name: st(&name),
                        url: st(&[base, b"/tags/", &slug, b"/"].concat()),
                        slug: st(&slug),
                        posts: Vec::new(),
                    });
                    by_slug.insert(slug, tags.len() - 1);
                    tags.len() - 1
                }
            };
            tags[t].posts.push(p);
            tag_refs[p].push(TagRef::Tag(t));
        }
    }
    for k in 0..kept.len() {
        if !kept[k].post {
            tag_refs[k] = own_tags(&kept[k])
                .into_iter()
                .map(|(name, slug)| match by_slug.get(&slug) {
                    Some(&t) => TagRef::Tag(t),
                    None => TagRef::Own(st(&name), st(&slug)),
                })
                .collect();
        }
    }
    let mut tag_list: Vec<usize> = (0..tags.len()).collect();
    tag_list.sort_by(|&x, &y| tags[y].posts.len().cmp(&tags[x].posts.len()).then_with(|| cmp_units(&tags[x].slug.b, &tags[y].slug.b)));

    phase!(t0, "posts+tags");
    // 7. Layouts.
    let empty = s(b"");
    let date_of = |k: usize| -> (V, V) {
        match &kept[k].fm.date {
            Some(d) => (s(d), s(&long_date(d))),
            None => (empty.clone(), empty.clone()),
        }
    };
    let card_fields = |k: usize| -> Vec<(u32, V)> {
        let src = &kept[k];
        let (date, date_long) = date_of(k);
        let card_tags: Vec<V> = tag_refs[k]
            .iter()
            .map(|r| match r {
                TagRef::Tag(t) => obj(vec![(tpl::K_NAME, V::Str(tags[*t].name.clone())), (tpl::K_SLUG, V::Str(tags[*t].slug.clone())), (tpl::K_URL, V::Str(tags[*t].url.clone()))]),
                TagRef::Own(name, slug) => obj(vec![(tpl::K_NAME, V::Str(name.clone())), (tpl::K_SLUG, V::Str(slug.clone())), (tpl::K_URL, empty.clone())]),
            })
            .collect();
        let minutes = words(src.body()).div_ceil(200).max(1);
        vec![
            (tpl::K_TITLE, s(src.fm.title.as_ref().unwrap())),
            (tpl::K_URL, V::Str(src.url.clone())),
            (tpl::K_DATE, date),
            (tpl::K_DATE_LONG, date_long),
            (tpl::K_SUMMARY, src.fm.summary.as_deref().map(s).unwrap_or_else(|| empty.clone())),
            (tpl::K_EXCERPT, V::Str(md_strs[k].1.clone())),
            (tpl::K_READING_MINUTES, V::Num(minutes as i64)),
            (tpl::K_TAGS, V::Arr(Rc::new(card_tags))),
        ]
    };
    let mut cards: Vec<Option<V>> = vec![None; kept.len()];
    for &p in &posts {
        cards[p] = Some(obj(card_fields(p)));
    }
    let card = |k: usize| cards[k].clone().unwrap();
    let site = obj(vec![(tpl::K_TITLE, s(inp.title)), (tpl::K_BASE_URL, s(base))]);
    let all_tags = V::Arr(Rc::new(
        tag_list
            .iter()
            .map(|&t| {
                let tg = &tags[t];
                obj(vec![(tpl::K_NAME, V::Str(tg.name.clone())), (tpl::K_SLUG, V::Str(tg.slug.clone())), (tpl::K_URL, V::Str(tg.url.clone())), (tpl::K_COUNT, V::Num(tg.posts.len() as i64))])
            })
            .collect(),
    ));
    let recent = V::Arr(Rc::new(posts.iter().take(inp.recent).map(|&p| card(p)).collect()));
    let link_of = |p: Option<usize>| -> V {
        match p {
            None => V::Null,
            Some(k) => {
                let (date, date_long) = date_of(k);
                obj(vec![(tpl::K_TITLE, s(kept[k].fm.title.as_ref().unwrap())), (tpl::K_URL, V::Str(kept[k].url.clone())), (tpl::K_DATE, date), (tpl::K_DATE_LONG, date_long)])
            }
        }
    };

    phase!(t0, "cards");
    let ctx = tpl::Ctx::new(&templates);
    let mut w = Out { out, first: true, keys: Set::default() };
    let render_top = |w: &mut Out, key: &[u8], name: &[u8], extra: Vec<(u32, V)>| {
        let t = &templates[name];
        let mut vars: Vec<(u32, &V)> = vec![(tpl::K_SITE, &site), (tpl::K_TAGS, &all_tags), (tpl::K_RECENT, &recent)];
        vars.extend(extra.iter().map(|(k, v)| (*k, v)));
        w.open(key);
        tpl::render(t, &Frame { vars: &vars, parent: None }, &ctx, 0, w.out);
        w.close();
    };
    for k in 0..kept.len() {
        let src = &kept[k];
        let mut fields = match &cards[k] {
            Some(V::Obj(c)) => (**c).clone(),
            _ => card_fields(k),
        };
        let toc = mds[k]
            .toc
            .iter()
            .map(|(level, id, html)| obj(vec![(tpl::K_LEVEL, V::Num(*level as i64)), (tpl::K_ID, s(id)), (tpl::K_HTML, V::Str(st_json(html.clone())))]))
            .collect();
        fields.push((tpl::K_SLUG, s(&src.slug)));
        fields.push((tpl::K_CONTENT, V::Str(md_strs[k].0.clone())));
        fields.push((tpl::K_TOC, V::Arr(Rc::new(toc))));
        fields.push((tpl::K_NEWER, link_of(newer[k])));
        fields.push((tpl::K_OLDER, link_of(older[k])));
        fields.push((tpl::K_IS_POST, V::Bool(src.post)));
        let key = [&src.out_dir[..], b"/index.html"].concat();
        render_top(&mut w, &key, &src.layout, vec![(tpl::K_PAGE, obj(fields))]);
    }
    phase!(t0, "pages");
    let per = inp.per_page;
    let pages = posts.len().div_ceil(per).max(1);
    let index_url = |k: usize| -> V {
        if k == 1 {
            s(&[base, b"/"].concat())
        } else {
            s(&[base, b"/page/", k.to_string().as_bytes(), b"/"].concat())
        }
    };
    if templates.contains_key(&b"index"[..]) {
        for k in 1..=pages {
            let lo = ((k - 1) * per).min(posts.len());
            let hi = (k * per).min(posts.len());
            let list = V::Arr(Rc::new(posts[lo..hi].iter().map(|&p| card(p)).collect()));
            let pager = obj(vec![
                (tpl::K_NUMBER, V::Num(k as i64)),
                (tpl::K_TOTAL, V::Num(pages as i64)),
                (tpl::K_URL, index_url(k)),
                (tpl::K_NEWER_URL, if k == 1 { empty.clone() } else { index_url(k - 1) }),
                (tpl::K_OLDER_URL, if k == pages { empty.clone() } else { index_url(k + 1) }),
            ]);
            let key = if k == 1 { b"index.html".to_vec() } else { [b"page/" as &[u8], k.to_string().as_bytes(), b"/index.html"].concat() };
            render_top(&mut w, &key, b"index", vec![(tpl::K_POSTS, list), (tpl::K_PAGER, pager)]);
        }
    }
    if templates.contains_key(&b"tag"[..]) {
        for &t in &tag_list {
            let tg = &tags[t];
            let tag = obj(vec![
                (tpl::K_NAME, V::Str(tg.name.clone())),
                (tpl::K_SLUG, V::Str(tg.slug.clone())),
                (tpl::K_URL, V::Str(tg.url.clone())),
                (tpl::K_COUNT, V::Num(tg.posts.len() as i64)),
                (tpl::K_POSTS, V::Arr(Rc::new(tg.posts.iter().map(|&p| card(p)).collect()))),
            ]);
            let key = [b"tags/" as &[u8], &tg.slug.b, b"/index.html"].concat();
            render_top(&mut w, &key, b"tag", vec![(tpl::K_TAG, tag)]);
        }
    }
    if templates.contains_key(&b"tags"[..]) {
        render_top(&mut w, b"tags/index.html", b"tags", Vec::new());
    }
    phase!(t0, "index+tags");
    for f in &statics {
        let path = f.0;
        // JavaScript puts array-index keys first and would overwrite a static build-report.txt in place: not taken here.
        if path == REPORT_FILE || (!path.is_empty() && path.iter().all(|c| c.is_ascii_digit())) {
            return false;
        }
        if w.keys.contains(path) {
            report.push(msg(&[b"shadowed ", path, b": a built page has its path"]));
        } else {
            w.open(path);
            json_into(w.out, f.1);
            w.close();
        }
    }

    phase!(t0, "statics");
    // 8. The report.
    let mut lines: Vec<Vec<u8>> = report;
    lines.extend(broken);
    lines.sort_by(|a, b| cmp_units(a, b));
    let n_pages = kept.iter().filter(|x| !x.post).count();
    lines.push(
        format!("built {} pages, {} posts, {} tags and {} static files", n_pages, posts.len(), tag_list.len(), statics.len()).into_bytes(),
    );
    let mut text = lines.join(&b'\n');
    text.push(b'\n');
    w.open(REPORT_FILE);
    json_into(w.out, &text);
    w.close();
    w.out.push(b'}');
    phase!(t0, "report");
    // The case's memory goes back to the arena whole when it is done (see lib.rs), so its values are not dropped one
    // by one.
    std::mem::forget(ctx);
    std::mem::forget((w, cards, md_strs, mds, tags, tag_refs, site, all_tags, recent, kept, templates, by_slug, slugs));
    true
}
