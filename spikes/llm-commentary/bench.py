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
import re
import os
import platform
import subprocess
import sys
import time
import urllib.error
import urllib.request

SYSTEM = """You are Marcus "Big M" Cole, the colour commentator on the radio call of a Continental Basketball League game, beside the play-by-play voice Dana Wells. After Dana's call you add ONE short remark: a warm, direct, veteran ex-player's take, sometimes addressed to Dana, sometimes to the listener's team as "you".

Rules: one spoken sentence of 6 to 20 words (two very short sentences at most). React to the angle; do not repeat Dana's call and do not read the score back. Use only the facts given. No number unless it appears in the facts. No names other than Dana and the two teams. No quotation marks, lists, emoji or speaker label.

Examples of your remarks:
Angle: your team's fifth straight basket. -> Somebody call a timeout, the Skyliners have hit five straight!
Angle: your team takes the lead for the first time. -> There it is, the first lead of the night for the Skyliners.
Angle: halftime, your team trails by eight. -> Shake it off. One good run changes everything.
Angle: the final buzzer, a win. -> I loved the effort, Dana. That one's in the books."""

# What the booth knows at each moment. The game would render `facts`
# from its state: the booth already knows the angle (a run, a lead
# change, a break), so the model is asked to react to it, not to find it.
SCENARIOS = [
    {
        "key": "run_of_five",
        "facts": "Game: Seattle Skyliners (the listener's team, you) at home against the Portland Cascades.\n"
        "Situation: third quarter, 7 possessions left in the quarter. Score: Skyliners 44, Cascades 41.\n"
        "Angle: the Skyliners have scored on five straight possessions, a 12 to 0 run.\n"
        "Dana just said: Bucket! Two points. The Skyliners lead 44 to 41.",
    },
    {
        "key": "lead_change",
        "facts": "Game: Seattle Skyliners (the listener's team, you) at home against the Portland Cascades.\n"
        "Situation: fourth quarter, 3 possessions left. Score: Skyliners 61, Cascades 60.\n"
        "Angle: the Skyliners just took the lead back after trailing by as many as 9; a comeback.\n"
        "Dana just said: Three! The Skyliners lead 61 to 60.",
    },
    {
        "key": "halftime",
        "facts": "Game: Seattle Skyliners (the listener's team, you) at home against the Portland Cascades.\n"
        "Situation: halftime. Score: Cascades 38, Skyliners 30.\n"
        "Angle: the Skyliners trail by 8 at the half; they missed 6 of their last 8 shots and were stripped twice. The Cascades led by as many as 11.\n"
        "Dana just said: That's the half. Cascades 38, Skyliners 30.",
    },
    {
        "key": "steal_late",
        "facts": "Game: Seattle Skyliners (the listener's team, you) at home against the Portland Cascades.\n"
        "Situation: fourth quarter, 1 possession left. Score: tied, Skyliners 70, Cascades 70.\n"
        "Angle: the Skyliners just stole the ball with the game tied and one possession left.\n"
        "Dana just said: Stripped! Your ball.",
    },
    {
        "key": "final_win",
        "facts": "Game: Seattle Skyliners (the listener's team, you) at home against the Portland Cascades.\n"
        "Situation: final buzzer. Final score: Skyliners 74, Cascades 70.\n"
        "Angle: a win; the Skyliners trailed by 8 at halftime and outscored the Cascades 44 to 32 in the second half.\n"
        "Dana just said: That's the game. Skyliners 74, Cascades 70.",
    },
    {
        "key": "blowout_loss",
        "facts": "Game: Seattle Skyliners (the listener's team, you) at home against the Portland Cascades.\n"
        "Situation: fourth quarter, 5 possessions left. Score: Cascades 68, Skyliners 49.\n"
        "Angle: the Cascades lead by 19, their largest lead of the night; the game is slipping away.\n"
        "Dana just said: Another one drops. Cascades 68, Skyliners 49.",
    },
]

