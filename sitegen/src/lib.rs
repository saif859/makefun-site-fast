//! makefun-site 1.0.0's build in Rust. As WebAssembly, main.js hands it cases.json's bytes and writes the answers it
//! builds; a case it does not take exactly (see build::Input::from_json and build::run) is marked for main.js to build
//! in JavaScript instead, and slugs of non-ASCII text call back into JavaScript for its NFKD (js_slug). As a Linux
//! program (main.rs), it builds the whole answers file or none of it.

pub mod build;
mod json;
mod md;
mod text;
mod tpl;

use std::cell::RefCell;

/// The allocator: blocks handed out in order from big chunks (WebAssembly memory grown 16 MB at a time, or 64 MB
/// from the system), freeing is nothing and the last block grows in place. A case's blocks are all given back at
/// once when it is done (mark and release), so one case's memory is the next one's.
pub mod arena {
    use std::alloc::{GlobalAlloc, Layout};
    use std::cell::Cell;

    #[cfg(feature = "timing")]
    thread_local! { pub static TAKEN: Cell<usize> = const { Cell::new(0) }; }
    #[cfg(feature = "timing")]
    thread_local! { pub static PEAK: Cell<usize> = const { Cell::new(0) }; }
    #[cfg(feature = "timing")]
    thread_local! { pub static CHUNK0: Cell<usize> = const { Cell::new(0) }; }
    #[cfg(feature = "timing")]
    pub fn peak() -> usize {
        PEAK.with(|p| p.get())
    }
    #[cfg(feature = "timing")]
    pub fn taken() -> usize {
        TAKEN.with(|t| t.get())
    }
    #[cfg(feature = "timing")]
    pub fn high() -> (usize, usize) {
        (super::ALLOC.next.get(), super::ALLOC.end.get())
    }

    pub struct Arena {
        next: Cell<usize>,
        end: Cell<usize>,
        /// Blocks given back, one list per size class, each block holding the next one's address: they are handed
        /// out again before fresh memory is (under gVisor every fresh page costs about as much as 2 µs of work).
        free: [Cell<usize>; CLASSES],
    }

    /// Size classes: 16 bytes apart up to 1 KB (classes 1 to 64), then powers of two up to BIG (65 to 78). A block
    /// of a class is that class's size, 16-byte aligned. Larger blocks, and any aligned beyond 16, have no class:
    /// they are taken at their own size and never reused.
    const CLASSES: usize = 79;
    const BIG: usize = 16 << 20;
    #[inline(always)]
    fn class(size: usize) -> usize {
        if size <= 1024 {
            (size + 15) >> 4
        } else {
            64 + size.next_power_of_two().trailing_zeros() as usize - 10
        }
    }
    #[inline(always)]
    fn class_size(c: usize) -> usize {
        if c <= 64 {
            c << 4
        } else {
            1 << (c - 64 + 10)
        }
    }
    #[inline(always)]
    fn classed(l: &Layout) -> bool {
        l.align() <= 16 && l.size() <= BIG
    }

    // The build runs on one thread.
    unsafe impl Sync for Arena {}

