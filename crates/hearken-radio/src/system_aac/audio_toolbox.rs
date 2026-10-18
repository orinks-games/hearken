//! AudioToolbox's AAC decoder (macOS), through an AudioConverter: one raw
//! AAC frame in, float PCM out.
//!
//! An AudioConverter is told the layers rather than finding them, so the
//! stream's AudioSpecificConfig names SBR and PS outright, as the stream
//! said or as its core suggests (see `Core::likely_sbr`). A stream named
//! HE-AAC that carries no SBR still plays, at the doubled rate.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::{self, NonNull};

use objc2_audio_toolbox::{
    kAudioConverterDecompressionMagicCookie, kAudioFormatProperty_FormatList,
    AudioConverterDispose, AudioConverterFillComplexBuffer, AudioConverterNew, AudioConverterRef,
    AudioConverterReset, AudioConverterSetProperty, AudioFormatGetProperty,
    AudioFormatGetPropertyInfo, AudioFormatInfo,
};
use objc2_core_audio_types::{
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsPacked, kAudioFormatLinearPCM, kAudioFormatMPEG4AAC,
    kAudioFormatMPEG4AAC_HE, kAudioFormatMPEG4AAC_HE_V2, AudioBuffer, AudioBufferList,
    AudioFormatListItem, AudioStreamBasicDescription, AudioStreamPacketDescription,
};

use super::config::Core;

/// What the input callback returns once its one packet is used: not an
/// error, a pause until the next packet.
const NO_MORE: i32 = i32::from_be_bytes(*b"more");
/// Frames asked for per call: one HE-AAC frame.
const FRAMES: u32 = 2048;

pub(crate) struct Decoder {
    converter: AudioConverterRef,
    // Boxed: the converter keeps the packet and its description it was
    // last handed until it asks for the next, which may be in a later call.
    input: Box<Input>,
    rate: u32,
    channels: usize,
    out: Vec<f32>,
}

// SAFETY: an AudioConverter has no thread affinity, and the decoder only
// calls it through `&mut self`, so never from two threads at once. `&self`
// methods touch no converter at all.
unsafe impl Send for Decoder {}
unsafe impl Sync for Decoder {}

