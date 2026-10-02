# Hearken core facade and internet radio

Date: 2026-10-01. Status: approved in conversation; this file is for review.

## Goal

Freight Fate moves onto Hearken (M6) with its radio intact and without BASS,
which Kira replaced so that no game ships a DLL or buys a licence. The radio
has about 6,000 stations: mostly Icecast/Shoutcast over HTTP (MP3 ~1,240,
AAC ~930, Ogg ~23), about 81 live HLS, and about 10 remote `.m3u`/`.pls`
links. It speaks the now-playing title on demand and reconnects a dead
station.

At the same time, games get one crate to depend on: `hearken`, the core
facade, with radio as its first optional feature.

**Success:**

- A game lists `hearken` alone (plus its `build.rs`) and builds as the
  template does today.
- With `features = ["radio"]`, Freight Fate's catalog plays through Hearken:
  HTTP stations in MP3, AAC and Ogg (Vorbis and Opus), remote playlists, and
  live HLS with ADTS or MPEG-TS segments, each with its title where the
  station sends one.
- Connecting, stalling and failing never block a frame.
- A network blip heals unheard; a dead station is reported within seconds.
- No new dependency is a DLL, and none needs a patent or commercial licence
  unless the game opts into `fdk-aac`.

## Decisions

| Decision | Notes |
|---|---|
| A `hearken` facade crate is the core a game depends on | Re-exports the must-haves; extras are its optional features, each its own crate |
| Radio is `hearken-radio`, behind the facade's `radio` feature | Not a feature of `hearken-audio`, which stays free of networking |
| `hearken-audio` gains a generic feed source | Audio a backend pulls; radio is its first user |
| Blocking worker thread per stream: `ureq` 2, `icy-metadata`, Symphonia | No async runtime; a stalled server blocks only its own thread. `ureq` 2, not 3: only 2 has a per-read timeout, which an endless body needs |
| HLS is built in the same worker on `m3u8-rs` and `mpeg2ts` | No Rust crate plays live HLS; `mpeg2ts` has no dependencies |
| The engine retries a dropped stream, then reports `Failed` | Any game gets working radio without its own reconnect loop |
| HE-AAC through `fdk-aac` is opt-in, off by default | Symphonia plays HE-AAC's AAC-LC core (duller) with no patent exposure |
| Tracker music is out of scope | Freight Fate supports it but ships no module files |

## Crate layout

```
crates/
  hearken          the facade: pub use hearken_app::*, speech, and optional extras
  hearken-app      unchanged, apart from the template moving to the facade
  hearken-audio    + the Feed source
  hearken-speech   unchanged
  hearken-synth    unchanged
  hearken-radio    new: Stream, the worker, HTTP/ICY, playlists, HLS
```

Dependency direction stays downward: `hearken` depends on everything it
re-exports; `hearken-radio` depends on `hearken-audio` (for `Feed`) and
nothing above it.

### `hearken`, the facade

- `pub use hearken_app::*;` brings everything a game touches today (`Ctx`,
  `Game`, `menu`, `screens`, `Bus`, `Sound`, `Vec3`, `synth`, `audio`, the
  speech types). Also `pub use hearken_speech as speech;`.
- `#[cfg(feature = "radio")] pub use hearken_radio as radio;`
- Features: `default = ["kira"]`; `kira = ["hearken-app/kira"]`;
  `radio = ["dep:hearken-radio"]`; `fdk-aac = ["radio",
  "hearken-radio/fdk-aac"]`. Later extras (`online`, `playtest`) are added
  the same way.
- `links = "hearken"`. Its `build.rs` reads `DEP_HEARKEN_SPEECH_LINK_ARGS`
  and re-emits it as its own `LINK_ARGS` metadata, so a game's `build.rs`
  reads `DEP_HEARKEN_LINK_ARGS` and no longer lists `hearken-speech`
  directly.
- The template depends on `hearken` alone; its imports become
  `hearken::...`; the README's build notes follow.

### The feed source in `hearken-audio`

```rust
/// Audio a backend pulls as it plays: a live stream, a call, a video's
/// soundtrack. Never blocks; what is not there yet is silence.
pub trait Feed: Send + Sync {
    /// Copies up to `out.len()` stereo frames available now into `out`,
    /// all at one sample rate, and returns how many and that rate. Stops
    /// early at a rate change; the next call starts at the new rate.
    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead;
}
pub struct FeedRead { pub frames: usize, pub sample_rate: u32 }
```

- `Source::Feed(Arc<dyn Feed>)`. The pool keeps the `Arc` on the live sound,
  so the deferred-backend replay hands the new backend a clone.
- `Audio::start_feed(feed: Arc<dyn Feed>, pos: Option<Vec3>, bus: Bus) ->
  Sound`: an ordinary sound, so bus volume, the duck, pause, fades, position
  and occlusion all apply. It never ends on its own, like a live voice.
- Kira plays every feed through one custom `Sound`: it pulls frames, pads
  what is missing with silence, and resamples from the feed's rate to the
  output's with the same linear step as `LiveVoice`.
- The recording and silent backends accept feeds like any source.

## `hearken-radio`

### API

```rust
let radio = Stream::open("https://ice.example/station.mp3"); // returns at once
let sound = audio.start_feed(radio.feed(), None, Bus::MUSIC);
radio.state();   // Connecting | Playing | Reconnecting | Failed(reason)
radio.title();   // Option<String>: the latest now-playing title
audio.stop(sound, 800);
// Stream::open_with(url, StreamOptions { .. }) changes the defaults below.
```

- `Stream` is a cheap `Clone` handle. `open` spawns the worker and returns;
  `state` and `title` read shared fields and never block, so a game may poll
  them every frame. `feed()` returns the `Arc<dyn Feed>` the pool plays.
