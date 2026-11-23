// The harness runs `node /bot/main.ts` (node22). It reads every case from /input/cases.json (a JSON array of
// {id, input}), answers each one alone with run() from solve.js, and writes /output/answers.json: a JSON array of the
// answers, one per case and in the cases' order. Outside a container MAKEFUN_INPUT_DIR and MAKEFUN_OUTPUT_DIR stand in
// for /input and /output.

import { isUtf8 } from 'node:buffer';
import { closeSync, openSync, readFileSync, writeSync } from 'node:fs';
import { join } from 'node:path';
import { setFlagsFromString } from 'node:v8';

// V8's incremental marking off, set before solve.js is imported (as the house baseline does).
setFlagsFromString('--no-incremental-marking');
// sitegen.wasm runs on Liftoff, V8's baseline WebAssembly compiler, alone: the build is one pass, and tiering its hot
// functions up to TurboFan costs more CPU (on compiler threads, which count) than the faster code saves.
setFlagsFromString('--liftoff-only');

type Run = (input: unknown) => unknown;
/** solve.js, loaded only when it builds a case. */
const loadSolve = async (): Promise<{ run: Run; runL1: Run }> => (await import(new URL('./solve.js', import.meta.url).href)) as { run: Run; runL1: Run };

const MARKS = /\p{M}/gu;
/** makefun-site's slug of non-ASCII text (as solve.js's slugifyRaw): NFKD, marks dropped, A-Z lowered, runs of other characters one "-", at most 64, "untitled". */
const slugOf = (text: string): string => {
  const plain = text.normalize('NFKD').replace(MARKS, '');
  let out = '';
  let gap = false;
  for (let i = 0; i < plain.length; i++) {
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
};

const INPUT_DIR = process.env.MAKEFUN_INPUT_DIR ?? '/input';
const OUTPUT_DIR = process.env.MAKEFUN_OUTPUT_DIR ?? '/output';

const bytes = readFileSync(join(INPUT_DIR, 'cases.json'));

/** Whether the JSON holds a \u escape of a non-ASCII character (an escaped backslash before a "u" counts too). */
const escapesNonAscii = (b: Buffer): boolean => {
  for (let at = b.indexOf('\\u'); at !== -1; at = b.indexOf('\\u', at + 2)) {
    const hex = b.toString('latin1', at + 2, at + 6);
    if (!/^00[0-7][0-9a-fA-F]$/.test(hex)) return true;
  }
  return false;
};

// One-byte mode (solve.js's runL1): the input read as Latin-1, so every string holds its UTF-8 one byte per char, and
// the answers written as Latin-1, which is the UTF-8 the real strings give. It takes input that is valid UTF-8 with no
// U+2028 or U+2029 and no \u escape of a non-ASCII character; anything else goes the ordinary way.
const oneByte = isUtf8(bytes) && bytes.indexOf('\u2028') === -1 && bytes.indexOf('\u2029') === -1 && !escapesNonAscii(bytes);
const ENC: BufferEncoding = oneByte ? 'latin1' : 'utf8';

type Case = { id: string; input: unknown } | null;

// The WebAssembly build (sitegen.wasm, from the Rust in sitegen/): makefun-site's build over cases.json's bytes, each
// case's answer written as the JSON object this file writes for it. It takes the input one-byte mode takes; a case it
// does not take exactly it leaves to solve.js, and if it fails at all solve.js builds every case.
type Built = { mem: Uint8Array; answers: ({ start: number; end: number } | null)[] };
const buildWasm = (): Built | null => {
  try {
    const module = new WebAssembly.Module(readFileSync(new URL('./sitegen.wasm', import.meta.url)));
    let memory: WebAssembly.Memory | null = null;
    const env = {
      // A slug of non-ASCII text, by solve.js (JavaScript's NFKD): its ASCII written at `out`, its length returned.
      js_slug: (p: number, n: number, out: number): number => {
        const m = new Uint8Array(memory!.buffer);
        const slug = slugOf(Buffer.from(m.buffer, p, n).toString('utf8'));
        for (let k = 0; k < slug.length; k++) m[out + k] = slug.charCodeAt(k);
        return slug.length;
      },
    };
    const x = new WebAssembly.Instance(module, { env }).exports as {
      memory: WebAssembly.Memory;
      alloc: (n: number) => number;
      build: (p: number, n: number) => number;
      out_ptr: () => number;
      case_info: (i: number, k: number) => number;
    };
    memory = x.memory;
    const p = x.alloc(bytes.length);
    new Uint8Array(memory.buffer, p, bytes.length).set(bytes);
    const count = x.build(p, bytes.length);
    if (count < 0) return null;
    const base = x.out_ptr();
    const answers = [];
    for (let i = 0; i < count; i++) answers.push(x.case_info(i, 0) === 1 ? { start: base + x.case_info(i, 1), end: base + x.case_info(i, 2) } : null);
    return { mem: new Uint8Array(memory.buffer), answers };
  } catch {
    return null;
  }
};
const built = oneByte ? buildWasm() : null;
let solve: Run = () => null;
if (built === null || built.answers.some((a) => a === null)) {
  const m = await loadSolve();
  solve = oneByte ? m.runL1 : m.run;
}
if (process.env.SITEGEN_TRACE) process.stderr.write(`sitegen: ${built === null ? 'not used' : `${built.answers.filter((a) => a !== null).length} of ${built.answers.length} cases`}\n`);
let parsed: Case[] | null = null;
/** The cases, parsed (when solve.js builds any). */
const casesOf = (): Case[] => (parsed ??= JSON.parse(bytes.toString(ENC)) as Case[]);

// The answers array is written one answer at a time, each as soon as it is built, so only one case's output is held
// in memory at once (the file is the same JSON array JSON.stringify(answers) would give, value for value).
const fd = openSync(join(OUTPUT_DIR, 'answers.json'), 'w');
/** `text` written whole: as a string, then whatever a short write left, from its bytes. */
const writeAll = (text: string): void => {
  const done = writeSync(fd, text, null, ENC);
  if (done === (oneByte ? text.length : Buffer.byteLength(text, 'utf8'))) return;
  const buf = Buffer.from(text, ENC);
  let off = done;
  while (off < buf.length) off += writeSync(fd, buf, off, buf.length - off);
};
// Text goes out in chunks of about CHUNK characters (small enough to stay out of V8's large-object space), so no
// string the size of a whole answer is ever built.
const CHUNK = 1 << 15;
let pending: string[] = [];
let size = 0;
const put = (text: string): void => {
  if (text.length >= CHUNK) {
    if (size > 0) {
      writeAll(pending.join(''));
      pending = [];
      size = 0;
    }
    writeAll(text);
    return;
  }
  pending.push(text);
  size += text.length;
  if (size >= CHUNK) {
    writeAll(pending.join(''));
    pending = [];
    size = 0;
  }
};
/** One answer as JSON: an object member by member (as JSON.stringify writes it), anything else whole. */
const putAnswer = (answer: unknown): void => {
  if (answer === null || typeof answer !== 'object' || Array.isArray(answer) || typeof (answer as { toJSON?: unknown }).toJSON === 'function') {
    put(answer === undefined ? 'null' : JSON.stringify(answer));
    return;
  }
  let first = true;
  for (const key of Object.keys(answer)) {
    const json = JSON.stringify((answer as Record<string, unknown>)[key]);
    if (json === undefined) continue;
    put(`${first ? '{' : ','}${JSON.stringify(key)}:`);
    put(json);
    first = false;
  }
  put(first ? '{}' : '}');
};
/** Bytes written whole, after the text waiting to go out. */
const putBytes = (buf: Uint8Array, start: number, end: number): void => {
  if (size > 0) {
    writeAll(pending.join(''));
    pending = [];
    size = 0;
  }
  let off = start;
  while (off < end) off += writeSync(fd, buf, off, end - off);
};
if (built !== null) {
  put('[');
  built.answers.forEach((a, k) => {
    if (k > 0) put(',');
    if (a !== null) putBytes(built.mem, a.start, a.end);
    else {
      const cases = casesOf();
      const c = cases[k]!;
      cases[k] = null;
      putAnswer(solve(c.input));
    }
  });
  put(']');
} else {
  const cases = casesOf();
  for (let k = 0; k < cases.length; k++) {
    const c = cases[k]!;
    cases[k] = null;
    put(k === 0 ? '[' : ',');
    putAnswer(solve(c.input));
  }
  put(cases.length === 0 ? '[]' : ']');
}
if (size > 0) writeAll(pending.join(''));
closeSync(fd);
