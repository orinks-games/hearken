//! Media Foundation's AAC decoder (Windows 7 and later), driven directly as
//! a synchronous transform: one raw AAC frame in, float PCM out.

use std::mem::ManuallyDrop;

use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaType, IMFSample, IMFTransform, MFAudioFormat_AAC, MFAudioFormat_Float,
    MFMediaType_Audio, MFSTARTUP_LITE, MFT_CATEGORY_AUDIO_DECODER, MFT_ENUM_FLAG,
    MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_FLUSH,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
    MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_AAC_PAYLOAD_TYPE, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_MT_USER_DATA, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};

use super::config::Core;
// Found at run time, so the game still starts without Media Foundation.
use super::mfplat::{
    MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFShutdown, MFStartup, MFTEnumEx,
};

/// Bytes of an output sample the decoder does not allocate itself: one
/// HE-AAC frame of 2048 frames in up to 6 channels of f32, with room over.
const OUT_BYTES: u32 = 2048 * 6 * 4 * 2;

pub(crate) struct Decoder {
    mft: IMFTransform,
    rate: u32,
    channels: usize,
    provides_samples: bool,
    out_bytes: u32,
    // Last, so Media Foundation shuts down after the transform is released.
    _startup: Startup,
}

// SAFETY: the transform is created in the multithreaded apartment, where
// COM objects may be called from any thread, and the decoder only ever
// calls it through `&mut self`, so never from two threads at once. `&self`
// methods touch no COM object at all.
unsafe impl Send for Decoder {}
unsafe impl Sync for Decoder {}

/// One MFStartup, undone when dropped.
struct Startup;

impl Startup {
    fn new() -> Result<Self, String> {
        // SAFETY: plain calls with no pointers. The thread joins the
        // multithreaded apartment; one already in an apartment stays as it
        // is (RPC_E_CHANGED_MODE), which Media Foundation tolerates.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_LITE).map_err(|e| format!("MFStartup: {e}"))?;
        }
        Ok(Self)
    }
}

impl Drop for Startup {
    fn drop(&mut self) {
        // SAFETY: balances the MFStartup in `new`.
        unsafe {
            let _ = MFShutdown();
        }
    }
}

impl Decoder {
    pub(crate) fn open(core: Core) -> Result<Self, String> {
        let startup = Startup::new()?;
        let mft = find_decoder()?;
        // SAFETY: every call is on interfaces this function owns, with
        // arguments that live across the call.
        unsafe {
            let input = MFCreateMediaType().map_err(text)?;
            input
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .map_err(text)?;
            input
                .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)
                .map_err(text)?;
            // The core's rate and channels: the decoder applies SBR and PS
            // itself when it finds them in the audio.
            input
                .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, core.rate())
                .map_err(text)?;
            input
                .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, core.channel_count())
                .map_err(text)?;
            input.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0).map_err(text)?;
            // HEAACWAVEINFO after its WAVEFORMATEX: payload type 0 (raw),
            // profile and level 0xFE (unknown), then the AudioSpecificConfig.
            let mut user = vec![0, 0, 0xfe, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            user.extend_from_slice(&core.implicit_asc());
            input.SetBlob(&MF_MT_USER_DATA, &user).map_err(text)?;
            mft.SetInputType(0, &input, 0)
                .map_err(|e| format!("input type: {e}"))?;
        }
        let mut decoder = Self {
            mft,
            rate: 0,
            channels: 0,
            provides_samples: false,
            out_bytes: OUT_BYTES,
            _startup: startup,
        };
        decoder.choose_output()?;
        // SAFETY: notifications on the transform this decoder owns.
        unsafe {
            let _ = decoder
                .mft
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);
            let _ = decoder
                .mft
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);
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
        let sample = input_sample(frame)?;
        // SAFETY: the sample lives across the call.
        unsafe { self.mft.ProcessInput(0, &sample, 0) }.map_err(|e| format!("input: {e}"))?;
        loop {
            let given = if self.provides_samples {
                None
            } else {
                Some(output_sample(self.out_bytes)?)
            };
            let mut buffer = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(given),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0;
            // SAFETY: one buffer, valid for the call; what it holds after is
            // taken back into ownership below and released once.
            let result = unsafe {
                self.mft
                    .ProcessOutput(0, std::slice::from_mut(&mut buffer), &mut status)
            };
            let sample = ManuallyDrop::into_inner(buffer.pSample);
            drop(ManuallyDrop::into_inner(buffer.pEvents));
            match result {
                Ok(()) => {
                    if let Some(sample) = sample {
                        append(&sample, out)?;
                    }
                }
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                // SBR or PS found in the audio: a new rate or channel count.
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => self.choose_output()?,
                Err(e) => return Err(format!("output: {e}")),
            }
        }
        Ok((self.rate, self.channels))
    }

    pub(crate) fn reset(&mut self) {
        // SAFETY: a flush on the transform this decoder owns.
        unsafe {
            let _ = self.mft.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
    }

    /// Sets float output, stereo or fewer channels where offered, and
    /// records its rate and channels.
    fn choose_output(&mut self) -> Result<(), String> {
        let mut chosen: Option<(IMFMediaType, u32)> = None;
        for index in 0.. {
            // SAFETY: an enumeration call; it fails past the last type.
            let Ok(kind) = (unsafe { self.mft.GetOutputAvailableType(0, index) }) else {
                break;
            };
            // SAFETY: reads on a media type this loop owns.
            let (subtype, channels) = unsafe {
                (
                    kind.GetGUID(&MF_MT_SUBTYPE).unwrap_or(GUID::zeroed()),
                    kind.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0),
                )
            };
            if subtype != MFAudioFormat_Float || channels == 0 {
                continue;
            }
            let better = match &chosen {
                None => true,
                // A stereo fold-down beats every channel of a 5.1 stream.
                Some((_, best)) => *best > 2 && channels <= 2,
            };
            if better {
                chosen = Some((kind, channels));
            }
        }
        let (kind, channels) = chosen.ok_or("no float output")?;
        // SAFETY: calls on interfaces this decoder owns.
        unsafe {
            self.mft
                .SetOutputType(0, &kind, 0)
                .map_err(|e| format!("output type: {e}"))?;
            self.rate = kind
                .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
                .map_err(text)?;
            let info = self.mft.GetOutputStreamInfo(0).map_err(text)?;
            self.provides_samples = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
            self.out_bytes = info.cbSize.max(OUT_BYTES);
        }
        self.channels = channels as usize;
        Ok(())
    }
}

