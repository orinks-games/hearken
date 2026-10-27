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

**A model fits comfortably; a model this small does not write well enough
to replace the pools, and it is not worth putting in the game before the
port ships.** Measured on GitHub's 4-vCPU Windows and Linux runners:

- **Cost is not the problem.** Qwen3-0.6B (Apache-2.0, a 397 MB file)
  writes a line in 0.3 to 0.6 s with four threads and under 1 s with two;
  Qwen3-1.7B (1.1 GB) in 0.5 to 1 s once its prompt prefix is cached, 1.5
  to 2.3 s when it is not. Loading takes about a second. Resident memory is
  roughly the file size plus 300 MB. The Rust binding (`llama-cpp-2`)
  builds llama.cpp from source on Windows, Linux and macOS in 3 to 6
  minutes and writes a correct line on all three; recomputing the whole
  prompt every time, a line costs about 1.5 s on the Windows and Linux
  runners.
- **Writing is the problem.** With a bare prompt every model under 1B read
  the score back or echoed Dana. With the angle stated and four example
  remarks in Big M's voice, the 0.6B model mostly repeated an example
  verbatim, the 0.8B model wrote fluent nonsense ("You are the Skyliners,
  the Skyliners are the Skyliners"), and only the 1.2B to 1.7B models wrote
  lines a listener would accept: "They went from trailing by nine to
  leading back", "Shake it off. The Skyliners missed 6 of their last 8
  shots and were stripped twice." Those are correct and specific, and
  still plainer than the written pools; and one in six copies an example
  or slips a fact.
- **Where it would earn its place:** the halftime and postgame recaps and
  the shows, where a line about *this* game's story matters and a second
  of latency is hidden. Not play-by-play, and not in-possession colour,
  where the pools are instant, better written and already varied.

Recommendation: ship Fastbreak with the pooled commentary (PR #9) and add a
model-written layer afterwards as an optional download, breaks and shows
only, on Qwen3-1.7B Q4_K_M through `llama-cpp-2`, with the pools as the
fallback. Tasks at the end. If Orinks wants fresh lines during play as
well, that is a 4B model and a 2.5 GB download, which is no longer
"comfortable" and is the point at which to stop.

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

### Results

Runs: round 1 (bare facts prompt, two lines per situation)
[40422318332](https://github.com/orinks-games/hearken/actions/runs/40422318332);
round 2 (angle plus four examples, three lines per situation, the filter)
[40423289770](https://github.com/orinks-games/hearken/actions/runs/40423289770).
Runners: `windows-2025` and `ubuntu-24.04` (4 vCPUs; the Linux runner
reported an AMD EPYC 9V45), `macos-15` (Apple silicon, 3 cores). A wall
time is one line end to end through `llama-server`, prompt and
generation, with the previous request's prompt prefix cached when the
server could reuse it. Round 2's Linux runner was noticeably slower than
round 1's (prompt processing a third of the speed on every model), so
compare within a table, not across.

**Speed, round 2, 4 threads.** Mean, median and worst wall time per line;
"keeps" is how many of 18 lines the game's filter would have accepted.

| Model | File | Windows mean / median / max | Windows keeps | Linux mean / median / max | Linux keeps |
|---|---|---|---|---|---|
| Qwen3-0.6B Q4_K_M | 397 MB | 0.44 / 0.38 / 0.86 s | 14 | 0.52 / 0.48 / 1.18 s | 14 |
| Qwen3-0.6B Q8_0 | 639 MB | 0.63 / 0.60 / 1.29 s | 12 | 0.55 / 0.48 / 1.07 s | 12 |
| Qwen3.5-0.8B Q4_0 | 563 MB | 0.49 / 0.39 / 1.01 s | 18 | 0.67 / 0.55 / 1.48 s | 18 |
| Qwen3.5-0.8B Q8_0 | 834 MB | 0.66 / 0.56 / 1.21 s | 16 | 0.68 / 0.52 / 1.43 s | 16 |
| **Qwen3-1.7B Q4_K_M** | 1107 MB | 0.90 / 0.73 / 1.52 s | 15 | 1.02 / 0.73 / 2.28 s | 18 |
| LFM2.5-1.2B Q4_K_M | 731 MB | 0.90 / 0.38 / 2.39 s | 18 | 1.46 / 0.43 / 4.36 s | 18 |
| LFM2.5-350M Q8_0 | 379 MB | 0.41 / 0.09 / 1.29 s | 2 | 0.51 / 0.09 / 1.61 s | 2 |
| LFM2-700M Q4_K_M | 469 MB | 0.82 / 0.76 / 1.14 s | 1 | 0.97 / 0.78 / 1.56 s | 0 |
| LFM2-350M Q8_0 | 379 MB | 0.65 / 0.60 / 1.00 s | 0 | 0.67 / 0.59 / 1.05 s | 1 |
| Qwen2.5-0.5B Q8_0 | 676 MB | 0.77 / 0.81 / 1.34 s | 3 | 0.78 / 0.82 / 1.32 s | 5 |
| SmolLM2-360M Q8_0 | 386 MB | 0.54 / 0.49 / 1.12 s | 4 | 0.54 / 0.51 / 1.26 s | 4 |
| Gemma 3 270M Q8_0 (reference) | 292 MB | 0.43 / 0.38 / 0.81 s | 10 | 0.44 / 0.45 / 0.91 s | 10 |

Generation speed with four threads, round 1 on Linux (the faster runner):
Qwen3-0.6B Q4 79 tokens/s, Qwen3.5-0.8B Q4 57, Qwen3-1.7B Q4 34,
LFM2.5-1.2B Q4 51; prompt processing 95 to 260 tokens/s. On Windows
about two thirds of that. A line is 12 to 25 tokens.

**Two threads, Linux, round 2** (a weak laptop): Qwen3-0.6B Q4 0.53 s mean
and 0.90 s worst; Qwen3.5-0.8B Q4 0.76 s and 1.52 s. The small models
barely notice losing half the cores.

**The prefix cache matters.** Within one situation the second and third
lines cost a third of the first (Qwen3-1.7B on Windows: 1.52 s, then
0.70 s and 0.64 s), because `llama-server` kept the processed system
prompt and only the new facts ran. A model whose prompt the game keeps
resident pays the full prompt (300 tokens, 1.5 to 2.3 s on 1.7B) only when
the prefix changes. LFM2's first lines are 2 to 4 s and the rest 0.3 s:
its hybrid convolution layers cannot resume a partial prefix, so a
changed prompt is a full reprocess. Qwen's attention layers resume.

**The Rust binding (rounds 2 and 3).** `llama-cpp-2` 0.1.160 with no
default features built from source in about 3.5 min on Linux and macOS and
5.5 min on Windows (cold; seconds with the cache), with CMake and the
platform compiler only. Round 3 ran Qwen3-0.6B Q4 with a fresh context per
line, so every line pays the whole 206-token prompt:

| Runner | Prompt tok/s | Generation tok/s | Per line (mean of 6) | A line it wrote |
|---|---|---|---|---|
| ubuntu-24.04, 4 threads | 188 to 195 | 55 to 56 | 1.69 s (1.3 to 1.6 s after a 3 s first line) | "The game was decided with the Skyliners winning 74-70." |
| windows-2025, 4 threads | 175 to 186 | 45 to 50 | 1.66 s (1.4 to 1.6 s) | "The Cascades led with 38, Skyliners 30." |
| macos-15, Metal, 3 threads | 131 to 149 | 4.4 to 5.8 | 6.97 s | "The Skyliners 74, Cascades 70." |

The text is real this time (round 2 printed empty strings: `encoding_rs`
writes only into the output string's spare capacity, so the binding's
`token_to_piece` bytes need `reserve` before decoding). The lines are
correct and plain, as expected from 0.6B with a short prompt. The macOS
row is the hosted runner, not a Mac; see the next paragraph.

**macOS: the hosted runner cannot time this.** Round 3's `macos-15` job
(3 cores, 7 GB, "Apple M1 (Virtual)") finished in 7.5 minutes on the four
models that matter, and every one of them generated at 3 to 7 tokens/s,
through llama.cpp's own Metal build of `llama-server` and through the Rust
binding alike, about ten times slower than the Linux runner's CPU:

| Model | Load | Line wall mean / median / max | Gen tok/s | Lines the filter keeps |
|---|---|---|---|---|
| Qwen3-0.6B Q4_K_M | 30.2 s (cold disk) | 5.37 / 4.99 / 8.91 s | 3.4 | 15/18 |
| Qwen3.5-0.8B Q4_0 | 4.2 s | 5.91 / 5.53 / 11.09 s | 3.4 | 18/18 |
| Qwen3-1.7B Q4_K_M | 9.9 s | 6.86 / 6.74 / 11.32 s | 2.7 | 16/18 |
| LFM2.5-1.2B Q4_K_M | 3.0 s | 3.06 / 2.68 / 5.53 s | 7.1 | 18/18 |

The reason is in the Rust job's log: `ggml_metal_init: found device:
Apple Paravirtual device`. The runner's GPU is a virtualised Metal device
that lacks most of what ggml's kernels need, so the model and KV cache sit
on "Metal" while the graph still splits 397 ways and crawls. These numbers
describe GitHub's macOS VM, not Apple silicon; a real M1 runs these models
on Metal faster than the Linux runner's four cores. What the job does
prove is that the build, Metal initialisation, loading and the lines all
work on macOS arm64. The first measurement on a real Mac belongs in W1,
and its CI job must not gate on macOS timings. The lines themselves
followed the Windows and Linux pattern: 0.6B copied the examples (nine of
its eighteen lines were "I love the effort, Dana. That one's in the
books."), 0.8B wrote fluent nonsense ("You're the one who knows the game
before the minutes"), 1.7B was correct but leaned on the example openers
("Shake it off" opened four lines, "Shout it out" three), and LFM2.5 read
the most naturally and said the least ("The Cascades are in a strong
position at halftime").

### What the lines read like

Round 2, the angle-led prompt. Full output is in each run's job log and
`report-*.md` artifact. Round 3 repeated the Windows and Linux passes with
the same prompt and the pattern held: Qwen3-0.6B Q4 kept 14 of 18 on both,
Qwen3.5-0.8B 18 of 18 but empty, Qwen3-1.7B 18 of 18 on Linux and 15 of
18 on Windows, LFM2.5-1.2B 18 of 18; LFM2 and SmolLM2 kept 0 to 4.

- **Qwen3-0.6B Q4** passed the filter 14 of 18 times and still failed:
  twelve of its lines were "I love the effort, Dana. That one's in the
  books", the last example with one word changed. The filter catches only
  verbatim copies. Too small to be given examples.
- **Qwen3.5-0.8B Q4** passed every time and was fluent and empty: "It is
  late for the Skyliners", "You are the Skyliners, the Skyliners are the
  Skyliners", "It's the first time the Skyliners have stolen the ball"
  (not in the facts). The best 0.8B lines were restatements.
- **Qwen3-1.7B Q4** was correct and specific: "They went from trailing by
  nine to leading back", "Shake it off. The Skyliners missed 6 of their
  last 8 shots and were stripped twice", "You've got to be kidding. The
  Skyliners are down 19 points." It leans on the examples' openers
  ("Shake it off" nine times across the two runners, three verbatim copies
  of the last example at the final buzzer) and produced one nonsense
  variant ("Shave it off"). Repetition penalties, more and rotating
  examples, and a list of the last lines said would fix most of it.
- **LFM2.5-1.2B Q4** had the best voice ("That's a big swing for the
  Skyliners, Dana", "You got back in the game with a comeback push!") and
  the worst grip on who is who: it told the Cascades to "keep it up" at
  halftime, had the Skyliners "closing the gap" after they won, and called
  a blowout "the game's turning". Its licence and prefix-cache behaviour
  count against it as well.
- **Everything under 0.6B** (SmolLM2, LFM2 350M and 700M, Qwen2.5-0.5B,
  Gemma 270M) echoed, hallucinated players and steals, or answered as a
  chatbot ("Okay, I'm ready to be your commentator!"). Out.

Against the pools: "Somebody call a timeout, the Skyliners have hit five
straight!" or "They were down 8, Dana. Not anymore!" are better lines
than anything the 1.7B model wrote, and they cost nothing. What the model
adds is a sentence the pools cannot have written in advance: the two
numbers that explain the half, the run that decided the game.

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

- **Build.** `llama-cpp-2` has open Windows issues, but the spike's
  `rust` job built and ran it on all three runners in every round, so the
  live risk is upstream churn (the crate has no semver) rather than a
  build that does not work. Pin the version; if a future one fails,
  candle's `quantized_qwen3` is the pure-Rust fallback at some CPU speed
  cost.
- **macOS speed is unmeasured.** The hosted macOS runner's GPU is a
  paravirtual Metal device and ran everything at 3 to 7 tokens/s, so the
  spike says nothing about a real Mac. Expectation, not measurement: Apple
  silicon on Metal beats the Linux runner. W1 must time it once on real
  hardware before "also during play" is offered there.
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

1. **Ship T9 as it is.** The pooled commentary with Kokoro is the
   better product today and it is done. Fresh lines are a layer on top, not
   a replacement.
2. **After T10, add a model-written layer for breaks and shows only**, as
   the optional "Fresh commentary" download: Qwen3-1.7B Q4_K_M (1.1 GB,
   Apache-2.0) through `llama-cpp-2` in a new optional `hearken-writer`
   crate, with the pooled line as the fallback for every written line and
   the pools' phrasings as the prompt's examples. Where it speaks: Big M's
   quarter-break, halftime and final-buzzer remarks, the Halftime Report's
   observation and the Postgame Wrap. Where it does not: play-by-play and
   in-possession colour.
3. **Do not pick a sub-1B model to save the download.** 0.6B and 0.8B were
   fast and not worth hearing. If 1.1 GB is too much, the right answer is
   to not do this, not to go smaller.
4. **Keep the door open for in-game colour.** The service design below
   supports it (pre-write the likely next cue at possession start, as the
   voice is primed), and a player with a strong machine could turn it on.
   It should wait until the breaks-only layer has been heard in play.

Model and runtime facts to pin: `unsloth/Qwen3-1.7B-GGUF`
`Qwen3-1.7B-Q4_K_M.gguf` (Apache-2.0, 1.11 GB, SHA-256 to be recorded at
install time as the voices are), `llama-cpp-2 = "0.1.160"`,
`default-features = false`, `n_gpu_layers` all on macOS, CPU elsewhere,
two to four threads, a 1024-token context, thinking off through the ChatML
prefill, sampling `temp 0.7, top_p 0.9, repeat penalty 1.1` plus the filter.

## Proposed execution tasks (Opus 5.5, medium)

Three tasks, after T10. Each is a PR with headless tests; the model
runs only in a CI job, as the voices do. Estimated at Opus 5.5 medium.

### W1. Hearken: `hearken-writer` crate (spike to service)

- New optional crate behind a `writer` feature on the facade, modelled on
  `hearken-voice`: `Writer<G>` with a worker thread that owns the model
  and a resident system-prompt prefix (one context, `memory_seq_rm` after
  the prefix between lines), `Brief` (persona id, examples, facts, angle,
  the last few lines said), `write(brief) -> Ticket`, `poll` from the game
  loop, a deadline per ticket, `FakeWriter` for `Headless`, the output
  filter (one or two sentences, 4 to 25 words, no quotes, label, markup
  or emoji, no digit outside the facts, no echo of the play-by-play line,
  no verbatim example), and a `reject` log target like the voice's
  transcript.
- Install path reusing the voice installer: a pinned SHA-256 download of
  one GGUF into the data folder (`writer/` beside `voices/`), a Settings
  row with the size, the Confirm screen; `find_writer` like `find_voices`.
- Build: `llama-cpp-2` with no default features, Metal on macOS, the
  `cmake` crate; document `LLAMA_BUILD_SHARED_LIBS` off and the Windows
  CRT choice. Record the cold build time per platform in the note.
- CI: a `writer` job (Linux on PRs, three OSs on dispatch) that builds the
  feature, installs the pinned model, writes six briefs and asserts every
  line passes the filter or falls back, with the lines in the job summary.
  The macOS job checks the build and the lines only; its GPU is
  paravirtual and its timings mean nothing.
- Accept: `cargo test -p hearken-writer` with `FakeWriter`; the CI job
  green on three platforms; the note updated with real-binding timings
  (prompt prefix resident, lines of 15 to 25 tokens) on each.

### W2. Fastbreak: Big M writes the breaks and the shows

- `fastbreak-core::commentary`: a `Brief` for each cue the writer may
  take (`color.break_*`, `color.final_*`, `halftime_observation`, the
  Halftime Report and Postgame Wrap segments) rendered from `Scene` plus
  what the booth already tracks (run, deepest deficit, lead changes,
  largest lead) and a per-game story log (half scores, the longest run,
  who led when), so the recap has something to say. The pooled line stays
  the `Call`; the brief rides beside it.
- The court, break screen and shows: when the writer is installed and the
  setting allows, send the brief at the moment the cue lands (breaks) or
  on entering the screen (shows), `prime` the pooled line regardless, and
  speak whichever is ready when Dana's line finishes; drop the written
  line if the pooled one has already started.
- Settings: "Fresh commentary: off / breaks and shows / also during play",
  default "breaks and shows" once installed, with the download row and the
  status sentence. Help and credits (model licence, Apache-2.0 notice).
- Accept: headless tests with `FakeWriter` showing each break and show cue
  produces one brief, the pooled line plays when the writer is slow or
  rejects, and "off" leaves the game byte-identical to T9; a CI `writer`
  job speaking three written recaps through Kokoro; an agent-server
  playtest of one full game with the layer on, the lines pasted into the
  PR for Orinks to read.

### W3. Fastbreak: in-game colour on a strong machine (optional, after W2 is heard)

- The "also during play" setting: at possession start, `Booth::upcoming`
  names the one or two colour cues that could land next (run continues,
  lead changes, double digits) and the writer pre-writes them with the
  voice primed behind; if the cue lands, the written line plays, else it
  is discarded. A rolling deadline miss counter drops the setting back to
  breaks only and says so once.
- Accept: headless tests for the speculation and the fallback; a playtest
  with CPU time per possession recorded; the note updated with the
  measured share of colour lines that were written rather than pooled.
