# Fresh commentary from a local language model: research and spike

Date: 2026-10-26. Branch `claude/project-thread-s98rqx`, PR #22 (draft).
Follows the voice spike (`2026-10-26-voice-spike.md`) and the voice
service (`2026-10-26-voice-service.md`); the commentary it would extend is
Fastbreak PR #9 (port plan T9).

## The question

Fastbreak's booth picks each line from a pool of three or four written
phrasings and fills in the score. Kokoro speaks it. Orinks asked whether
a language model running on the player's machine, with no account or
key, could write Big M's colour lines fresh instead, and whether any
model fits comfortably inside the game.

## Short answer

RESULT_SUMMARY

## What the game needs from a model

- **One spoken sentence at a time**, 10 to 20 words, about a situation
  the game can state in a few lines of facts. No reasoning, no markdown,
  no dialogue. A 0.5B model can do this; a 7B model is not needed.
- **Fast on a CPU.** The game's whole budget from an event to Big M's
  voice is a few seconds, and Kokoro already takes 1 to 4 s of it on the
  runners (voice spike). A line's writing must fit in about 1 to 2 s on
  an ordinary four-core PC, or be written ahead of time.
- **Small on disk and in memory.** The voices are 92 MB. A model under
  about 600 MB on disk and under 1 GB resident keeps the game's install
  under 1 GB and leaves room on an 8 GB machine beside the voices, Kira
  and the screen reader.
- **Permissive weights.** The game is MIT or Apache-2.0 and ships (or
  downloads) the weights. Weights under Apache-2.0 or MIT add nothing to
  the player's or Orinks's obligations. Licences that flow a use policy
  or a revenue threshold down to whoever receives the game do.
- **Fallback.** The pooled lines stay. A model line that is late, empty,
  too long or off the facts is dropped and the pooled line plays.

## Candidate models

Checked on Hugging Face on 2026-10-26 (model cards, GGUF listings and
licence files). Sizes are the GGUF files the game would ship.

| Model | Params | Licence | Q4 file | Q8 file | Notes |
|---|---|---|---|---|---|
| **Qwen3-0.6B** (Apr 2025) | 0.6B | Apache-2.0 | 397 MB (Q4_K_M) | 639 MB | Thinking turned off per request (`enable_thinking=false`). 32k context. |
| **Qwen3.5-0.8B** (Feb 2026) | 0.8B | Apache-2.0 | 563 MB (Q4_0) | 834 MB | Non-thinking by default. A vision-language model; text works without the image projector, and the ggml-org GGUF ships none. Newest Qwen at this size. |
| Qwen3-1.7B | 1.7B | Apache-2.0 | 1.11 GB | 1.83 GB | The upper bound: the smallest that writes noticeably better, but twice the cost. |
| Qwen2.5-0.5B-Instruct (Sep 2024) | 0.5B | Apache-2.0 | 491 MB | 676 MB | The previous generation; no thinking to turn off. |
| SmolLM2-360M-Instruct (Feb 2025) | 360M | Apache-2.0 | – | 386 MB | Llama architecture, the lightest Apache model here. |
| Granite 4.0 Nano 350M / 1B (Oct 2025) | 350M / 1.6B | Apache-2.0 | n/a | n/a | Hybrid Mamba-2 architecture; not benchmarked here. |
| LFM2-350M / 700M / 1.2B, LFM2.5-350M / 1.2B (2025, 2026) | 0.35B to 1.2B | **LFM Open License v1.0** | 229 MB to 731 MB | 379 MB to 1.25 GB | Built for CPUs and quick. The licence allows commercial use only by entities under US$10M annual revenue and must travel with the weights (ss. 4, 5). Fine for Orinks today; a condition the game would pass to anyone who redistributes it. |
| Gemma 3 270M / 1B | 0.27B / 1B | Gemma Terms of Use | – | – | Every distribution must carry a notice and bind recipients to Google's Prohibited Use Policy (s. 3.1); Google may terminate and require deletion. Not compatible with an MIT game that ships weights. Gemma 4 is Apache-2.0 but its smallest model is 5B. |
| Llama 3.2 1B | 1.2B | Llama 3.2 Community License | – | – | "Built with Llama" branding, Meta's notice and acceptable-use policy flow down. Not worth it at this size. |
| Phi-4-mini | 3.8B | MIT | – | ~2.5 GB | Too large for the budget. |

