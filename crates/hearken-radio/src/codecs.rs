//! Radio's own codec registry: Symphonia's enabled codecs (with AAC, for
//! the many AAC stations) and libopus, which Symphonia lacks.

use std::sync::OnceLock;

use symphonia::core::codecs::registry::CodecRegistry;
use symphonia_adapter_libopus::OpusDecoder;

pub(crate) fn registry() -> &'static CodecRegistry {
    static CODECS: OnceLock<CodecRegistry> = OnceLock::new();
    CODECS.get_or_init(|| {
        let mut codecs = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut codecs);
        codecs.register_audio_decoder::<OpusDecoder>();
        codecs
    })
}
