//! AAC through the operating system's own decoder: Media Foundation on
//! Windows, AudioToolbox on macOS. Both decode HE-AAC (SBR) and HE-AAC v2
//! (parametric stereo), which Symphonia's decoder does not: it plays only
//! the AAC-LC core, at half the rate and, for v2, in mono. Microsoft and
//! Apple license the AAC patents for their decoders, so a game using them
//! ships no AAC decoder of its own and needs no licence of its own.
//!
//! Where the system decoder cannot open (a Windows N edition without the
//! Media Feature Pack, or a stream it does not take), Symphonia's decoder
//! plays the stream instead, as before.

#[cfg(target_os = "macos")]
mod audio_toolbox;
pub(crate) mod config;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
mod mfplat;

#[cfg(target_os = "macos")]
use audio_toolbox::Decoder as Platform;
#[cfg(windows)]
use mf::Decoder as Platform;

#[cfg(any(windows, target_os = "macos"))]
pub(crate) use decoder::SystemAac;

#[cfg(any(windows, target_os = "macos"))]
mod decoder {
    use std::sync::atomic::{AtomicBool, Ordering};

    use symphonia::core::audio::{
        AsGenericAudioBufferRef, Audio, AudioBuffer, AudioMut, AudioSpec, Channels,
        GenericAudioBufferRef,
    };
    use symphonia::core::codecs::audio::well_known::profiles::{
        CODEC_PROFILE_AAC_HE, CODEC_PROFILE_AAC_HE_V2, CODEC_PROFILE_AAC_LC,
    };
    use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
    use symphonia::core::codecs::audio::{
        AudioCodecParameters, AudioDecoder, AudioDecoderOptions, FinalizeResult,
    };
    use symphonia::core::codecs::registry::{RegisterableAudioDecoder, SupportedAudioCodec};
    use symphonia::core::codecs::CodecInfo;
    use symphonia::core::errors::{Error, Result};
    use symphonia::core::packet::PacketRef;
    use symphonia::core::{codec_profile, support_audio_codec};

    use super::config::Core;
    use super::Platform;

    /// Frames an output buffer starts with: one HE-AAC frame.
    const FRAMES: usize = 2048;

    /// A system AAC decoder behind Symphonia's decoder interface.
    pub(crate) struct SystemAac {
        platform: Platform,
        params: AudioCodecParameters,
        buf: AudioBuffer<f32>,
        pcm: Vec<f32>,
    }

    impl SystemAac {
        fn try_new(
            params: &AudioCodecParameters,
            opts: &AudioDecoderOptions,
        ) -> Result<Box<dyn AudioDecoder>> {
            let opened = Core::from_params(params)
                .ok_or_else(|| "a stream the system decoder does not take".to_string())
                .and_then(Platform::open);
            match opened {
                Ok(platform) => Ok(Box::new(Self {
                    platform,
                    params: params.clone(),
                    buf: AudioBuffer::new(AudioSpec::new(1, Channels::Discrete(1)), 0),
                    pcm: Vec::new(),
                })),
                Err(why) => {
                    // Once: a station that reconnects reopens its decoder.
                    static SAID: AtomicBool = AtomicBool::new(false);
                    if !SAID.swap(true, Ordering::Relaxed) {
                        log::warn!("radio: system AAC decoder unavailable ({why}); HE-AAC plays its base layer only");
                    }
                    symphonia::default::codecs::AacDecoder::try_registry_new(params, opts)
                }
            }
        }
    }

    impl AudioDecoder for SystemAac {
        fn reset(&mut self) {
            self.platform.reset();
            self.buf.clear();
        }

        fn codec_info(&self) -> &CodecInfo {
            &Self::supported_codecs()[0].info
        }

        fn codec_params(&self) -> &AudioCodecParameters {
            &self.params
        }

        fn decode_ref(&mut self, packet: &PacketRef) -> Result<GenericAudioBufferRef<'_>> {
            self.pcm.clear();
            self.buf.clear();
            let (rate, channels) =
                self.platform
                    .decode(packet.data, &mut self.pcm)
                    .map_err(|why| {
                        log::debug!("radio: system AAC decoder: {why}");
                        Error::DecodeError("aac: the system decoder refused a packet")
                    })?;
            if channels == 0 || rate == 0 {
                return Ok(self.buf.as_generic_audio_buffer_ref());
            }
            let frames = self.pcm.len() / channels;
            let spec = AudioSpec::new(rate, Channels::Discrete(channels as u16));
            if *self.buf.spec() != spec || self.buf.capacity() < frames {
                self.buf = AudioBuffer::new(spec, frames.max(FRAMES));
            }
            self.buf.render_uninit(Some(frames));
            self.buf
                .copy_from_slice_interleaved(&&self.pcm[..frames * channels]);
            Ok(self.buf.as_generic_audio_buffer_ref())
        }

        fn finalize(&mut self) -> FinalizeResult {
            FinalizeResult::default()
        }

        fn last_decoded(&self) -> GenericAudioBufferRef<'_> {
            self.buf.as_generic_audio_buffer_ref()
        }
    }

    impl RegisterableAudioDecoder for SystemAac {
        fn try_registry_new(
            params: &AudioCodecParameters,
            opts: &AudioDecoderOptions,
        ) -> Result<Box<dyn AudioDecoder>> {
            Self::try_new(params, opts)
        }

        fn supported_codecs() -> &'static [SupportedAudioCodec] {
            &[support_audio_codec!(
                CODEC_ID_AAC,
                "aac",
                "Advanced Audio Coding",
                &[
                    codec_profile!(CODEC_PROFILE_AAC_LC, "aac-lc", "Low Complexity"),
                    codec_profile!(CODEC_PROFILE_AAC_HE, "aac-he", "High Efficiency"),
                    codec_profile!(CODEC_PROFILE_AAC_HE_V2, "aac-he-v2", "High Efficiency V2"),
                ]
            )]
        }
    }
}

