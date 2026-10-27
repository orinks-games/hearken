//! Installs the voice package: `cargo run -p hearken-voice --example install
//! --features download -- <voices folder>`.
use std::sync::atomic::AtomicBool;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "voices".into());
    let mut last = 0;
    let done = hearken_voice::install(
        std::path::Path::new(&dir),
        &mut |got, total| {
            let pct = got * 100 / total.max(1);
            if pct >= last + 10 {
                last = pct;
                eprintln!("{pct}% of {} MB", total / 1_000_000);
            }
        },
        &AtomicBool::new(false),
    );
    match done {
        Ok(path) => println!("installed {}", path.display()),
        Err(err) => {
            eprintln!("install failed: {err}");
            std::process::exit(1);
        }
    }
}
