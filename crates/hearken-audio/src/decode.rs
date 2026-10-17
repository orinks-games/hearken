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
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::units::{Time, TimeBase};
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

/// Loop points a clip carries in its own file, in frames from its start:
/// repeat `start..end`, or `start..` to the clip's end when `end` is `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LoopPoints {
    pub(crate) start: usize,
    pub(crate) end: Option<usize>,
}

/// What a clip decodes to: its rate, and its length where the file says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipInfo {
    pub sample_rate: u32,
    /// Seconds, when the container knows its frame count up front.
    pub length_s: Option<f64>,
}

/// Opens `bytes` with the codecs the Kira backend plays through and decodes
/// their first packet of audio, without a device: a game's start-up check
/// that its packed sounds will play, or a quick look at a clip's length.
pub fn probe_clip(bytes: Arc<[u8]>) -> Result<ClipInfo, String> {
    let mut clip = ClipDecoder::open(bytes).map_err(|err| err.to_string())?;
    // A codec may hand back nothing for its first packets (Vorbis primes
    // its window on the first), so read on until a chunk has frames.
    loop {
        match clip.next_chunk().map_err(|err| err.to_string())? {
            Some(frames) if !frames.is_empty() => break,
            Some(_) => continue,
            None => return Err("no audio".into()),
        }
    }
    let rate = clip.sample_rate;
    Ok(ClipInfo {
        sample_rate: rate,
        length_s: clip.num_frames.map(|n| n as f64 / f64::from(rate)),
    })
}

/// Decodes a whole clip into memory, with any loop points its file carries.
pub(crate) fn decode_whole(
    bytes: Arc<[u8]>,
) -> Result<(StaticSoundData, Option<LoopPoints>), FromFileError> {
    let mut clip = ClipDecoder::open(bytes)?;
    let mut frames = Vec::new();
    while let Some(chunk) = clip.next_chunk()? {
        frames.extend(chunk);
    }
    let points = clip.loop_points;
    Ok((
        StaticSoundData {
            sample_rate: clip.sample_rate,
            frames: frames.into(),
            settings: StaticSoundSettings::default(),
            slice: None,
        },
        points,
    ))
}

/// Streams a clip from a decoder thread as Kira plays it, with any loop
/// points its file carries.
pub(crate) fn stream(
    bytes: Arc<[u8]>,
) -> Result<(StreamingSoundData<FromFileError>, Option<LoopPoints>), FromFileError> {
    let clip = ClipDecoder::open(bytes)?;
    if clip.num_frames.is_none() {
        return Err(FromFileError::UnknownDuration);
    }
    let points = clip.loop_points;
    Ok((StreamingSoundData::from_decoder(clip), points))
}

/// The loop points in a clip's file, if it has usable ones: a WAV's `smpl`
/// chunk (what Wavosaur, GoldWave, Edison and most samplers write), or the
/// `LOOPSTART` tag with `LOOPEND` or `LOOPLENGTH` in an Ogg, Opus or FLAC
/// file's comments (the RPG Maker convention, which any tag editor can add),
/// all counted in samples. Points past the clip's end, or an end not after
/// the start, are ignored, so a bad tag loops the whole clip instead.
fn file_loop_points(
    bytes: &[u8],
    reader: &mut dyn FormatReader,
    num_frames: Option<usize>,
) -> Option<LoopPoints> {
    let points = wav_smpl_loop(bytes).or_else(|| tagged_loop(reader))?;
    let fits = |at: usize| num_frames.is_none_or(|n| at <= n);
    let ordered = points.end.is_none_or(|end| end > points.start);
    (fits(points.start) && points.end.is_none_or(fits) && ordered).then_some(points)
}

/// The first loop in a RIFF WAVE file's `smpl` chunk. Its end sample is
/// inclusive, so the region's end is one past it.
fn wav_smpl_loop(bytes: &[u8]) -> Option<LoopPoints> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u32_at = |data: &[u8], at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4)? as usize;
        let body = bytes.get(at + 8..(at + 8).saturating_add(size))?;
        if id == b"smpl" {
            // 36 bytes of header, the loop count at 28, then 24-byte loops:
            // cue id, type, start, end, fraction, play count.
            if u32_at(body, 28)? == 0 {
                return None;
            }
            let start = u32_at(body, 36 + 8)? as usize;
            let end = u32_at(body, 36 + 12)? as usize;
            return Some(LoopPoints {
                start,
                end: Some(end + 1),
            });
        }
        // Chunks are padded to an even length.
        at += 8 + size + (size & 1);
    }
    None
}

/// `LOOPSTART` with `LOOPEND` or `LOOPLENGTH`, from a file's comments.
fn tagged_loop(reader: &mut dyn FormatReader) -> Option<LoopPoints> {
    use symphonia::core::meta::RawValue;
    let mut metadata = reader.metadata();
    let revision = metadata.skip_to_latest()?;
    let tags = revision.media.tags.iter().chain(
        revision
            .per_track
            .iter()
            .flat_map(|t| t.metadata.tags.iter()),
    );
    let (mut start, mut end, mut length) = (None, None, None);
    for tag in tags {
        let value = match &tag.raw.value {
            RawValue::String(s) => s.trim().parse::<usize>().ok(),
            RawValue::UnsignedInt(n) => usize::try_from(*n).ok(),
            RawValue::SignedInt(n) => usize::try_from(*n).ok(),
            _ => None,
        };
        match tag.raw.key.to_ascii_uppercase().as_str() {
            "LOOPSTART" | "LOOP_START" => start = value,
            "LOOPEND" | "LOOP_END" => end = value,
            "LOOPLENGTH" | "LOOP_LENGTH" => length = value,
            _ => {}
        }
    }
    let start = start?;
    let end = end.or_else(|| length.map(|length| start + length));
    Some(LoopPoints { start, end })
}

