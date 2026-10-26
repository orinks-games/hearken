#!/usr/bin/env python3
"""Fresh-commentary spike: how fast and how well do tiny local language
models write one-line basketball colour on a CPU?

For each GGUF model given, this starts llama.cpp's `llama-server` on the
CPU, sends a fixed set of game situations through its OpenAI-style chat
endpoint (so each model's own chat template applies), and records what
the model wrote and how long the prompt and the generation took. It
writes a Markdown report and a JSON file of the raw numbers.

Usage:
  bench.py --server PATH/TO/llama-server --out DIR --threads 4 \
           MODEL.gguf [MODEL2.gguf ...]

Models are named by file. Qwen3 models get thinking turned off through
`chat_template_kwargs`; every model gets the same prompts.
"""

import argparse
import json
import os
import platform
import subprocess
import sys
import time
import urllib.error
import urllib.request

SYSTEM = (
    "You are Marcus \"Big M\" Cole, the colour commentator on a basketball "
    "radio broadcast of the Continental Basketball League. Dana Wells does "
    "the play-by-play; you add one short remark after her call. Speak like a "
    "warm, veteran ex-player. Write exactly one spoken sentence of at most "
    "20 words. Use only the facts given. Mention a number only if it is in "
    "the facts. No lists, no quotation marks, no stage directions, no emoji."
)

# What the booth knows at each moment. `facts` is what the game would
# render from its state; the model turns it into a line.
SCENARIOS = [
    {
        "key": "run_of_five",
        "facts": "Home team: Seattle Skyliners (the listener's team). Away team: Portland Cascades. "
        "Third quarter, 7 possessions left in the quarter. Score: Skyliners 44, Cascades 41. "
        "The Skyliners have just scored on five straight possessions, a 12 to 0 run. "
        "Dana just said: Bucket! Two points. The Skyliners lead 44 to 41.",
    },
    {
        "key": "lead_change",
        "facts": "Home: Seattle Skyliners (the listener's team). Away: Portland Cascades. "
        "Fourth quarter, 3 possessions left. Score: Skyliners 61, Cascades 60. "
        "The Skyliners just took the lead for the first time since the first quarter. "
        "Earlier they trailed by as many as 9. "
        "Dana just said: Three! The Skyliners lead 61 to 60.",
    },
    {
        "key": "halftime",
        "facts": "Halftime. Home: Seattle Skyliners (the listener's team). Away: Portland Cascades. "
        "Score: Cascades 38, Skyliners 30. The Cascades led by as many as 11. "
        "The Skyliners missed 6 of their last 8 shots and were stripped twice. "
        "Dana just said: That's the half. Cascades 38, Skyliners 30.",
    },
    {
        "key": "steal_late",
        "facts": "Home: Seattle Skyliners (the listener's team). Away: Portland Cascades. "
        "Fourth quarter, 1 possession left. Score: Skyliners 70, Cascades 70. "
        "The Skyliners just stole the ball. "
        "Dana just said: Stripped! Your ball.",
    },
    {
        "key": "final_win",
        "facts": "Final buzzer. Home: Seattle Skyliners (the listener's team). Away: Portland Cascades. "
        "Final score: Skyliners 74, Cascades 70. The Skyliners trailed by 8 at halftime "
        "and outscored the Cascades 44 to 32 in the second half. "
        "Dana just said: That's the game. Skyliners 74, Cascades 70.",
    },
    {
        "key": "blowout_loss",
        "facts": "Home: Seattle Skyliners (the listener's team). Away: Portland Cascades. "
        "Fourth quarter, 5 possessions left. Score: Cascades 68, Skyliners 49. "
        "The Cascades lead by 19, their largest lead of the night. "
        "Dana just said: Another one drops. Cascades 68, Skyliners 49.",
    },
]

# Each scenario is sent this many times with the sampler seed changed, so
# the report shows how varied the wording is.
VARIANTS = 2


def log(msg):
    print(msg, flush=True)


