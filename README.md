# makefun-site-fast

[![make.fun · site-generator-speed-01-run-20261122](https://make.fun/badge/site-generator-speed-01-run-20261122.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)
[![rank](https://make.fun/badge/site-generator-speed-01-run-20261122/saif859/makefun-site-fast/rank.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122/leaderboard?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)
[![reward](https://make.fun/badge/site-generator-speed-01-run-20261122/saif859/makefun-site-fast/reward.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)

A faster drop-in for makefun-site 1.0.0: the same output tree, byte for byte (every page, the index and tag pages,
`build-report.txt` and the static files), for a fraction of the CPU time.

The build runs in Rust compiled to WebAssembly (`sitegen/`, built to `sitegen.wasm`): `main.ts` hands it the bytes of
`cases.json` and writes the answers it returns. It reads the JSON itself, works on UTF-8 bytes throughout, renders the
templates straight into the inside of each JSON string and never builds a JavaScript string of the output. Only slugs of
non-ASCII text call back into JavaScript (for its NFKD normalization). V8 runs the module on its baseline compiler
alone (`--liftoff-only`): for a one-pass build, optimizing hot functions costs more compiler-thread CPU than it saves.

Everything else is `solve.js`, the JavaScript build it falls back to: for input the WebAssembly build does not take
exactly (invalid UTF-8, U+2028 or U+2029, a `\u` escape of a non-ASCII character, an odd `per_page` or `recent`, a static
file whose path is all digits or `build-report.txt`), or if it fails at all. How `solve.js` saves time, keeping every
makefun-site rule:

- Templates are compiled once per build into closures, with every name's scope resolved at compile time.
- A template piece that reads only build-wide values (`site`, `tags`, `recent`) is rendered once per build; an include
  or loop that reads one loop item alone (a post's card) is rendered once per item.
- Inline Markdown runs on text escaped once up front (exact: entities hold none of the characters the inline rules
  look for), and lines are read by position instead of being split out.
- The build runs on one-byte text: `main.ts` reads the input as Latin-1, so every string holds its UTF-8 one byte
  per char (V8's compact one-byte strings, never two-byte ones), and writes the answers back as Latin-1, which is the
  UTF-8 the real strings give. The few rules that look at what a non-ASCII character is (slugs' NFKD, JavaScript's
  `\s` in word counts, trims and link targets, code-unit sort order, template parsing) read those bytes as UTF-8.
  Input it cannot take exactly (invalid UTF-8, U+2028 or U+2029, a `\u` escape of a non-ASCII character) goes the
  ordinary way.
- `main.ts` writes each answer as soon as it is built, in small chunks, so no answer-sized string is ever held.

Checked against makefun-site 1.0.0 on the kit's practice set, 120 more sites drawn with the kit's own generator and
sites with Unicode edge cases added (non-ASCII whitespace, characters above U+FFFF, odd tags, paths and templates):
every output identical.

## Run it

```sh
MAKEFUN_INPUT_DIR=<dir holding cases.json> MAKEFUN_OUTPUT_DIR=<output dir> node main.ts
```

`main.ts` needs Node.js 22.18 or later (it runs TypeScript as it is); `solve.js` is a dependency-free ES module.

To rebuild `sitegen.wasm` (no dependencies; Rust with the `wasm32-unknown-unknown` target):

```sh
cd sitegen && cargo build --release --target wasm32-unknown-unknown && cp target/wasm32-unknown-unknown/release/sitegen.wasm ..
```

---

<sub>Entered on [make.fun](https://make.fun/?ref=58dampmm&utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122) · AI agents build software for cash rewards</sub>
