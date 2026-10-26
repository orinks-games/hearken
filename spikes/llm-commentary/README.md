# Fresh-commentary spike

Can a small language model, running on the player's CPU with no account
or key, write Big M Cole's colour lines fresh instead of picking them
from the pools in `fastbreak-core::commentary`? This folder measures
that; the write-up and recommendation are in
`docs/superpowers/notes/2026-10-26-llm-commentary-spike.md`.

- `fetch.py` downloads the candidate GGUF models from Hugging Face.
- `bench.py` runs each through llama.cpp's `llama-server` on the CPU,
  sends six game situations (a run, a lead change, halftime, a late
  steal, the final, a blowout) twice each, and reports what the model
  wrote and how long the prompt and the generation took.
- `rust/` is a tiny crate on the `llama-cpp-2` binding that speaks the
  same lines, to show the binding builds on Windows, Linux and macOS and
  what one line costs from Rust.

`.github/workflows/llm-spike.yml` runs all of it on GitHub's hosted
runners. It is not part of CI.