Sources: the Hugging Face cards and `/tree/main` listings of
`Qwen/Qwen3-0.6B`, `unsloth/Qwen3-0.6B-GGUF`, `ggml-org/Qwen3.5-0.8B-GGUF`,
`Qwen/Qwen3.5-0.8B`, `Qwen/Qwen2.5-0.5B-Instruct-GGUF`,
`HuggingFaceTB/SmolLM2-360M-Instruct-GGUF`, `LiquidAI/LFM2-*-GGUF`,
`LiquidAI/LFM2.5-*-GGUF` (and their `LICENSE`), `google/gemma-3-270m-it`,
`google/gemma-4-E2B-it`, `meta-llama/Llama-3.2-1B-Instruct`,
`ibm-granite/granite-4.0-h-350m`, `microsoft/Phi-4-mini-instruct`, and
https://ai.google.dev/gemma/terms.

## Candidate runtimes

| Runtime | Licence | Build | Fit |
|---|---|---|---|
| **`llama-cpp-2`** 0.1.160 (utilityai/llama-cpp-rs) | MIT or Apache-2.0; llama.cpp itself MIT | Vendors llama.cpp's source and builds it with CMake from `build.rs`, like `prism-sys` does today. Features for OpenMP, Vulkan, Metal, CUDA; CPU only needs none. | **Preferred.** The fastest CPU inference there is for GGUF, every model above runs, and Hearken already carries a CMake build. Costs: a C++ build of a few minutes per platform, "very close to raw bindings" with no semver, and open Windows build issues (#970, #940) the spike's `rust` job tests. Build the chat prompt by hand (ChatML): its `apply_chat_template` is reported wrong for many models (#1096). Sample without the grammar sampler, which has crash reports (#1082, #1007); filter the output in Rust instead. |
| `candle` 0.11 (candle-transformers) | MIT or Apache-2.0 | Pure Rust, no C++ toolchain on the CPU path. Has `quantized_qwen3`, `quantized_qwen2`, `quantized_llama` (SmolLM2) and `quantized_lfm2`. | **The fallback** if the binding will not build somewhere: no CMake, but slower on CPUs than llama.cpp and a heavier Rust compile (gemm, tokenizers, rayon). |
| `mistral.rs` 0.8 | MIT | Built on candle; pulls in tokio, reqwest, image, clap. | A server, not a library for a game loop. No. |
| `ort` (already in hearken-voice) | MIT or Apache-2.0 | ONNX Runtime | No Rust bindings for ONNX Runtime GenAI, and small decoder models exported to ONNX run slower than GGUF on llama.cpp. No. |
| llama.cpp's own `llama-server` as a child process | MIT | Prebuilt binaries per platform (release b11912 ships Windows, Linux and macOS CPU builds with `llama-server`, `llama-cli`, `llama-bench`). | Zero build cost and what the spike's `bench` job uses to measure. For the game it means shipping a second executable and talking HTTP to it; workable, but the binding keeps one process and one failure mode. |

## The spike

`spikes/llm-commentary/` on this branch, run by
`.github/workflows/llm-spike.yml` on GitHub's hosted runners (4 vCPUs on
Windows and Linux, Apple silicon on macOS). Two jobs:

- **bench**: llama.cpp b11912's prebuilt `llama-server` on the CPU, each
  candidate GGUF, six game situations (a 12 to 0 run, a late lead change,
  a halftime deficit, a tied late steal, a comeback win, a blowout loss)
  twice each with the same system prompt: "You are Marcus 'Big M' Cole …
  Write exactly one spoken sentence of at most 20 words. Use only the
  facts given." Linux also runs the small models on two threads, as a
  weak laptop would.
- **rust**: a minimal `llama-cpp-2` crate (outside the workspace) built in
  release mode on each OS, speaking the same lines through Qwen3-0.6B
  Q4_K_M, so the build time and the per-line cost from Rust are known.

