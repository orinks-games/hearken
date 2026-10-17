//! The metadata a station interleaves with its audio, taken back out.

use std::io::{self, Read};
use std::num::NonZeroUsize;

/// Reads a station's audio with its metadata blocks taken out, handing each
/// non-empty block to `on_block`, its zero padding removed. The station sends a length byte (in
/// 16-byte units) and that many bytes of metadata after every `interval`
/// bytes of audio; none at all when the interval is `None`.
///
/// Not `icy_metadata::IcyMetadataReader`: that assumes every read it makes
/// fills its buffer, and a socket's rarely does. On a live station it took
/// audio for a block length, then panicked.
pub(crate) struct IcyReader<R, F> {
    inner: R,
    interval: Option<usize>,
    /// Audio bytes before the next block.
    left: usize,
    on_block: F,
}

impl<R, F> IcyReader<R, F>
where
    R: Read,
    F: Fn(&[u8]),
{
    pub(crate) fn new(inner: R, interval: Option<NonZeroUsize>, on_block: F) -> Self {
        let interval = interval.map(NonZeroUsize::get);
        IcyReader {
            inner,
            interval,
            left: interval.unwrap_or(0),
            on_block,
        }
    }

    fn block(&mut self) -> io::Result<()> {
        let mut units = [0];
        self.inner.read_exact(&mut units)?;
        // Most blocks are empty: the title has not changed.
        if units[0] == 0 {
            return Ok(());
        }
        let mut block = vec![0; usize::from(units[0]) * 16];
        self.inner.read_exact(&mut block)?;
        // A block is padded with zeros to a multiple of 16 bytes.
        let end = block
            .iter()
            .rposition(|&byte| byte != 0)
            .map_or(0, |at| at + 1);
        (self.on_block)(&block[..end]);
        Ok(())
    }
}

impl<R, F> Read for IcyReader<R, F>
where
    R: Read,
    F: Fn(&[u8]),
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(interval) = self.interval else {
            return self.inner.read(buf);
        };
        if self.left == 0 {
            self.block()?;
            self.left = interval;
        }
        // Whatever the socket gives, up to the next block.
        let end = buf.len().min(self.left);
        let read = self.inner.read(&mut buf[..end])?;
        self.left -= read;
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A socket that hands over at most `at_most` bytes per read.
    struct Dribble<'a>(&'a [u8], usize);

    impl Read for Dribble<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(self.1).min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    /// The wire for a 12-byte interval, and the 28 audio bytes in it: 12, an
    /// empty block, 12, a titled block, and 4.
    fn station() -> (Vec<u8>, Vec<u8>) {
        let audio: Vec<u8> = (1..=28).collect();
        let mut wire = audio[..12].to_vec();
        wire.push(0);
        wire.extend(&audio[12..24]);
        wire.push(1);
        wire.extend(b"StreamTitle='A';");
        wire.extend(&audio[24..]);
        (wire, audio)
    }

    #[test]
    fn audio_comes_out_whole_however_the_socket_chops_it() {
        let (wire, audio) = station();
        for at_most in [1, 5, 12, 13, 100] {
            let titles = Arc::new(Mutex::new(Vec::new()));
            let seen = titles.clone();
            let mut reader = IcyReader::new(
                Dribble(&wire, at_most),
                NonZeroUsize::new(12),
                move |block| seen.lock().unwrap().push(block.to_vec()),
            );
            let mut heard = Vec::new();
            reader.read_to_end(&mut heard).unwrap();
            assert_eq!(heard, audio, "{at_most} bytes at a time");
            assert_eq!(*titles.lock().unwrap(), [b"StreamTitle='A';".to_vec()]);
        }
    }

    #[test]
    fn without_an_interval_the_body_passes_through() {
        let mut heard = Vec::new();
        IcyReader::new(Dribble(b"plain audio", 3), None, |_| panic!("no blocks"))
            .read_to_end(&mut heard)
            .unwrap();
        assert_eq!(heard, b"plain audio");
    }
}
