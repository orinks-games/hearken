//! Radio's own codec registry: Symphonia's enabled codecs (with AAC, for
//! the many AAC stations) and libopus, which Symphonia lacks. On Windows and
//! macOS the system's AAC decoder takes AAC, adding HE-AAC (see
//! `system_aac`); under `fdk-aac`, FDK AAC takes it instead.

use std::sync::OnceLock;

use symphonia::core::codecs::registry::CodecRegistry;
use symphonia_adapter_libopus::OpusDecoder;

pub(crate) fn registry() -> &'static CodecRegistry {
    static CODECS: OnceLock<CodecRegistry> = OnceLock::new();
    CODECS.get_or_init(|| {
        let mut codecs = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut codecs);
        // Preferred, not Standard: at one tier the decoder registered last
        // takes the codec. At Preferred, FDK wins in either order.
        #[cfg(feature = "fdk-aac")]
        codecs.register_audio_decoder_at_tier::<symphonia_adapter_fdk_aac::AacDecoder>(
            symphonia::core::common::Tier::Preferred,
        );
        #[cfg(all(any(windows, target_os = "macos"), not(feature = "fdk-aac")))]
        codecs.register_audio_decoder_at_tier::<crate::system_aac::SystemAac>(
            symphonia::core::common::Tier::Preferred,
        );
        codecs.register_audio_decoder::<OpusDecoder>();
        codecs
    })
}

#[cfg(all(test, feature = "fdk-aac"))]
mod tests {
    use symphonia::core::audio::Channels;
    use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
    use symphonia::core::codecs::audio::AudioCodecParameters;

    use super::registry;

    /// Both AAC decoders call themselves "aac", so the profiles tell them
    /// apart: only FDK lists HE-AAC.
    #[test]
    fn fdk_aac_takes_aac_when_enabled() {
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_AAC)
            .with_sample_rate(44_100)
            .with_channels(Channels::Discrete(2));
        let decoder = registry()
            .make_audio_decoder(&params, &Default::default())
            .expect("an AAC decoder");
        let profiles = decoder.codec_info().profiles;
        assert!(profiles.iter().any(|p| p.short_name == "aac-he"));
    }
}
