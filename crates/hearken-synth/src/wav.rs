//! Minimal 16-bit PCM mono WAV encoding.

/// Encode mono `f32` samples (any range; clamped to `[-1.0, 1.0]`) as a WAV
/// file: 16-bit PCM, one channel, at `sample_rate`.
pub fn wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = sample_rate * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format 1 = PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // 1 channel
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());

    for s in samples {
        let clamped = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&clamped.to_le_bytes());
    }

    out
}
