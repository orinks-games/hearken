#!/usr/bin/env python3
"""Downloads the spike's GGUF models from Hugging Face (public repos, no
account). Each entry names a repo and a pattern; the first file in the
repo's listing that matches is fetched into the output folder, skipped
if already there. A repo that cannot be listed is reported and skipped,
so one moved file does not sink the run.

Usage: fetch.py --out DIR [--only NAME,...]
"""

import argparse
import json
import os
import re
import sys
import urllib.request

# Name, repo, file pattern. Sizes are the spread the game could ship:
# a 4-bit 0.6B (about 400 MB) up to a 4-bit 1.7B (about 1.1 GB).
MODELS = [
    ("qwen3-0.6b-q4", "unsloth/Qwen3-0.6B-GGUF", r"^Qwen3-0\.6B-Q4_K_M\.gguf$"),
    ("qwen3-0.6b-q8", "Qwen/Qwen3-0.6B-GGUF", r"^Qwen3-0\.6B-Q8_0\.gguf$"),
    # ggml-org ships Qwen3.5-0.8B as Q4_0 and Q8_0 only; it is a vision-language
    # model, but text works without the image projector.
    ("qwen3.5-0.8b-q4", "ggml-org/Qwen3.5-0.8B-GGUF", r"(?i)q4_0\.gguf$"),
    ("qwen3.5-0.8b-q8", "ggml-org/Qwen3.5-0.8B-GGUF", r"(?i)q8_0\.gguf$"),
    ("qwen2.5-0.5b-q8", "Qwen/Qwen2.5-0.5B-Instruct-GGUF", r"(?i)^qwen2\.5-0\.5b-instruct-q8_0\.gguf$"),
    ("smollm2-360m-q8", "HuggingFaceTB/SmolLM2-360M-Instruct-GGUF", r"(?i)q8_0\.gguf$"),
    ("lfm2-350m-q8", "LiquidAI/LFM2-350M-GGUF", r"(?i)q8_0\.gguf$"),
    ("lfm2-700m-q4", "LiquidAI/LFM2-700M-GGUF", r"(?i)q4_k_m\.gguf$"),
    ("lfm2.5-350m-q8", "LiquidAI/LFM2.5-350M-GGUF", r"(?i)q8_0\.gguf$"),
    ("lfm2.5-1.2b-q4", "LiquidAI/LFM2.5-1.2B-Instruct-GGUF", r"(?i)q4_k_m\.gguf$"),
    ("qwen3-1.7b-q4", "unsloth/Qwen3-1.7B-GGUF", r"^Qwen3-1\.7B-Q4_K_M\.gguf$"),
    # Gemma's terms rule it out for the game (use-policy flow-down); here
    # only as a quality reference for its size.
    ("gemma3-270m-q8", "ggml-org/gemma-3-270m-it-GGUF", r"(?i)q8_0\.gguf$"),
]


def listing(repo):
    url = f"https://huggingface.co/api/models/{repo}/tree/main"
    with urllib.request.urlopen(url, timeout=60) as r:
        return [e["path"] for e in json.loads(r.read().decode()) if e.get("type") == "file"]


def download(url, dest):
    tmp = dest + ".part"
    req = urllib.request.Request(url, headers={"User-Agent": "hearken-llm-spike/1"})
    with urllib.request.urlopen(req, timeout=120) as r, open(tmp, "wb") as f:
        while True:
            chunk = r.read(1 << 20)
            if not chunk:
                break
            f.write(chunk)
    os.replace(tmp, dest)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--only", default="")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    only = {s for s in a.only.split(",") if s}
    got, missed = [], []
    for name, repo, pattern in MODELS:
        if only and name not in only:
            continue
        try:
            files = [f for f in listing(repo) if re.search(pattern, f)]
        except Exception as e:  # noqa: BLE001
            print(f"{name}: cannot list {repo}: {e}", flush=True)
            missed.append(name)
            continue
        if not files:
            print(f"{name}: nothing in {repo} matches {pattern}", flush=True)
            missed.append(name)
            continue
        # The game would ship one file; one short name per model keeps the
        # report readable.
        dest = os.path.join(a.out, f"{name}.gguf")
        if os.path.exists(dest):
            print(f"{name}: already here ({os.path.getsize(dest) // 1_000_000} MB)", flush=True)
            got.append(dest)
            continue
        url = f"https://huggingface.co/{repo}/resolve/main/{files[0]}"
        print(f"{name}: {url}", flush=True)
        try:
            download(url, dest)
            print(f"{name}: {os.path.getsize(dest) // 1_000_000} MB", flush=True)
            got.append(dest)
        except Exception as e:  # noqa: BLE001
            print(f"{name}: download failed: {e}", flush=True)
            missed.append(name)
    print(f"\n{len(got)} models ready, {len(missed)} missing: {', '.join(missed) or 'none'}")
    if not got:
        sys.exit(1)


if __name__ == "__main__":
    main()
