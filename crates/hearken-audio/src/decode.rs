//! Clip decoding for the Kira backend: Symphonia's codecs plus libopus.
//!
//! Kira's own loaders only know Symphonia's built-in codecs, which have no
//! Opus, and take no others. So every clip decodes here instead, through one
//! registry that has both, and reaches Kira as decoded frames: whole for a
//! short clip, or through Kira's streaming [`Decoder`] for a long one.

use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use kira::sound::streaming::{Decoder, StreamingSoundData};
use kira::sound::FromFileError;
use kira::Frame;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::AudioDecoder;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::{FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia_adapter_libopus::OpusDecoder;

/// Symphonia's enabled codecs, and Opus.
fn codecs() -> &'static CodecRegistry {
    static CODECS: OnceLock<CodecRegistry> = OnceLock::new();
    CODECS.get_or_init(|| {
        let mut codecs = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut codecs);
        codecs.register_audio_decoder::<OpusDecoder>();
        codecs
    })
}

/// Decodes a whole clip into memory.
pub(crate) fn decode_whole(bytes: Arc<[u8]>) -> Result<StaticSoundData, FromFileError> {
    let mut clip = ClipDecoder::open(bytes)?;
    let mut frames = Vec::new();
    while let Some(chunk) = clip.next_chunk()? {
        frames.extend(chunk);
    }
    Ok(StaticSoundData {
        sample_rate: clip.sample_rate,
        frames: frames.into(),
        settings: StaticSoundSettings::default(),
        slice: None,
    })
}

/// Streams a clip from a decoder thread as Kira plays it.
pub(crate) fn stream(bytes: Arc<[u8]>) -> Result<StreamingSoundData<FromFileError>, FromFileError> {
    let clip = ClipDecoder::open(bytes)?;
    if clip.num_frames.is_none() {
        return Err(FromFileError::UnknownDuration);
    }
    Ok(StreamingSoundData::from_decoder(clip))
}

struct ClipDecoder {
    /// Kept to reopen the clip when Kira seeks back to its start.
    bytes: Arc<[u8]>,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    sample_rate: u32,
    /// Known up front for most files; streaming needs it, decoding whole
    /// does not.
    num_frames: Option<usize>,
    /// Frames handed to Kira since the start.
    at: usize,
    /// Nothing read yet, so a seek to the start has nothing to undo.
    untouched: bool,
}

impl ClipDecoder {
    fn open(bytes: Arc<[u8]>) -> Result<ClipDecoder, FromFileError> {
        let source =
            MediaSourceStream::new(Box::new(Cursor::new(bytes.clone())), Default::default());
        let reader = symphonia::default::get_probe().probe(
            &Default::default(),
            source,
            Default::default(),
            Default::default(),
        )?;
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or(FromFileError::NoDefaultTrack)?;
        let Some(CodecParameters::Audio(params)) = &track.codec_params else {
            return Err(FromFileError::NoDefaultTrack);
        };
        let sample_rate = params.sample_rate.ok_or(FromFileError::UnknownSampleRate)?;
        let decoder = codecs().make_audio_decoder(params, &Default::default())?;
        // Symphonia's Ogg reader counts Opus's pre-skip in the track's
        // frames, which libopus then drops, so Kira would wait for frames
        // that never come.
        let pre_skip = match params.codec {
            CODEC_ID_OPUS => track.delay.unwrap_or(0),
            _ => 0,
        };
        let num_frames = track
            .num_frames
            .map(|n| n.saturating_sub(pre_skip.into()))
            .and_then(|n| usize::try_from(n).ok());
        let track = track.id;
        Ok(ClipDecoder {
            bytes,
            reader,
            decoder,
            track,
            sample_rate,
            num_frames,
            at: 0,
            untouched: true,
        })
    }

    /// The next packet's frames (possibly none), or `None` at the end.
    fn next_chunk(&mut self) -> Result<Option<Vec<Frame>>, FromFileError> {
        self.untouched = false;
        let packet = loop {
            match self.reader.next_packet()? {
                Some(packet) if packet.track_id == self.track => break packet,
                Some(_) => continue,
                None => return Ok(None),
            }
        };
        let buffer = self.decoder.decode(&packet)?;
        let mut samples: Vec<f32> = Vec::new();
        buffer.copy_to_vec_interleaved(&mut samples);
        match buffer.num_planes() {
            1 => Ok(Some(samples.into_iter().map(Frame::from_mono).collect())),
            2 => Ok(Some(
                samples
                    .as_chunks()
                    .0
                    .iter()
                    .map(|&[left, right]| Frame::new(left, right))
                    .collect(),
            )),
            _ => Err(FromFileError::UnsupportedChannelConfiguration),
        }
    }
}

impl Decoder for ClipDecoder {
    type Error = FromFileError;

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn num_frames(&self) -> usize {
        self.num_frames.unwrap_or(0)
    }

    fn decode(&mut self) -> Result<Vec<Frame>, FromFileError> {
        let frames = match self.next_chunk()? {
            Some(frames) => frames,
            // A file that counts more frames than it holds would leave Kira
            // waiting for the rest forever, so make them up in silence.
            None => vec![Frame::ZERO; self.num_frames().saturating_sub(self.at).min(4_096)],
        };
        self.at += frames.len();
        Ok(frames)
    }

    /// Reopens the clip and reports the start, which Kira then decodes
    /// forward from. Hearken only ever seeks to the start (a loop wrapping),
    /// and reopening replays it exactly, where a container seek would not
    /// (libopus trims the pre-skip only once).
    fn seek(&mut self, _index: usize) -> Result<usize, FromFileError> {
        if !self.untouched {
            *self = ClipDecoder::open(self.bytes.clone())?;
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPUS: &[u8] = include_bytes!("../tests/fixtures/tone.opus");

    /// Every frame `decode` yields until it yields none.
    fn drain(clip: &mut ClipDecoder) -> Vec<Frame> {
        let mut frames = Vec::new();
        loop {
            let chunk = clip.decode().unwrap();
            if chunk.is_empty() {
                return frames;
            }
            frames.extend(chunk);
        }
    }

    #[test]
    fn an_opus_clip_streams_exactly_the_frames_it_counts() {
        let mut clip = ClipDecoder::open(OPUS.into()).unwrap();
        let count = clip.num_frames();
        assert_eq!(count, 48_000, "1 s at 48 kHz, its pre-skip not counted");
        assert_eq!(drain(&mut clip).len(), count);
    }

    #[test]
    fn a_clip_that_over_counts_pads_with_silence_to_its_count() {
        let mut clip = ClipDecoder::open(OPUS.into()).unwrap();
        clip.num_frames = Some(clip.num_frames() + 5_000);
        let frames = drain(&mut clip);
        assert_eq!(
            frames.len(),
            clip.num_frames(),
            "Kira reaches the end it was promised"
        );
        assert!(frames[frames.len() - 5_000..]
            .iter()
            .all(|f| *f == Frame::ZERO));
    }

    #[test]
    fn seeking_back_to_the_start_replays_the_same_frames() {
        let mut clip = ClipDecoder::open(OPUS.into()).unwrap();
        let first = drain(&mut clip);
        assert_eq!(clip.seek(0).unwrap(), 0);
        assert!(drain(&mut clip) == first, "the second pass differs");
    }
}
