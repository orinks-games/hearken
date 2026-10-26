# Voice spike: Kokoro through local-tts (port plan T7)

Date: 2026-10-26. Branch `claude/project-thread-gsu378`, PR #19.

## Result

local-tts-core 0.7.0 (git, pinned to `61efef0`) with the `kokoro-en-q8`
package builds and speaks on Windows, Linux and macOS. The CI `voice` job
installs the pinned package, synthesizes a line, and checks that
`hearken_audio::probe_clip` decodes the WAV at the model's rate and length,
in `bf_emma` and `am_michael`. **T8 can go ahead**, with the follow-ups
listed below.

Runners: `windows-2025` (x64), `ubuntu-24.04` (x64), `macos-15` (arm64).
Runs: [40383373155](https://github.com/orinks-games/hearken/actions/runs/40383373155)
and [40387372595](https://github.com/orinks-games/hearken/actions/runs/40387372595)
(both `workflow_dispatch`, all three platforms).

## How ONNX Runtime links

`ort 2.0.0-rc.10` downloads pyke's prebuilt ONNX Runtime 1.22 at build time
(from `cdn.pyke.io`; a build with no network needs `ORT_LIB_LOCATION`).

| Platform | ONNX Runtime | Other runtime dependencies of the test binary |
|---|---|---|
| Linux | static | libstdc++, libgcc_s, libasound (already needed by cpal) |
| macOS | static | system frameworks only, CoreML.framework among them |
| Windows | static | **DirectML.dll**, copied beside the binary by ort; MSVCP140/VCRUNTIME140 (the dynamic MSVC runtime) |

The Windows prebuilt includes the DirectML execution provider, so a game
needs `DirectML.dll` next to its exe. DirectML's redistributable is under
Microsoft's own licence, not MIT. T8 should decide between shipping it (and
adding its notice) or linking a CPU-only ONNX Runtime through
`ORT_LIB_LOCATION`. A Windows release also needs the VC++ runtime, which the
game's other C++ dependencies may need anyway.

## Build time

First (cold) build on the runner, hearken-voice with its dev-dependencies
(hearken-audio and Kira), no sccache hits:

| Platform | dev profile, all targets | test profile (second build of the same crates) |
|---|---|---|
| Linux | 2 min 43 s | 1 min 52 s |
| macOS | 6 min 2 s | about 3 min 50 s |
| Windows | 6 min 29 s | about 4 min 55 s |

With rust-cache and sccache warm, the single test-profile build took 176 s
(Linux), 136 s (macOS) and 226 s (Windows). The job now builds once.

## Synthesis latency

Ten commentary lines of 2.9 to 5.4 s each (35.9 s of audio in all), one model
loaded once, after a warm-up line. Test profile (dependencies at opt-level 2;
ONNX Runtime is prebuilt either way).

| Platform | Load | Run 1: mean / worst / real-time factor | Run 2: mean / worst / real-time factor |
|---|---|---|---|
| Linux | 0.5 s | 1.24 s / 1.83 s / 0.35 | 3.12 s / 4.67 s / 0.87 |
| macOS | 1.1 s | 2.62 s / 4.85 s / 0.73 | 3.09 s / 4.53 s / 0.86 |
| Windows | 1.0 s | 2.83 s / 4.17 s / 0.79 | 4.24 s / 6.33 s / 1.18 |

Hosted runners are shared 3 to 4 core machines and vary a lot between runs
(Linux was 2.5 times slower in run 2). Read these as an upper bound for a
modest player machine, not a benchmark. The longest line (the final score)
is always the worst.

What it means for T8:

- A dynamic line starts 1 to 6 s after it is asked for on this hardware,
  and a CI-class Windows machine can fall behind real time. So `prime()`
  matters: fixed lines for a screen must be synthesized before they are
  needed, and the disk cache keeps them for later sessions.
- The queue's staleness rule should count synthesis time: a Normal line
  that is still synthesizing when its moment passes is dropped.
- Short lines are cheaper. Score updates and calls kept to one sentence
  stay near 1 to 3 s.
- Loading takes 0.5 to 1.3 s, so it belongs on the worker thread at
  startup, as the plan says.

## Other notes

- local-tts is git-only (not on crates.io). It uses edition 2024, which
  Hearken's 1.98 toolchain builds.
- `Kokoro::load` verifies the pinned SHA-256 of the model, tokenizer and
  default voice. Unknown words are refused unless the fallback is on;
  hearken-voice turns it on, so team and player names are guessed.
  `add_lexicon` takes CMU-format overrides for T9's `voices/lexicon.txt`.
- The installer (`download` feature) fetches about 92 MB from Hugging Face
  plus the CMU dictionary from GitHub, with no account. A sandbox that
  blocks `huggingface.co` or `cdn.pyke.io` can build only with a local ONNX
  Runtime, and cannot run the model test.
- `cargo check -p hearken --features voice` passes on Linux. Linking a
  whole game with both Prism and ONNX Runtime on Windows is not proven yet:
  T8 should build the template (or a scratch game) with `voice` on Windows
  in CI before relying on it.