/// The first AAC decoder Media Foundation lists, best first.
fn find_decoder() -> Result<IMFTransform, String> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Audio,
        guidSubtype: MFAudioFormat_AAC,
    };
    let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0);
    let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: MFTEnumEx fills `list` with `count` activation objects in one
    // CoTaskMemAlloc block. Each is read out once, which takes over its
    // reference so it is released when dropped, and the block is freed.
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_AUDIO_DECODER,
            flags,
            Some(&input),
            None,
            &mut list,
            &mut count,
        )
        .map_err(|e| format!("MFTEnumEx: {e}"))?;
        let activates: Vec<Option<IMFActivate>> = (0..count as usize)
            .map(|i| std::ptr::read(list.add(i)))
            .collect();
        CoTaskMemFree(Some(list as *const _));
        activates
            .into_iter()
            .flatten()
            .find_map(|activate| activate.ActivateObject::<IMFTransform>().ok())
            .ok_or_else(|| "no AAC decoder installed".to_string())
    }
}

fn input_sample(frame: &[u8]) -> Result<IMFSample, String> {
    let len = u32::try_from(frame.len()).map_err(text)?;
    // SAFETY: the buffer is locked for at least `len` bytes while the frame
    // is copied in, and unlocked before anything else uses it.
    unsafe {
        let buffer = MFCreateMemoryBuffer(len.max(1)).map_err(text)?;
        let mut data = std::ptr::null_mut();
        buffer.Lock(&mut data, None, None).map_err(text)?;
        std::ptr::copy_nonoverlapping(frame.as_ptr(), data, frame.len());
        buffer.Unlock().map_err(text)?;
        buffer.SetCurrentLength(len).map_err(text)?;
        let sample = MFCreateSample().map_err(text)?;
        sample.AddBuffer(&buffer).map_err(text)?;
        Ok(sample)
    }
}

fn output_sample(bytes: u32) -> Result<IMFSample, String> {
    // SAFETY: creation calls whose results this function owns.
    unsafe {
        let buffer = MFCreateMemoryBuffer(bytes).map_err(text)?;
        let sample = MFCreateSample().map_err(text)?;
        sample.AddBuffer(&buffer).map_err(text)?;
        Ok(sample)
    }
}

/// Appends a decoded sample's floats to `out`.
fn append(sample: &IMFSample, out: &mut Vec<f32>) -> Result<(), String> {
    // SAFETY: the buffer is locked while its `len` valid bytes are read,
    // byte by byte (no alignment assumed), then unlocked.
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().map_err(text)?;
        let mut data = std::ptr::null_mut();
        let mut len = 0u32;
        buffer.Lock(&mut data, None, Some(&mut len)).map_err(text)?;
        let bytes = std::slice::from_raw_parts(data, len as usize);
        out.extend(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
        buffer.Unlock().map_err(text)?;
    }
    Ok(())
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}