def wait_health(url, proc, timeout=180):
    start = time.time()
    while time.time() - start < timeout:
        if proc.poll() is not None:
            raise RuntimeError(f"llama-server exited early with {proc.returncode}")
        try:
            with urllib.request.urlopen(url + "/health", timeout=2) as r:
                if r.status == 200:
                    return time.time() - start
        except (urllib.error.URLError, ConnectionError, OSError):
            pass
        time.sleep(0.5)
    raise RuntimeError("llama-server did not become healthy")


def chat(url, model_name, facts, seed, no_think):
    body = {
        "model": model_name,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": "Facts: " + facts + "\n\nYour one sentence:"},
        ],
        "max_tokens": 48,
        "temperature": 0.8,
        "top_p": 0.9,
        "seed": seed,
        "stream": False,
    }
    if no_think:
        # Qwen3 and friends: no reasoning block before the answer.
        body["chat_template_kwargs"] = {"enable_thinking": False}
    data = json.dumps(body).encode()
    req = urllib.request.Request(
        url + "/v1/chat/completions", data=data, headers={"Content-Type": "application/json"}
    )
    t0 = time.perf_counter()
    with urllib.request.urlopen(req, timeout=300) as r:
        out = json.loads(r.read().decode())
    wall = time.perf_counter() - t0
    text = out["choices"][0]["message"].get("content") or ""
    timings = out.get("timings", {})
    usage = out.get("usage", {})
    return {
        "text": text.strip(),
        "wall_s": round(wall, 3),
        "prompt_tokens": usage.get("prompt_tokens") or timings.get("prompt_n"),
        "completion_tokens": usage.get("completion_tokens") or timings.get("predicted_n"),
        "prompt_ms": timings.get("prompt_ms"),
        "predicted_ms": timings.get("predicted_ms"),
        "prompt_per_second": timings.get("prompt_per_second"),
        "predicted_per_second": timings.get("predicted_per_second"),
        "cache_n": timings.get("cache_n"),
        "finish": out["choices"][0].get("finish_reason"),
    }


def bench_model(server, model_path, threads, port):
    name = os.path.basename(model_path)
    no_think = "qwen3" in name.lower()
    url = f"http://127.0.0.1:{port}"
    args = [
        server,
        "-m", model_path,
        "--host", "127.0.0.1",
        "--port", str(port),
        "-t", str(threads),
        "-tb", str(threads),
        "-c", "2048",
        "-n", "64",
        "--no-webui",
        "--log-disable",
        # Keep the processed system prompt between requests, as the game would.
        "--cache-reuse", "64",
        "--slots",
        # The model's own chat template, and no reasoning block for the
        # models that have one (Qwen3): the game wants the line, not the
        # thinking.
        "--jinja",
        "--reasoning-budget", "0",
    ]
    log(f"\n=== {name} (threads={threads}) ===")
    size_mb = round(os.path.getsize(model_path) / 1e6, 1)
    t0 = time.perf_counter()
    proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    results = {"model": name, "size_mb": size_mb, "threads": threads, "runs": []}
    try:
        load_s = wait_health(url, proc)
        results["load_s"] = round(time.perf_counter() - t0, 2)
        log(f"loaded in {results['load_s']} s ({size_mb} MB)")
        # One warm-up so the first measured line is not paying one-off costs.
        chat(url, name, SCENARIOS[0]["facts"], 1, no_think)
        for sc in SCENARIOS:
            for v in range(VARIANTS):
                r = chat(url, name, sc["facts"], 100 + v, no_think)
                r["scenario"] = sc["key"]
                r["variant"] = v
                results["runs"].append(r)
                log(
                    f"[{sc['key']}#{v}] {r['wall_s']:.2f} s "
                    f"(prompt {r['prompt_tokens']} tok @ {r['prompt_per_second'] or 0:.0f} t/s, "
                    f"gen {r['completion_tokens']} tok @ {r['predicted_per_second'] or 0:.1f} t/s, "
                    f"cached {r['cache_n']}): {r['text']!r}"
                )
    except Exception as e:  # noqa: BLE001
        results["error"] = str(e)
        log(f"ERROR: {e}")
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
    return results


