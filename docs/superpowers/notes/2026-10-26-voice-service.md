# Voice service (port plan T8)

Date: 2026-10-26. Builds on the spike (`2026-10-26-voice-spike.md`).

## Shape

- `hearken-app` gains a commentary slot, not a dependency: `Commentary<G>`
  (a trait), `Line<C>`, and `Ctx::{set_commentary, commentate,
  prime_commentary, stop_commentary, say_line, hold_duck, transcribe}`. The
  app updates the commentary each frame; `stop_speech` (F2) and
  `pause_event_speech` stop it; the duck holds while it speaks. With none
  installed, `commentate` goes to `say_event_with`.
- `hearken-voice` implements it twice: `Voice` (Kokoro on a worker thread,
  memory and disk cache, the queue) and `FakeVoice` (records at once, for
  `Headless`). Both build without the `voice` feature; only Kokoro needs it,
  so a game's tests need no model and the CI tests use a stand-in engine.

## Decisions

- **Staleness counts synthesis.** A line's wait starts when it is asked for.
  Waits are 4 s (Ambient) and 8 s (Important), longer than the event
  pacer's 3 s and 0.8 s, because synthesis alone is 1 to 6 s on the spike's
  runners. A dropped line's synthesis still lands in the cache.
- **Cache key** is FNV-1a of an engine tag, the lexicon's bytes, the voice,
  the rate in whole percent and the text. Changing the lexicon or upgrading
  local-tts re-synthesizes. Disk cache is capped at 256 MB, oldest first,
  trimmed when the worker starts.
- **Windows ships DirectML.dll.** pyke's prebuilt ONNX Runtime for Windows
  always links DirectML, so the program cannot start without the DLL. The
  options were shipping it, or every game building a CPU-only ONNX Runtime
  and pointing `ORT_LIB_LOCATION` at it (an ONNX Runtime source build, tens
  of minutes, in every game's CI and on every developer's machine). Shipping
  costs nothing: `hearken release` already copies every DLL beside the exe,
  and it already ships Microsoft's Visual C++ runtime under Microsoft's
  terms; DirectML is published as a redistributable in the same way. The
  voices run on the CPU (local-tts registers no execution provider), so
  DirectML is loaded, not used. The CPU-only route stays documented in the
  README for a game that wants no Microsoft DLL.
- **DirectML's licence, checked.** The redistributable (NuGet
  `Microsoft.AI.DirectML` 1.15.4, `LICENSE.txt`) section 1(a): "You may copy
  and distribute the software ... in applications and services you develop
  ... and/or games that run on Windows and Xbox." Not on its own (3e), not
  with its notices removed (3c), Windows and Xbox only. Its
  `ThirdPartyNotices.txt` covers GSL and half (MIT) and Zstandard
  (BSD-3-Clause, notice required with binaries), so `hearken release`
  writes both files beside the DLL (copies in `crates/hearken-cli/licenses`).
  Section 2(a) says it may collect usage data. It is free to ship in a free
  or open-source game; the DLL itself stays under Microsoft's terms.
- **Installing.** Both routes exist: `voices/` in the release zip
  (`include`), and the `voice-download` feature's Settings row and
  `install_confirm` screen, which install into the data folder. Which one
  Fastbreak uses is Orinks's call (T10).
