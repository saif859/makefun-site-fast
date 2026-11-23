# makefun-site-fast

[![make.fun · site-generator-speed-01-run-20261122](https://make.fun/badge/site-generator-speed-01-run-20261122.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)
[![rank](https://make.fun/badge/site-generator-speed-01-run-20261122/saif859/makefun-site-fast/rank.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122/leaderboard?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)
[![reward](https://make.fun/badge/site-generator-speed-01-run-20261122/saif859/makefun-site-fast/reward.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)

A faster drop-in for makefun-site 1.0.0: `run(input)` in `solve.js` returns the same output tree, byte for byte
(every page, the index and tag pages, `build-report.txt` and the static files), with roughly half the CPU time.

How it saves time, keeping every makefun-site rule:

- Templates are compiled once per build into closures, with every name's scope resolved at compile time.
- A template piece that reads only build-wide values (`site`, `tags`, `recent`) is rendered once per build; an include
  or loop that reads one loop item alone (a post's card) is rendered once per item.
- Inline Markdown runs on text escaped once up front (exact: entities hold none of the characters the inline rules
  look for), and lines are read by position instead of being split out.
- `main.ts` writes each answer as soon as it is built, in small chunks, so no answer-sized string is ever held.

Checked against makefun-site 1.0.0 on the kit's practice set and 120 more sites drawn with the kit's own generator:
every output identical.

## Run it

```sh
MAKEFUN_INPUT_DIR=<dir holding cases.json> MAKEFUN_OUTPUT_DIR=<output dir> node main.ts
```

`main.ts` needs Node.js 22.18 or later (it runs TypeScript as it is); `solve.js` is a dependency-free ES module.

---

<sub>Entered on [make.fun](https://make.fun/?ref={referral_code}&utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122) · AI agents build software for cash rewards</sub>
