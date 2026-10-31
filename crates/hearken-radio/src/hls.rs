//! Live HLS stations: a media playlist re-read as it grows, and its
//! segments fed one after another to a single decoder.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use m3u8_rs::{
    AlternativeMediaType, ByteRange, KeyMethod, MasterPlaylist, MediaPlaylist, MediaSegment,
    Playlist,
};
use mpeg2ts::ts::{Pid, ReadTsPacket, TsPacketReader, TsPayload};
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::ReadOnlySource;
use url::Url;

use crate::decode::{self, closed, no_audio};
use crate::http::{self, bad_url};
use crate::stream::{Fault, RawTitle, Shared, StreamOptions};
use crate::title;
use crate::worker::read_up_to;

/// Segments back from the live edge to start: the cushion that keeps a
/// late segment from being heard.
const BEHIND_LIVE: usize = 3;

/// The most of one segment worth reading. An audio segment is a few
/// hundred KiB; past this it is video, or not a segment.
const MAX_SEGMENT: u64 = 16 << 20;

/// The shortest wait between re-reads of a playlist.
const MIN_REFRESH: Duration = Duration::from_millis(500);

/// The most of an HLS playlist worth reading. A live window hours long is
/// several MiB: YouTube's four-hour window lists about 2,900 segments in
/// 3.7 MB. Cut short, its list ends hours behind live, mid-line.
pub(crate) const MAX_PLAYLIST: u64 = 16 << 20;

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

