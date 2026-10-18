//! An AAC stream's core layer, read from what Symphonia knows of it, and the
//! AudioSpecificConfig a system decoder is opened with.

use symphonia::core::codecs::audio::AudioCodecParameters;

/// MPEG-4 sample rates, by sampling frequency index.
const RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// Audio object types this module tells apart.
const AOT_LC: u32 = 2;
const AOT_SBR: u32 = 5;
const AOT_PS: u32 = 29;

/// The AAC-LC layer every HE-AAC stream is built on, and what the stream
/// says about the layers on top of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Core {
    /// Sampling frequency index of the core layer (before SBR).
    pub rate_index: u8,
    /// Channel configuration of the core layer, 1 to 7 (before PS).
    pub channels: u8,
    /// Whether the stream says it carries SBR, when it says. ADTS and short
    /// configurations leave it unsaid: the decoder finds it in the audio.
    pub sbr: Option<bool>,
    /// Whether the stream says it carries parametric stereo, when it says.
    pub ps: Option<bool>,
}

impl Core {
    /// The core from Symphonia's codec parameters: the AudioSpecificConfig
    /// when the container carried one (MP4), otherwise the rate and channels
    /// an ADTS header gave. `None` for a stream no system decoder takes.
    pub fn from_params(params: &AudioCodecParameters) -> Option<Self> {
        if let Some(asc) = params.extra_data.as_deref() {
            return Self::from_asc(asc);
        }
        let rate_index = rate_index(params.sample_rate?)?;
        let channels = match params.channels.as_ref()?.count() {
            n @ 1..=6 => n as u8,
            8 => 7,
            _ => return None,
        };
        Some(Self {
            rate_index,
            channels,
            sbr: None,
            ps: None,
        })
    }

    /// The core from an AudioSpecificConfig (ISO/IEC 14496-3, 1.6.2.1),
    /// signalled either way: hierarchically (object type 5 or 29 first) or
    /// backward compatibly (LC first, then the 0x2b7 sync extension).
    pub fn from_asc(asc: &[u8]) -> Option<Self> {
        let mut bits = Bits::new(asc);
        let mut aot = bits.object_type()?;
        let rate_index = bits.rate_index()?;
        let channels = bits.read(4)? as u8;
        let (mut sbr, mut ps) = (None, None);
        if aot == AOT_SBR || aot == AOT_PS {
            sbr = Some(true);
            ps = Some(aot == AOT_PS);
            bits.rate_index()?;
            aot = bits.object_type()?;
        }
        // Only an LC core of 1024-sample frames with a channel configuration
        // (not a program config element) is something every system decoder
        // takes.
        if aot != AOT_LC || !(1..=7).contains(&channels) {
            return None;
        }
        // GASpecificConfig: frameLengthFlag, dependsOnCoreCoder, extensionFlag.
        if bits.read(1)? != 0 {
            return None;
        }
        if bits.read(1)? != 0 {
            bits.read(14)?;
        }
        bits.read(1)?;
        if sbr.is_none() && bits.read(11) == Some(0x2b7) && bits.object_type() == Some(AOT_SBR) {
            let present = bits.read(1) == Some(1);
            sbr = Some(present);
            if present {
                bits.rate_index()?;
                if bits.read(11) == Some(0x548) {
                    ps = Some(bits.read(1) == Some(1));
                }
            }
        }
        // A rate past the table (escaped) has no index to signal it with.
        if rate_index > 12 {
            return None;
        }
        Some(Self {
            rate_index,
            channels,
            sbr,
            ps,
        })
    }

    /// The core sample rate in Hz.
    pub fn rate(&self) -> u32 {
        RATES[usize::from(self.rate_index)]
    }

    /// Channels in the core layer, the LFE included.
    pub fn channel_count(&self) -> u32 {
        match self.channels {
            7 => 8,
            n => u32::from(n),
        }
    }

    /// Whether the stream most likely carries SBR: it says so, or, unsaid,
    /// its core runs at 24 kHz or below, where AAC-LC alone is rare and
    /// HE-AAC is the rule (an SBR stream's core is half its output rate).
    #[cfg(any(target_os = "macos", test))]
    pub fn likely_sbr(&self) -> bool {
        self.sbr.unwrap_or(self.rate() <= 24_000)
    }

    /// Whether the stream most likely carries parametric stereo: it says
    /// so, or, unsaid, it is a mono HE-AAC core, which is what HE-AAC v2 is.
    #[cfg(any(target_os = "macos", test))]
    pub fn likely_ps(&self) -> bool {
        self.ps.unwrap_or(self.likely_sbr() && self.channels == 1)
    }

    /// A two-byte AudioSpecificConfig naming only the LC core. A decoder
    /// given it finds SBR and PS in the audio itself (implicit signalling).
    pub fn implicit_asc(&self) -> [u8; 2] {
        let mut bits = Writer::default();
        bits.write(AOT_LC, 5);
        bits.write(u32::from(self.rate_index), 4);
        bits.write(u32::from(self.channels), 4);
        bits.write(0, 3);
        let bytes = bits.finish();
        [bytes[0], bytes[1]]
    }