/// HE-AAC fixtures, 48 kHz stereo: 1 kHz on the left, 3 kHz on the right,
/// and 15 kHz on both, which only SBR carries (the core stops at 12 kHz).
/// FFmpeg's decoder hears the tones on their own sides, a thousand to one.
/// Encoded with FDK AAC's encoder, used only to make the files: `tone-he.aac`
/// is HE-AAC at 64 kb/s, `tone-hev2.aac` HE-AAC v2 at 32 kb/s, both ADTS, and
/// `tone-hev2.m4a` is the latter in MP4 (`ffmpeg -i tone-hev2.aac -c copy`).
#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::TrackType;
    use symphonia::core::io::MediaSourceStream;

    use crate::codecs;

    /// Whether this build decodes the full stream: the system decoder, or
    /// FDK AAC.
    const FULL: bool = cfg!(any(windows, target_os = "macos", feature = "fdk-aac"));

    /// The decoded rate and interleaved stereo frames of a fixture.
    fn decode(bytes: &'static [u8], extension: &str) -> (u32, Vec<[f32; 2]>) {
        let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
        let mut hint = Hint::new();
        hint.with_extension(extension);
        let mut reader = symphonia::default::get_probe()
            .probe(&hint, stream, Default::default(), Default::default())
            .expect("the fixture probes");
        let track = reader.default_track(TrackType::Audio).expect("a track");
        let id = track.id;
        let Some(symphonia::core::codecs::CodecParameters::Audio(params)) = &track.codec_params
        else {
            panic!("an audio track");
        };
        let mut decoder = codecs::registry()
            .make_audio_decoder(params, &Default::default())
            .expect("a decoder");
        let (mut rate, mut frames, mut samples) = (0, Vec::new(), Vec::new());
        while let Ok(Some(packet)) = reader.next_packet() {
            if packet.track_id != id {
                continue;
            }
            let Ok(buffer) = decoder.decode(&packet) else {
                continue;
            };
            let channels = buffer.num_planes();
            if channels == 0 || buffer.frames() == 0 {
                continue;
            }
            rate = buffer.spec().rate();
            buffer.copy_to_vec_interleaved(&mut samples);
            let right = channels.min(2) - 1;
            frames.extend(samples.chunks_exact(channels).map(|f| [f[0], f[right]]));
        }
        (rate, frames)
    }

    /// Power at `hz`, over a second from the middle of the tone.
    fn power(frames: &[[f32; 2]], rate: u32, side: usize, hz: f32) -> f32 {
        let start = frames.len() / 4;
        let window = &frames[start..start + rate as usize / 2];
        let step = std::f32::consts::TAU * hz / rate as f32;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, frame) in window.iter().enumerate() {
            let phase = step * n as f32;
            re += frame[side] * phase.cos();
            im += frame[side] * phase.sin();
        }
        (re * re + im * im) / (window.len() as f32).powi(2)
    }

    /// The left channel's energy from 12.5 to 20 kHz, where only SBR puts
    /// anything, against 0.5 to 12 kHz, over 2048 frames from the middle.
    fn high_band(frames: &[[f32; 2]], rate: u32) -> f64 {
        const N: usize = 2048;
        let start = frames.len() / 4;
        let window = &frames[start..start + N];
        let band = |lo: f64, hi: f64| -> f64 {
            let bins = (lo * N as f64 / f64::from(rate)) as usize
                ..(hi * N as f64 / f64::from(rate)) as usize;
            bins.map(|bin| {
                let step = std::f64::consts::TAU * bin as f64 / N as f64;
                let (re, im) = window
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(re, im), (k, f)| {
                        let phase = step * k as f64;
                        (
                            re + f64::from(f[0]) * phase.cos(),
                            im + f64::from(f[0]) * phase.sin(),
                        )
                    });
                re * re + im * im
            })
            .sum()
        };
        band(12_500.0, 20_000.0) / band(500.0, 12_000.0)
    }

    fn check(bytes: &'static [u8], extension: &str) {
        let (rate, frames) = decode(bytes, extension);
        assert!(
            frames.len() > rate as usize,
            "{extension}: a second of audio"
        );
        if !FULL {
            // Symphonia's decoder: the core alone.
            assert_eq!(rate, 24_000, "{extension}");
            return;
        }
        assert_eq!(rate, 48_000, "{extension}: SBR doubles the core's rate");
        // FFmpeg's decoder gives 0.12 for HE-AAC and 0.02 for v2.
        let high = high_band(&frames, rate);
        assert!(high > 0.005, "{extension}: SBR's high band ({high})");
        let left = (
            power(&frames, rate, 0, 1_000.0),
            power(&frames, rate, 1, 1_000.0),
        );
        let right = (
            power(&frames, rate, 0, 3_000.0),
            power(&frames, rate, 1, 3_000.0),
        );
        assert!(
            left.0 > 2.0 * left.1,
            "{extension}: 1 kHz on the left ({left:?})"
        );
        assert!(
            right.1 > 2.0 * right.0,
            "{extension}: 3 kHz on the right ({right:?})"
        );
    }

    #[test]
    fn he_aac_plays_in_full() {
        check(include_bytes!("../../tests/fixtures/tone-he.aac"), "aac");
    }

    #[test]
    fn he_aac_v2_plays_in_stereo() {
        check(include_bytes!("../../tests/fixtures/tone-hev2.aac"), "aac");
    }

    #[test]
    fn he_aac_v2_in_mp4_plays_in_stereo() {
        check(include_bytes!("../../tests/fixtures/tone-hev2.m4a"), "m4a");
    }
}