    /// Fresh memory of at least `min` bytes: (start, end). On WebAssembly it starts where memory ended.
    #[cfg(target_arch = "wasm32")]
    fn more(min: usize) -> Option<(usize, usize)> {
        const PAGE: usize = 0x10000;
        let pages = (min.max(16 << 20) + PAGE - 1) / PAGE;
        let old = core::arch::wasm32::memory_grow(0, pages);
        if old == usize::MAX {
            return None;
        }
        Some((old * PAGE, (old + pages) * PAGE))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn more(min: usize) -> Option<(usize, usize)> {
        #[cfg(all(feature = "probe", target_os = "linux"))]
        if let Some(r) = probe_pop::populated(min) {
            return Some(r);
        }
        let size = (min + 4096).max(64 << 20);
        let p = unsafe { std::alloc::System.alloc(Layout::from_size_align(size, 4096).ok()?) };
        if p.is_null() {
            return None;
        }
        #[cfg(all(feature = "probe", target_os = "linux"))]
        probe_pop::first(p as usize);
        Some((p as usize, p as usize + size))
    }

    /// Under the probe feature: SG_POP=<MB> maps the first chunk that size with MAP_POPULATE (every page present
    /// before the build touches it), and the arena's high-water mark in its first chunk is kept for the report.
    #[cfg(all(feature = "probe", target_os = "linux"))]
    pub mod probe_pop {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
        static DONE: AtomicBool = AtomicBool::new(false);
        static START: AtomicUsize = AtomicUsize::new(0);
        static PEAK: AtomicUsize = AtomicUsize::new(0);
        extern "C" {
            fn getenv(n: *const u8) -> *const u8;
            fn mmap(a: *mut u8, l: usize, p: i32, f: i32, fd: i32, o: i64) -> *mut u8;
        }
        pub fn first(start: usize) {
            let _ = START.compare_exchange(0, start, Relaxed, Relaxed);
        }
        pub fn seen(next: usize) {
            let s = START.load(Relaxed);
            if s != 0 && next >= s && next - s < (1 << 32) {
                PEAK.fetch_max(next - s, Relaxed);
            }
        }
        pub fn peak() -> usize {
            PEAK.load(Relaxed)
        }
        static FREED: AtomicUsize = AtomicUsize::new(0);
        static ALLOCS: AtomicUsize = AtomicUsize::new(0);
        /// Bytes given back (or left behind by a realloc that moved) and blocks taken.
        pub fn count(freed: usize, allocs: usize) {
            FREED.fetch_add(freed, Relaxed);
            ALLOCS.fetch_add(allocs, Relaxed);
        }
        pub fn counts() -> (usize, usize) {
            (FREED.load(Relaxed), ALLOCS.load(Relaxed))
        }
        pub fn populated(min: usize) -> Option<(usize, usize)> {
            if DONE.swap(true, Relaxed) {
                return None;
            }
            let v = unsafe { getenv(b"SG_POP\0".as_ptr()) };
            if v.is_null() {
                return None;
            }
            let (mut mb, mut p) = (0usize, v);
            unsafe {
                while (*p).is_ascii_digit() {
                    mb = mb * 10 + (*p - b'0') as usize;
                    p = p.add(1);
                }
            }
            if mb == 0 {
                return None;
            }
            let size = (mb << 20).max(min + 4096);
            // PROT_READ | PROT_WRITE; MAP_PRIVATE | MAP_ANONYMOUS | MAP_POPULATE
            let a = unsafe { mmap(core::ptr::null_mut(), size, 3, 0x22 | 0x8000, -1, 0) };
            if a as isize == -1 {
                return None;
            }
            first(a as usize);
            Some((a as usize, a as usize + size))
        }
    }

    impl Arena {
        pub const fn new() -> Arena {
            Arena { next: Cell::new(0), end: Cell::new(0), free: [const { Cell::new(0) }; CLASSES] }
        }

        unsafe fn take(&self, size: usize, align: usize) -> *mut u8 {
            #[cfg(feature = "timing")]
            {
                TAKEN.with(|t| t.set(t.get() + size));
                PEAK.with(|p| p.set(p.get().max(self.next.get().wrapping_sub(CHUNK0.with(|c| c.get())).min(1 << 40))));
            }
            let a = (self.next.get() + align - 1) & !(align - 1);
            if self.next.get() != 0 && a + size <= self.end.get() {
                self.next.set(a + size);
                #[cfg(all(feature = "probe", target_os = "linux"))]
                probe_pop::seen(a + size);
                return a as *mut u8;
            }
            let Some((start, end)) = more(size + align) else { return core::ptr::null_mut() };
            #[cfg(feature = "timing")]
            CHUNK0.with(|c| if c.get() == 0 { c.set(start) });
            // Memory that carries on from the chunk in use extends it; any other is a new chunk.
            if start != self.end.get() || self.next.get() == 0 {
                self.next.set(start);
            }
            self.end.set(end);
            let a = (self.next.get() + align - 1) & !(align - 1);
            if a + size > end {
                return core::ptr::null_mut();
            }
            self.next.set(a + size);
            a as *mut u8
        }

        pub fn mark(&self) -> (usize, usize) {
            (self.next.get(), self.end.get())
        }

        /// Every block taken since `m` given back: none of them may be in use any more.
        pub fn release(&self, m: (usize, usize)) {
            if self.end.get() == m.1 && m.0 != 0 {
                self.next.set(m.0);
                // Blocks given back may lie past the mark, in memory that is fresh again now.
                for f in &self.free {
                    f.set(0);
                }
            }
        }
    }

    unsafe impl GlobalAlloc for Arena {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            #[cfg(all(feature = "probe", target_os = "linux"))]
            probe_pop::count(0, 1);
            if !classed(&l) {
                return self.take(l.size(), l.align());
            }
            let c = class(l.size());
            let head = self.free[c].get();
            if head != 0 {
                self.free[c].set(*(head as *const usize));
                return head as *mut u8;
            }
            self.take(class_size(c), 16)
        }

        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            #[cfg(all(feature = "probe", target_os = "linux"))]
            probe_pop::count(l.size(), 0);
            if classed(&l) {
                let c = class(l.size());
                *(p as *mut usize) = self.free[c].get();
                self.free[c].set(p as usize);
            }
        }

        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            let at = p as usize;
            // What the block spans now, and what it would span at the new size (its class's size when classed).
            let span = if classed(&l) { class_size(class(l.size())) } else { l.size() };
            let nl = Layout::from_size_align_unchecked(new, l.align());
            let want = if classed(&nl) { class_size(class(new)) } else { new };
            if classed(&l) && classed(&nl) && want <= span {
                // It fits the block it has (a shrink to a smaller class wastes the rest, as dealloc will file it
                // under that class).
                return p;
            }
            if at + span == self.next.get() && at + want <= self.end.get() && (classed(&l) == classed(&nl)) {
                // The last block taken grows in place.
                self.next.set(at + want);
                return p;
            }
            let q = self.alloc(nl);
            if !q.is_null() {
                core::ptr::copy_nonoverlapping(p, q, l.size().min(new));
                self.dealloc(p, l);
            }
            q
        }
    }
}