EXAMPLE_LINES = [
    "somebody call a timeout, the skyliners have hit five straight",
    "there it is, the first lead of the night for the skyliners",
    "shake it off. one good run changes everything",
    "i loved the effort, dana. that one's in the books",
]

# Each scenario is sent this many times with the sampler seed changed, so
# the report shows how varied the wording is.
VARIANTS = 3


def judge(text, facts):
    """Would the game's filter let this line through? Returns the reason
    it would not, or None."""
    t = text.strip().replace("\u2019", "'").replace("\u2014", ", ")
    if not t:
        return "empty"
    if "\n" in t:
        return "several lines"
    if re.search(r'["\u201c\u201d*#`\[\]]', t):
        return "quotes or markup"
    if re.match(r"(?i)^(marcus|big m|cole|dana|m:|big m cole)\b[^a-z]*:", t):
        return "speaker label"
    if any(ord(c) > 0x2FFF for c in t):
        return "emoji"
    words = re.findall(r"[A-Za-z0-9']+", t)
    if not 4 <= len(words) <= 25:
        return f"{len(words)} words"
    if t[-1] not in ".!?":
        return "cut off"
    if len(re.findall(r"[.!?](\s|$)", t)) > 2:
        return "more than two sentences"
    if re.search(r"(?i)dana (just )?said", t):
        return "echoes Dana"
    dana = re.search(r"Dana just said: (.*)$", facts)
    if dana and dana.group(1).strip().rstrip(".").lower() in t.lower():
        return "repeats Dana's call"
    nums = set(re.findall(r"\d+", t))
    if nums - set(re.findall(r"\d+", facts)):
        return "number not in facts"
    low = re.sub(r"[^a-z0-9 ]", "", t.lower())
    if any(re.sub(r"[^a-z0-9 ]", "", e) in low for e in EXAMPLE_LINES):
        return "copies an example"
    return None


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
            {"role": "user", "content": facts + "\n\nYour remark:"},
        ],
        "max_tokens": 48,
        "temperature": 0.7,
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
                r["reject"] = judge(r["text"], sc["facts"])
                results["runs"].append(r)
                log(
                    f"[{sc['key']}#{v}] {r['wall_s']:.2f} s "
                    f"(prompt {r['prompt_tokens']} tok @ {r['prompt_per_second'] or 0:.0f} t/s, "
                    f"gen {r['completion_tokens']} tok @ {r['predicted_per_second'] or 0:.1f} t/s, "
                    f"cached {r['cache_n']}) {'OK' if not r['reject'] else 'X ' + r['reject']}: {r['text']!r}"
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
    passed = sum(1 for r in runs if not r.get("reject"))
    return {
        "pass": f"{passed}/{len(runs)}",
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
    lines.append("| Model | Size | Threads | Load | Line wall mean / median / max | Gen tok/s | Prompt tok/s | Tokens/line | Lines the filter keeps |")
    lines.append("|---|---|---|---|---|---|---|---|---|")
    for res in all_results:
        s = summarize(res)
        if not s:
            lines.append(f"| {res['model']} | {res['size_mb']} MB | {res['threads']} | – | error: {res.get('error','?')} | | | | |")
            continue
        lines.append(
            f"| {res['model']} | {res['size_mb']} MB | {res['threads']} | {res.get('load_s','?')} s | "
            f"{s['wall_mean_s']} / {s['wall_median_s']} / {s['wall_max_s']} s | {s['gen_tps_mean']} | "
            f"{s['prompt_tps_mean']} | {s['completion_tokens_mean']} | {s['pass']} |"
        )
    lines.append("")
    for res in all_results:
        lines.append(f"## {res['model']}")
        lines.append("")
        for r in res.get("runs", []):
            verdict = "kept" if not r.get("reject") else f"dropped: {r['reject']}"
            lines.append(
                f"- **{r['scenario']}** ({r['wall_s']} s, {r['completion_tokens']} tok, {verdict}): {r['text']}"
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
