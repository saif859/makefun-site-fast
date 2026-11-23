//! sitegen as a Linux program: `sitegen <cases.json> <answers.json> [<node> <main.js>]`. When it takes every case it
//! writes answers.json and exits 0. Otherwise, given node and main.js, it becomes `node main.js` (with
//! SITEGEN_NO_NATIVE=1), which builds the answers the other ways; without them it exits 3, writing nothing.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 && args.len() != 5 {
        std::process::exit(2);
    }
    let built = std::fs::read(&args[1]).ok().and_then(|mut bytes| sitegen::build_all(&mut bytes));
    if let Some(out) = built {
        if std::fs::write(&args[2], out).is_ok() {
            return;
        }
    }
    hand_over(&args)
}

/// main.js in this process's place, when given, else exit 3.
fn hand_over(args: &[String]) -> ! {
    #[cfg(unix)]
    if args.len() == 5 {
        use std::os::unix::process::CommandExt;
        let _ = std::process::Command::new(&args[3]).arg(&args[4]).env("SITEGEN_NO_NATIVE", "1").exec();
    }
    let _ = args;
    std::process::exit(3)
}
