// Native timing of the build on a cases.json, best of N runs per phase:
// cargo run --release --features timing --example timing -- <cases.json> [runs]
fn main() {
    let path = std::env::args().nth(1).expect("cases.json");
    let runs: usize = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(20);
    let bytes = std::fs::read(path).unwrap();
    let mut best_total = std::time::Duration::MAX;
    let mut best: std::collections::BTreeMap<&str, std::time::Duration> = Default::default();
    for _ in 0..runs {
        let before = sitegen::arena::taken();
        // Room first: a block taken during a case is given back after it (see the arena).
        sitegen::PHASES.with(|p| { p.borrow_mut().clear(); p.borrow_mut().reserve(1 << 10); });
        sitegen::BYTES.with(|p| { p.borrow_mut().clear(); p.borrow_mut().reserve(1 << 10); });
        let base = sitegen::arena::taken();
        let mut input = bytes.clone();
        let t = std::time::Instant::now();
        let out = sitegen::build_to_file(&mut input, "C:/Users/SAFIUD~1/AppData/Local/Temp/claude/a479ac777d22ffec/7cea6c6b-5f06-47bb-a4c2-f69219ad7cc1/scratchpad/timing-answers.json");
        assert!(out, "built");
        let total = t.elapsed();
        std::hint::black_box(out);
        if best_total == std::time::Duration::MAX { eprintln!("one run takes {} MB", (sitegen::arena::taken() - before) >> 20); }
        best_total = best_total.min(total);
        let mut sums: std::collections::BTreeMap<&str, std::time::Duration> = Default::default();
        sitegen::PHASES.with(|p| for (n, d) in p.borrow().iter() { *sums.entry(n).or_default() += *d; });
        for (n, d) in sums { let e = best.entry(n).or_insert(d); if d < *e { *e = d; } }
    }
    // Bytes taken by phase, in the last run (first case's).
    sitegen::BYTES.with(|b| { let b = b.borrow(); let mut prev = 0usize; let start = b.first().map(|_| 0).unwrap_or(0); let _ = start; for (n, t) in b.iter().take(9) { println!("  bytes {:<12} {} KB", n, (t - prev.max(*t.min(&prev))) >> 10); prev = *t; } });
    println!("arena peak (first chunk) {} KB", sitegen::arena::peak() >> 10);
    println!("best total {:?} over {} runs; {} MB taken in all", best_total, runs, sitegen::arena::taken() >> 20);
    for (n, d) in best { println!("  {:<12} {:?}", n, d); }
}