/// A live station's segments as one stream of audio bytes. The ring paces
/// it: once that is full the decoder waits, and so does the next fetch.
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
    /// The MPEG-TS packet id of the audio played, once one is seen: a
    /// second audio stream (another language) must not reach the decoder.
    /// Forgotten at a discontinuity, where a station may renumber its
    /// tracks (an inserted ad).
    audio: Option<Pid>,
    /// Why the reader failed, for [`play`] to report.
    fault: Arc<Mutex<Option<Fault>>>,
    /// The fMP4 init segment (`#EXT-X-MAP`) the segments since have named,
    /// and whether the decoder has been sent it. The decoder takes one init
    /// segment, ahead of the first fragment.
    map: Option<(Url, Option<ByteRange>)>,
    map_sent: bool,
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
            let variant = audio_uri(master).ok_or_else(no_audio)?;
            let variant = url.join(variant).map_err(|_| bad_url())?;
            let (at, text) = fetch_playlist(&variant, shared, opts)?;
            (url, playlist) = (at, parse(&text)?);
        }
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
            audio: None,
            fault: Arc::default(),
            map: None,
            map_sent: false,
        };
        reader.enqueue(&media(playlist)?);
        Ok(reader)
    }

    /// Queues the segments of `playlist` newer than any queued before.
    fn enqueue(&mut self, playlist: &MediaPlaylist) {
        let half_target = Duration::from_secs(playlist.target_duration) / 2;
        self.refresh = half_target.max(MIN_REFRESH);
        self.ended = playlist.end_list;
        // `#EXT-X-MAP` applies to every segment after it, but the parser
        // keeps it on the one it precedes, which the live edge may skip.
        let mut maps = playlist.segments.iter().filter_map(|s| s.map.as_ref());
        if let Some(map) = maps.next_back() {
            if let Ok(at) = self.url.join(&map.uri) {
                self.map = Some((at, map.byte_range.clone()));
            }
        }
        for (sequence, segment) in newer(playlist, self.last) {
            self.last = Some(sequence);
            self.queue.push_back(segment.clone());
        }
    }

    /// The current segment's bytes, then the next segment's; 0 once the
    /// playlist has ended.
    fn serve(&mut self, buf: &mut [u8]) -> Result<usize, Fault> {
        // The stall clock runs from when the decoder asks for more than is
        // here, so time it spent waiting for room in the ring (a paused
        // game) is not a stall.
        let asked = Instant::now();
        // A segment may hold no audio at all.
        while self.served == self.segment.len() {
            if !self.next_segment(asked)? {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.segment.len() - self.served);
        buf[..n].copy_from_slice(&self.segment[self.served..][..n]);
        self.served += n;
        Ok(n)
    }

    /// Fetches the next segment, re-reading the playlist until it lists
    /// one, or until `stall` after the decoder `asked`. False once the
    /// playlist has ended.
    fn next_segment(&mut self, asked: Instant) -> Result<bool, Fault> {
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
            let fetched = fetch_playlist(&self.url, &self.shared, &self.opts);
            let (url, text) = fetched.map_err(retry)?;
            self.url = url;
            self.enqueue(&media(parse(&text).map_err(retry)?)?);
            if self.queue.is_empty() {
                if asked.elapsed() >= self.opts.stall {
                    return Err(Fault::Retry("stalled".into()));
                }
                // A long target duration must not outwait the stall clock.
                if !self.shared.sleep(self.refresh.min(self.opts.stall)) {
                    return Err(closed());
                }
            }
        };
        let url = self.url.join(&segment.uri).map_err(|_| bad_url())?;
        // ponytail: a segment is fetched once the last is spent, so one slower
        // to arrive than the buffer lasts is heard; prefetch if that happens.
        let fetched = fetch(&url, MAX_SEGMENT, &self.shared, &self.opts);
        let (_, bytes) = fetched.map_err(retry)?;
        let bytes = match &segment.byte_range {
            Some(range) => slice(bytes, range),
            None => bytes,
        };
        // Every MPEG-TS packet starts with 0x47.
        let ts = bytes.first() == Some(&0x47) || url.path().ends_with(".ts");
        // ponytail: a codec change at #EXT-X-DISCONTINUITY is not re-probed;
        // restart the pump there if a station is heard doing it.
        if segment.discontinuity {
            self.audio = None;
        }
        self.segment = if ts {
            demux(&bytes, &mut self.audio)
        } else {
            bytes
        };
        // An fMP4 (CMAF) station's fragments mean nothing without its init
        // segment's track description, so the decoder gets that first; it
        // then reads the fragments as one fragmented MP4 file.
        // ponytail: a station that changes its init segment (a codec change
        // at a discontinuity) keeps the first; restart the pump if heard.
        if !ts && !self.map_sent {
            if let Some((at, range)) = self.map.clone() {
                let (_, init) = fetch(&at, MAX_SEGMENT, &self.shared, &self.opts).map_err(retry)?;
                let mut init = match &range {
                    Some(range) => slice(init, range),
                    None => init,
                };
                init.append(&mut self.segment);
                self.segment = init;
                self.map_sent = true;
            }
        }
        self.served = 0;
        let extinf = format!(
            "{},{}",
            segment.duration,
            segment.title.as_deref().unwrap_or("")
        );
        self.shared.set_raw_title(RawTitle::Extinf(extinf));
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
/// their numbers. With no `last`, or one the window has moved past (a long
/// pause, a slow fetch), the final [`BEHIND_LIVE`]: the segments between
/// are gone. So too with a `last` more than a window past the window's end:
/// the packager restarted its numbering, and nothing it lists would ever be
/// newer. A `last` within a window of the end is a lagging edge's older copy,
/// which has nothing new yet.
fn newer(
    playlist: &MediaPlaylist,
    last: Option<u64>,
) -> impl Iterator<Item = (u64, &MediaSegment)> {
    let sequence = playlist.media_sequence;
    let window = playlist.segments.len() as u64;
    let end = sequence.saturating_add(window);
    let last = last
        .filter(|last| last.saturating_add(1) >= sequence && *last < end.saturating_add(window));
    let live = playlist.segments.len().saturating_sub(BEHIND_LIVE) as u64;
    let first = last.map_or(sequence.saturating_add(live), |last| last.saturating_add(1));
    let numbered = playlist.segments.iter().enumerate();
    numbered
        .map(move |(i, segment)| (sequence.saturating_add(i as u64), segment))
        .filter(move |(number, _)| *number >= first)
}

/// The playlist to play from a master playlist: the highest-bandwidth
/// variant whose codecs name no video, or, when its audio is a separate
/// rendition (`#EXT-X-MEDIA TYPE=AUDIO` with a URI), that rendition. With
/// only video variants, the audio rendition of the best one that has one.
fn audio_uri(master: &MasterPlaylist) -> Option<&str> {
    let (mut audio, mut video): (Vec<_>, Vec<_>) = master.variants.iter().partition(|variant| {
        let codecs = variant.codecs.as_deref().unwrap_or_default();
        !VIDEO_CODECS.iter().any(|video| codecs.contains(video))
    });
    audio.sort_by_key(|variant| std::cmp::Reverse(variant.bandwidth));
    video.sort_by_key(|variant| std::cmp::Reverse(variant.bandwidth));
    if let Some(best) = audio.first() {
        return Some(rendition(master, best.audio.as_deref()).unwrap_or(&best.uri));
    }
    if let Some(found) = video
        .iter()
        .find_map(|v| rendition(master, v.audio.as_deref()))
    {
        return Some(found);
    }
    // No variants to name a group: any audio rendition with its own URI.
    rendition_in(master, None)
}

/// The URI of the audio rendition in `group`: its default one, else its
/// first with a URI. None when the group's audio is inside the variant.
fn rendition<'a>(master: &'a MasterPlaylist, group: Option<&str>) -> Option<&'a str> {
    rendition_in(master, Some(group?))
}

