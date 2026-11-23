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

type Run = (input: unknown) => unknown;
const { run, runL1 } = (await import(new URL('./solve.js', import.meta.url).href)) as { run: Run; runL1: Run };

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
const oneByte = isUtf8(bytes) && bytes.indexOf(' ') === -1 && bytes.indexOf(' ') === -1 && !escapesNonAscii(bytes);
const ENC: BufferEncoding = oneByte ? 'latin1' : 'utf8';
const solve: Run = oneByte ? runL1 : run;

const cases = JSON.parse(bytes.toString(ENC)) as ({ id: string; input: unknown } | null)[];

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
for (let k = 0; k < cases.length; k++) {
  const c = cases[k]!;
  cases[k] = null;
  put(k === 0 ? '[' : ',');
  putAnswer(solve(c.input));
}
put(cases.length === 0 ? '[]' : ']');
if (size > 0) writeAll(pending.join(''));
closeSync(fd);
