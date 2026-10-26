//! What turns text into speech: Kokoro, or a test's stand-in.

use crate::{Error, Speech};

/// A speech engine. It lives on the voice's worker thread, so it need not be
/// `Send`; its [`Loader`] must be.
pub trait Synth {
    /// Speaks `text` in `voice` at `rate` (1 is normal, above 1 faster).
    fn synthesize(&mut self, text: &str, voice: &str, rate: f32) -> Result<Speech, Error>;
    /// The voices it has, the default first.
    fn speakers(&self) -> Vec<String>;
}

/// Makes the engine, on the worker thread, once.
pub type Loader = Box<dyn FnOnce() -> Result<Box<dyn Synth>, Error> + Send>;