impl Decoder {
    pub(crate) fn open(core: Core) -> Result<Self, String> {
        let cookie = esds(&core.explicit_asc());
        let input = format_from_cookie(&cookie).unwrap_or_else(|| fallback_format(core));
        let channels = input.mChannelsPerFrame.max(1);
        let output = AudioStreamBasicDescription {
            mSampleRate: input.mSampleRate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
            mBytesPerPacket: 4 * channels,
            mFramesPerPacket: 1,
            mBytesPerFrame: 4 * channels,
            mChannelsPerFrame: channels,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        let mut converter: AudioConverterRef = ptr::null_mut();
        // SAFETY: both descriptions and the out pointer live across the call.
        let status = unsafe {
            AudioConverterNew(
                NonNull::from(&input),
                NonNull::from(&output),
                NonNull::from(&mut converter),
            )
        };
        if status != 0 || converter.is_null() {
            return Err(format!("AudioConverterNew: {status}"));
        }
        let decoder = Self {
            converter,
            input: Box::new(Input {
                held: Vec::new(),
                next: Vec::new(),
                channels: input.mChannelsPerFrame,
                desc: AudioStreamPacketDescription {
                    mStartOffset: 0,
                    mVariableFramesInPacket: 0,
                    mDataByteSize: 0,
                },
            }),
            rate: input.mSampleRate as u32,
            channels: channels as usize,
            out: Vec::new(),
        };
        // SAFETY: the cookie lives across the call, which copies it.
        let status = unsafe {
            AudioConverterSetProperty(
                decoder.converter,
                kAudioConverterDecompressionMagicCookie,
                cookie.len() as u32,
                NonNull::new_unchecked(cookie.as_ptr() as *mut c_void),
            )
        };
        if status != 0 {
            return Err(format!("magic cookie: {status}"));
        }
        Ok(decoder)
    }

    /// Decodes one raw AAC frame onto `out` as interleaved f32 and returns
    /// the rate and channel count it is in.
    pub(crate) fn decode(
        &mut self,
        frame: &[u8],
        out: &mut Vec<f32>,
    ) -> Result<(u32, usize), String> {
        self.input.next.clear();
        self.input.next.extend_from_slice(frame);
        self.out.resize(FRAMES as usize * self.channels, 0.0);
        loop {
            let mut frames = FRAMES;
            let mut list = AudioBufferList {
                mNumberBuffers: 1,
                mBuffers: [AudioBuffer {
                    mNumberChannels: self.channels as u32,
                    mDataByteSize: (self.out.len() * 4) as u32,
                    mData: self.out.as_mut_ptr().cast(),
                }],
            };
            // SAFETY: `input` and the output buffer outlive the call; the
            // callback touches only `input`, and the converter writes at
            // most `frames` frames, which the buffer holds.
            let status = unsafe {
                AudioConverterFillComplexBuffer(
                    self.converter,
                    Some(supply),
                    (&mut *self.input as *mut Input).cast(),
                    NonNull::from(&mut frames),
                    NonNull::from(&mut list),
                    ptr::null_mut(),
                )
            };
            if status != 0 && status != NO_MORE {
                return Err(format!("decode: {status}"));
            }
            let samples = (frames as usize * self.channels).min(self.out.len());
            out.extend_from_slice(&self.out[..samples]);
            if status == NO_MORE || frames == 0 {
                break;
            }
        }
        Ok((self.rate, self.channels))
    }

    pub(crate) fn reset(&mut self) {
        // SAFETY: the converter this decoder owns.
        unsafe {
            AudioConverterReset(self.converter);
        }
        self.input.next.clear();
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the converter this decoder owns, disposed once.
        unsafe {
            AudioConverterDispose(self.converter);
        }
    }
}

/// The packets the converter is handed.
struct Input {
    /// The packet it was last given, kept until it asks for another.
    held: Vec<u8>,
    /// The packet the current decode call brought, until it is asked for.
    next: Vec<u8>,
    channels: u32,
    desc: AudioStreamPacketDescription,
}

/// The converter's input callback: the call's packet once, then `NO_MORE`.
unsafe extern "C-unwind" fn supply(
    _converter: AudioConverterRef,
    packets: NonNull<u32>,
    data: NonNull<AudioBufferList>,
    descriptions: *mut *mut AudioStreamPacketDescription,
    user: *mut c_void,
) -> i32 {
    // SAFETY: `user` is the decoder's boxed `Input`, alive for the call;
    // the other pointers are the converter's own, valid for the callback.
    unsafe {
        let input = &mut *user.cast::<Input>();
        if input.next.is_empty() {
            *packets.as_ptr() = 0;
            return NO_MORE;
        }
        std::mem::swap(&mut input.held, &mut input.next);
        input.next.clear();
        input.desc.mDataByteSize = input.held.len() as u32;
        let list = &mut *data.as_ptr();
        list.mNumberBuffers = 1;
        list.mBuffers[0] = AudioBuffer {
            mNumberChannels: input.channels,
            mDataByteSize: input.held.len() as u32,
            mData: input.held.as_mut_ptr().cast(),
        };
        if !descriptions.is_null() {
            *descriptions = &mut input.desc;
        }
        *packets.as_ptr() = 1;
    }
    0
}

/// The best layer AudioToolbox reads from the cookie: HE-AAC v2, HE-AAC or
/// LC, in that order, with its output rate and channels.
fn format_from_cookie(cookie: &[u8]) -> Option<AudioStreamBasicDescription> {
    let info = AudioFormatInfo {
        mASBD: AudioStreamBasicDescription {
            mSampleRate: 0.0,
            mFormatID: kAudioFormatMPEG4AAC,
            mFormatFlags: 0,
            mBytesPerPacket: 0,
            mFramesPerPacket: 0,
            mBytesPerFrame: 0,
            mChannelsPerFrame: 0,
            mBitsPerChannel: 0,
            mReserved: 0,
        },
        mMagicCookie: NonNull::new(cookie.as_ptr() as *mut c_void)?,
        mMagicCookieSize: cookie.len() as u32,
    };
    let specifier = (&info as *const AudioFormatInfo).cast::<c_void>();
    let mut size = 0u32;
    // SAFETY: the specifier and cookie live across both calls, and the list
    // is sized from what the first call reports.
    unsafe {
        let status = AudioFormatGetPropertyInfo(
            kAudioFormatProperty_FormatList,
            size_of::<AudioFormatInfo>() as u32,
            specifier,
            NonNull::from(&mut size),
        );
        let count = size as usize / size_of::<AudioFormatListItem>();
        if status != 0 || count == 0 {
            return None;
        }
        let mut items: Vec<AudioFormatListItem> = Vec::with_capacity(count);
        let status = AudioFormatGetProperty(
            kAudioFormatProperty_FormatList,
            size_of::<AudioFormatInfo>() as u32,
            specifier,
            &mut size,
            items.as_mut_ptr().cast(),
        );
        if status != 0 {
            return None;
        }
        items.set_len((size as usize / size_of::<AudioFormatListItem>()).min(count));
        items
            .first()
            .map(|item| item.mASBD)
            .filter(|asbd| asbd.mSampleRate > 0.0 && asbd.mChannelsPerFrame > 0)
    }
}

/// The input format from the core alone, when AudioToolbox lists none.
fn fallback_format(core: Core) -> AudioStreamBasicDescription {
    let (id, rate, channels, frames) = if core.likely_ps() {
        (kAudioFormatMPEG4AAC_HE_V2, core.rate() * 2, 2, 2048)
    } else if core.likely_sbr() {
        (
            kAudioFormatMPEG4AAC_HE,
            core.rate() * 2,
            core.channel_count(),
            2048,
        )
    } else {
        (
            kAudioFormatMPEG4AAC,
            core.rate(),
            core.channel_count(),
            1024,
        )
    };
    AudioStreamBasicDescription {
        mSampleRate: f64::from(rate),
        mFormatID: id,
        mFormatFlags: 0,
        mBytesPerPacket: 0,
        mFramesPerPacket: frames,
        mBytesPerFrame: 0,
        mChannelsPerFrame: channels,
        mBitsPerChannel: 0,
        mReserved: 0,
    }
}

/// An MPEG-4 elementary stream descriptor around `asc`, the magic cookie
/// AudioToolbox reads AAC's configuration from (ISO/IEC 14496-1, 7.2.6).
fn esds(asc: &[u8]) -> Vec<u8> {
    // Object type 0x40 (MPEG-4 audio), stream type 0x15 (audio), then a
    // zero buffer size and bit rates.
    let mut config = vec![0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    config.extend(descriptor(0x05, asc));
    // ES_ID 0 and no flags, the decoder config, then SL config 2 (MP4).
    let mut es = vec![0, 0, 0];
    es.extend(descriptor(0x04, &config));
    es.extend(descriptor(0x06, &[0x02]));
    descriptor(0x03, &es)
}

/// A descriptor: its tag, its length in the four-byte form, its body.
fn descriptor(tag: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len() as u32;
    let mut out = vec![
        tag,
        0x80 | (len >> 21) as u8 & 0x7f,
        0x80 | (len >> 14) as u8 & 0x7f,
        0x80 | (len >> 7) as u8 & 0x7f,
        len as u8 & 0x7f,
    ];
    out.extend_from_slice(body);
    out
}
