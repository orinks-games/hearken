//! Live HLS stations: a media playlist re-read as it grows, and its
//! segments fed one after another to a single decoder.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use m3u8_rs::{KeyMethod, MasterPlaylist, MediaPlaylist, MediaSegment, Playlist, VariantStream};
use mpeg2ts::pes::{PesPacketReader, ReadPesPacket};
use mpeg2ts::ts::TsPacketReader;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::ReadOnlySource;
use url::Url;

use crate::decode::{self, no_audio};
use crate::http::{self, bad_url};
use crate::stream::{Fault, Shared, StreamOptions};
use crate::title;
use crate::worker::{read_up_to, MAX_PLAYLIST};

/// Segments back from the live edge to start: the cushion that keeps a
/// late segment from being heard.
const BEHIND_LIVE: usize = 3;

/// The most of one segment worth reading. An audio segment is a few
/// hundred KiB; past this it is video, or not a segment.
const MAX_SEGMENT: u64 = 16 << 20;

/// The shortest wait between re-reads of a playlist.
const MIN_REFRESH: Duration = Duration::from_millis(500);

/// How often a reader that is ahead of time looks at the clock again.
const TICK: Duration = Duration::from_millis(50);

/// Codecs that make a variant video.
const VIDEO_CODECS: [&str; 5] = ["avc1", "hvc1", "hev1", "vp09", "av01"];

/// Plays the HLS station at `url`, whose playlist `playlist` is already
/// fetched, until it fails or closes.
pub(crate) fn play(url: &str, playlist: &str, shared: &Arc<Shared>, opts: &StreamOptions) -> Fault {
    let reader = match HlsReader::open(url, playlist, shared, opts) {
        Ok(reader) => reader,
        Err(fault) => return fault,
    };
    let kept = reader.fault.clone();
    let source = Box::new(ReadOnlySource::new(reader));
    let pumped = decode::pump(source, Hint::new(), shared, opts);
    // Symphonia passes on only an io::Error; the reader kept the real reason.
    let kept = kept.lock().unwrap_or_else(PoisonError::into_inner).take();
    kept.unwrap_or(pumped)
}

/// A live station's segments as one stream of audio bytes, paced to real
/// time as a live HTTP station is: all at once, the backlog would overflow
/// the ring.
struct HlsReader {
    /// The media playlist, as last answered: segment URIs are relative to
    /// it.
    url: Url,
    shared: Arc<Shared>,
    opts: StreamOptions,
    /// Segments listed but not yet fetched, oldest first.
    queue: VecDeque<MediaSegment>,
    /// The media sequence number of the newest segment queued.
    last: Option<u64>,
    /// How long to wait between re-reads of the playlist.
    refresh: Duration,
    /// The playlist says no segment will follow those it lists.
    ended: bool,
    /// The current segment's audio, and how much of it has been served.
    segment: Vec<u8>,
    served: usize,
    /// When the current segment is due to start playing, and its length.
    start: Instant,
    duration: Duration,
    /// When audio was last served: `stall` without it, the station is lost.
    fed: Instant,
    /// Why the reader failed, for [`play`] to report.
    fault: Arc<Mutex<Option<Fault>>>,
}

impl HlsReader {
    /// A reader [`BEHIND_LIVE`] segments from the live edge of `text`, or of
    /// its best audio variant when it is a master playlist.
    fn open(
        url: &str,
        text: &str,
        shared: &Arc<Shared>,
        opts: &StreamOptions,
    ) -> Result<HlsReader, Fault> {
        let mut url = Url::parse(url).map_err(|_| bad_url())?;
        let mut playlist = parse(text.as_bytes())?;
        if let Playlist::MasterPlaylist(master) = &playlist {
            let variant = audio_variant(master).ok_or_else(no_audio)?;
            let variant = url.join(&variant.uri).map_err(|_| bad_url())?;
            let (at, text) = fetch(&variant, MAX_PLAYLIST, opts)?;
            (url, playlist) = (at, parse(&text)?);
        }
        let now = Instant::now();
        let mut reader = HlsReader {
            url,
            shared: shared.clone(),
            opts: opts.clone(),
            queue: VecDeque::new(),
            last: None,
            refresh: MIN_REFRESH,
            ended: false,
            segment: Vec::new(),
            served: 0,
            start: now,
            duration: Duration::ZERO,
            fed: now,
            fault: Arc::default(),
        };
        reader.enqueue(&media(playlist)?);
        Ok(reader)
    }

