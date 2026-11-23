// Native timing of the build on a cases.json, best of N runs per phase:
// cargo run --release --features timing --example timing -- <cases.json> [runs]
fn main() {
    let path = std::env::args().nth(1).expect("cases.json");
    let runs: usize = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(20);
    let bytes = std::fs::read(path).unwrap();
    let mut best_total = std::time::Duration::MAX;
    let mut best: std::collections::BTreeMap<&str, std::time::Duration> = Default::default();
    for _ in 0..runs {
        // Room first: a block taken during a case is given back after it (see the arena).
        sitegen::PHASES.with(|p| { p.borrow_mut().clear(); p.borrow_mut().reserve(4096); });
        let mut input = bytes.clone();
        let t = std::time::Instant::now();
        let out = sitegen::build_all(&mut input).expect("built");
        let total = t.elapsed();
        std::hint::black_box(out);
        best_total = best_total.min(total);
        let mut sums: std::collections::BTreeMap<&str, std::time::Duration> = Default::default();
        sitegen::PHASES.with(|p| for (n, d) in p.borrow().iter() { *sums.entry(n).or_default() += *d; });
        for (n, d) in sums { let e = best.entry(n).or_insert(d); if d < *e { *e = d; } }
    }
    println!("best total {:?} over {} runs", best_total, runs);
    for (n, d) in best { println!("  {:<12} {:?}", n, d); }
}
