// A Cloudflare Worker serving one game's verified runs. Wrangler imports
// each .wasm file as a compiled WebAssembly.Module; the build ID (the
// module's SHA-256, which `hearken release` prints) is given beside it,
// since a compiled module's bytes cannot be hashed here.
//
// State lives in a KV namespace (RUNS) for claims and rate limits, and the
// accounts in whatever the site already uses. KV is eventually consistent,
// so for claims that must be atomic use a Durable Object or D1 instead;
// this example keeps to KV for brevity and says where that matters.

import { createService, publishedRules } from "@hearken/online";
import dive1 from "./rules/my-game-1.0.0-rules.wasm";
import { handlers } from "./handlers.js";
import { accountsFor } from "./accounts.js";

const rules = publishedRules([
  { module: dive1, build: "<the 64-hex rules ID hearken release printed>" },
]);

function kvRuns(kv) {
  return {
    // Not atomic on KV: two copies sent in the same instant could both be
    // accepted. A Durable Object or D1 (INSERT ... ON CONFLICT DO NOTHING)
    // makes this exact.
    async claim(key, record) {
      const existing = await kv.get(`run:${key}`, "json");
      if (existing) return { claimed: false, existing };
      const nonce = crypto.randomUUID();
      await kv.put(`run:${key}`, JSON.stringify({ ...record, nonce }), { expirationTtl: 120 });
      return { claimed: true, nonce };
    },
    async complete(key, claim, reply) {
      const row = await kv.get(`run:${key}`, "json");
      if (row?.nonce === claim.nonce) await kv.put(`run:${key}`, JSON.stringify({ ...row, reply }));
    },
    async release(key, claim) {
      const row = await kv.get(`run:${key}`, "json");
      if (row?.nonce === claim.nonce && row.reply === undefined) await kv.delete(`run:${key}`);
    },
  };
}

function kvLimiter(kv) {
  return {
    async allow(key, limit, windowMs) {
      const slot = Math.floor(Date.now() / windowMs);
      const name = `rate:${key}:${slot}`;
      const count = Number((await kv.get(name)) ?? 0) + 1;
      await kv.put(name, String(count), { expirationTtl: Math.max(60, Math.ceil(windowMs / 1000) * 2) });
      return count <= limit;
    },
  };
}

export default {
  async fetch(request, env) {
    const service = createService({
      rules,
      accounts: accountsFor(env),
      runs: kvRuns(env.RUNS),
      limiter: kvLimiter(env.RUNS),
      handlers: handlers(env),
      clientKey: (r) => r.headers.get("cf-connecting-ip") ?? "unknown",
    });
    return service.fetch(request);
  },
};
