//! One commentary line through llama.cpp from Rust, timed.
//!
//! `llm-commentary-spike MODEL.gguf [THREADS]` loads the model on the CPU,
//! and for each game situation builds a ChatML prompt (Qwen's format, with
//! Qwen3's thinking block left empty so the first token is the line),
//! decodes it, samples up to 48 tokens, and prints the line with the
//! prompt and generation times. A fresh context per line stands in for
//! the game's cleared booth; the real service would keep the system
//! prompt's cache across lines.

use std::num::NonZeroU32;
use std::time::Instant;

use anyhow::{Context, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;

const SYSTEM: &str = "You are Marcus \"Big M\" Cole, the colour commentator on a basketball \
radio broadcast of the Continental Basketball League. Dana Wells does the play-by-play; you \
add one short remark after her call. Speak like a warm, veteran ex-player. Write exactly one \
spoken sentence of at most 20 words. Use only the facts given. Mention a number only if it is \
in the facts. No lists, no quotation marks, no stage directions, no emoji.";

const SCENARIOS: &[(&str, &str)] = &[
    (
        "run_of_five",
        "Home team: Seattle Skyliners (the listener's team). Away team: Portland Cascades. \
Third quarter, 7 possessions left in the quarter. Score: Skyliners 44, Cascades 41. The \
Skyliners have just scored on five straight possessions, a 12 to 0 run. Dana just said: \
Bucket! Two points. The Skyliners lead 44 to 41.",
    ),
    (
        "halftime",
        "Halftime. Home: Seattle Skyliners (the listener's team). Away: Portland Cascades. \
Score: Cascades 38, Skyliners 30. The Cascades led by as many as 11. The Skyliners missed 6 \
of their last 8 shots and were stripped twice. Dana just said: That's the half. Cascades 38, \
Skyliners 30.",
    ),
    (
        "final_win",
        "Final buzzer. Home: Seattle Skyliners (the listener's team). Away: Portland Cascades. \
Final score: Skyliners 74, Cascades 70. The Skyliners trailed by 8 at halftime and outscored \
the Cascades 44 to 32 in the second half. Dana just said: That's the game. Skyliners 74, \
Cascades 70.",
    ),
];

const MAX_TOKENS: i32 = 48;

fn chatml(facts: &str) -> String {
    format!(
        "<|im_start|>system\n{SYSTEM}<|im_end|>\n<|im_start|>user\nFacts: {facts}\n\nYour one \
sentence:<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
    )
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().context("usage: llm-commentary-spike MODEL.gguf [THREADS]")?;
    let threads: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4);

    let backend = LlamaBackend::init()?;
    let t = Instant::now();
    let model = LlamaModel::load_from_file(&backend, &path, &LlamaModelParams::default())
        .with_context(|| format!("loading {path}"))?;
    println!("loaded {path} in {:.2} s, {threads} threads", t.elapsed().as_secs_f64());

    let mut totals = (0.0f64, 0.0f64, 0i32, 0i32);
    for (round, (key, facts)) in SCENARIOS.iter().cycle().take(SCENARIOS.len() * 2).enumerate() {
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(1024))
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let mut ctx = model.new_context(&backend, ctx_params)?;

        let prompt = chatml(facts);
        // The ChatML markers are special tokens; the model adds no BOS of its own.
        let tokens = model.vocab().tokenize(prompt.as_bytes(), false, true);
        let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
        let last = tokens.len() - 1;
        for (i, tok) in tokens.iter().enumerate() {
            batch.add(*tok, i as i32, &[0], i == last)?;
        }
        let t0 = Instant::now();
        ctx.decode(&mut batch)?;
        let prompt_s = t0.elapsed().as_secs_f64();

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::temp(0.8),
            LlamaSampler::dist(100 + round as u32),
        ]);
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let mut text = String::new();
        let mut n_cur = batch.n_tokens();
        let mut n_gen = 0;
        let t1 = Instant::now();
        while n_gen < MAX_TOKENS {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);
            if model.vocab().is_eog(token) {
                break;
            }
            let bytes = model.vocab().token_to_piece(token, true, None);
            let _ = decoder.decode_to_string(&bytes, &mut text, false);
            batch.clear();
            batch.add(token, n_cur, &[0], true)?;
            n_cur += 1;
            n_gen += 1;
            ctx.decode(&mut batch)?;
        }
        let gen_s = t1.elapsed().as_secs_f64();
        println!(
            "[{key}] prompt {} tok in {prompt_s:.2} s ({:.0} tok/s), gen {n_gen} tok in {gen_s:.2} s \
({:.1} tok/s), total {:.2} s: {}",
            tokens.len(),
            tokens.len() as f64 / prompt_s.max(1e-9),
            n_gen as f64 / gen_s.max(1e-9),
            prompt_s + gen_s,
            text.trim().replace('\n', " ")
        );
        totals.0 += prompt_s;
        totals.1 += gen_s;
        totals.2 += tokens.len() as i32;
        totals.3 += n_gen;
    }
    println!(
        "mean per line: {:.2} s ({:.0} prompt tok/s, {:.1} gen tok/s)",
        (totals.0 + totals.1) / (SCENARIOS.len() * 2) as f64,
        totals.2 as f64 / totals.0.max(1e-9),
        totals.3 as f64 / totals.1.max(1e-9)
    );
    Ok(())
}
