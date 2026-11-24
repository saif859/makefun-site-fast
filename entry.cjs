// The program the harness runs. The harness runs `node /bot/main.ts` (node22), and main.ts is a symbolic link to this
// file: Node runs a main module from its real path, so it loads this file as CommonJS JavaScript and has no
// TypeScript to strip (stripping main.ts's types would load Node's TypeScript compiler, amaro, which costs more CPU
// than the whole build). On x86-64 Linux the build runs in this process, in Rust: loading sitegen-linux-x64.node
// (process.dlopen) runs it, and when it takes the input it writes answers.json and ends the process there and then.
// Should the addon not load, this process becomes sitegen-linux-x64, the same build as a program (process.execve).
// Input neither takes, and anywhere else, main.js builds here.
'use strict';
if (process.platform === 'linux' && process.arch === 'x64' && !process.env.SITEGEN_NO_NATIVE) {
  let loaded = false;
  try {
    process.dlopen({ exports: {} }, `${__dirname}/sitegen-linux-x64.node`);
    loaded = true;
  } catch {
    // Not loaded here: the program below.
  }
  if (loaded) {
    // It ran and did not take the input; neither would the program.
    process.env.SITEGEN_NO_NATIVE = '1';
  } else if (typeof process.execve === 'function') {
    const program = `${__dirname}/sitegen-linux-x64`;
    const input = `${process.env.MAKEFUN_INPUT_DIR ?? '/input'}/cases.json`;
    const output = `${process.env.MAKEFUN_OUTPUT_DIR ?? '/output'}/answers.json`;
    try {
      // A failed execve ends the process (Node aborts), so only an executable program is run.
      require('node:fs').accessSync(program, require('node:fs').constants.X_OK);
      process.emitWarning = () => {}; // process.execve is experimental in Node 22: no warning for it
      process.execve(program, [program, input, output, process.execPath, `${__dirname}/main.js`]);
    } catch {
      // Not run (no such program, or not executable here): main.js below.
    }
  }
}
import('./main.js').catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
