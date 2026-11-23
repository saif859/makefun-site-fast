// The harness runs `node /bot/main.ts` (node22). On x86-64 Linux the build runs in this process, in Rust: loading
// sitegen-linux-x64.node (process.dlopen) runs it, and when it takes the input it writes answers.json and ends the
// process there and then. Should the addon not load, this process becomes sitegen-linux-x64, the same build as a
// program (process.execve). Input neither takes, and anywhere else, main.js builds here.
if (process.platform === 'linux' && process.arch === 'x64' && !process.env.SITEGEN_NO_NATIVE) {
  const here = (name: string): string => decodeURIComponent(new URL(name, import.meta.url).pathname);
  let loaded = false;
  try {
    process.dlopen({ exports: {} }, here('./sitegen-linux-x64.node'));
    loaded = true;
  } catch {
    // Not loaded here: the program below.
  }
  if (loaded) {
    // It ran and did not take the input; neither would the program.
    process.env.SITEGEN_NO_NATIVE = '1';
  } else if (typeof process.execve === 'function') {
    const program = here('./sitegen-linux-x64');
    const input = `${process.env.MAKEFUN_INPUT_DIR ?? '/input'}/cases.json`;
    const output = `${process.env.MAKEFUN_OUTPUT_DIR ?? '/output'}/answers.json`;
    try {
      process.emitWarning = () => {}; // process.execve is experimental in Node 22: no warning for it
      process.execve(program, [program, input, output, process.execPath, here('./main.js')]);
    } catch {
      // Not run (no such program, or not executable here): main.js below.
    }
  }
}
await import('./main.js');