#[global_allocator]
static ALLOC: arena::Arena = arena::Arena::new();

/// One case built into `out` (its answer's JSON object): whether it was, every block it took given back after (out
/// was made big enough beforehand; if it had to grow, nothing is given back).
fn build_case(j: &json::J, out: &mut Vec<u8>, flush: &mut dyn FnMut(&mut Vec<u8>)) -> bool {
    let mark = ALLOC.mark();
    let cap = out.capacity();
    let ok = match build::Input::from_json(j) {
        Some(inp) => build::run(&inp, out, flush),
        None => false,
    };
    if out.capacity() == cap {
        ALLOC.release(mark);
    }
    ok
}

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static CASES: RefCell<Vec<[u32; 3]>> = const { RefCell::new(Vec::new()) };
}

/// n bytes for main.js to fill.
#[no_mangle]
pub extern "C" fn alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Builds every case of the cases.json held in the n bytes at p (unescaped in place as it is read): the case count, or -1 for a file this build does
/// not read. Each case's answer (its JSON object) is then out_ptr()[case_info(i, 1)..case_info(i, 2)], when
/// case_info(i, 0) is 1; 0 marks a case left to JavaScript.
#[no_mangle]
pub extern "C" fn build(p: *mut u8, n: usize) -> i32 {
    let src = unsafe { std::slice::from_raw_parts_mut(p, n) };
    let Some(inputs) = json::parse_cases(src) else { return -1 };
    let mut out = Vec::with_capacity(n * 3);
    let mut cases = Vec::with_capacity(inputs.len());
    for j in &inputs {
        let start = out.len();
        let ok = build_case(j, &mut out, &mut |_: &mut Vec<u8>| {});
        if !ok {
            out.truncate(start);
        }
        cases.push([ok as u32, start as u32, out.len() as u32]);
    }
    let count = cases.len() as i32;
    OUT.with(|o| *o.borrow_mut() = out);
    CASES.with(|c| *c.borrow_mut() = cases);
    count
}