struct ClipDecoder {
    /// Kept to reopen the clip when Kira seeks back to its start.
    bytes: Arc<[u8]>,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    /// The track's timestamp unit, for turning a container seek's landing
    /// point back into frames.
    time_base: Option<TimeBase>,
    sample_rate: u32,
    /// Known up front for most files; streaming needs it, decoding whole
    /// does not.
    num_frames: Option<usize>,
    /// Loop points the file carries, if any.
    loop_points: Option<LoopPoints>,
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
        let time_base = track.time_base;
        let track = track.id;
        let mut reader = reader;
        let loop_points = file_loop_points(&bytes, reader.as_mut(), num_frames);
        Ok(ClipDecoder {
            bytes,
            reader,
            decoder,
            track,
            time_base,
            sample_rate,
            num_frames,
            loop_points,
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

/// A seek closer than this to the start decodes forward from the start
/// instead: cheap, and exact.
const SEEK_AFTER_S: usize = 2;

impl ClipDecoder {
    /// A container seek to at or before frame `index`; the frame it landed
    /// on, or `None` if the reader cannot seek there.
    fn seek_near(&mut self, index: usize) -> Option<usize> {
        let time = Time::try_from_secs_f64(index as f64 / f64::from(self.sample_rate))?;
        let seeked = self
            .reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track),
                },
            )
            .ok()?;
        self.decoder.reset();
        self.untouched = false;
        let landed = self.time_base?.calc_time(seeked.actual_ts)?.as_secs_f64();
        let at = (landed.max(0.0) * f64::from(self.sample_rate)).round() as usize;
        self.at = at.min(index);
        Some(self.at)
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

    /// To the start, or to a loop region's start near it (a loop
    /// wrapping): reopens the clip and reports the start, which Kira then
    /// decodes forward from to the frame it asked for. Reopening replays the
    /// clip exactly, where a container seek would not (libopus trims the
    /// pre-skip only once). Further in (a programme tuned into ten minutes
    /// in) a container seek lands on a packet at or before the frame and
    /// reports where, so Kira decodes forward only from there, not ten
    /// minutes of audio; a container that cannot seek falls back to the
    /// reopen.
    fn seek(&mut self, index: usize) -> Result<usize, FromFileError> {
        if !self.untouched {
            *self = ClipDecoder::open(self.bytes.clone())?;
        }
        if index < self.sample_rate as usize * SEEK_AFTER_S {
            return Ok(0);
        }
        match self.seek_near(index) {
            Some(at) => Ok(at),
            None => {
                *self = ClipDecoder::open(self.bytes.clone())?;
                Ok(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPUS: &[u8] = include_bytes!("../tests/fixtures/tone.opus");
    const TAGGED: &[u8] = include_bytes!("../tests/fixtures/loop_tagged.opus");

    /// A WAV of `frames` silent mono frames with a `smpl` loop of
    /// `start..=end`.
    fn wav_with_smpl(frames: usize, start: u32, end: u32) -> Arc<[u8]> {
        let mut wav = hearken_synth::wav_bytes(&vec![0.0; frames], 44_100);
        let mut smpl = vec![0u8; 36];
        smpl[28..32].copy_from_slice(&1u32.to_le_bytes());
        for field in [0, 0, start, end, 0, 0] {
            smpl.extend(field.to_le_bytes());
        }
        wav.extend(b"smpl");
        wav.extend((smpl.len() as u32).to_le_bytes());
        wav.extend(smpl);
        let riff = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&riff.to_le_bytes());
        wav.into()
    }

    #[test]
    fn a_wavs_smpl_chunk_gives_its_loop_with_the_end_inclusive() {
        let clip = ClipDecoder::open(wav_with_smpl(1_000, 100, 799)).unwrap();
        assert_eq!(
            clip.loop_points,
            Some(LoopPoints {
                start: 100,
                end: Some(800)
            })
        );
    }

    #[test]
    fn loop_points_past_the_end_are_ignored() {
        let clip = ClipDecoder::open(wav_with_smpl(1_000, 100, 5_000)).unwrap();
        assert_eq!(clip.loop_points, None);
    }

    #[test]
    fn an_opus_files_loop_tags_give_its_loop() {
        let clip = ClipDecoder::open(TAGGED.into()).unwrap();
        assert_eq!(
            clip.loop_points,
            Some(LoopPoints {
                start: 14_400,
                end: Some(28_800)
            })
        );
    }

    #[test]
    fn a_clip_without_loop_points_has_none() {
        assert_eq!(ClipDecoder::open(OPUS.into()).unwrap().loop_points, None);
    }

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
