//! The code `scattered` writes, compiled and run, gives back the key.

use std::process::Command;

#[test]
fn the_scattered_code_rebuilds_the_key() {
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("main.rs");
    let code = format!(
        "fn key() -> Option<[u8; 32]> {}\nfn main() {{ for b in key().unwrap() {{ print!(\"{{b:02x}}\"); }} }}\n",
        hearken_bake::scattered(&key)
    );
    std::fs::write(&src, code).unwrap();
    let exe = dir.path().join("rebuilt");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let built = Command::new(rustc)
        .arg("--edition=2021")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .status()
        .unwrap();
    assert!(built.success());
    let out = Command::new(&exe).output().unwrap();
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), hex);
    assert_eq!(hearken_bake::parse_hex(&hex), Some(key));
}
