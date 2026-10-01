//! Internet radio for Hearken games. [`Stream::open`] returns at once; a
//! worker thread connects, decodes and buffers, and the game plays the
//! stream's [`feed`](Stream::feed) like any other sound. Connecting,
//! stalling and failing never block a frame.

mod codecs;
mod decode;
mod hls;
mod http;
mod playlist;
mod ring;
mod stream;
mod title;
mod worker;

pub use stream::{Stream, StreamOptions, StreamState};