    /// An AudioSpecificConfig that names SBR, and PS when likely, outright
    /// (hierarchical signalling), for a decoder that is told the layers
    /// rather than finding them. Only the implicit one when SBR is unlikely.
    #[cfg(any(target_os = "macos", test))]
    pub fn explicit_asc(&self) -> Vec<u8> {
        if !self.likely_sbr() {
            return self.implicit_asc().to_vec();
        }
        let mut bits = Writer::default();
        bits.write(if self.likely_ps() { AOT_PS } else { AOT_SBR }, 5);
        bits.write(u32::from(self.rate_index), 4);
        bits.write(u32::from(self.channels), 4);
        // SBR doubles the rate: three places up the table.
        bits.write(u32::from(self.rate_index.saturating_sub(3)), 4);
        bits.write(AOT_LC, 5);
        bits.write(0, 3);
        bits.finish()
    }
}

fn rate_index(rate: u32) -> Option<u8> {
    RATES.iter().position(|&r| r == rate).map(|i| i as u8)
}

/// Most significant bit first, as MPEG-4 writes them.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read(&mut self, count: usize) -> Option<u32> {
        let mut value = 0;
        for _ in 0..count {
            let byte = self.data.get(self.pos / 8)?;
            value = (value << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(value)
    }

    fn object_type(&mut self) -> Option<u32> {
        match self.read(5)? {
            31 => Some(32 + self.read(6)?),
            aot => Some(aot),
        }
    }

    /// A sampling frequency index; an escaped explicit rate maps to its
    /// index when it is a table rate, else to 15 (none).
    fn rate_index(&mut self) -> Option<u8> {
        match self.read(4)? as u8 {
            15 => Some(rate_index(self.read(24)?).unwrap_or(15)),
            index if index <= 12 => Some(index),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    used: usize,
}

impl Writer {
    fn write(&mut self, value: u32, count: usize) {
        for bit in (0..count).rev() {
            if self.used.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let last = self.bytes.len() - 1;
            self.bytes[last] |= (((value >> bit) & 1) as u8) << (7 - self.used % 8);
            self.used += 1;
        }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use symphonia::core::audio::Channels;
    use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;

    use super::*;

    #[test]
    fn adts_parameters_give_the_core() {
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_AAC)
            .with_sample_rate(24_000)
            .with_channels(Channels::Discrete(2));
        let core = Core::from_params(&params).expect("a core");
        assert_eq!((core.rate(), core.channel_count()), (24_000, 2));
        assert!(core.likely_sbr() && !core.likely_ps());
    }

    #[test]
    fn a_full_rate_core_is_plain_lc() {
        let core = Core::from_asc(&[0x12, 0x10]).expect("LC 44.1 kHz stereo");
        assert_eq!((core.rate(), core.channels), (44_100, 2));
        assert!(!core.likely_sbr());
        assert_eq!(core.implicit_asc(), [0x12, 0x10]);
        assert_eq!(core.explicit_asc(), [0x12, 0x10]);
    }

    #[test]
    fn a_mono_low_rate_core_is_likely_v2() {
        // LC, 24 kHz, mono.
        let core = Core::from_asc(&[0x13, 0x08]).expect("a core");
        assert!(core.likely_sbr() && core.likely_ps());
        // PS, 24 kHz, mono, SBR at 48 kHz, LC core, GASpecificConfig.
        let asc = core.explicit_asc();
        let read = Core::from_asc(&asc).expect("the explicit config reads back");
        assert_eq!((read.rate(), read.channels), (24_000, 1));
        assert_eq!((read.sbr, read.ps), (Some(true), Some(true)));
    }

    #[test]
    fn hierarchical_he_aac_reads_its_core() {
        // SBR, 24 kHz core, stereo, 48 kHz output, LC.
        let core = Core::from_asc(&[0x2b, 0x11, 0x88, 0x00]).expect("HE-AAC");
        assert_eq!((core.rate(), core.channels), (24_000, 2));
        assert_eq!((core.sbr, core.ps), (Some(true), Some(false)));
        assert_eq!(core.implicit_asc(), [0x13, 0x10]);
    }

    #[test]
    fn backward_compatible_signalling_says_no_sbr() {
        // LC 22.05 kHz stereo, then sync 0x2b7, SBR, not present.
        let core = Core::from_asc(&[0x13, 0x90, 0x56, 0xe5, 0x00]).expect("a core");
        assert_eq!(core.sbr, Some(false));
        assert!(!core.likely_sbr());
    }

    #[test]
    fn a_program_config_element_or_960_frames_is_refused() {
        assert_eq!(Core::from_asc(&[0x12, 0x00]), None);
        assert_eq!(Core::from_asc(&[0x12, 0x14]), None);
    }
}
