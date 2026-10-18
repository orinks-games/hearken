//! Internet radio for Hearken games. [`Stream::open`] returns at once; a
//! worker thread connects, decodes and buffers, and the game plays the
//! stream's [`feed`](Stream::feed) like any other sound. Connecting,
//! stalling and failing never block a frame.

mod codecs;
mod decode;
mod hls;
mod http;
mod icy;
mod playlist;
mod ring;
mod stream;
#[cfg(any(windows, target_os = "macos", test))]
mod system_aac;
mod title;
mod worker;

pub use stream::{RawTitle, Stream, StreamOptions, StreamState, DEFAULT_USER_AGENT};
