# Hearken Facade and Radio Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Games depend on one `hearken` crate, and with its `radio` feature play internet radio (HTTP/ICY, playlists, live HLS) with titles and reconnects, without BASS.

**Architecture:** `hearken` is a facade over the existing crates. `hearken-audio` gains a generic `Feed` source that backends pull frames from; Kira plays it through one custom `Sound`. The new `hearken-radio` runs one blocking worker thread per `Stream` (`ureq` 2, `icy-metadata`, Symphonia, `m3u8-rs`, `mpeg2ts`) that fills a rate-tagged ring the `Feed` reads.

**Tech Stack:** Rust 1.95 (`rust-toolchain.toml`), Kira 0.12, Symphonia 0.6 (+ `symphonia-adapter-libopus` 0.3, optional `symphonia-adapter-fdk-aac` 0.2), `ureq` 2.12, `icy-metadata` 0.6, `m3u8-rs` 6, `mpeg2ts` 0.6, `url` 2.

**Spec:** `docs/superpowers/specs/2026-10-01-hearken-radio-design.md`

## Global Constraints

- Branch `radio`; one commit per task; never push.
- No DLLs, no async runtime (no tokio), no new C dependency except under `fdk-aac`.
- The only identifying header sent is `User-Agent: Hearken/{CARGO_PKG_VERSION}`; never the user's name or email anywhere.
- Defaults (`StreamOptions::default()`): `timeout` 10 s (connect and every read), `retry_delays` [1, 2, 4, 8, 16] s, `healthy_after` 30 s, `stall` 10 s, `prebuffer` 1.5 s, `buffer` 4 s.
- At most 3 playlist hops; redirects up to 10; HLS starts 3 segments from the live edge and re-reads the playlist every half target duration (at least 0.5 s).
- Nothing blocks the game loop; no thread is ever joined (the `Backend` contract).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -- -D warnings` clean before each commit; run `actionlint` after any workflow edit.
- Focused tests while working; the full suite (`cargo test --workspace --locked`) once, in Task 8.
- Fixtures are made with ffmpeg at half scale (`sine=frequency=440` plus `-af volume=12dB`, since ffmpeg's sine is 1/8 scale); each fixture's command goes in a comment where the test includes it.
- Comments match the codebase: doc comments say why; a known ceiling gets a `ponytail:` comment naming the upgrade path.

## Review Focus

1. A station whose Content-Type is wrong or missing (MP3 sent as `application/octet-stream`, a playlist as `text/plain`) should still play, by sniffing the first bytes. Test in Task 5.
2. A server that accepts the connection and then sends nothing, before or mid-stream, should time out after `timeout` and be retried, never leave the worker hung. Test in Task 7.
3. An old Shoutcast v1 server answering `ICY 200 OK` instead of `HTTP/1.x 200` should play. Test in Task 3.
4. An ICY title in Latin-1 rather than UTF-8 (common on Shoutcast) should read as Latin-1, not vanish. Test in Task 4.
5. A game dropping its `Stream` while the worker is blocked connecting should see the worker exit within `timeout`, with no panic. Test in Task 3.

---

### Task 1: The `hearken` facade

**Files:**
- Create: `crates/hearken/Cargo.toml`, `crates/hearken/build.rs`, `crates/hearken/src/lib.rs`
- Modify: `Cargo.toml` (workspace deps), `template/Cargo.toml`, `template/build.rs`, `template/src/*.rs`, `README.md` (Building), `.github/workflows/ci.yml` ("Build without Kira")

**Interfaces:**
- Produces: crate `hearken` with `pub use hearken_app::*;` and `pub use hearken_speech as speech;`; features `default = ["kira"]`, `kira = ["hearken-app/kira"]`; `links = "hearken"`, whose build script emits `cargo:link_args=<DEP_HEARKEN_SPEECH_LINK_ARGS>`, so a game reads `DEP_HEARKEN_LINK_ARGS`.

- [ ] **Step 1: Move the template onto `hearken`.** `template/Cargo.toml` depends on `hearken.workspace = true` only. `template/build.rs` reads `DEP_HEARKEN_LINK_ARGS`; its warning names `hearken` as the missing direct dependency. Every `hearken_app::` in `template/src` becomes `hearken::`.
- [ ] **Step 2: Run `cargo build -p hello-hearken`.** Expected: FAIL, no crate `hearken`.
- [ ] **Step 3: Create the facade.**
  - Workspace deps: `hearken = { path = "crates/hearken" }`. `hearken-app` gains `default-features = false`, so the facade's `kira` feature decides.
  - `crates/hearken/build.rs`: `rerun-if-env-changed=DEP_HEARKEN_SPEECH_LINK_ARGS`, then print `cargo:link_args=` with the value, or empty when it is unset.
  - `lib.rs` crate doc: one dependency for a game; extras are features.
- [ ] **Step 4: Verify.**
  - `cargo build -p hello-hearken 2>&1 | grep -c "link arguments are missing"` prints `0`.
  - `cargo test -p hello-hearken` passes.
  - `cargo check -p hearken --no-default-features --locked` builds.
- [ ] **Step 5: CI and docs.** The CI step "Build without Kira" runs `cargo check -p hearken --no-default-features --locked`; run `actionlint`. The README's `build.rs` paragraph names `hearken` as the direct dependency instead of `hearken-speech`.
- [ ] **Step 6: Commit** `feat: a hearken facade crate; the template depends on it alone`.

### Task 2: The feed source in `hearken-audio`

**Files:**
- Create: `crates/hearken-audio/src/feed.rs`
- Modify: `backend.rs` (`Source::Feed`), `lib.rs` (exports), `pool.rs` (`Live`, `start_feed`, `play_with_fallback`), `open.rs` (`swap_backend` replay), `recording.rs` (a feed records `clip: None`), `kira.rs` (`Playing::Feed`, `FeedSound`)
- Test: `crates/hearken-audio/tests/kira.rs`, `crates/hearken-audio/tests/open.rs`

**Interfaces:**
- Produces:
  ```rust
  pub trait Feed: Send + Sync { fn read(&self, out: &mut [[f32; 2]]) -> FeedRead; }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub struct FeedRead { pub frames: usize, pub sample_rate: u32 } // sample_rate is meaningless when frames == 0
  Source::Feed(Arc<dyn Feed>)
  impl Audio { pub fn start_feed(&mut self, feed: Arc<dyn Feed>, pos: Option<Vec3>, bus: Bus) -> Sound }
  ```
  `start_feed` uses key `"feed"`, `looping: true`, volume 1.0. `Live` gains `feed: Option<Arc<dyn Feed>>`, and `swap_backend` replays it with a clone. `play_with_fallback` never falls back to a default clip for a feed (as for a voice).

- [ ] **Step 1: Write the failing tests.** In `tests/kira.rs`, a `TestFeed` gives a 441 Hz sine at an adjustable `rate: AtomicU32` and runs dry while `dry: AtomicBool` is set.
  ```rust
  #[test] fn a_feed_is_heard_at_its_own_pitch_across_a_rate_change()
  // start_feed at 22_050 Hz; crossings(&r.render(4_410)[..4_410]) in 80..=96;
  // set rate 48_000; render(1_024) to settle; crossings again in 80..=96.
  #[test] fn a_feed_that_runs_dry_is_silence_not_a_stop()
  // dry: rms(&r.render(2_000)[500..]) < 1e-4 and r.b.is_playing(ch); not dry: rms(...) > 0.1.
  #[test] fn pause_silences_a_feed()
  // pause(ch, true); rms(&r.render(2_000)[500..]) < 1e-4.
  ```
  In `tests/open.rs`:
  ```rust
  #[test] fn a_feed_moves_over_when_the_device_opens()
  // Audio::deferred over a RecordingBackend; start_feed before it opens;
  // after the open: rec.played() == ["feed"] and that Call::Play has clip None.
  ```
- [ ] **Step 2: Run `cargo test -p hearken-audio --test kira feed` and `--test open feed`.** Expected: FAIL to compile (no `Feed`, no `start_feed`).
- [ ] **Step 3: Implement.** `FeedSound` in `kira.rs` is a Kira `Sound`:
  - It pulls from the feed into a preallocated 1,024-frame buffer, so nothing allocates on the audio thread.
  - It reads frames out by linear interpolation, stepping `feed_rate * pitch / output_rate`. A new `sample_rate` from `read` takes effect from that block on.
  - When the feed returns no frames it outputs `Frame::ZERO` and zeroes its interpolation pair, so silence carries no DC.
  - Pitch arrives through an `Arc<AtomicU32>`, as with `LiveVoice`. `finished()` is always false.
  - `Entry::alive` is true for a feed until it is stopped.
- [ ] **Step 4: Run the Step 2 commands.** Expected: PASS; the whole `tests/kira.rs` and `tests/open.rs` still pass.
- [ ] **Step 5: Commit** `feat(audio): a feed source backends pull from`.

### Task 3: `hearken-radio`: the stream, the ring, HTTP audio

**Files:**
- Create: `crates/hearken-radio/Cargo.toml`, `src/lib.rs`, `src/stream.rs` (handle, `Shared`), `src/ring.rs`, `src/worker.rs`, `src/http.rs`, `src/decode.rs`, `src/codecs.rs`
- Create: `tests/support/mod.rs`, `tests/http.rs`, `tests/fixtures/tone.mp3`
- Modify: `Cargo.toml` (workspace deps), `crates/hearken/Cargo.toml` and `src/lib.rs` (`radio` feature)

**Interfaces:**
- Consumes: `hearken_audio::{Feed, FeedRead}` (Task 2).
- Produces (public):
  ```rust
  #[derive(Clone)] pub struct Stream;          // wraps Arc<Handle>; Handle: Feed; Handle::drop closes the worker
  impl Stream {
      pub fn open(url: &str) -> Stream;          // open_with(url, StreamOptions::default())
      pub fn open_with(url: &str, options: StreamOptions) -> Stream;
      pub fn state(&self) -> StreamState;
      pub fn title(&self) -> Option<String>;
      pub fn feed(&self) -> Arc<dyn Feed>;
  }
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum StreamState { Connecting, Playing, Reconnecting, Failed(String) }
  #[derive(Clone, Debug)]
  pub struct StreamOptions { pub timeout: Duration, pub retry_delays: Vec<Duration>, pub healthy_after: Duration,
                             pub stall: Duration, pub prebuffer: Duration, pub buffer: Duration }
  ```
- Produces (crate):
  - `enum Fault { Fatal(String), Retry(String) }`.
  - `Shared`, with `set_state`, `set_title(Option<String>)`, `push(rate: u32, frames: Vec<[f32; 2]>)`, `buffered() -> Duration`, `closed() -> bool`, and `sleep(d) -> bool`, which returns false at once if the stream is closed, woken by a `Condvar`.
  - `ring::Ring { new(cap: Duration), push(&mut self, rate, frames), read(&mut self, out) -> FeedRead, buffered(&self) -> Duration }`.
  - `http::open(url: &str, opts: &StreamOptions) -> Result<Response, Fault>`, where `Response { content_type: Option<String>, icy_metaint: Option<NonZeroUsize>, body: Box<dyn Read + Send> }`.
  - `codecs::registry() -> &'static CodecRegistry`.
  - `decode::pump(source: Box<dyn MediaSource>, hint: Hint, shared: &Shared, opts: &StreamOptions) -> Fault`, which returns why it stopped.
- Test support (`tests/support/mod.rs`):
  ```rust
  pub enum Reply {
      Body(u16, &'static str, Vec<u8>),               // status, Content-Type, body, then close
      Endless(&'static str, Vec<u8>, Option<Vec<u8>>), // 200; body repeated forever; Some(title bytes): ICY every 8_192 bytes when asked
      Cut(&'static str, Vec<u8>, usize),              // like Endless without ICY, closing after n body bytes
      Hang,                                           // accept, read the request, send nothing
      IcyV1(Vec<u8>),                                 // "ICY 200 OK", content-type audio/mpeg, body repeated forever
  }
  impl TestServer {
      pub fn start(route: impl Fn(&str) -> Reply + Send + Sync + 'static) -> TestServer;
      pub fn url(&self, path: &str) -> String;
      pub fn hits(&self, path: &str) -> usize;
      pub fn open_connections(&self) -> usize;
      pub fn last_request(&self) -> String;           // raw request head
  }
  pub fn fast() -> StreamOptions; // timeout 1 s, retry_delays [50 ms, 50 ms], stall 1 s, healthy_after 1 s, prebuffer 0.2 s, buffer 1 s
  pub fn wait_for(what: &str, within: Duration, done: impl FnMut() -> bool); // panics naming `what`
  pub fn drain(stream: &Stream) -> Vec<[f32; 2]>;
  ```
  Endless, Cut and IcyV1 bodies are paced at 4 KiB every 125 ms (32 KiB/s). An ICY block is `StreamTitle='<title>';` zero-padded to a multiple of 16, behind a length byte of len/16.
- Workspace deps:
  - `ureq = { version = "2.12", default-features = false, features = ["tls", "native-certs"] }`, `icy-metadata = { version = "0.6", default-features = false }`, `url = "2"`.
  - `hearken-radio = { path = "crates/hearken-radio" }`.
  - `hearken-radio` turns on Symphonia's `aac` feature.

- [ ] **Step 1: Make the fixture.** `tone.mp3`: 2 s mono MP3, `-c:a libmp3lame -b:a 64k -write_xing 0 -id3v2_version 0`, bare frames so that repeating it end to end is a clean stream.
- [ ] **Step 2: Write the failing tests.** Ring unit tests in `ring.rs`:
  ```rust
  #[test] fn reads_come_out_in_order()
  #[test] fn a_read_stops_at_a_rate_change()                 // push(44_100, 10 frames), push(48_000, 10): read(32) -> {10, 44_100}, then {10, 48_000}
  #[test] fn a_full_ring_drops_the_oldest_audio()            // cap 1 s at 1_000 Hz: push 1_500 frames valued by index; buffered() == 1 s; the first frame read is 500
  #[test] fn buffered_counts_seconds_at_each_chunks_rate()   // 22_050 frames at 44_100 + 24_000 at 48_000 == 1 s
  ```
  `tests/http.rs`:
  ```rust
  #[test] fn an_mp3_station_plays()                   // Endless audio/mpeg: Playing within 5 s; rms of drain() > 0.1
  #[test] fn the_request_names_hearken_and_nothing_else() // last_request() contains "User-Agent: Hearken/" and "Icy-MetaData: 1"; no "@"
  #[test] fn a_missing_station_fails_at_once()        // Body(404): Failed within 2 s, reason contains "404"; hits == 1
  #[test] fn a_url_that_is_not_one_fails_without_panicking() // open("not a url"): Failed within 1 s
  #[test] fn a_shoutcast_v1_station_plays()           // IcyV1(tone.mp3): Playing within 5 s
  #[test] fn dropping_every_handle_stops_the_worker()  // Endless: Playing; drop stream and feed; open_connections() == 0 within 3 s
  #[test] fn dropping_while_connecting_to_a_silent_server_ends_within_the_timeout() // Hang, fast(): drop at once; open_connections() == 0 within 3 s
  ```
- [ ] **Step 3: Run `cargo test -p hearken-radio`.** Expected: FAIL to compile.
- [ ] **Step 4: Implement.**
  - `Stream::open_with` spawns the worker with `std::thread::Builder`; a failed spawn sets `Failed("could not start")`. `worker::run` executes inside `catch_unwind`; a panic sets `Failed("internal error")`. It opens, then pumps; for now every `Fault` ends in `Failed(reason)`, and Task 7 adds retries. The state is `Connecting` until `buffered() >= prebuffer`, then `Playing`.
  - `http::open` builds a ureq `AgentBuilder` with `timeout_connect` and `timeout_read` (both `opts.timeout`), `redirects(10)`, the user agent, and the header `Icy-MetaData: 1`.
    - A 4xx status is `Fatal("HTTP <code>")`, a 5xx is `Retry("HTTP <code>")`, and transport errors are `Retry`.
    - An unparseable URL is `Fatal("bad URL")`.
    - `ErrorKind::BadStatus` on an `http://` URL retries once over a raw `TcpStream` (HTTP/1.0 request, same headers and timeouts). That fallback accepts `ICY 200` or `HTTP/1.x 200` and reads `content-type` and `icy-metaint` case-insensitively.
  - `decode::pump` probes the source through `ReadOnlySource` with `codecs::registry()`. A probe failure is `Fatal("no playable audio")`. Decode errors on a single packet skip that packet. Frames reach the ring as stereo, with mono duplicated to both sides and only the first two channels kept. The pump stops on `shared.closed()`.
  - `codecs::registry()` is Symphonia's `register_enabled_codecs` plus `OpusDecoder`.
- [ ] **Step 5: Run `cargo test -p hearken-radio`.** Expected: PASS.
- [ ] **Step 6: Facade feature.** `crates/hearken`: `radio = ["dep:hearken-radio"]` and `#[cfg(feature = "radio")] pub use hearken_radio as radio;`. Verify with `cargo check -p hearken --features radio`.
- [ ] **Step 7: Commit** `feat(radio): Stream plays an HTTP station through a feed`.

### Task 4: Titles

**Files:**
- Create: `crates/hearken-radio/src/title.rs`, `tests/titles.rs`, `tests/fixtures/first.ogg`, `tests/fixtures/second.ogg`
- Modify: `http.rs` (wrap the body in `IcyMetadataReader` when `icy_metaint` is set), `decode.rs` (chained Ogg, metadata titles)

**Interfaces:**
- Produces: `title::icy(block: &[u8]) -> Option<String>`, `title::ogg(artist: Option<&str>, title: Option<&str>) -> Option<String>`, `title::hls(extinf_title: &str) -> Option<String>`. All three trim, collapse whitespace runs to one space, and return `None` when the result is empty.

- [ ] **Step 1: Make the fixtures.** `first.ogg` and `second.ogg`: 1 s each, Vorbis (`-c:a libvorbis`), with `-metadata artist=Hearken -metadata title=First` and `title=Second`.
- [ ] **Step 2: Write the failing tests.** Unit tests in `title.rs`:
  ```rust
  #[test] fn icy_reads_the_stream_title()        // b"StreamTitle='Band - Song';StreamUrl='';" -> "Band - Song"
  #[test] fn an_empty_icy_title_is_none()        // b"StreamTitle='';" -> None
  #[test] fn latin1_icy_bytes_read_as_latin1()   // b"StreamTitle='Caf\xe9 del Mar';" -> "Café del Mar"
  #[test] fn ogg_joins_artist_and_title()        // (Some("Band"), Some("Song")) -> "Band - Song"; (None, Some("Song")) -> "Song"
  #[test] fn hls_drops_no_desc()                 // "no desc" and "  " -> None
  ```
  `tests/titles.rs`:
  ```rust
  #[test] fn an_icy_station_reports_its_title()          // Endless with Some(b"Band - Song"): title() == Some("Band - Song") within 5 s
  #[test] fn a_latin1_icy_title_reads_as_latin1()        // title bytes b"Caf\xe9": title() == Some("Café") within 5 s
  #[test] fn a_chained_ogg_station_reports_each_tracks_title() // Body(application/ogg, first + second), fast(): title() == Some("Hearken - Second") within 5 s
  ```
- [ ] **Step 3: Run `cargo test -p hearken-radio title`.** Expected: FAIL.
- [ ] **Step 4: Implement.**
  - ICY: the `IcyMetadataReader` callback sets the title from `stream_title()`. On `MetadataParseError::InvalidUtf8(e)` it decodes `e.into_bytes()` as Latin-1, one byte per char, and parses that with `str::parse::<IcyMetadata>()`.
    - `// ponytail: Latin-1, not Windows-1252; the 0x80-0x9F punctuation reads as controls. Use encoding_rs if a station is heard doing that.`
    - An empty title clears to `None`.
  - Ogg: Symphonia reports a new chained stream as `Error::ResetRequired`. On that, re-read the default track, make a new decoder from its parameters, and set the title from the latest metadata revision's artist and title tags, matching tag keys case-insensitively.
- [ ] **Step 5: Run `cargo test -p hearken-radio`.** Expected: PASS.
- [ ] **Step 6: Commit** `feat(radio): now-playing titles from ICY and Ogg comments`.

### Task 5: Playlists and sniffing

**Files:**
- Create: `crates/hearken-radio/src/playlist.rs`, `tests/playlists.rs`
- Modify: `worker.rs` (kind dispatch, hop count), `http.rs` (peek the first bytes through a `BufReader`)

**Interfaces:**
- Produces:
  - `enum Kind { Audio(Option<&'static str>), Playlist, Hls }`, where the `Audio` payload is a Symphonia extension hint.
  - `fn kind(content_type: Option<&str>, url: &str, head: &[u8]) -> Kind`.
  - `playlist::first_stream(text: &str, base: &url::Url) -> Option<String>`.
- Rules for `kind`:
  - The Content-Type is matched case-insensitively, with parameters stripped.
    - `audio/x-scpls` is a playlist.
    - The m3u types (`audio/x-mpegurl`, `audio/mpegurl`, `application/x-mpegurl`, `application/vnd.apple.mpegurl`) are `Hls` if the head contains `#EXT-X-`, otherwise a playlist.
    - `audio/mpeg` is `Audio(Some("mp3"))`; `audio/aac`, `audio/aacp` and `audio/x-aac` are `Some("aac")`; `application/ogg`, `audio/ogg` and `audio/opus` are `Some("ogg")`.
  - Any other or missing type is sniffed from the head:
    - `[playlist]` is a playlist.
    - `#EXTM3U` is `Hls` with `#EXT-X-`, otherwise a playlist.
    - A head starting with `http` is a playlist.
    - Otherwise the URL's extension decides (`.pls`, `.m3u`, `.m3u8`), and failing that it's `Audio(None)`.

- [ ] **Step 1: Write the failing tests.** Unit tests:
  ```rust
  #[test] fn pls_gives_its_first_file()
  #[test] fn m3u_skips_comments_and_blank_lines()
  #[test] fn a_relative_entry_resolves_against_the_playlist()      // "live.mp3" against http://h/radio/list.m3u -> http://h/radio/live.mp3
  #[test] fn kind_from_content_type_ignores_case_and_parameters()  // "Audio/MPEG; charset=x" -> Audio(Some("mp3"))
  #[test] fn kind_sniffs_a_playlist_served_as_text_plain()
  #[test] fn kind_sniffs_hls_by_its_tags()
  ```
  `tests/playlists.rs`:
  ```rust
  #[test] fn a_pls_link_plays_its_first_station()   // /list.pls -> "[playlist]\nFile1=<url /live.mp3>": Playing within 5 s
  #[test] fn an_mp3_served_as_octet_stream_plays()  // Endless("application/octet-stream", tone.mp3): Playing within 5 s
  #[test] fn a_playlist_loop_fails_after_three_hops() // /a.m3u -> /b.m3u -> /c.m3u -> /d.m3u -> /a.m3u: Failed("too many playlist hops"); total hits <= 4
  #[test] fn a_playlist_with_no_stream_fails()      // "#EXTM3U\n# nothing\n": Failed("empty playlist")
  ```
- [ ] **Step 2: Run `cargo test -p hearken-radio playlist`.** Expected: FAIL.
- [ ] **Step 3: Implement** the rules above. The worker follows a playlist by re-opening the entry's URL, counting hops from the station URL. `Hls` routes to `hls::play` once Task 6 lands; until then it is `Fatal("HLS not supported")`.
- [ ] **Step 4: Run `cargo test -p hearken-radio`.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(radio): playlist links and sniffing a station's real format`.

### Task 6: Live HLS

**Files:**
- Create: `crates/hearken-radio/src/hls.rs`, `tests/hls.rs`, `tests/fixtures/tone.aac`, `tests/fixtures/tone.ts`
- Modify: `Cargo.toml` (workspace deps `m3u8-rs = "6"`, `mpeg2ts = "0.6"`), `worker.rs`

**Interfaces:**
- Consumes: `http::open`, `title::hls`, `decode::pump`, `Shared`, `Fault`.
- Produces: `hls::play(url: &str, playlist: &str, shared: &Shared, opts: &StreamOptions) -> Fault`. It builds an `HlsReader: Read` and pumps it with no hint.
- `HlsReader::read` serves the current segment's bytes. When those run out it:
  - fetches the next segment;
  - re-reads the media playlist every `max(target_duration / 2, 0.5 s)` until a segment newer than the last one served (by media sequence) appears;
  - checks `shared.closed()` between fetches.
  It returns `io::ErrorKind::TimedOut` after `opts.stall` with no new segment, which the pump reports as `Retry("stalled")`.

- [ ] **Step 1: Make the fixtures.** `tone.aac`: 1 s ADTS (`-c:a aac -b:a 64k -f adts`). `tone.ts`: 1 s MPEG-TS (`-c:a aac -b:a 64k -f mpegts`).
- [ ] **Step 2: Write the failing tests.** The test server's `/live.m3u8` is a sliding window. Let `n` be the whole seconds since the server started. The playlist then has `#EXT-X-TARGETDURATION:1` and `#EXT-X-MEDIA-SEQUENCE:n`, followed by 4 segments `seg{n}.aac` … `seg{n+3}.aac`. Each segment is `#EXTINF:1.0,Hearken - Live`. Every `/seg*.aac` path serves `tone.aac`, and `/ts.m3u8` does the same with `.ts` segments.
  ```rust
  // unit, hls.rs
  #[test] fn the_live_edge_starts_three_segments_back()  // 6 segments: first served is index 3
  #[test] fn a_ts_segment_demuxes_to_its_audio()         // tone.ts -> bytes starting 0xFF 0xF? (ADTS sync)
  // tests/hls.rs
  #[test] fn a_live_hls_station_with_adts_segments_plays_with_its_title() // Playing within 5 s; title "Hearken - Live"; rms of drain() > 0.1
  #[test] fn a_live_hls_station_with_ts_segments_plays()
  #[test] fn a_master_playlist_plays_its_best_audio_variant() // /low.m3u8 BANDWIDTH=64000 (404), /live.m3u8 BANDWIDTH=128000: Playing; hits("/low.m3u8") == 0
  #[test] fn an_encrypted_hls_station_fails_at_once()     // #EXT-X-KEY:METHOD=AES-128: Failed("encrypted HLS"); no segment hits
  #[test] fn an_hls_playlist_that_stops_growing_counts_as_a_stall() // static 3-segment playlist without ENDLIST, fast(): leaves Playing within 3 s
  ```
- [ ] **Step 3: Run `cargo test -p hearken-radio hls`.** Expected: FAIL.
- [ ] **Step 4: Implement.**
  - **Master playlist:** take the variant with the highest `bandwidth` among those whose `codecs` names no video codec (`avc1`, `hvc1`, `hev1`, `vp09`, `av01`).
  - **Encryption:** any segment key whose method is not `NONE` is `Fatal("encrypted HLS")`.
  - **Segment format:** a segment that starts with `0x47`, or whose URI ends `.ts`, goes through `mpeg2ts::pes::PesPacketReader::new(TsPacketReader::new(..))`, keeping the PES data whose stream id is in `0xC0..=0xDF`. Other segments pass through unchanged.
  - **Titles:** a segment's `#EXTINF` title passes through `title::hls`. A `None` leaves the previous title in place.
  - `// ponytail: a codec change at #EXT-X-DISCONTINUITY is not re-probed; restart the pump there if a station is heard doing it.`
- [ ] **Step 5: Run `cargo test -p hearken-radio`.** Expected: PASS.
- [ ] **Step 6: Commit** `feat(radio): live HLS with ADTS and MPEG-TS segments`.

### Task 7: Backpressure, retries and stalls

**Files:**
- Modify: `crates/hearken-radio/src/worker.rs`, `decode.rs` (stall clock), `stream.rs` (`Shared::push` waits), `ring.rs` (no drop-oldest), `hls.rs` (pacing removed), `http.rs`, `tests/support/mod.rs` (`listen`)
- Test: `crates/hearken-radio/tests/reconnect.rs`, `tests/backpressure.rs`, ring unit tests

**Interfaces:**
- Consumes: `Fault`, `Shared::sleep`, `StreamOptions`.
- Produces: `Shared::push(rate, frames)` blocks until the ring has room for the chunk or the stream is closed (the feed's `read` notifies after taking frames; `Handle::drop` notifies too). `Ring::push` never drops audio. Test support `pub fn listen(stream: &Stream) -> Listener`: a thread that drains the feed every 10 ms until the `Listener` drops, standing in for the audio thread.

Decided with the user on 2026-10-01: the drop-oldest ring and Task 6's HLS pacing go. When the ring is full, the worker waits for room, so the network paces the download as BASS did. A paused stream resumes a few seconds behind live, and a pause long enough for the server to drop the connection ends in a reconnect on resume.

- [ ] **Step 1: Write the failing tests** (all with `fast()`; every test that needs playback to flow holds a `listen(&stream)`):
  ```rust
  // ring.rs: replace a_full_ring_drops_the_oldest_audio with
  #[test] fn a_full_ring_keeps_all_its_audio()               // push past cap: nothing is dropped; reads come out in order
  // tests/backpressure.rs
  #[test] fn an_unheard_stream_stops_reading_when_its_buffer_is_full() // Endless, no listener: after Playing, the server's bytes sent stop growing (TestServer gains `bytes_sent(path)`)
  #[test] fn a_paused_stream_resumes_where_it_left_off()     // listen, stop listening 2 s, listen again: no Reconnecting, audio continues
  #[test] fn dropping_a_stream_blocked_on_a_full_buffer_ends_the_worker() // Endless, no listener; drop: open_connections() == 0 within 3 s
  // tests/reconnect.rs
  #[test] fn a_dropped_stream_reconnects_and_plays_again()   // first hit Cut(.., 64 KiB), later Endless: states seen in order Playing, Reconnecting, Playing
  #[test] fn a_server_error_is_retried_then_reported()        // Body(503) always: Failed within 3 s; hits == 3 (1 + retry_delays.len())
  #[test] fn a_server_that_never_answers_is_retried()         // Hang always: Failed within 6 s; hits == 3
  #[test] fn good_playback_resets_the_retry_count()           // every hit Cut(.., 48 KiB) (~1.5 s at the pacing): after 5 hits, state is not Failed
  #[test] fn the_first_connects_retries_stay_connecting()     // Body(503) twice, then Endless: Reconnecting never observed; ends Playing
  ```
- [ ] **Step 2: Run `cargo test -p hearken-radio --test reconnect`.** Expected: FAIL.
- [ ] **Step 3: Implement** the worker loop:
  - A `Fatal` fault sets `Failed(reason)`.
  - A `Retry` fault resets the attempt count if playback had lasted `healthy_after`. Once the attempts reach `retry_delays.len()` the stream is `Failed(reason)`. Otherwise the state becomes `Reconnecting`, but only if the stream ever reached `Playing` (before that it stays `Connecting`), and the worker waits `retry_delays[attempt]` through `Shared::sleep`, exiting if that returns false.
  - The pump returns `Retry("stalled")` after `stall` with no decoded audio, and `Retry("stream ended")` at the end of the stream. Time spent waiting for room in the ring is not a stall.
  - Backpressure: `Shared::push` waits on a `Condvar` until the chunk fits in `buffer` or the stream closes; `Ring` no longer drops audio; delete Task 6's HLS pacing and its pacing test (backpressure paces segment fetches).
  - Carried from Task 3's review:
    - Clamp `prebuffer` strictly below `buffer` in one place, so a game's options can never leave a stream silent in `Connecting`.
    - `ureq::ErrorKind::TooManyRedirects` is `Fatal`, not `Retry`.
    - Wrap every response body in a `Read` that returns an error once `shared.closed()`, so the closed check is truly per read (probe and playlist reads included).
    - After a reconnect, re-gate the feed (or count the prebuffer from the reconnect) so `Reconnecting` returns to `Playing`.
    - Tests: `a_url_that_is_not_one_fails_without_panicking` asserts the reason is `"bad URL"`; `dropping_every_handle_stops_the_worker` first drops only the `Stream` and asserts the connection stays open while the feed lives.
  - `log::info!` lines:
    - `radio: <url> playing after <ms> ms`
    - `radio: <url> lost (<reason>); retry <n> in <s> s`
    - `radio: <url> failed (<reason>)`
- [ ] **Step 4: Run `cargo test -p hearken-radio`.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(radio): retry a dropped station, then report it failed`.

### Task 8: `fdk-aac`, docs, CI and the listening pass

**Files:**
- Modify: `crates/hearken-radio/Cargo.toml`, `src/codecs.rs`, `crates/hearken/Cargo.toml`, `README.md`, `.github/workflows/ci.yml`
- Create: `crates/hearken-radio/tests/live.rs`

**Interfaces:**
- Produces:
  - Feature `fdk-aac = ["dep:symphonia-adapter-fdk-aac"]` on `hearken-radio`, with `symphonia-adapter-fdk-aac = "0.2"` as a workspace dep.
  - Facade feature `fdk-aac = ["radio", "hearken-radio/fdk-aac"]`.

- [ ] **Step 1: Write the failing test** in `codecs.rs`: `#[cfg(feature = "fdk-aac")] #[test] fn fdk_aac_takes_aac_when_enabled()`. It makes an AAC decoder from `codecs::registry()` and asserts that its `codec_info().profiles` list HE-AAC, which only the fdk adapter's do (both decoders are named "aac").
- [ ] **Step 2: Run `cargo test -p hearken-radio --features fdk-aac fdk`.** Expected: FAIL.
- [ ] **Step 3: Implement.** Under `fdk-aac`, register the fdk decoder at `Tier::Preferred`, so it takes AAC (within one tier the last registration wins; at Preferred, FDK wins in either order).
- [ ] **Step 4: Run `cargo test -p hearken-radio --features fdk-aac`.** Expected: every radio test passes with fdk decoding the AAC fixtures.
- [ ] **Step 5: Listening pass.** `tests/live.rs` has one `#[ignore]` test. It plays four stations from Freight Fate's `data/radio_catalog.json`: the first MP3, AAC, Ogg and `.m3u8` URL there, copied in as constants. Each plays for 10 s, and the test prints its URL, state and title. Run `cargo test -p hearken-radio --test live -- --ignored --nocapture` once and read the output.
- [ ] **Step 6: Docs and CI.**
  - README: a "Features" list with `radio` (internet radio) and `fdk-aac`. The `fdk-aac` entry says Fraunhofer's licence permits use only under AAC patent licences the game holds, and grants none.
  - The licence list gains the new crates, per the spec's Licences section.
  - CI adds a step `cargo check -p hearken --features fdk-aac --locked`; run `actionlint`.
- [ ] **Step 7: Final gates.**
  - `cargo fmt --all --check`.
  - Clippy, run as in Global Constraints and again with `-p hearken-radio --features fdk-aac`.
  - `cargo test --workspace --locked` once. Expected: all pass.
- [ ] **Step 8: Commit** `feat(radio): opt-in HE-AAC through fdk-aac; docs and CI`.
