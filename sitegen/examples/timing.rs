// Native timing of the build on a cases.json: cargo run --release --example timing -- <cases.json>
fn main() {
    let path = std::env::args().nth(1).expect("cases.json");
    let bytes = std::fs::read(path).unwrap();
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let n = sitegen::build(bytes.as_ptr(), bytes.len());
        println!("{} cases in {:?}", n, t.elapsed());
    }
}
