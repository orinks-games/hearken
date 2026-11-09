//! The Pebbles rules (`hearken_replay::pebbles`) as a server module, for the
//! server's tests and a local dev server:
//! `cargo build -p hearken-replay --example pebbles --target wasm32-unknown-unknown --release`.

pub use hearken_replay::pebbles::Pebbles;

hearken_replay::export_rules!(Pebbles);

#[allow(dead_code)]
fn main() {}