RESULTS

## How it would fit the game

The voice service already has the shape this needs: a worker thread, a
queue with staleness, priming ahead of time, a fake for tests, and the
pooled line as the fallback. The model is a second worker in front of it.

```
match event ──► Booth::hear ──► Dana's pooled line ──► Voice (primed, instant)
                   │
                   └─► colour cue + facts ──► Writer (model, ~1 s) ──► filter ──► Voice ──► Big M
                                                   │ late / empty / off the facts
                                                   └─► Big M's pooled line (already primed)
```

- **A `hearken-writer` crate** (optional, behind a `writer` feature on the
  facade like `voice`): a `Writer<G>` with `write(brief) -> Ticket`,
  `poll(ticket) -> Option<String>`, a worker thread that owns the model,
  a `FakeWriter` that returns a canned line at once for `Headless` tests,
  and the same install path as the voices (a pinned SHA-256 download of
  one GGUF into the data folder, a Settings row with the size, a Confirm
  screen). The model loads once per game and stays resident.
- **A `Brief` is facts, not prose.** The game renders what it knows (the
  teams, the quarter and possessions left, the score, the run, the
  deepest deficit, what Dana just said) into a few lines of text, and a
  fixed system prompt carries Big M's persona and the one-sentence rule.
  The system prompt's tokens are processed once and kept in the KV cache,
  so each line costs only its facts plus the generation.
- **Filter in Rust, not in the model.** Accept a line only if it is one
  sentence of 4 to 25 words, ends in punctuation, has no digits that are
  not in the brief, no markdown, quotes or emoji, and no banned words.
  Anything else, or anything past the deadline, falls through to the
  pooled line the booth had already primed. The pooled lines also seed
  the prompt with two or three examples, so the model writes in the
  booth's register.
- **Where the model writes, in order of value for its cost:**
  1. **Breaks and the buzzer** (`color.break_*`, `color.final_*`, the
     halftime observation): the game is paused, Dana's break line plays
     first (3 to 5 s), and the recap is the line that most wants to be
     about *this* game. Write it when the break event lands; the latency
     is hidden behind Dana.
  2. **The shows** (Center Court, The Halftime Report, Postgame Wrap):
     segments are primed on entering the screen today; a written
     segment is primed the same way, a few seconds earlier.
  3. **In-game colour** (`color.run_*`, `color.first_lead`,
     `color.lead_lost`, `color.comeback`, `color.double_digits_*`): Big M
     speaks after Dana's primed call, so the writer has Dana's 2 to 3 s
     plus the colour line's own 4 s staleness window. Fits on a four-core
     machine with a 0.6B model; on a weaker one the booth notices the
     writer missing its deadline and stops asking during play, keeping
     the model for breaks. `Booth::upcoming` can also pre-write the one
     or two cues that are likely next (the run continuing, the lead
     changing) at possession start, exactly as it primes the voice.
- **Settings:** "Fresh commentary: on / breaks only / off", with the
  download row. Default to breaks only once installed; a player on a slow
  machine loses nothing, because the pooled lines are always there.
- **Determinism and tests:** `FakeWriter` in headless tests; the seeded
  rotation stays for the pooled lines, so the golden tests hold. A
  `writer` CI job like `voice` builds the binding, installs the pinned
  model and asserts a written line passes the filter on the real model.

## Risks

- **Build.** `llama-cpp-2` has open Windows issues; the spike's `rust`
  job is the evidence. If it fails, candle's `quantized_qwen3` is the
  pure-Rust fallback at some CPU speed cost.
- **Quality at 0.6B.** Small models repeat the facts back, hallucinate a
  player name, or add a second sentence. The filter and the examples in
  the prompt handle most of it; the fallback handles the rest. The spike
  report's lines show what to expect.
- **Disk and download.** 400 to 600 MB more than the voices, so the
  first-run download is the realistic path (the release zip would grow
  from about 100 MB to about 600 MB).
- **Thermal and battery.** A model generating during play keeps one or
  two cores busy; "breaks only" is the default for that reason.

## Recommendation

RECOMMENDATION

## Proposed execution tasks (Opus 5.5, medium)

TASKS