- The worker stops when the game's last `Stream` and the pool's feed are
  both dropped.

### The worker

1. **Connect.** `ureq` GET with `Icy-MetaData: 1` and `User-Agent:
   Hearken/<version>`; nothing about the user is sent. Connect and every
   read time out at 10 s. HTTPS is rustls with the OS certificate store
   (`ureq`'s `native-certs`), so stations BASS's Windows TLS accepted are
   accepted. Redirects follow, up to 10. An old Shoutcast v1 server's `ICY
   200 OK` status line, which `ureq` rejects, gets one retry over a plain
   socket that accepts it.
2. **What came back**, by Content-Type, then the URL's extension, then the
   first bytes:
   - `.pls` or plain `.m3u`: take the first stream URL and connect to it, at
     most 3 hops.
   - HLS (`#EXTM3U` with `#EXT-X-` tags): from a master playlist, the
     highest-bandwidth audio variant. Start 3 segments from the live edge;
     re-read the media playlist about every half target duration; fetch each
     new segment in order and feed it to one decoder. ADTS segments go
     straight in; MPEG-TS segments are demuxed to their audio elementary
     stream with `mpeg2ts`. `#EXT-X-KEY` (encrypted) fails.
   - Anything else is audio: Symphonia on a non-seekable source, hinted by
     Content-Type (`audio/mpeg` MP3, `audio/aac` and `audio/aacp` ADTS,
     `ogg` Ogg).
3. **Decode** with radio's own codec registry: Symphonia's enabled codecs,
   AAC (its `aac` feature, which also registers the ADTS reader), libopus,
   and under `fdk-aac` the `symphonia-adapter-fdk-aac` 0.2 decoder,
   registered at the preferred tier so it takes AAC (a same-tier
   registration is replaced by Symphonia's, whichever goes first).
4. **Titles**, whichever arrived last:
   - ICY `StreamTitle`, from radio's own ICY reader around the response
     (`icy-metadata`'s reader panics on the short reads of a real socket),
     parsed with `icy-metadata`.
   - Ogg comments from Symphonia's metadata as each chained track starts,
     as `ARTIST - TITLE` (either alone when the other is missing).
   - HLS `#EXTINF` titles, dropping empty ones and "no desc".
5. **Buffer.** Decoded stereo frames, chunked with their sample rate, go into
   a ring of about 4 s. `Playing` once 1.5 s is buffered (Freight Fate's
   BASS pre-buffer). When full, the worker waits for room instead of
   reading on, so the network paces the download (HLS segments included),
   as BASS did. A paused stream resumes where it paused, a few seconds
   behind live; a pause long enough for the server to drop us ends in a
   reconnect on resume (decided 2026-10-01, replacing drop-oldest).

### States, retries and failure

- `Connecting` until the first 1.5 s is buffered; `Playing`; `Reconnecting`
  while retrying (the ring plays out, then silence; the title is kept);
  `Failed(reason)`, terminal. A game retries by opening a new `Stream`.
- **Fail at once:** HTTP 4xx, no decodable audio, encrypted HLS, a playlist
  with no usable entry, more than 3 playlist hops.
- **Retry:** timeout, reset connection, HTTP 5xx, the server ending the
  stream, or 10 s with no decoded audio. Back off 1, 2, 4, 8, 16 s, then
  `Failed`. 30 s of good playback resets the count.
- A corrupt packet is skipped; decoding goes on.
- **Never blocking:** a failed thread spawn is `Failed` at once. Dropping the
  last handle sets a flag the worker checks after every read and wakes on
  between retries; the worker is detached, never joined. A panic in it is
  caught and becomes `Failed("internal error")`.
- `log::info` lines for connect, reconnect and failure, with the URL and
  timings (request to first audio), like Freight Fate's tune timing line.

## Licences

- New crates: `ureq` (MIT/Apache), `icy-metadata` (MIT/Apache), `m3u8-rs`
  (MIT), `mpeg2ts` (MIT), `url` (MIT/Apache), Symphonia's AAC (MPL-2.0, as
  the rest of Symphonia). No DLLs.
- `fdk-aac` adds `symphonia-adapter-fdk-aac` ((MIT or Apache-2.0) and
  MPL-2.0) and Fraunhofer's FDK AAC library, compiled in. Fraunhofer's
  licence allows shipping it but grants no patent rights; a game that turns
  it on takes the HE-AAC patent question on itself. The README says so where
  it lists the feature.

## Testing

- **Feed:** Kira mock-output tests as for clips: a feed is heard, resampled
  to the right pitch across a rate change, silent where it runs dry, and
  silenced by pause; the pool replays a feed after the deferred backend
  opens.
- **Radio:** a small HTTP server on a local port, inside the tests, serves
  canned responses: an ICY MP3 stream with titles, an Ogg stream with
  chained comments, a `.pls` pointing at one, HLS with ADTS segments and with
  TS segments, a 404, an encrypted playlist, and a server that drops
  mid-stream (Reconnecting, then Playing again). Fixtures are short files
  made with ffmpeg, the command noted beside them. CI never touches the real
  network.
- **Listening pass:** an ignored test plays a handful of real Freight Fate
  stations (HTTP MP3, AAC, Ogg and HLS) and prints each one's state and
  title.

## Out of scope

- Tracker music (MOD/XM/S3M/IT): `xmrsplayer` can come later if a game
  ships module files.
- Local playlist files: Freight Fate parses its own; a game hands `Stream`
  URLs and plays local files as clips.
- Seeking, recording or saving a stream; proxies; encrypted HLS.
