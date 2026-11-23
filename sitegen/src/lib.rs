//! makefun-site 1.0.0's build in Rust, for WebAssembly: main.ts hands it cases.json's bytes and writes the answers
//! it builds. A case it does not take exactly (see build::Input::from_json and build::run) is marked for main.ts to
//! build in JavaScript instead; slugs of non-ASCII text call back into JavaScript for its NFKD (js_slug).

mod build;
mod json;
mod md;
mod text;
mod tpl;

use std::cell::RefCell;

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
