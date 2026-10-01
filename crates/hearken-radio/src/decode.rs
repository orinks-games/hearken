//! Decoding a station as it arrives, into the ring.

use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::TrackType;
use symphonia::core::io::{MediaSource, MediaSourceStream};

use crate::codecs;
use crate::stream::{Fault, Shared, StreamOptions, StreamState};

/// Rates a feed may carry; anything else is a broken stream, not audio.
const RATES: std::ops::RangeInclusive<u32> = 8_000..=384_000;

/// Decodes `source` into `shared`'s ring as stereo until the stream ends,
/// fails or closes, and returns why it stopped.
pub(crate) fn pump(
    source: Box<dyn MediaSource>,
    hint: Hint,
    shared: &Shared,
    opts: &StreamOptions,
) -> Fault {
    let stream = MediaSourceStream::new(source, Default::default());
    let probed = symphonia::default::get_probe().probe(
        &hint,
        stream,
        Default::default(),
        Default::default(),
    );
    let mut reader = match probed {
        Ok(reader) => reader,
        // A socket failing mid-probe is worth calling again; running out of
        // bytes without finding audio is not.
        Err(Error::IoError(error)) if error.kind() != std::io::ErrorKind::UnexpectedEof => {
            return error.into()
        }
        Err(_) => return no_audio(),
    };
    let Some(track) = reader.default_track(TrackType::Audio) else {
        return no_audio();
    };
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return no_audio();
    };
    let Ok(mut decoder) = codecs::registry().make_audio_decoder(params, &Default::default()) else {
        return no_audio();
    };
    let track = track.id;
    let mut samples: Vec<f32> = Vec::new();
    while !shared.closed() {
        let packet = match reader.next_packet() {
            Ok(Some(packet)) if packet.track_id == track => packet,
            Ok(Some(_)) => continue,
            Ok(None) => return Fault::Retry("stream ended".into()),
            Err(Error::IoError(error)) => return error.into(),
            Err(error) => return Fault::Retry(error.to_string()),
        };
        // A corrupt packet is skipped; the next one decodes on.
        let Ok(buffer) = decoder.decode(&packet) else {
            continue;
        };
        let rate = buffer.spec().rate();
        if !RATES.contains(&rate) {
            return Fault::Fatal("unsupported sample rate".into());
        }
        let channels = buffer.num_planes();
        if channels == 0 {
            continue;
        }
        buffer.copy_to_vec_interleaved(&mut samples);
        // Mono to both sides; past stereo, the first two channels.
        let right = channels.min(2) - 1;
        let frames = samples.chunks_exact(channels).map(|f| [f[0], f[right]]);
        shared.push(rate, frames.collect());
        if shared.buffered() >= opts.prebuffer {
            shared.set_state(StreamState::Playing);
        }
    }
    // Nobody is left to read this.
    Fault::Fatal("closed".into())
}

fn no_audio() -> Fault {
    Fault::Fatal("no playable audio".into())
}
