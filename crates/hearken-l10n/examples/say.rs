//! Translate lines from stdin with one or more PO files, for checking a
//! catalog by hand: `cargo run -p hearken-l10n --example say -- es a.po [b.po]`.
//! A line that is not fully translated is marked with `!`.

use std::io::BufRead;

fn main() {
    let mut args = std::env::args().skip(1);
    let code = args.next().expect("usage: say CODE FILE.po [MORE.po]");
    let mut text = String::new();
    for (i, path) in args.enumerate() {
        let more = std::fs::read_to_string(&path).expect("readable PO file");
        if i > 0 {
            // Later files lose their header, as a game's merged catalog does.
            text.push('\n');
            text.push_str(more.split_once("\n\n").map_or("", |(_, rest)| rest));
        } else {
            text = more;
        }
    }
    let catalog = hearken_l10n::Catalog::from_po(&code, &text).expect("valid PO text");
    let misses = std::env::temp_dir().join(format!("say-misses-{}.txt", std::process::id()));
    let translator = hearken_l10n::Translator::new(catalog).log_misses_to(misses.clone());
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("text on stdin");
        let before = std::fs::read_to_string(&misses).unwrap_or_default().len();
        let out = translator.translate(&line);
        let after = std::fs::read_to_string(&misses).unwrap_or_default().len();
        let mark = if after > before { "! " } else { "  " };
        println!("{mark}{out}");
    }
    let _ = std::fs::remove_file(misses);
}
