//! makefun-site 1.0.0's build in Rust, for WebAssembly: main.ts hands it cases.json's bytes and writes the answers
//! it builds. A case it does not take exactly (see build::Input::from_json and build::run) is marked for main.ts to
//! build in JavaScript instead; slugs of non-ASCII text call back into JavaScript for its NFKD (js_slug).

mod build;
mod json;
mod md;
mod text;
mod tpl;

use std::cell::RefCell;

/// The allocator on WebAssembly: one pass over one input needs no freeing, so blocks are handed out in order from
/// memory grown 16 MB at a time, freeing is nothing, and the last block grows in place.
#[cfg(target_arch = "wasm32")]
mod bump {
    use std::alloc::{GlobalAlloc, Layout};
    use std::cell::Cell;

    pub struct Bump {
        next: Cell<usize>,
        end: Cell<usize>,
    }

    // WebAssembly here runs on one thread.
    unsafe impl Sync for Bump {}

    const PAGE: usize = 0x10000;

    impl Bump {
        pub const fn new() -> Bump {
            Bump { next: Cell::new(0), end: Cell::new(0) }
        }

        /// Room up to address z, or false.
        fn reach(&self, z: usize) -> bool {
            if z <= self.end.get() {
                return true;
            }
            let pages = ((z - self.end.get()) / PAGE + 1).max(256);
            let old = core::arch::wasm32::memory_grow(0, pages);
            if old == usize::MAX {
                return false;
            }
            if self.end.get() != old * PAGE {
                // The first growth (or memory grown by anything else): start over at the new pages.
                self.next.set(old * PAGE);
                self.end.set(old * PAGE);
                if z <= old * PAGE {
                    self.end.set((old + pages) * PAGE);
                    return true;
                }
                self.end.set((old + pages) * PAGE);
                return false;
            }
            self.end.set((old + pages) * PAGE);
            z <= self.end.get()
        }
    }

    unsafe impl GlobalAlloc for Bump {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            loop {
                let a = (self.next.get() + l.align() - 1) & !(l.align() - 1);
                let z = a + l.size();
                if z <= self.end.get() && self.next.get() != 0 {
                    self.next.set(z);
                    return a as *mut u8;
                }
                let before = self.end.get();
                if !self.reach(z.max(self.next.get() + l.size() + l.align())) && self.end.get() == before {
                    return core::ptr::null_mut();
                }
            }
        }

        unsafe fn dealloc(&self, _p: *mut u8, _l: Layout) {}

        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            let at = p as usize;
            if at + l.size() == self.next.get() && self.reach(at + new) && self.next.get() == at + l.size() {
                self.next.set(at + new);
                return p;
            }
            let q = self.alloc(Layout::from_size_align_unchecked(new, l.align()));
            if !q.is_null() {
                core::ptr::copy_nonoverlapping(p, q, l.size().min(new));
            }
            q
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: bump::Bump = bump::Bump::new();

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static CASES: RefCell<Vec<[u32; 3]>> = const { RefCell::new(Vec::new()) };
}

/// n bytes for main.ts to fill.
#[no_mangle]
pub extern "C" fn alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Builds every case of the cases.json held in the n bytes at p: the case count, or -1 for a file this build does
/// not read. Each case's answer (its JSON object) is then out_ptr()[case_info(i, 1)..case_info(i, 2)], when
/// case_info(i, 0) is 1; 0 marks a case left to JavaScript.
#[no_mangle]
pub extern "C" fn build(p: *const u8, n: usize) -> i32 {
    let src = unsafe { std::slice::from_raw_parts(p, n) };
    #[cfg(not(target_arch = "wasm32"))]
    let t = std::time::Instant::now();
    let Some(inputs) = json::parse_cases(src) else { return -1 };
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("  json         {:?}", t.elapsed());
    let mut out = Vec::with_capacity(n * 3);
    let mut cases = Vec::with_capacity(inputs.len());
    for j in &inputs {
        let start = out.len();
        let ok = match build::Input::from_json(j) {
            Some(inp) => build::run(&inp, &mut out),
            None => false,
        };
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

#[no_mangle]
pub extern "C" fn out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn case_info(i: usize, k: usize) -> u32 {
    CASES.with(|c| c.borrow()[i][k])
}
