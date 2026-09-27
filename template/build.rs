//! Link Prism the way it needs, or the game will not start (or will crash)
//! on a machine missing one of the screen readers' client DLLs.
//! hearken-speech works out the arguments; a link argument only reaches the
//! final link, which is this crate's, so every game passes them on here.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_HEARKEN_SPEECH_LINK_ARGS");
    match std::env::var("DEP_HEARKEN_SPEECH_LINK_ARGS") {
        Ok(args) => {
            for arg in args.split(';').filter(|arg| !arg.is_empty()) {
                println!("cargo:rustc-link-arg={arg}");
            }
        }
        // Cargo hands a build script this list only from a direct dependency.
        Err(_) => println!(
            "cargo:warning=Prism's link arguments are missing, so the game will not start on \
             a machine missing a screen reader's DLL. Fix: add hearken-speech to this \
             package's [dependencies]."
        ),
    }
}
