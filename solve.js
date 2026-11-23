// A faster drop-in for makefun-site 1.0.0's build: run(input) returns the same output tree (path -> text), byte for
// byte. The rules are makefun-site's own (its README and code); this module runs them with less work:
//
//  - inline Markdown is scanned by char code, plain runs escaped in one go instead of one character at a time;
//  - templates are compiled once per build into closures, every name's scope resolved when compiling;
//  - a template piece whose output depends only on build-wide values (site, tags, recent) is rendered once per build,
//    and an include or loop that depends only on one loop item (a post's card) is rendered once per item;
//  - HTML escaping, slugs, word counts and line trimming use char-code loops with fast paths.
//
// Nothing is cached between calls: every cache lives inside one run(input).

const REPORT_FILE = 'build-report.txt';
const RESERVED = ['posts', 'tags', 'page'];
const INCLUDE_DEPTH = 8;

// ---------------------------------------------------------------- text

const ESC_RE = /[&<>"']/;

const AMP = /&/g;
const LT = /</g;
const GT = />/g;
const QUOT = /"/g;
const APOS = /'/g;

/** `text` with & < > " and ' escaped (makefun-site's table), & first. */
function esc(s) {
  if (!ESC_RE.test(s)) return s;
  if (s.indexOf('&') !== -1) s = s.replace(AMP, '&amp;');
  if (s.indexOf('<') !== -1) s = s.replace(LT, '&lt;');
  if (s.indexOf('>') !== -1) s = s.replace(GT, '&gt;');
  if (s.indexOf('"') !== -1) s = s.replace(QUOT, '&quot;');
  if (s.indexOf("'") !== -1) s = s.replace(APOS, '&#39;');
  return s;
}

const MARKS = /\p{M}/gu;

/** makefun-site's slug: NFKD, marks dropped, A-Z lowered, runs of other characters one "-", at most 64, "untitled". */
function slugifyRaw(text) {
  let plain = text;
  for (let i = 0; i < text.length; i++) {
    if (text.charCodeAt(i) > 127) {
      plain = (L1 ? dec(text) : text).normalize('NFKD').replace(MARKS, '');
      break;
    }
  }
  let out = '';
  let gap = false;
  const n = plain.length;
  for (let i = 0; i < n; i++) {
    let c = plain.charCodeAt(i);
    if (c >= 65 && c <= 90) c += 32;
    else if (!((c >= 97 && c <= 122) || (c >= 48 && c <= 57))) {
      gap = out.length > 0;
      continue;
    }
    if (gap) out += '-';
    gap = false;
    out += String.fromCharCode(c);
  }
  if (out.length > 64) {
    out = out.slice(0, 64);
    if (out.charCodeAt(63) === 45) out = out.slice(0, 63);
  }
  return out === '' ? 'untitled' : out;
}

function uniqueSlug(slug, taken) {
  let out = slug;
  for (let n = 2; taken.has(out); n++) out = `${slug}-${n}`;
  taken.add(out);
  return out;
}

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

function longDate(date) {
  const [y, m, d] = date.split('-');
  return `${Number(d)} ${MONTHS[Number(m) - 1]} ${y}`;
}

/** Whether char code `c` is JavaScript's \s. */
function isSpace(c) {
  if (c <= 32) return c === 32 || (c >= 9 && c <= 13);
  if (c < 160) return false;
  return c === 160 || c === 0x1680 || (c >= 0x2000 && c <= 0x200a) || c === 0x2028 || c === 0x2029 || c === 0x202f || c === 0x205f || c === 0x3000 || c === 0xfeff;
}

/** The count of /\S+/g matches in `text`. */
function wordsOf(text) {
  let count = 0;
  let inWord = false;
  const n = text.length;
  for (let i = 0; i < n; i++) {
    const c = text.charCodeAt(i);
    if (c > 32 && c < 160) {
      if (!inWord) {
        inWord = true;
        count++;
      }
    } else if (c <= 32 ? c === 32 || (c >= 9 && c <= 13) : isSpace(c)) inWord = false;
    else if (!inWord) {
      inWord = true;
      count++;
    }
  }
  return count;
}

/** A line with spaces and tabs at either end dropped. */
function trimLine(line) {
  let a = 0;
  let z = line.length;
  while (a < z) {
    const c = line.charCodeAt(a);
    if (c !== 32 && c !== 9) break;
    a++;
  }
  while (z > a) {
    const c = line.charCodeAt(z - 1);
    if (c !== 32 && c !== 9) break;
    z--;
  }
  return a === 0 && z === line.length ? line : line.slice(a, z);
}

// ---------------------------------------------------------------- one-byte text
//
// runL1(input) builds from one-byte text: every string holds the UTF-8 of the real one, one byte per char (a Latin-1
// reading of UTF-8), so its output, written out as Latin-1, is the UTF-8 the real build writes. Every rule works on
// it unchanged except those that look at what a non-ASCII character is; those read L1 and take the bytes as UTF-8:
// slugs (NFKD), JavaScript's \s (word counts, trims, link targets, list continuation lines), code-unit order (sorts)
// and template parsing. main.ts uses it only for input that is valid UTF-8 with no U+2028 or U+2029 (which a regex
// "." stops at) and no \u escape of a non-ASCII character.

let L1 = false;
/** One-byte text's real string. */
const dec = (s) => Buffer.from(s, 'latin1').toString('utf8');
/** A real string's one-byte text. */
const enc = (s) => Buffer.from(s, 'utf8').toString('latin1');

/**
 * In one-byte text s, the byte length of the JavaScript \s character whose UTF-8 starts at i with byte c (>= 0x80),
 * or 0: U+00A0, U+1680, U+2000-U+200A, U+2028, U+2029, U+202F, U+205F, U+3000 and U+FEFF.
 */
function wsAt(s, i, c) {
  if (c === 0xc2) return s.charCodeAt(i + 1) === 0xa0 ? 2 : 0;
  if (c === 0xe2) {
    const d = s.charCodeAt(i + 1);
    const e = s.charCodeAt(i + 2);
    if (d === 0x80) return (e >= 0x80 && e <= 0x8a) || e === 0xa8 || e === 0xa9 || e === 0xaf ? 3 : 0;
    return d === 0x81 && e === 0x9f ? 3 : 0;
  }
  if (c === 0xe1) return s.charCodeAt(i + 1) === 0x9a && s.charCodeAt(i + 2) === 0x80 ? 3 : 0;
  if (c === 0xe3) return s.charCodeAt(i + 1) === 0x80 && s.charCodeAt(i + 2) === 0x80 ? 3 : 0;
  if (c === 0xef) return s.charCodeAt(i + 1) === 0xbb && s.charCodeAt(i + 2) === 0xbf ? 3 : 0;
  return 0;
}

/** String.prototype.trim of one-byte text (its \s and line terminators dropped at either end). */
function trimL1(s) {
  let a = 0;
  let z = s.length;
  while (a < z) {
    const c = s.charCodeAt(a);
    if (c === 32 || (c >= 9 && c <= 13)) a++;
    else if (c >= 0xc2) {
      const w = wsAt(s, a, c);
      if (w === 0) break;
      a += w;
    } else break;
  }
  while (z > a) {
    const c = s.charCodeAt(z - 1);
    if (c === 32 || (c >= 9 && c <= 13)) z--;
    else if (c === 0xa0 && z - 2 >= a && s.charCodeAt(z - 2) === 0xc2) z -= 2;
    else if (c >= 0x80 && z - 3 >= a && wsAt(s, z - 3, s.charCodeAt(z - 3)) === 3) z -= 3;
    else break;
  }
  return a === 0 && z === s.length ? s : s.slice(a, z);
}

const trimU = (s) => (L1 ? trimL1(s) : s.trim());

/** Whether one-byte text holds a JavaScript \s character. */
function hasSpaceL1(s) {
  const n = s.length;
  for (let i = 0; i < n; i++) {
    const c = s.charCodeAt(i);
    if (c === 32 || (c >= 9 && c <= 13)) return true;
    if (c >= 0xc2 && wsAt(s, i, c) !== 0) return true;
  }
  return false;
}

/** The count of /\S+/g matches in one-byte text. */
function wordsOfL1(text) {
  let count = 0;
  let inWord = false;
  const n = text.length;
  for (let i = 0; i < n; i++) {
    const c = text.charCodeAt(i);
    if (c > 32 && c < 0xc2) {
      if (!inWord) {
        inWord = true;
        count++;
      }
    } else if (c <= 32 ? c === 32 || (c >= 9 && c <= 13) : wsAt(text, i, c) !== 0) {
      inWord = false;
      if (c > 32) i += wsAt(text, i, c) - 1;
    } else if (!inWord) {
      inWord = true;
      count++;
    }
  }
  return count;
}

/** /^ {2,}\S/ on one-byte text. */
function continuedL1(line) {
  let k = 0;
  while (line.charCodeAt(k) === 32) k++;
  if (k < 2 || k >= line.length) return false;
  const c = line.charCodeAt(k);
  if (c < 0x80) return !(c >= 9 && c <= 13);
  return c < 0xc2 || wsAt(line, k, c) === 0;
}

/** Whether one-byte strings a < b (as bytes) order the other way as UTF-16 code units: U+E000-U+FFFF against U+10000 up. */
function l1Inverted(a, b) {
  const n = Math.min(a.length, b.length);
  let i = 0;
  while (i < n && a.charCodeAt(i) === b.charCodeAt(i)) i++;
  if (i === n) return false;
  const x = a.charCodeAt(i);
  return (x === 0xee || x === 0xef) && b.charCodeAt(i) >= 0xf0;
}

/** Whether a line is only spaces and tabs. */
function isBlank(line) {
  for (let i = 0; i < line.length; i++) {
    const c = line.charCodeAt(i);
    if (c !== 32 && c !== 9) return false;
  }
  return true;
}

// ---------------------------------------------------------------- front matter (makefun-site's own grammar)

const KEYS = ['title', 'date', 'tags', 'draft', 'layout', 'slug', 'summary'];
const FM_LINE = /^([a-z_]+):(?: (.*))?$/;
const DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const NAME = /^[a-z0-9-]+$/;
const SLUG = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const TAG = /^[^"[\]]+$/;

function daysIn(y, m) {
  if (m === 2) return (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0 ? 29 : 28;
  return m === 4 || m === 6 || m === 9 || m === 11 ? 30 : 31;
}

function stringOf(raw) {
  if (raw.startsWith('"')) {
    if (raw.length < 2 || !raw.endsWith('"')) return { error: 'a quoted string is not closed' };
    let out = '';
    for (let i = 1; i < raw.length - 1; i++) {
      const c = raw[i];
      if (c === '\\') {
        const n = raw[i + 1];
        if ((n !== '"' && n !== '\\') || i + 1 === raw.length - 1) return { error: 'a quoted string holds an escape other than \\" or \\\\' };
        out += n;
        i++;
      } else if (c === '"') return { error: 'a quoted string holds a stray quote' };
      else out += c;
    }
    return { value: out };
  }
  if (raw.startsWith('[')) return { error: 'a bare string opens on [' };
  return { value: raw };
}

function valueOf(key, raw) {
  switch (key) {
    case 'title':
    case 'summary': {
      const s = stringOf(raw);
      return 'error' in s ? { error: `key "${key}": ${s.error}` } : s;
    }
    case 'date': {
      const m = DATE.exec(raw);
      const y = m ? Number(m[1]) : 0;
      const mo = m ? Number(m[2]) : 0;
      const d = m ? Number(m[3]) : 0;
      if (!m || y < 1970 || y > 2099 || mo < 1 || mo > 12 || d < 1 || d > daysIn(y, mo)) return { error: 'key "date": not a date from 1970-01-01 to 2099-12-31 (YYYY-MM-DD)' };
      return { value: raw };
    }
    case 'tags': {
      if (!raw.startsWith('[') || !raw.endsWith(']')) return { error: 'key "tags": not a list [a, b]' };
      const inner = raw.slice(1, -1);
      if (trimU(inner) === '') return { value: [] };
      const tags = inner.split(',').map(trimU);
      if (tags.some((t) => t === '' || !TAG.test(t))) return { error: 'key "tags": an empty tag, or one holding " [ or ]' };
      return { value: tags };
    }
    case 'draft':
      return raw === 'true' ? { value: true } : raw === 'false' ? { value: false } : { error: 'key "draft": true or false' };
    case 'layout':
      return NAME.test(raw) ? { value: raw } : { error: 'key "layout": not a template name' };
    case 'slug':
      return SLUG.test(raw) && raw.length <= 64 ? { value: raw } : { error: 'key "slug": not a slug' };
    default:
      return { error: `unknown key "${key}"` };
  }
}

/** A source's { data, body } or { error }; `text` has "\n" line ends. */
function parseFrontMatter(text) {
  if (!text.startsWith('---') || (text.length > 3 && text.charCodeAt(3) !== 10)) return { error: 'no front matter' };
  const lines = [];
  let pos = 4;
  let bodyAt = -1;
  const n = text.length;
  while (pos <= n && n > 3) {
    let nl = text.indexOf('\n', pos);
    if (nl === -1) nl = n;
    const line = text.slice(pos, nl);
    if (line === '---') {
      bodyAt = nl + 1;
      break;
    }
    lines.push(line);
    pos = nl + 1;
  }
  if (bodyAt === -1) return { error: 'the front matter is not closed' };
  const data = {};
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (isBlank(line) || line.startsWith('#')) continue;
    const m = FM_LINE.exec(line);
    if (!m) return { error: `line ${i + 2} is not "key: value"` };
    const key = m[1];
    const raw = (m[2] ?? '').replace(/[ \t]+$/, '');
    if (!KEYS.includes(key)) return { error: `unknown key "${key}"` };
    if (Object.hasOwn(data, key)) return { error: `key "${key}" is given twice` };
    if (raw === '') return { error: `key "${key}" has no value` };
    const v = valueOf(key, raw);
    if ('error' in v) return v;
    data[key] = v.value;
  }
  if (!Object.hasOwn(data, 'title')) return { error: 'no title' };
  return { data, body: bodyAt >= n ? '' : text.slice(bodyAt) };
}

// ---------------------------------------------------------------- Markdown (makefun-site's subset)

const FENCE_OPEN = /^```([A-Za-z0-9_+-]*)[ \t]*$/;
const FENCE_CLOSE = /^```[ \t]*$/;
const HEADING = /^(#{1,6})[ \t]+(.*?)[ \t]*$/;
const HR = /^(?:---|\*\*\*)[ \t]*$/;
const BULLET = /^[-*] (.*)$/;
const ORDERED = /^(\d{1,9})\. (.*)$/;
const CONTINUED = /^ {2,}\S/;
const SPACE_RE = /\s/;
const SCHEME = /^[A-Za-z][A-Za-z0-9+.-]*:/;

function headingOf(line) {
  if (line.charCodeAt(0) !== 35) return null;
  const m = HEADING.exec(line);
  return m === null || m[2] === '' ? null : m;
}

function endsParagraph(line) {
  if (line.length === 0) return true;
  const c = line.charCodeAt(0);
  if (c === 32 || c === 9) return isBlank(line);
  if (c === 96) return FENCE_OPEN.test(line);
  if (c === 35) return headingOf(line) !== null;
  if (c === 45 || c === 42) return HR.test(line);
  return false;
}

const ESCAPABLE = new Uint8Array(128);
for (const ch of '\\`*_[]()#+-.!>') ESCAPABLE[ch.charCodeAt(0)] = 1;

/** First ']' from `from` outside a backslash escape, or -1. */
function findClose(s, from) {
  const n = s.length;
  for (let j = from; j < n; j++) {
    const c = s.charCodeAt(j);
    if (c === 92 && j + 1 < n) {
      const d = s.charCodeAt(j + 1);
      if (d < 128 && ESCAPABLE[d] === 1) {
        j++;
        continue;
      }
    }
    if (c === 93) return j;
  }
  return -1;
}

/** First '**' from `from` outside a backslash escape, or -1. */
function findDouble(s, from) {
  const n = s.length;
  for (let j = from; j <= n - 2; j++) {
    const c = s.charCodeAt(j);
    if (c === 92) {
      const d = s.charCodeAt(j + 1);
      if (d < 128 && ESCAPABLE[d] === 1) j++;
      continue;
    }
    if (c === 42 && s.charCodeAt(j + 1) === 42) return j;
  }
  return -1;
}

/** First single '*' from `from` (its run not '**'), outside an escape, or -1. */
function findSingleStar(s, from) {
  const n = s.length;
  for (let j = from; j < n; j++) {
    const c = s.charCodeAt(j);
    if (c === 92) {
      if (j + 1 < n) {
        const d = s.charCodeAt(j + 1);
        if (d < 128 && ESCAPABLE[d] === 1) j++;
      }
      continue;
    }
    if (c !== 42) continue;
    if (s.charCodeAt(j + 1) === 42) {
      j++;
      continue;
    }
    return j;
  }
  return -1;
}

const UNESC = { '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#39;': "'" };
const ENTITY = /&(?:amp|lt|gt|quot|#39);/g;
const unesc = (s) => (s.indexOf('&') === -1 ? s : s.replace(ENTITY, (m) => UNESC[m]));

/**
 * makefun-site's inline Markdown for raw text t, given `s` = esc(t): the same HTML. Escaping first is exact here:
 * entities hold none of the characters the inline rules look for (\ ` * [ ] ( ) and whitespace), and of the characters
 * a backslash escapes only ">" changes, so "\&gt;" is the escaped ">" and every other "\&" is a literal backslash.
 */
function inline(s, link) {
  let out = '';
  let run = 0;
  let i = 0;
  const n = s.length;
  while (i < n) {
    const c = s.charCodeAt(i);
    if (c === 92) {
      if (i + 1 < n) {
        const d = s.charCodeAt(i + 1);
        if (d < 128 && ESCAPABLE[d] === 1) {
          if (i > run) out += s.slice(run, i);
          run = i + 1;
          i += 2;
          continue;
        }
        if (d === 38 && s.startsWith('gt;', i + 2)) {
          if (i > run) out += s.slice(run, i);
          run = i + 1;
          i += 5;
          continue;
        }
      }
      i++;
      continue;
    }
    if (c === 96) {
      const j = s.indexOf('`', i + 1);
      if (j > i + 1) {
        if (i > run) out += s.slice(run, i);
        out += `<code>${s.slice(i + 1, j)}</code>`;
        i = j + 1;
        run = i;
      } else i++;
      continue;
    }
    if (c === 42) {
      if (s.charCodeAt(i + 1) === 42) {
        const j = findDouble(s, i + 2);
        if (j > i + 2) {
          if (i > run) out += s.slice(run, i);
          out += `<strong>${inline(s.slice(i + 2, j), link)}</strong>`;
          i = j + 2;
          run = i;
        } else i += 2;
        continue;
      }
      const j = findSingleStar(s, i + 1);
      if (j > i + 1) {
        if (i > run) out += s.slice(run, i);
        out += `<em>${inline(s.slice(i + 1, j), link)}</em>`;
        i = j + 1;
        run = i;
      } else i++;
      continue;
    }
    if (c === 91) {
      const k = findClose(s, i + 1);
      const m = k !== -1 && s.charCodeAt(k + 1) === 40 ? s.indexOf(')', k + 2) : -1;
      if (m !== -1 && m > k + 2) {
        const target = s.slice(k + 2, m);
        if (!(L1 ? hasSpaceL1(target) : SPACE_RE.test(target))) {
          const raw = unesc(target);
          const href = link(raw);
          if (i > run) out += s.slice(run, i);
          out += `<a href="${href === raw ? target : esc(href)}">${inline(s.slice(i + 1, k), link)}</a>`;
          i = m + 1;
          run = i;
          continue;
        }
      }
      i++;
      continue;
    }
    i++;
  }
  if (run === 0) return s;
  return run < n ? out + s.slice(run) : out;
}

/** Whether char code c is a space or a tab. */
const isST = (c) => c === 32 || c === 9;

/**
 * `src` rendered: { html, toc, excerpt }, as makefun-site's lex and renderMarkdown, with lines read by position in src
 * (split at "\n", as lex splits them) rather than copied out.
 */
function renderMarkdown(src, link) {
  const ids = new Set();
  const toc = [];
  let excerpt = null;
  const parts = [];
  const n = src.length;
  const LS = [];
  const LE = [];
  for (let p = 0; ; ) {
    const q = src.indexOf('\n', p);
    LS.push(p);
    if (q === -1) {
      LE.push(n);
      break;
    }
    LE.push(q);
    p = q + 1;
  }
  const L = LS.length;
  const line = (k) => src.slice(LS[k], LE[k]);
  const blank = (k) => {
    const z = LE[k];
    for (let f = LS[k]; f < z; f++) if (!isST(src.charCodeAt(f))) return false;
    return true;
  };
  /** Whether line k ends a paragraph: blank, a fence, a heading or a thematic break. */
  const ends = (k) => {
    const a = LS[k];
    if (a === LE[k]) return true;
    const c = src.charCodeAt(a);
    if (isST(c)) return blank(k);
    if (c === 96) return FENCE_OPEN.test(line(k));
    if (c === 35) return headingOf(line(k)) !== null;
    if (c === 45 || c === 42) return HR.test(line(k));
    return false;
  };
  /** Line k with spaces and tabs at either end dropped. */
  const trimmed = (k) => {
    let a = LS[k];
    let z = LE[k];
    while (a < z && isST(src.charCodeAt(a))) a++;
    while (z > a && isST(src.charCodeAt(z - 1))) z--;
    return src.slice(a, z);
  };
  let i = 0;
  while (i < L) {
    if (blank(i)) {
      i++;
      continue;
    }
    const c0 = src.charCodeAt(LS[i]);
    if (c0 === 96) {
      const fence = FENCE_OPEN.exec(line(i));
      if (fence !== null) {
        i++;
        const first = i;
        while (i < L && !(LS[i] < LE[i] && src.charCodeAt(LS[i]) === 96 && FENCE_CLOSE.test(line(i)))) i++;
        const body = i > first ? esc(`${src.slice(LS[first], LE[i - 1])}\n`) : '';
        i++;
        const lang = fence[1];
        parts.push(lang === '' ? '<pre><code>' : `<pre><code class="language-${esc(lang)}">`, body, '</code></pre>\n');
        continue;
      }
    }
    if (c0 === 35) {
      const h = headingOf(line(i));
      if (h !== null) {
        const depth = h[1].length;
        const text = h[2];
        const id = uniqueSlug(slugifyRaw(text), ids);
        const ih = inline(esc(text), link);
        parts.push(`<h${depth} id="${id}">`, ih, `</h${depth}>\n`);
        if (depth === 2 || depth === 3) toc.push({ level: depth, id, html: ih });
        i++;
        continue;
      }
    }
    if ((c0 === 45 || c0 === 42) && HR.test(line(i))) {
      parts.push('<hr>\n');
      i++;
      continue;
    }
    if (c0 === 62) {
      const kept = [];
      while (i < L && LS[i] < LE[i] && src.charCodeAt(LS[i]) === 62) {
        let a = LS[i] + 1;
        const z = LE[i];
        if (a < z && src.charCodeAt(a) === 32) a++;
        while (a < z && isST(src.charCodeAt(a))) a++;
        let e = z;
        while (e > a && isST(src.charCodeAt(e - 1))) e--;
        if (e > a) kept.push(src.slice(a, e));
        i++;
      }
      parts.push(kept.length === 0 ? '<blockquote>\n</blockquote>\n' : `<blockquote>\n<p>${inline(esc(kept.join('\n')), link)}</p>\n</blockquote>\n`);
      continue;
    }
    if (c0 === 45 || c0 === 42 || (c0 >= 48 && c0 <= 57)) {
      const first = line(i);
      const bullet = c0 === 45 || c0 === 42 ? BULLET.exec(first) : null;
      const ordered = bullet === null && c0 >= 48 && c0 <= 57 ? ORDERED.exec(first) : null;
      if (bullet !== null || ordered !== null) {
        const isOrdered = ordered !== null;
        const marker = isOrdered ? ORDERED : BULLET;
        const start = isOrdered ? Number(ordered[1]) : 1;
        parts.push(isOrdered ? (start !== 1 ? `<ol start="${start}">\n` : '<ol>\n') : '<ul>\n');
        let cur = null;
        while (i < L) {
          const ln = line(i);
          const m = marker.exec(ln);
          if (m !== null) {
            if (cur !== null) parts.push('<li>', inline(esc(cur), link), '</li>\n');
            cur = trimLine(isOrdered ? m[2] : m[1]);
            i++;
          } else if (L1 ? continuedL1(ln) : CONTINUED.test(ln)) {
            cur += `\n${trimLine(ln)}`;
            i++;
          } else break;
        }
        parts.push('<li>', inline(esc(cur), link), isOrdered ? '</li>\n</ol>\n' : '</li>\n</ul>\n');
        continue;
      }
    }
    // A paragraph: this line and the next ones up to one that ends it, each trimmed, joined by "\n".
    const first = i;
    let clean = true;
    do {
      if (isST(src.charCodeAt(LS[i])) || isST(src.charCodeAt(LE[i] - 1))) clean = false;
      i++;
    } while (i < L && !ends(i));
    let text;
    if (clean) text = src.slice(LS[first], LE[i - 1]);
    else {
      text = trimmed(first);
      for (let k = first + 1; k < i; k++) text += `\n${trimmed(k)}`;
    }
    const ih = inline(esc(text), link);
    if (excerpt === null) excerpt = ih;
    parts.push('<p>', ih, '</p>\n');
  }
  return { html: parts.join(''), toc, excerpt: excerpt ?? '' };
}

// ---------------------------------------------------------------- templates (makefun-site's dialect)

const PATH = /^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$/;
const FOR = /^for ([A-Za-z_][A-Za-z0-9_]*) in (\S+)$/;
const IF = /^if (not )?(\S+)$/;
const INCLUDE = /^include "([a-z0-9-]+)"$/;
const WS_ONLY = /^[ \t]*$/;

class TemplateError extends Error {}

function pathOf(name, text) {
  if (!PATH.test(text)) throw new TemplateError(`template "${name}": ${JSON.stringify(text)} is not a path`);
  return text.split('.');
}

function tokenize(name, src) {
  const tokens = [];
  let text = '';
  let i = 0;
  while (i < src.length) {
    const open = src.indexOf('{', i);
    if (open === -1 || open === src.length - 1) {
      text += src.slice(i);
      break;
    }
    const next = src[open + 1];
    if (next !== '{' && next !== '%') {
      text += src.slice(i, open + 1);
      i = open + 1;
      continue;
    }
    text += src.slice(i, open);
    if (next === '{') {
      const raw = src[open + 2] === '{';
      const close = src.indexOf(raw ? '}}}' : '}}', open + (raw ? 3 : 2));
      if (close === -1) throw new TemplateError(`template "${name}": a ${raw ? '{{{' : '{{'} is not closed`);
      if (text !== '') tokens.push({ t: 'text', text });
      text = '';
      tokens.push({ t: raw ? 'raw' : 'var', path: pathOf(name, src.slice(open + (raw ? 3 : 2), close).trim()) });
      i = close + (raw ? 3 : 2);
      continue;
    }
    const close = src.indexOf('%}', open + 2);
    if (close === -1) throw new TemplateError(`template "${name}": a {% is not closed`);
    const end = close + 2;
    const lineStart = src.lastIndexOf('\n', open - 1) + 1;
    const nl = src.indexOf('\n', end);
    const lineEnd = nl === -1 ? src.length : nl;
    const standalone = WS_ONLY.test(src.slice(lineStart, open)) && WS_ONLY.test(src.slice(end, lineEnd));
    if (standalone) text = text.slice(0, text.length - (open - lineStart));
    if (text !== '') tokens.push({ t: 'text', text });
    text = '';
    tokens.push({ t: 'tag', tag: src.slice(open + 2, close).trim() });
    i = standalone ? (nl === -1 ? src.length : nl + 1) : end;
  }
  if (text !== '') tokens.push({ t: 'text', text });
  return tokens;
}

function parseTemplate(name, src) {
  const root = { kind: 'root', body: [] };
  const stack = [root];
  let body = root.body;
  for (const tok of tokenize(name, src)) {
    if (tok.t === 'text') body.push({ kind: 'text', text: tok.text });
    else if (tok.t === 'var' || tok.t === 'raw') body.push({ kind: tok.t, path: tok.path });
    else {
      const tag = tok.tag;
      const top = stack[stack.length - 1];
      let m;
      if ((m = FOR.exec(tag)) !== null) {
        const node = { kind: 'for', name: m[1], path: pathOf(name, m[2]), body: [] };
        body.push(node);
        stack.push(node);
        body = node.body;
      } else if ((m = IF.exec(tag)) !== null) {
        const node = { kind: 'if', not: m[1] !== undefined, path: pathOf(name, m[2]), then: [], else: null };
        body.push(node);
        stack.push(node);
        body = node.then;
      } else if (tag === 'else') {
        if (top.kind !== 'if' || top.else !== null) throw new TemplateError(`template "${name}": an else outside an if`);
        top.else = [];
        body = top.else;
      } else if (tag === 'endfor' || tag === 'endif') {
        if (top.kind !== (tag === 'endfor' ? 'for' : 'if')) throw new TemplateError(`template "${name}": an ${tag} with no open ${tag.slice(3)}`);
        stack.pop();
        const parent = stack[stack.length - 1];
        body = parent.kind === 'root' ? parent.body : parent.kind === 'for' ? parent.body : parent.else ?? parent.then;
      } else if ((m = INCLUDE.exec(tag)) !== null) body.push({ kind: 'include', name: m[1] });
      else throw new TemplateError(`template "${name}": {% ${tag} %} is not a tag`);
    }
  }
  if (stack.length > 1) throw new TemplateError(`template "${name}": a ${stack[stack.length - 1].kind} is not closed`);
  return root.body;
}

function textOf(v) {
  if (typeof v === 'string') return v;
  if (typeof v === 'number') return String(v);
  if (typeof v === 'boolean') return v ? 'true' : 'false';
  if (Array.isArray(v)) return v.map(textOf).join(', ');
  return '';
}

function truthy(v) {
  if (Array.isArray(v)) return v.length > 0;
  return v !== undefined && v !== null && v !== false && v !== '' && v !== 0;
}

/** The value at the property path `keys` (from index 1) under `v`: own properties only, as makefun-site's lookup. */
function walk(v, keys) {
  for (let k = 1; k < keys.length; k++) {
    if (v === null || typeof v !== 'object' || !Object.hasOwn(v, keys[k])) return undefined;
    v = v[keys[k]];
  }
  return v;
}

/** The top-level names each kind of render gives (beside site, tags and recent). */
const TOP = { page: ['page'], index: ['posts', 'pager'], tag: ['tag'], tags: [] };
const GLOBAL_NAMES = ['site', 'tags', 'recent'];

class TooBig extends Error {}

/**
 * Compiles the parsed templates for one build. A compiled piece is { fn, deps, item }: fn(e) renders it, where e[0] is
 * the render's own top-level vars and e[2L-1], e[2L] are the item and loop record of the for at level L; deps is a Set of
 * what the piece reads that varies ("V" a top-level name of the render, "I<L>" or "L<L>" a loop's item or record).
 */
function compiler(templates, globals) {
  const memo = new Map();
  let count = 0;

  const resolve = (chain, kind, name) => {
    for (let L = chain.length; L >= 1; L--) {
      if (name === 'loop') return { at: 2 * L, dep: `L${L}` };
      if (name === chain[L - 1]) return { at: 2 * L - 1, dep: `I${L}` };
    }
    if (GLOBAL_NAMES.includes(name)) return { value: globals[name] };
    if (TOP[kind].includes(name)) return { top: name, dep: 'V' };
    return { value: undefined };
  };

  /** A getter for `path` (or its constant value) and the dep it adds. */
  const getter = (chain, kind, path) => {
    const r = resolve(chain, kind, path[0]);
    if ('value' in r) return { constant: true, value: walk(r.value, path) };
    if (r.top !== undefined) {
      const key = r.top;
      if (path.length === 1) return { get: (e) => e[0][key], dep: r.dep };
      return { get: (e) => walk(e[0][key], path), dep: r.dep };
    }
    const at = r.at;
    if (path.length === 1) return { get: (e) => e[at], dep: r.dep };
    if (path.length === 2) {
      const k1 = path[1];
      return { get: (e) => {
        const v = e[at];
        return v === null || typeof v !== 'object' || !Object.hasOwn(v, k1) ? undefined : v[k1];
      }, dep: r.dep };
    }
    return { get: (e) => walk(e[at], path), dep: r.dep };
  };

  const constPiece = (s) => ({ fn: () => s, deps: new Set(), str: s });

  /** A piece that renders once and is then kept, for pieces with no varying deps. */
  const once = (fn) => {
    let cached = null;
    return { fn: (e) => (cached === null ? (cached = fn(e)) : cached), deps: new Set() };
  };

  function compileNode(n, chain, kind, depth) {
    switch (n.kind) {
      case 'text':
        return constPiece(n.text);
      case 'var':
      case 'raw': {
        const g = getter(chain, kind, n.path);
        const raw = n.kind === 'raw';
        if (g.constant) return constPiece(raw ? textOf(g.value) : esc(textOf(g.value)));
        const get = g.get;
        const fn = raw
          ? (e) => {
              const v = get(e);
              return typeof v === 'string' ? v : textOf(v);
            }
          : (e) => {
              const v = get(e);
              return esc(typeof v === 'string' ? v : textOf(v));
            };
        return { fn, deps: new Set([g.dep]) };
      }
      case 'if': {
        const g = getter(chain, kind, n.path);
        const then = compileBody(n.then, chain, kind, depth);
        const other = n.else === null ? null : compileBody(n.else, chain, kind, depth);
        const not = n.not;
        if (g.constant) {
          const branch = truthy(g.value) !== not ? then : other;
          return branch === null ? constPiece('') : branch;
        }
        const get = g.get;
        const tf = then.fn;
        const of = other === null ? null : other.fn;
        const deps = new Set([g.dep, ...then.deps, ...(other === null ? [] : other.deps)]);
        const fn = of === null
          ? (e) => (truthy(get(e)) !== not ? tf(e) : '')
          : (e) => (truthy(get(e)) !== not ? tf(e) : of(e));
        return { fn, deps };
      }
      case 'for': {
        const g = getter(chain, kind, n.path);
        const L = chain.length + 1;
        const inner = [...chain, n.name];
        const body = compileBody(n.body, inner, kind, depth);
        const deps = new Set();
        if (!g.constant) deps.add(g.dep);
        const mine = `I${L}`;
        const myLoop = `L${L}`;
        let usesLoop = false;
        for (const d of body.deps) {
          if (d === myLoop) usesLoop = true;
          else if (d !== mine) deps.add(d);
        }
        const bf = body.fn;
        const atItem = 2 * L - 1;
        const atLoop = 2 * L;
        const loopIsItem = n.name === 'loop';
        const iterate = (xs, e) => {
          if (!Array.isArray(xs)) return '';
          let out = '';
          const len = xs.length;
          for (let k = 0; k < len; k++) {
            if (usesLoop || loopIsItem) {
              const rec = { index: k + 1, first: k === 0, last: k === len - 1, length: len };
              e[atLoop] = rec;
              e[atItem] = loopIsItem ? rec : xs[k];
            } else e[atItem] = xs[k];
            out += bf(e);
          }
          return out;
        };
        if (g.constant) {
          const xs = g.value;
          return withCache({ fn: (e) => iterate(xs, e), deps }, chain);
        }
        const get = g.get;
        return withCache({ fn: (e) => iterate(get(e), e), deps }, chain);
      }
      case 'include': {
        if (depth >= INCLUDE_DEPTH || !templates.has(n.name)) return constPiece('');
        return withCache(compileTemplate(n.name, chain, kind, depth + 1), chain);
      }
    }
    throw new Error(`unknown node ${n.kind}`);
  }

  /** The piece cached by what it reads: once per build, or once per loop item when it reads one item alone. */
  function withCache(piece, chain) {
    if ('str' in piece) return piece;
    const deps = piece.deps;
    if (deps.size === 0) return once(piece.fn);
    if (deps.size === 1) {
      const d = deps.values().next().value;
      if (d[0] === 'I') {
        const at = 2 * Number(d.slice(1)) - 1;
        const fn = piece.fn;
        const cache = new Map();
        return {
          fn: (e) => {
            const x = e[at];
            let r = cache.get(x);
            if (r === undefined) {
              r = fn(e);
              cache.set(x, r);
            }
            return r;
          },
          deps,
        };
      }
    }
    return piece;
  }

  function compileBody(nodes, chain, kind, depth) {
    const parts = [];
    for (const n of nodes) {
      const p = compileNode(n, chain, kind, depth);
      const last = parts[parts.length - 1];
      if ('str' in p && last !== undefined && 'str' in last) parts[parts.length - 1] = constPiece(last.str + p.str);
      else if ('str' in p && p.str === '') continue;
      else parts.push(p);
    }
    if (parts.length === 0) return constPiece('');
    if (parts.length === 1) return parts[0];
    const deps = new Set();
    for (const p of parts) for (const d of p.deps) deps.add(d);
    const fns = parts.map((p) => p.fn);
    let fn;
    if (fns.length === 2) {
      const [a, b] = fns;
      fn = (e) => a(e) + b(e);
    } else if (fns.length === 3) {
      const [a, b, c] = fns;
      fn = (e) => a(e) + b(e) + c(e);
    } else {
      const m = fns.length;
      fn = (e) => {
        let out = '';
        for (let k = 0; k < m; k++) out += fns[k](e);
        return out;
      };
    }
    const piece = { fn, deps };
    return deps.size === 0 ? once(fn) : piece;
  }

  function compileTemplate(name, chain, kind, depth) {
    const key = `${name}|${depth}|${kind}|${chain.join(',')}`;
    let p = memo.get(key);
    if (p === undefined) {
      if (++count > 20000) throw new TooBig();
      p = compileBody(templates.get(name), chain, kind, depth);
      memo.set(key, p);
    }
    return p;
  }

  return (name, kind) => compileTemplate(name, [], kind, 0).fn;
}

// ---------------------------------------------------------------- the reference interpreter, a fallback for huge template graphs

function lookupScope(scope, path) {
  let s = scope;
  while (s !== null && !Object.hasOwn(s.vars, path[0])) s = s.parent;
  if (s === null) return undefined;
  return walk(s.vars[path[0]], path);
}

function renderNodes(nodes, scope, templates, depth) {
  let out = '';
  for (const n of nodes) {
    switch (n.kind) {
      case 'text':
        out += n.text;
        break;
      case 'var':
        out += esc(textOf(lookupScope(scope, n.path)));
        break;
      case 'raw':
        out += textOf(lookupScope(scope, n.path));
        break;
      case 'if': {
        const branch = truthy(lookupScope(scope, n.path)) !== n.not ? n.then : n.else;
        if (branch !== null) out += renderNodes(branch, scope, templates, depth);
        break;
      }
      case 'for': {
        const xs = lookupScope(scope, n.path);
        if (!Array.isArray(xs)) break;
        xs.forEach((x, k) => {
          const vars = { [n.name]: x, loop: { index: k + 1, first: k === 0, last: k === xs.length - 1, length: xs.length } };
          out += renderNodes(n.body, { vars, parent: scope }, templates, depth);
        });
        break;
      }
      case 'include': {
        const t = templates.get(n.name);
        if (t !== undefined && depth < INCLUDE_DEPTH) out += renderNodes(t, scope, templates, depth + 1);
        break;
      }
    }
  }
  return out;
}

// ---------------------------------------------------------------- the build

const byCodeUnits = (a, b) => (a < b ? (L1 && l1Inverted(a, b) ? 1 : -1) : a > b ? (L1 && l1Inverted(b, a) ? -1 : 1) : 0);

/** parseTemplate on one-byte text: parsed as the real text (its trims are \s ones), its text and errors one-byte again. */
function parseTemplateL1(name, src) {
  let nodes;
  try {
    nodes = parseTemplate(dec(name), dec(src));
  } catch (err) {
    if (err instanceof TemplateError) throw new TemplateError(enc(err.message));
    throw err;
  }
  const fix = (list) => {
    for (const n of list) {
      if (n.kind === 'text') n.text = enc(n.text);
      else if (n.kind === 'for') fix(n.body);
      else if (n.kind === 'if') {
        fix(n.then);
        if (n.else !== null) fix(n.else);
      }
    }
  };
  fix(nodes);
  return nodes;
}

/**
 * `rel` from directory `dir`, or null when it climbs out of the root: leading "../" steps taken off `dir`, then the
 * rest appended as it is when it holds no ".", ".." or empty segment; anything else goes the general way.
 */
function resolvePath(dir, rel) {
  if (dir.indexOf('//') === -1 && dir.charCodeAt(0) !== 47 && dir.charCodeAt(dir.length - 1) !== 47) {
    let base = dir;
    let at = 0;
    while (rel.startsWith('../', at)) {
      if (base === '') return null;
      const k = base.lastIndexOf('/');
      base = k === -1 ? '' : base.slice(0, k);
      at += 3;
    }
    const rest = at === 0 ? rel : rel.slice(at);
    const c0 = rest.charCodeAt(0);
    if (rest !== '' && c0 !== 46 && c0 !== 47 && rest.charCodeAt(rest.length - 1) !== 47 && rest.indexOf('/.') === -1 && rest.indexOf('//') === -1) {
      return base === '' ? rest : `${base}/${rest}`;
    }
  }
  return resolvePathSlow(dir, rel);
}

function resolvePathSlow(dir, rel) {
  const parts = dir === '' ? [] : dir.split('/');
  for (const seg of rel.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') {
      if (parts.length === 0) return null;
      parts.pop();
    } else parts.push(seg);
  }
  return parts.join('/');
}

const dirOf = (path) => {
  const k = path.lastIndexOf('/');
  return k === -1 ? '' : path.slice(0, k);
};

/** One site's output tree: path -> text, exactly as makefun-site 1.0.0's build. */
export function run(input) {
  const slugCache = new Map();
  const slugify = (text) => {
    let s = slugCache.get(text);
    if (s === undefined) {
      s = slugifyRaw(text);
      slugCache.set(text, s);
    }
    return s;
  };

  // Templates, parsed in name order; one that does not parse is reported.
  const templates = new Map();
  const report = [];
  for (const name of Object.keys(input.templates).sort(byCodeUnits)) {
    try {
      templates.set(name, L1 ? parseTemplateL1(name, input.templates[name]) : parseTemplate(name, input.templates[name]));
    } catch (err) {
      if (!(err instanceof TemplateError)) throw err;
      report.push(err.message);
    }
  }
  const base = input.site.base_url.replace(/\/+$/, '');

  // 1. Sources in code-unit order of path; .md are sources, the rest static.
  const files = [...input.files].sort((x, y) => byCodeUnits(x.path, y.path));
  const statics = [];
  let sources = [];
  for (const f of files) {
    if (f.path.endsWith('.md')) {
      const c = f.content;
      sources.push({ path: f.path, text: c.indexOf('\r') === -1 ? c : c.replace(/\r\n?/g, '\n') });
    } else statics.push(f);
  }

  // 2. Front matter.
  const kept = [];
  for (const s of sources) {
    const fm = parseFrontMatter(s.text);
    if ('error' in fm) {
      report.push(`refused ${s.path}: ${fm.error}`);
      continue;
    }
    s.data = fm.data;
    s.body = fm.body;
    s.post = s.path.startsWith('posts/');
    if (s.path.startsWith('tags/')) {
      report.push(`refused ${s.path}: tags/ is the generator's own`);
      continue;
    }
    if (s.path.startsWith('page/')) {
      report.push(`refused ${s.path}: page/ is the generator's own`);
      continue;
    }
    if (s.post && !Object.hasOwn(s.data, 'date')) {
      report.push(`refused ${s.path}: a post needs a date`);
      continue;
    }
    const layout = s.data.layout ?? (s.post ? 'post' : 'page');
    if (!templates.has(layout)) {
      report.push(`refused ${s.path}: layout "${layout}" is no template that parses`);
      continue;
    }
    s.layout = layout;
    if (s.data.draft === true) {
      report.push(`draft ${s.path}`);
      continue;
    }
    kept.push(s);
  }
  sources = kept;

  // 3. Permalinks.
  const taken = new Map();
  const bySource = new Map();
  for (const s of sources) {
    const dir = s.post ? 'posts' : dirOf(s.path);
    let set = taken.get(dir);
    if (set === undefined) {
      set = new Set(dir === '' ? RESERVED : []);
      taken.set(dir, set);
    }
    const slug = uniqueSlug(s.data.slug ?? slugify(s.data.title), set);
    s.slug = slug;
    s.outDir = dir === '' ? slug : `${dir}/${slug}`;
    s.url = `${base}/${s.outDir}/`;
    bySource.set(s.path, s);
  }

  // 4. Markdown, links resolved.
  const staticPaths = new Set(statics.map((f) => f.path));
  const broken = new Set();
  for (const s of sources) {
    const dir = dirOf(s.path);
    const link = (target) => {
      const c = target.charCodeAt(0);
      if (c === 47 || c === 35 || SCHEME.test(target)) return target;
      const hash = target.indexOf('#');
      const rel = hash === -1 ? target : target.slice(0, hash);
      const frag = hash === -1 ? '' : target.slice(hash);
      const path = rel === '' ? null : resolvePath(dir, rel);
      if (path !== null) {
        const hit = bySource.get(path);
        if (hit !== undefined) return `${hit.url}${frag}`;
        if (staticPaths.has(path)) return `${base}/${path}${frag}`;
      }
      broken.add(`broken-link ${s.path} -> ${target}`);
      return target;
    };
    const md = renderMarkdown(s.body, link);
    s.html = md.html;
    s.toc = md.toc;
    s.excerpt = md.excerpt;
  }

  // 5. Posts, newest first then by slug.
  const posts = sources.filter((s) => s.post);
  posts.sort((x, y) => byCodeUnits(y.data.date, x.data.date) || byCodeUnits(x.slug, y.slug));
  for (let k = 0; k < posts.length; k++) {
    posts[k].newer = k === 0 ? null : posts[k - 1];
    posts[k].older = k === posts.length - 1 ? null : posts[k + 1];
  }

  // 6. Tags.
  const bySlug = new Map();
  const ownTags = (s) => {
    const list = s.data.tags;
    if (list === undefined || list.length === 0) return [];
    const seen = new Set();
    const out = [];
    for (const name of list) {
      const slug = slugify(name);
      if (seen.has(slug)) continue;
      seen.add(slug);
      out.push(name);
    }
    return out;
  };
  for (const p of posts) {
    p.tagRefs = [];
    for (const name of ownTags(p)) {
      const slug = slugify(name);
      let tag = bySlug.get(slug);
      if (tag === undefined) {
        tag = { name, slug, url: `${base}/tags/${slug}/`, posts: [] };
        bySlug.set(slug, tag);
      }
      tag.posts.push(p);
      p.tagRefs.push(tag);
    }
  }
  for (const s of sources) {
    if (!s.post) s.tagRefs = ownTags(s).map((name) => bySlug.get(slugify(name)) ?? { name, slug: slugify(name), url: '', posts: [] });
  }
  const tagList = [...bySlug.values()].sort((x, y) => y.posts.length - x.posts.length || byCodeUnits(x.slug, y.slug));

  // 7. Layouts.
  const cardOf = (s) => ({
    title: s.data.title,
    url: s.url,
    date: s.data.date ?? '',
    date_long: s.data.date === undefined ? '' : longDate(s.data.date),
    summary: s.data.summary ?? '',
    excerpt: s.excerpt,
    reading_minutes: Math.max(1, Math.ceil((L1 ? wordsOfL1(s.body) : wordsOf(s.body)) / 200)),
    tags: s.tagRefs.map((t) => ({ name: t.name, slug: t.slug, url: t.url })),
  });
  const linkOf = (s) => (s === null ? null : { title: s.data.title, url: s.url, date: s.data.date ?? '', date_long: s.data.date === undefined ? '' : longDate(s.data.date) });
  const cards = new Map();
  for (const p of posts) cards.set(p, cardOf(p));
  const site = { title: input.site.title, base_url: base };
  const allTags = tagList.map((t) => ({ name: t.name, slug: t.slug, url: t.url, count: t.posts.length }));
  const recent = posts.slice(0, input.site.recent).map((p) => cards.get(p));
  const globals = { site, tags: allTags, recent };

  let renderTop;
  try {
    const compile = compiler(templates, globals);
    const fns = new Map();
    renderTop = (name, kind, vars) => {
      if (!templates.has(name)) return null;
      const key = `${name}|${kind}`;
      let fn = fns.get(key);
      if (fn === undefined) {
        fn = compile(name, kind);
        fns.set(key, fn);
      }
      return fn([vars]);
    };
    // Compile every template a render will use up front, so a TooBig falls back before any output.
    for (const name of new Set(sources.map((s) => s.layout))) compile(name, 'page');
    if (templates.has('index')) compile('index', 'index');
    if (templates.has('tag')) compile('tag', 'tag');
    if (templates.has('tags')) compile('tags', 'tags');
  } catch (err) {
    if (!(err instanceof TooBig)) throw err;
    renderTop = (name, kind, vars) => {
      const nodes = templates.get(name);
      return nodes === undefined ? null : renderNodes(nodes, { vars: { site, tags: allTags, recent, ...vars }, parent: null }, templates, 0);
    };
  }

  const output = {};
  for (const s of sources) {
    const page = {
      ...(s.post ? cards.get(s) : cardOf(s)),
      slug: s.slug,
      content: s.html,
      toc: s.toc.map((h) => ({ level: h.level, id: h.id, html: h.html })),
      newer: linkOf(s.newer ?? null),
      older: linkOf(s.older ?? null),
      is_post: s.post,
    };
    output[`${s.outDir}/index.html`] = renderTop(s.layout, 'page', { page });
  }
  const per = input.site.per_page;
  const pages = Math.max(1, Math.ceil(posts.length / per));
  const indexUrl = (k) => (k === 1 ? `${base}/` : `${base}/page/${k}/`);
  if (templates.has('index')) {
    for (let k = 1; k <= pages; k++) {
      const html = renderTop('index', 'index', {
        posts: posts.slice((k - 1) * per, k * per).map((p) => cards.get(p)),
        pager: { number: k, total: pages, url: indexUrl(k), newer_url: k === 1 ? '' : indexUrl(k - 1), older_url: k === pages ? '' : indexUrl(k + 1) },
      });
      output[k === 1 ? 'index.html' : `page/${k}/index.html`] = html;
    }
  }
  if (templates.has('tag')) {
    for (const t of tagList) {
      const tag = { name: t.name, slug: t.slug, url: t.url, count: t.posts.length, posts: t.posts.map((p) => cards.get(p)) };
      output[`tags/${t.slug}/index.html`] = renderTop('tag', 'tag', { tag });
    }
  }
  if (templates.has('tags')) output['tags/index.html'] = renderTop('tags', 'tags', {});
  for (const f of statics) {
    if (Object.hasOwn(output, f.path)) report.push(`shadowed ${f.path}: a built page has its path`);
    else output[f.path] = f.content;
  }

  // 8. The report.
  const lines = report.concat([...broken]).sort(byCodeUnits);
  let nPages = 0;
  for (const s of sources) if (!s.post) nPages++;
  lines.push(`built ${nPages} pages, ${posts.length} posts, ${tagList.length} tags and ${statics.length} static files`);
  output[REPORT_FILE] = `${lines.join('\n')}\n`;
  return output;
}

/** makefun-site's slug of one-byte text (for the WebAssembly build's non-ASCII titles, tags and headings). */
export function slugL1(text) {
  L1 = true;
  try {
    return slugifyRaw(text);
  } finally {
    L1 = false;
  }
}

/** run on one-byte text (see "one-byte text" above): the same tree as one-byte text. */
export function runL1(input) {
  L1 = true;
  try {
    return run(input);
  } finally {
    L1 = false;
  }
}