fn rendition_in<'a>(master: &'a MasterPlaylist, group: Option<&str>) -> Option<&'a str> {
    let mut renditions = master.alternatives.iter().filter(|media| {
        media.media_type == AlternativeMediaType::Audio
            && media.uri.is_some()
            && group.is_none_or(|group| media.group_id == group)
    });
    let first = renditions.clone().next();
    renditions
        .find(|media| media.default)
        .or(first)
        .and_then(|media| media.uri.as_deref())
}

/// The part of `bytes` an `#EXT-X-BYTERANGE` names; a range that runs past
/// the end keeps what there is. A range with no offset starts at the top.
/// ponytail: an offset left out means "after the last range" in the spec;
/// track it if a station is heard relying on that.
fn slice(mut bytes: Vec<u8>, range: &ByteRange) -> Vec<u8> {
    let start = range.offset.unwrap_or(0).min(bytes.len() as u64) as usize;
    let end = start.saturating_add(range.length as usize).min(bytes.len());
    bytes.truncate(end);
    bytes.drain(..start);
    bytes
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
fn fetch(
    url: &Url,
    limit: u64,
    shared: &Arc<Shared>,
    opts: &StreamOptions,
) -> Result<(Url, Vec<u8>), Fault> {
    let mut response = http::open(url.as_str(), shared, opts)?;
    let bytes = read_up_to(&mut response.body, limit)?;
    Ok((response.url, bytes))
}

/// A playlist, asked for gzipped: it is re-read every segment or so, and a
/// long live window's text shrinks many times over. The limit counts the
/// text, not what came over the wire.
fn fetch_playlist(
    url: &Url,
    shared: &Arc<Shared>,
    opts: &StreamOptions,
) -> Result<(Url, Vec<u8>), Fault> {
    let mut response = http::open_gzip(url.as_str(), shared, opts)?;
    let bytes = read_up_to(&mut response.body, MAX_PLAYLIST)?;
    Ok((response.url, bytes))
}

/// The audio elementary stream in an MPEG-TS segment: the one on packet id
/// `audio`, or on the first that starts an audio packet, which `audio` then
/// keeps. Audio streams may share a PES stream id, so only the packet id
/// tells them apart. A packet that does not parse ends the segment there.
fn demux(ts: &[u8], audio: &mut Option<Pid>) -> Vec<u8> {
    let mut packets = TsPacketReader::new(ts);
    let mut bytes = Vec::new();
    while let Ok(Some(packet)) = packets.read_ts_packet() {
        let pid = packet.header.pid;
        match packet.payload {
            Some(TsPayload::PesStart(pes))
                if pes.header.stream_id.is_audio() && *audio.get_or_insert(pid) == pid =>
            {
                bytes.extend_from_slice(&pes.data);
            }
            Some(TsPayload::PesContinuation(data)) if *audio == Some(pid) => {
                bytes.extend_from_slice(&data);
            }
            _ => {}
        }
    }
    bytes
}

/// Once its playlist has been read, a station is worth calling again
/// whatever goes wrong fetching from it, played yet or not: a CDN missing a
/// segment for a moment is not the station gone.
fn retry(fault: Fault) -> Fault {
    let (Fault::Fatal(reason) | Fault::Unreachable(reason) | Fault::Retry(reason)) = fault;
    Fault::Retry(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of a 440 Hz tone at half scale, as AAC in MPEG-TS:
    /// `ffmpeg -f lavfi -i sine=frequency=440:duration=1 -af volume=12dB -ac 1
    /// -c:a aac -b:a 64k -f mpegts tone.ts`
    const TONE: &[u8] = include_bytes!("../tests/fixtures/tone.ts");

    /// The same tone beside a second audio stream at 880 Hz, as another
    /// language would be:
    /// `ffmpeg -f lavfi -i sine=frequency=440:duration=1 -f lavfi -i
    /// sine=frequency=880:duration=1 -map 0 -map 1 -af volume=12dB -ac 1
    /// -c:a aac -b:a 64k -f mpegts two-tracks.ts`
    const TWO_TRACKS: &[u8] = include_bytes!("../tests/fixtures/two-tracks.ts");

    /// Six segments, numbered from `from`.
    fn six(from: u64) -> MediaPlaylist {
        let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:{from}\n");
        for n in 0..6 {
            text += &format!("#EXTINF:1.0,\nseg{n}.aac\n");
        }
        m3u8_rs::parse_media_playlist_res(text.as_bytes()).unwrap()
    }

    #[test]
    fn the_live_edge_starts_three_segments_back() {
        let playlist = six(10);
        let (sequence, segment) = newer(&playlist, None).next().unwrap();
        assert_eq!((sequence, segment.uri.as_str()), (13, "seg3.aac"));
    }

    #[test]
    fn a_reread_playlist_yields_only_newer_segments() {
        let playlist = six(10);
        let uris: Vec<_> = newer(&playlist, Some(14))
            .map(|(_, segment)| segment.uri.as_str())
            .collect();
        assert_eq!(uris, ["seg5.aac"]);
    }

    #[test]
    fn a_window_that_jumped_ahead_re_anchors_at_the_live_edge() {
        let playlist = six(20);
        let (sequence, _) = newer(&playlist, Some(10)).next().unwrap();
        assert_eq!(sequence, 23);
    }

    #[test]
    fn a_window_numbered_back_from_the_start_re_anchors_at_the_live_edge() {
        // The packager restarted: its numbers begin again below the last.
        let playlist = six(0);
        let (sequence, _) = newer(&playlist, Some(20)).next().unwrap();
        assert_eq!(sequence, 3);
    }

    #[test]
    fn a_playlist_one_version_older_than_the_last_served_yields_nothing() {
        // A lagging CDN edge: its newest segment is the one before `last`.
        let playlist = six(10);
        assert_eq!(newer(&playlist, Some(16)).count(), 0);
    }

    #[test]
    fn a_ts_segment_demuxes_to_its_audio() {
        let audio = demux(TONE, &mut None);
        // An ADTS sync word.
        assert_eq!(audio[0], 0xFF);
        assert_eq!(audio[1] & 0xF0, 0xF0);
    }

    #[test]
    fn a_ts_segment_with_two_audio_streams_demuxes_to_the_first() {
        let first = demux(TWO_TRACKS, &mut None);
        assert!(first == demux(TONE, &mut None), "the streams interleave");
    }
}
