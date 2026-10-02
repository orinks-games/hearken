//! Decoding a station as it arrives, into the ring.

use symphonia::core::codecs::audio::AudioDecoder;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};

use crate::stream::{Fault, Shared, StreamOptions, StreamState};
use crate::{codecs, title};

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
    let Some((mut decoder, mut track)) = open_track(&*reader) else {
        return no_audio();
    };
    // Not cleared when there is none: an ICY title may already have arrived.
    if let Some(current) = comments_title(&mut *reader) {
        shared.set_title(Some(current));
    }
    let mut samples: Vec<f32> = Vec::new();
    while !shared.closed() {
        let packet = match reader.next_packet() {
            Ok(Some(packet)) if packet.track_id == track => packet,
            Ok(Some(_)) => continue,
            Ok(None) => return Fault::Retry("stream ended".into()),
            // A chained Ogg stream is the next track: its own decoder, and
            // its own comments, which replace the last track's title.
            Err(Error::ResetRequired) => {
                let Some(next) = open_track(&*reader) else {
                    return no_audio();
                };
                (decoder, track) = next;
                shared.set_title(comments_title(&mut *reader));
                continue;
            }
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
    closed()
}

/// A decoder for the default audio track, and the track's id.
fn open_track(reader: &dyn FormatReader) -> Option<(Box<dyn AudioDecoder>, u32)> {
    let track = reader.default_track(TrackType::Audio)?;
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return None;
    };
    let decoder = codecs::registry()
        .make_audio_decoder(params, &Default::default())
        .ok()?;
    Some((decoder, track.id))
}

/// The newest tags' artist and title. Vorbis comment keys have no fixed
/// case, so they match any.
fn comments_title(reader: &mut dyn FormatReader) -> Option<String> {
    let mut metadata = reader.metadata();
    let tags = &metadata.skip_to_latest()?.media.tags;
    let tag = |key: &str| {
        let tag = tags
            .iter()
            .find(|tag| tag.raw.key.eq_ignore_ascii_case(key));
        tag.map(|tag| tag.raw.value.to_string())
    };
    title::ogg(tag("artist").as_deref(), tag("title").as_deref())
}

/// Nobody is left to hear the stream.
pub(crate) fn closed() -> Fault {
    Fault::Fatal("closed".into())
}

pub(crate) fn no_audio() -> Fault {
    Fault::Fatal("no playable audio".into())
}
