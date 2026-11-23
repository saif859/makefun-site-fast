//! sitegen as a Linux program: `sitegen <cases.json> <answers.json>`. Exit 0: answers.json written, every case
//! built. Exit 3: input this build does not take exactly, nothing written (main.js then builds it the other ways).

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        std::process::exit(2);
    }
    let Ok(bytes) = std::fs::read(&args[1]) else { std::process::exit(2) };
    let Some(out) = sitegen::build_all(&bytes) else { std::process::exit(3) };
    if std::fs::write(&args[2], out).is_err() {
        std::process::exit(2);
    }
}
