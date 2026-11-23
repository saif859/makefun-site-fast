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
        let size = (min + 4096).max(64 << 20);
        let p = unsafe { std::alloc::System.alloc(Layout::from_size_align(size, 4096).ok()?) };
        if p.is_null() {
            return None;
        }
        Some((p as usize, p as usize + size))
    }

    impl Arena {
        pub const fn new() -> Arena {
            Arena { next: Cell::new(0), end: Cell::new(0) }
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
            }
        }
    }

    unsafe impl GlobalAlloc for Arena {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            self.take(l.size(), l.align())
        }

        unsafe fn dealloc(&self, _p: *mut u8, _l: Layout) {}

        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            let at = p as usize;
            if at + l.size() == self.next.get() && at + new <= self.end.get() {
                self.next.set(at + new);
                return p;
            }
            let q = self.take(new, l.align());
            if !q.is_null() {
                core::ptr::copy_nonoverlapping(p, q, l.size().min(new));
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

/// The answers file (the JSON array of every case's answer) when this build takes every case of cases.json exactly,
/// else None: valid UTF-8 with no U+2028 or U+2029 (which a regex "." stops at), as main.js's one-byte check asks.
/// Whether this build reads the input at all: valid UTF-8 with no U+2028 or U+2029 (which a regex "." stops at).
fn readable(src: &[u8]) -> bool {
    if std::str::from_utf8(src).is_err() {
        return false;
    }
    let mut at = 0;
    while let Some(k) = text::find_byte(&src[at..], 0xe2) {
        let i = at + k;
        if src.get(i + 1) == Some(&0x80) && matches!(src.get(i + 2), Some(&0xa8) | Some(&0xa9)) {
            return false;
        }
        at = i + 1;
    }
    true
}

/// The answers file written to `path`, streamed through a small buffer, when this build takes every case of
/// cases.json exactly: true. False otherwise, and the file may then hold part of it (main.js writes it anew).
pub fn build_to_file(src: &mut [u8], path: &str) -> bool {
    use std::io::Write;
    if !readable(src) {
        return false;
    }
    let Some(inputs) = json::parse_cases(src) else { return false };
    let Ok(mut file) = std::fs::File::create(path) else { return false };
    let ok = std::cell::Cell::new(true);
    let mut flush = |b: &mut Vec<u8>| {
        if ok.get() && file.write_all(b).is_err() {
            ok.set(false);
        }
        b.clear();
    };
    let mut out = Vec::with_capacity(1 << 20);
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
    flush(&mut out);
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

/// As a Node addon on x86-64 Linux (sitegen-linux-x64.node, which main.ts loads with process.dlopen), the module's
/// registration is the build: it reads cases.json and, when it takes every case, writes answers.json and ends the
/// process at once (exit_group, so nothing more of Node runs; the file is closed by then). For input it does not
/// take it returns, and main.ts goes on to main.js. It calls nothing of Node's.
#[cfg(all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"))]
#[no_mangle]
pub unsafe extern "C" fn napi_register_module_v1(_env: *mut core::ffi::c_void, exports: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
    let dir = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let input = format!("{}/cases.json", dir("MAKEFUN_INPUT_DIR", "/input"));
    let output = format!("{}/answers.json", dir("MAKEFUN_OUTPUT_DIR", "/output"));
    if let Ok(mut bytes) = std::fs::read(&input) {
        if build_to_file(&mut bytes, &output) {
            core::arch::asm!("syscall", in("rax") 231usize, in("rdi") 0usize, options(noreturn));
        }
    }
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
