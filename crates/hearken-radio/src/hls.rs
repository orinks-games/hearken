//! Live HLS stations.

use crate::stream::{Fault, Shared, StreamOptions};

/// Plays the HLS station at `url`, whose playlist `playlist` is already
/// fetched. Not played yet: it ends the stream, so a station that sends
/// HLS fails with a reason instead of being decoded as garbage.
pub(crate) fn play(_url: &str, _playlist: &str, _shared: &Shared, _opts: &StreamOptions) -> Fault {
    Fault::Fatal("HLS not supported".into())
}