    /// Queues the segments of `playlist` newer than any queued before.
    fn enqueue(&mut self, playlist: &MediaPlaylist) {
        let half_target = Duration::from_secs(playlist.target_duration) / 2;
        self.refresh = half_target.max(MIN_REFRESH);
        self.ended = playlist.end_list;
        for (sequence, segment) in newer(playlist, self.last) {
            self.last = Some(sequence);
            self.queue.push_back(segment.clone());
        }
    }

    /// The current segment's bytes as they come due, then the next
    /// segment's; 0 once the playlist has ended.
    fn serve(&mut self, buf: &mut [u8]) -> Result<usize, Fault> {
        loop {
            if self.served == self.segment.len() && !self.next_segment()? {
                return Ok(0);
            }
            let due = self.due();
            if due > self.served {
                let n = buf.len().min(due - self.served);
                buf[..n].copy_from_slice(&self.segment[self.served..][..n]);
                self.served += n;
                self.fed = Instant::now();
                return Ok(n);
            }
            if !self.shared.sleep(TICK) {
                return Err(closed());
            }
        }
    }

    /// How many of the current segment's bytes are due: those up to half
    /// the ring ahead of real time, so the decoder never pushes more than
    /// the ring holds.
    fn due(&self) -> usize {
        let ahead = (Instant::now() + self.opts.buffer / 2).saturating_duration_since(self.start);
        if ahead >= self.duration {
            return self.segment.len();
        }
        let due = self.segment.len() as u128 * ahead.as_nanos() / self.duration.as_nanos();
        due as usize
    }

    /// Fetches the next segment, re-reading the playlist until it lists
    /// one. False once the playlist has ended.
    fn next_segment(&mut self) -> Result<bool, Fault> {
        let segment = loop {
            if self.shared.closed() {
                return Err(closed());
            }
            if let Some(segment) = self.queue.pop_front() {
                break segment;
            }
            if self.ended {
                return Ok(false);
            }
            let (url, text) = fetch(&self.url, MAX_PLAYLIST, &self.opts).map_err(retry)?;
            self.url = url;
            self.enqueue(&media(parse(&text).map_err(retry)?)?);
            if self.queue.is_empty() {
                if self.fed.elapsed() >= self.opts.stall {
                    return Err(Fault::Retry("stalled".into()));
                }
                if !self.shared.sleep(self.refresh) {
                    return Err(closed());
                }
            }
        };
        let url = self.url.join(&segment.uri).map_err(|_| retry(bad_url()))?;
        // ponytail: a segment is fetched once the last is spent, so one slower
        // than half the buffer to arrive is heard; prefetch if that happens.
        let (_, bytes) = fetch(&url, MAX_SEGMENT, &self.opts).map_err(retry)?;
        // Every MPEG-TS packet starts with 0x47.
        let ts = bytes.first() == Some(&0x47) || url.path().ends_with(".ts");
        // ponytail: a codec change at #EXT-X-DISCONTINUITY is not re-probed;
        // restart the pump there if a station is heard doing it.
        self.segment = if ts { demux(&bytes) } else { bytes };
        self.served = 0;
        // On time, this segment follows the last; late, it starts now.
        self.start = (self.start + self.duration).max(Instant::now());
        self.duration = Duration::try_from_secs_f32(segment.duration).unwrap_or_default();
        if let Some(title) = segment.title.as_deref().and_then(title::hls) {
            self.shared.set_title(Some(title));
        }
        Ok(true)
    }
}

impl Read for HlsReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.serve(buf).map_err(|fault| {
            *self.fault.lock().unwrap_or_else(PoisonError::into_inner) = Some(fault);
            io::ErrorKind::Other.into()
        })
    }
}