def summarize(results):
    runs = [r for r in results.get("runs", []) if r.get("wall_s") is not None]
    if not runs:
        return None
    walls = sorted(r["wall_s"] for r in runs)
    gen = [r["predicted_per_second"] for r in runs if r.get("predicted_per_second")]
    pp = [r["prompt_per_second"] for r in runs if r.get("prompt_per_second")]
    toks = [r["completion_tokens"] for r in runs if r.get("completion_tokens")]
    return {
        "n": len(runs),
        "wall_mean_s": round(sum(walls) / len(walls), 2),
        "wall_median_s": round(walls[len(walls) // 2], 2),
        "wall_max_s": round(walls[-1], 2),
        "gen_tps_mean": round(sum(gen) / len(gen), 1) if gen else None,
        "prompt_tps_mean": round(sum(pp) / len(pp), 0) if pp else None,
        "completion_tokens_mean": round(sum(toks) / len(toks), 1) if toks else None,
    }


def write_report(all_results, out_dir, host):
    lines = [f"# Fresh-commentary spike: {host}", ""]
    lines.append(
        f"Platform: {platform.platform()} · CPU: {platform.processor() or platform.machine()} · "
        f"logical CPUs: {os.cpu_count()}"
    )
    lines.append("")
    lines.append("| Model | Size | Threads | Load | Line wall mean / median / max | Gen tok/s | Prompt tok/s | Tokens/line |")
    lines.append("|---|---|---|---|---|---|---|---|")
    for res in all_results:
        s = summarize(res)
        if not s:
            lines.append(f"| {res['model']} | {res['size_mb']} MB | {res['threads']} | – | error: {res.get('error','?')} | | | |")
            continue
        lines.append(
            f"| {res['model']} | {res['size_mb']} MB | {res['threads']} | {res.get('load_s','?')} s | "
            f"{s['wall_mean_s']} / {s['wall_median_s']} / {s['wall_max_s']} s | {s['gen_tps_mean']} | "
            f"{s['prompt_tps_mean']} | {s['completion_tokens_mean']} |"
        )
    lines.append("")
    for res in all_results:
        lines.append(f"## {res['model']}")
        lines.append("")
        for r in res.get("runs", []):
            lines.append(
                f"- **{r['scenario']}** ({r['wall_s']} s, {r['completion_tokens']} tok, {r['finish']}): {r['text']}"
            )
        lines.append("")
    report = "\n".join(lines)
    with open(os.path.join(out_dir, f"report-{host}.md"), "w", encoding="utf-8") as f:
        f.write(report)
    with open(os.path.join(out_dir, f"results-{host}.json"), "w", encoding="utf-8") as f:
        json.dump(all_results, f, indent=1)
    log("\n" + report)
    # GitHub's job summary, when running under Actions.
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(report + "\n")


def find_server(path):
    """`llama-server` itself, or the one inside an unpacked release folder."""
    if os.path.isfile(path):
        return path
    for root, _dirs, files in os.walk(path):
        for f in files:
            if f in ("llama-server", "llama-server.exe"):
                return os.path.join(root, f)
    raise SystemExit(f"no llama-server under {path}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", required=True, help="llama-server, or a folder that holds it")
    ap.add_argument("--out", required=True)
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--host-label", default=platform.system().lower())
    ap.add_argument("--port", type=int, default=18080)
    ap.add_argument("models", nargs="+")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    server = find_server(a.server)
    log(f"server: {server}")
    all_results = []
    for m in a.models:
        if not os.path.exists(m):
            log(f"missing model {m}, skipped")
            continue
        all_results.append(bench_model(server, m, a.threads, a.port))
    write_report(all_results, a.out, a.host_label)
    if not all_results or all("error" in r for r in all_results):
        sys.exit(1)


if __name__ == "__main__":
    main()
