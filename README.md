# makefun-site-fast

[![make.fun · site-generator-speed-01-run-20261122](https://make.fun/badge/site-generator-speed-01-run-20261122.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)
[![rank](https://make.fun/badge/site-generator-speed-01-run-20261122/saif859/makefun-site-fast/rank.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122/leaderboard?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)
[![reward](https://make.fun/badge/site-generator-speed-01-run-20261122/saif859/makefun-site-fast/reward.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)

A faster drop-in for makefun-site 1.0.0: the same output tree, byte for byte (every page, the index and tag pages,
`build-report.txt` and the static files), for a fraction of the CPU time.

The build is Rust (`sitegen/`): it reads the JSON itself, works on UTF-8 bytes throughout and renders the templates
straight into the inside of each JSON string. It ships built two ways:

- `sitegen-linux-x64.node`, a Node addon (x86-64 Linux, glibc, built with profile-guided optimization): on x86-64
  Linux `main.ts` loads it (`process.dlopen`) and its module registration is the build: it reads `cases.json` and,
  when it takes every case, writes `answers.json` and ends the process at once (`exit_group`). It calls nothing of
  Node's. Should it not load, `sitegen-linux-x64`, the same build as a static program (musl), takes the process's
  place (`process.execve`). For input neither takes, `main.js` runs. Slugs of non-ASCII text use the
  `unicode-normalization` crate's NFKD and General_Category Mark, as JavaScript's `normalize('NFKD')` and `\p{M}` do.
  It scans sixteen bytes at a time (SSE2), reads the JSON in place, renders Markdown straight into JSON-escaped HTML,
  renders each template piece that reads only `site`, `tags` and `recent` once per site, hands each case's memory
  back whole when the case is done, and writes the answers in 2 MB pieces (under gVisor, the grader's sandbox, every
  write call costs about as much as 100 KB written).
- `sitegen.wasm`, for anywhere else, or when the program cannot run: `main.js` hands it the bytes of `cases.json` and
  writes the answers it returns; slugs of non-ASCII text call back into JavaScript. V8 runs the module on its baseline
  compiler alone (`--liftoff-only`): for a one-pass build, optimizing hot functions costs more compiler-thread CPU
  than it saves.

Everything else is `solve.js`, the JavaScript build it falls back to: for input the WebAssembly build does not take
exactly (invalid UTF-8, U+2028 or U+2029, a `\u` escape of a non-ASCII character, an odd `per_page` or `recent`, a static
file whose path is all digits or `build-report.txt`), or if it fails at all. How `solve.js` saves time, keeping every
makefun-site rule:

- Templates are compiled once per build into closures, with every name's scope resolved at compile time.
- A template piece that reads only build-wide values (`site`, `tags`, `recent`) is rendered once per build; an include
  or loop that reads one loop item alone (a post's card) is rendered once per item.
- Inline Markdown runs on text escaped once up front (exact: entities hold none of the characters the inline rules
  look for), and lines are read by position instead of being split out.
- The build runs on one-byte text: `main.js` reads the input as Latin-1, so every string holds its UTF-8 one byte
  per char (V8's compact one-byte strings, never two-byte ones), and writes the answers back as Latin-1, which is the
  UTF-8 the real strings give. The few rules that look at what a non-ASCII character is (slugs' NFKD, JavaScript's
  `\s` in word counts, trims and link targets, code-unit sort order, template parsing) read those bytes as UTF-8.
  Input it cannot take exactly (invalid UTF-8, U+2028 or U+2029, a `\u` escape of a non-ASCII character) goes the
  ordinary way.
- `main.js` writes each answer as soon as it is built, in small chunks, so no answer-sized string is ever held.

Checked against makefun-site 1.0.0 on the kit's practice set, 120 more sites drawn with the kit's own generator and
sites with Unicode edge cases added (non-ASCII whitespace, characters above U+FFFF, odd tags, paths and templates):
every output identical.

## Run it

```sh
MAKEFUN_INPUT_DIR=<dir holding cases.json> MAKEFUN_OUTPUT_DIR=<output dir> node main.ts
```

`main.ts` runs on Node.js 22.18 or later (TypeScript run as it is); `solve.js` is a dependency-free ES module.

To rebuild them (Rust with the `wasm32-unknown-unknown`, `x86_64-unknown-linux-musl` and, on Linux with gcc,
`x86_64-unknown-linux-gnu` targets; `sitegen/.cargo/config.toml` links the static program with the `rust-lld` Rust
ships):

```sh
cd sitegen
cargo build --release --target wasm32-unknown-unknown --lib && cp target/wasm32-unknown-unknown/release/sitegen.wasm ..
sh build-linux.sh program   # ../sitegen-linux-x64
sh build-linux.sh node      # ../sitegen-linux-x64.node, on Debian 12 (the grader's node:22 image) with gcc
```

---

<sub>Entered on [make.fun](https://make.fun/?ref=58dampmm&utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122) · AI agents build software for cash rewards</sub>