/// Whether this build reads the input at all: valid UTF-8 with no U+2028 or U+2029 (which a regex "." stops at).
/// Every byte below 0x80 is a character of its own in UTF-8 and every byte of a longer character is above 0x7f, so
/// the input is valid exactly when each run of bytes above 0x7f is valid on its own, and those runs hold any U+2028
/// or U+2029 (E2 80 A8, E2 80 A9): only the runs are read, the ASCII between them skipped 64 bytes at a time.
fn readable(src: &[u8]) -> bool {
    let mut i = text::find_high(src, 0, true);
    while i < src.len() {
        let j = text::find_high(src, i, false);
        let run = &src[i..j];
        if std::str::from_utf8(run).is_err() {
            return false;
        }
        let mut at = 0;
        while let Some(k) = text::find_byte(&run[at..], 0xe2) {
            let x = at + k;
            if run.get(x + 1) == Some(&0x80) && matches!(run.get(x + 2), Some(&0xa8) | Some(&0xa9)) {
                return false;
            }
            at = x + 1;
        }
        i = text::find_high(src, j, true);
    }
    true
}

/// Under the probe feature (a measuring build, never shipped): the wall time of each step of the addon's build, from
/// main.ts's SG_T0 (its process.hrtime just before process.dlopen), printed to stderr before the process ends, and
/// the time spent in writes; SG_PROBE=<step> ends it after that step (load, read, utf8, parse; noout writes nothing).
#[cfg(feature = "probe")]
pub mod probe {
    use std::cell::{Cell, RefCell};
    extern "C" {
        fn clock_gettime(clk: i32, ts: *mut [i64; 2]) -> i32;
    }
    pub fn now() -> i64 {
        let mut t = [0i64; 2];
        unsafe { clock_gettime(1, &mut t) };
        t[0] * 1_000_000_000 + t[1]
    }
    thread_local! {
        static MARKS: RefCell<Vec<(&'static str, i64)>> = const { RefCell::new(Vec::new()) };
        static WRITES: Cell<(i64, i64)> = const { Cell::new((0, 0)) };
    }
    pub fn mark(name: &'static str) {
        MARKS.with(|m| m.borrow_mut().push((name, now())));
    }
    pub fn stop(at: &str) -> bool {
        std::env::var("SG_PROBE").map(|p| p == at).unwrap_or(false)
    }
    pub fn wrote(since: i64) {
        WRITES.with(|w| {
            let (t, n) = w.get();
            w.set((t + now() - since, n + 1))
        });
    }
    pub fn report() {
        let mut last = std::env::var("SG_T0").ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        let mut s = String::from("PROBE");
        MARKS.with(|m| {
            for &(n, t) in m.borrow().iter() {
                s.push_str(&format!(" {}={:.2}", n, (t - last) as f64 / 1e6));
                last = t;
            }
        });
        let (t, n) = WRITES.with(|w| w.get());
        #[cfg(target_os = "linux")]
        let (peak, (freed, allocs)) = (super::arena::probe_pop::peak() as f64 / (1 << 20) as f64, super::arena::probe_pop::counts());
        #[cfg(not(target_os = "linux"))]
        let (peak, (freed, allocs)) = (0.0, (0usize, 0usize));
        eprintln!("{} write={:.2} writes={} peak={:.1} freed={:.1} allocs={}", s, t as f64 / 1e6, n, peak, freed as f64 / (1 << 20) as f64, allocs);
    }
}

/// The answers file written to `path`, streamed through a small buffer, when this build takes every case of
/// cases.json exactly: true. False otherwise, and the file may then hold part of it (main.js writes it anew).
pub fn build_to_file(src: &mut [u8], path: &str) -> bool {
    use std::io::Write;
    if !readable(src) {
        return false;
    }
    #[cfg(feature = "probe")]
    {
        probe::mark("utf8");
        if probe::stop("utf8") {
            return true;
        }
    }
    let Some(inputs) = json::parse_cases(src) else { return false };
    #[cfg(feature = "probe")]
    {
        probe::mark("parse");
        if probe::stop("parse") {
            return true;
        }
    }
    #[cfg(feature = "probe")]
    let null_out = probe::stop("noout");
    #[cfg(not(feature = "probe"))]
    let null_out = false;
    let Ok(mut file) = std::fs::File::create(path) else { return false };
    let ok = std::cell::Cell::new(true);
    let mut flush = |b: &mut Vec<u8>| {
        #[cfg(feature = "probe")]
        let t = probe::now();
        if !null_out && ok.get() && file.write_all(b).is_err() {
            ok.set(false);
        }
        #[cfg(feature = "probe")]
        probe::wrote(t);
        b.clear();
    };
    let mut out = Vec::with_capacity(2 * build::flush_at());
    out.push(b'[');
    for (k, j) in inputs.iter().enumerate() {
        if k > 0 {
            out.push(b',');
        }
        if !build_case(j, &mut out, &mut flush) {
            return false;
        }
    }
    out.push(b']');
    #[cfg(feature = "probe")]
    probe::mark("build");
    flush(&mut out);
    #[cfg(feature = "probe")]
    probe::mark("last");
    ok.get()
}

pub fn build_all(src: &mut [u8]) -> Option<Vec<u8>> {
    if std::str::from_utf8(src).is_err() {
        return None;
    }
    let mut at = 0;
    while let Some(k) = text::find_byte(&src[at..], 0xe2) {
        let i = at + k;
        if src.get(i + 1) == Some(&0x80) && matches!(src.get(i + 2), Some(&0xa8) | Some(&0xa9)) {
            return None;
        }
        at = i + 1;
    }
    let n = src.len();
    let inputs = json::parse_cases(src)?;
    let mut out = Vec::with_capacity(n * 3 + 2);
    out.push(b'[');
    for (k, j) in inputs.iter().enumerate() {
        if k > 0 {
            out.push(b',');
        }
        if !build_case(j, &mut out, &mut |_: &mut Vec<u8>| {}) {
            return None;
        }
    }
    out.push(b']');
    Some(out)
}

/// As a Node addon on x86-64 Linux (sitegen-linux-x64.node, which main.ts loads with require), the module's
/// registration is the build: it reads cases.json and, when it takes every case, writes answers.json and ends the
/// process at once (exit_group, so nothing more of Node runs; the file is closed by then). For input it does not
/// take it sets SITEGEN_NO_NATIVE and returns, and main.ts goes on to main.js. It calls nothing of Node's.
#[cfg(all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"))]
#[no_mangle]
pub unsafe extern "C" fn napi_register_module_v1(_env: *mut core::ffi::c_void, exports: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
    let dir = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let input = format!("{}/cases.json", dir("MAKEFUN_INPUT_DIR", "/input"));
    let output = format!("{}/answers.json", dir("MAKEFUN_OUTPUT_DIR", "/output"));
    #[cfg(feature = "probe")]
    {
        probe::mark("load");
        if probe::stop("load") {
            probe::report();
            core::arch::asm!("syscall", in("rax") 231usize, in("rdi") 0usize, options(noreturn));
        }
    }
    if let Ok(mut bytes) = std::fs::read(&input) {
        #[cfg(feature = "probe")]
        {
            probe::mark("read");
            if probe::stop("read") {
                probe::report();
                core::arch::asm!("syscall", in("rax") 231usize, in("rdi") 0usize, options(noreturn));
            }
        }
        if build_to_file(&mut bytes, &output) {
            #[cfg(feature = "probe")]
            probe::report();
            core::arch::asm!("syscall", in("rax") 231usize, in("rdi") 0usize, options(noreturn));
        }
    }
    // Not taken: main.ts goes on to main.js, which this tells to leave the native builds alone (process.env reads
    // the process's own environment).
    extern "C" {
        fn setenv(name: *const u8, value: *const u8, overwrite: i32) -> i32;
    }
    setenv(b"SITEGEN_NO_NATIVE\0".as_ptr(), b"1\0".as_ptr(), 1);
    exports
}

#[no_mangle]
pub extern "C" fn out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn case_info(i: usize, k: usize) -> u32 {
    CASES.with(|c| c.borrow()[i][k])
}

#[cfg(feature = "timing")]
pub use build::{BYTES, PHASES};
