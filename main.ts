// The harness runs `node /bot/main.ts` (node22). On x86-64 Linux this process becomes sitegen-linux-x64 at once
// (process.execve, so nothing more of Node runs): the Rust build writes answers.json itself, and for input it does not
// take exactly it becomes `node main.js` in turn. Anywhere else, or if that cannot be done, main.js runs here.
if (process.platform === 'linux' && process.arch === 'x64' && !process.env.SITEGEN_NO_NATIVE && typeof process.execve === 'function') {
  const here = (name: string): string => decodeURIComponent(new URL(name, import.meta.url).pathname);
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
await import('./main.js');