/// The segments of `playlist` after media sequence number `last`, with
/// their numbers; with no `last`, the final [`BEHIND_LIVE`].
fn newer(
    playlist: &MediaPlaylist,
    last: Option<u64>,
) -> impl Iterator<Item = (u64, &MediaSegment)> {
    let sequence = playlist.media_sequence;
    let live = playlist.segments.len().saturating_sub(BEHIND_LIVE) as u64;
    let first = last.map_or(sequence.saturating_add(live), |last| last.saturating_add(1));
    let numbered = playlist.segments.iter().enumerate();
    numbered
        .map(move |(i, segment)| (sequence.saturating_add(i as u64), segment))
        .filter(move |(number, _)| *number >= first)
}

/// The highest-bandwidth variant whose codecs name no video.
fn audio_variant(master: &MasterPlaylist) -> Option<&VariantStream> {
    // ponytail: an audio rendition beside video variants (#EXT-X-MEDIA
    // TYPE=AUDIO) is not followed; take one if a station is heard needing it.
    master
        .variants
        .iter()
        .filter(|variant| {
            let codecs = variant.codecs.as_deref().unwrap_or_default();
            !VIDEO_CODECS.iter().any(|video| codecs.contains(video))
        })
        .max_by_key(|variant| variant.bandwidth)
}

/// `playlist`, if it is a media playlist that can be played.
fn media(playlist: Playlist) -> Result<MediaPlaylist, Fault> {
    let Playlist::MediaPlaylist(playlist) = playlist else {
        return Err(no_audio());
    };
    let mut keys = playlist.segments.iter().filter_map(|s| s.key.as_ref());
    if keys.any(|key| key.method != KeyMethod::None) {
        return Err(Fault::Fatal("encrypted HLS".into()));
    }
    Ok(playlist)
}

fn parse(text: &[u8]) -> Result<Playlist, Fault> {
    m3u8_rs::parse_playlist_res(text).map_err(|_| Fault::Fatal("bad HLS playlist".into()))
}

/// What `url` answers, up to `limit` bytes, and where the answer came from.
fn fetch(url: &Url, limit: u64, opts: &StreamOptions) -> Result<(Url, Vec<u8>), Fault> {
    let mut response = http::open(url.as_str(), opts)?;
    let bytes = read_up_to(&mut response.body, limit)?;
    Ok((response.url, bytes))
}

/// The audio elementary stream in an MPEG-TS segment. A packet that does
/// not parse ends the segment there.
fn demux(ts: &[u8]) -> Vec<u8> {
    let mut packets = PesPacketReader::new(TsPacketReader::new(ts));
    let mut audio = Vec::new();
    while let Ok(Some(packet)) = packets.read_pes_packet() {
        if packet.header.stream_id.is_audio() {
            audio.extend(packet.data);
        }
    }
    audio
}

/// A station that has played is worth calling again whatever goes wrong
/// fetching from it: a CDN missing a segment for a moment is not the
/// station gone.
fn retry(fault: Fault) -> Fault {
    let (Fault::Fatal(reason) | Fault::Retry(reason)) = fault;
    Fault::Retry(reason)
}

fn closed() -> Fault {
    Fault::Fatal("closed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six segments, numbered from 10.
    fn six() -> MediaPlaylist {
        let mut text = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:10\n");
        for n in 0..6 {
            text += &format!("#EXTINF:1.0,\nseg{n}.aac\n");
        }
        m3u8_rs::parse_media_playlist_res(text.as_bytes()).unwrap()
    }

    #[test]
    fn the_live_edge_starts_three_segments_back() {
        let playlist = six();
        let (sequence, segment) = newer(&playlist, None).next().unwrap();
        assert_eq!((sequence, segment.uri.as_str()), (13, "seg3.aac"));
    }

    #[test]
    fn a_reread_playlist_yields_only_newer_segments() {
        let playlist = six();
        let uris: Vec<_> = newer(&playlist, Some(14))
            .map(|(_, segment)| segment.uri.as_str())
            .collect();
        assert_eq!(uris, ["seg5.aac"]);
    }

    #[test]
    fn a_ts_segment_demuxes_to_its_audio() {
        let audio = demux(include_bytes!("../tests/fixtures/tone.ts"));
        // An ADTS sync word.
        assert_eq!(audio[0], 0xFF);
        assert_eq!(audio[1] & 0xF0, 0xF0);
    }
}
