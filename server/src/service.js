// The service a Hearken game talks to, as one web-standard
// `fetch(Request) -> Response` function. Mount it under the game's base URL
// on any host that speaks Request and Response: a Vercel or Next.js route,
// Netlify, a Cloudflare Worker, Deno, Bun, or Node through `./node.js`.
//
// Routes, relative to the base URL:
//   POST start?kind=<kind>    what to play: the handler's seed and context
//   POST runs?kind=<kind>     a run file; replies with the handler's result
//   POST activate/start       begins connecting an account
//   POST activate/poll        { device_code }: whether the player approved it
//   GET  <kind>/<path...>     a handler's own read route, when it has one
//
// docs/online.md in the Hearken repository is the protocol in full.

import { BUILD_OFFSET, buildOf, MAX_RUN_BYTES, RulesError, sha256Hex } from "./rules.js";

export const KIND_PATTERN = /^[a-z0-9][a-z0-9-]{0,31}$/;

/** Thrown by a handler to refuse a run: becomes 422 { error: code, message }. */
export class Refusal extends Error {
  constructor(code, message) {
    super(message ?? code);
    this.code = code;
  }
}

export function refuse(code, message) {
  return new Refusal(code, message);
}

function json(body, status = 200, headers = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store", ...headers },
  });
}

function error(status, code, message, headers) {
  return json(message ? { error: code, message } : { error: code }, status, headers);
}

function bearer(request) {
  const header = request.headers.get("authorization") ?? "";
  const match = /^Bearer ([A-Za-z0-9._~+/=-]{16,512})$/.exec(header);
  return match ? match[1] : null;
}

/** Reads at most `limit` bytes of the body; null when it is longer. */
async function readBody(request, limit) {
  const length = Number(request.headers.get("content-length") ?? "0");
  if (length > limit) return null;
  if (!request.body) return new Uint8Array(0);
  const reader = request.body.getReader();
  const chunks = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.length;
    if (total > limit) {
      await reader.cancel().catch(() => {});
      return null;
    }
    chunks.push(value);
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}

async function readJson(request, limit = 4096) {
  const body = await readBody(request, limit);
  if (body === null) return undefined;
  if (body.length === 0) return {};
  try {
    return JSON.parse(new TextDecoder().decode(body));
  } catch {
    return undefined;
  }
}

/**
 * Makes the service for one game.
 *
 * @param {object} options
 * @param {{ get(build: string): Promise<import("./rules.js").Rules | null> }} options.rules
 *   The published rules modules, by build ID (see `publishedRules`).
 * @param {object} options.accounts  Players and their tokens (see `memoryAccounts`).
 * @param {object} options.runs      Where run IDs are claimed (see `memoryRuns`).
 * @param {object} options.limiter   Rate limits (see `memoryLimiter`).
 * @param {Record<string, object>} options.handlers  The game's handler for each kind.
 * @param {string} [options.basePath]  The path the service is mounted at.
 * @param {(rules, run, expected) => Promise<object>} [options.replay]
 *   Runs a verification; the default runs it inline. Node can pass
 *   `workerReplay()` from `./node.js` for a hard time limit.
 * @param {(request: Request) => string} [options.clientKey]
 *   Who is asking, for the activation rate limit; the first
 *   X-Forwarded-For address by default.
 * @param {object} [options.limits]
 */
export function createService(options) {
  const {
    rules,
    accounts,
    runs,
    limiter,
    handlers,
    basePath = "",
    replay = async (module, run, expected) => module.verify(run, expected),
    // The last X-Forwarded-For address is the one the nearest proxy saw;
    // earlier ones are whatever the client sent. Hosts that name the client
    // another way (Cloudflare's CF-Connecting-IP) should pass their own.
    clientKey = (request) =>
      (request.headers.get("x-forwarded-for") ?? "").split(",").pop().trim() || "unknown",
    log = (event) => console.warn("hearken-online", event),
  } = options;
  const limits = {
    runsPerMinute: 20,
    activationsPerMinute: 10,
    pollsPerMinute: 30,
    ...options.limits,
  };
  for (const kind of Object.keys(handlers)) {
    if (!KIND_PATTERN.test(kind)) throw new Error(`"${kind}" cannot name a kind of run`);
    if (typeof handlers[kind].expect !== "function" || typeof handlers[kind].accept !== "function") {
      throw new Error(`the handler for "${kind}" needs expect() and accept()`);
    }
  }
  const base = basePath.replace(/\/+$/, "");

  async function submit(request, url) {
    const kind = url.searchParams.get("kind") ?? "";
    if (!KIND_PATTERN.test(kind) || !Object.hasOwn(handlers, kind)) {
      return error(404, "unknown-kind");
    }
    const handler = handlers[kind];
    const token = bearer(request);
    if (!token) return error(401, "signed-out");
    const player = await accounts.player(token);
    if (!player) return error(401, "signed-out");
    if (!(await limiter.allow(`runs:${player.id}`, limits.runsPerMinute, 60_000))) {
      return error(429, "rate-limited", undefined, { "retry-after": "60" });
    }
    const body = await readBody(request, MAX_RUN_BYTES);
    if (body === null) return error(413, "too-large");
    const build = buildOf(body);
    if (!build) return error(422, "not-a-run");
    const module = await rules.get(build);
    if (!module) return error(422, "unknown-build");

    const runId = await sha256Hex(body);
    // Copies are told apart by what was played, not by which published
    // module the file names: the same run stamped with another build of the
    // same rules is the same run.
    const played = body.slice();
    played.fill(0, BUILD_OFFSET, BUILD_OFFSET + 32);
    const playId = await sha256Hex(played);
    const info = module.inspect(body);
    if (!info.ok) return error(422, info.error);

    // The run is claimed before any work, so a repeat is answered from what
    // was stored and two copies sent at once cannot both be accepted. A run
    // is claimed once per kind: another player sending the same run file
    // (a copied ghost, say) is refused, unless the handler allows copies.
    const key = handler.copies === "allow" ? `${kind}:${player.id}:${playId}` : `${kind}:${playId}`;
    const claim = await runs.claim(key, { player: player.id, kind });
    if (!claim.claimed) {
      const prior = claim.existing ?? {};
      if (prior.player !== player.id) return error(409, "already-submitted");
      if (prior.reply === undefined) return error(409, "in-progress");
      return json({ ok: true, run_id: runId, repeat: true, result: prior.reply });
    }

    try {
      const expected = await handler.expect({ kind, player, run: info });
      if (!expected || typeof expected.rules !== "string") {
        throw new Error(`the handler for "${kind}" expected nothing`);
      }
      const verified = await replay(module, body, expected);
      if (!verified.ok) {
        await runs.release(key, claim);
        return error(422, verified.error);
      }
      const reply = await handler.accept({ kind, player, verified, run: info, expected });
      await runs.complete(key, claim, reply ?? null);
      return json({ ok: true, run_id: runId, result: reply ?? null });
    } catch (e) {
      await runs.release(key, claim);
      if (e instanceof Refusal || e instanceof RulesError) return error(422, e.code, e.message);
      throw e;
    }
  }

  // What a player should play for a kind, when its handler hands out seeds:
  // a seed made from a secret and revealed only when its day begins, or one
  // of the player's own, so a run another player made cannot be passed off.
  async function begin(request, url) {
    const kind = url.searchParams.get("kind") ?? "";
    if (!KIND_PATTERN.test(kind) || !Object.hasOwn(handlers, kind) || typeof handlers[kind].start !== "function") {
      return error(404, "unknown-kind");
    }
    const token = bearer(request);
    const player = token ? await accounts.player(token) : null;
    if (!player) return error(401, "signed-out");
    if (!(await limiter.allow(`start:${player.id}`, limits.runsPerMinute, 60_000))) {
      return error(429, "rate-limited", undefined, { "retry-after": "60" });
    }
    try {
      const plan = await handlers[kind].start({ kind, player });
      if (!plan || !/^(0|[1-9][0-9]{0,19})$/.test(String(plan.seed))) {
        throw new Error(`the handler for "${kind}" started no seed`);
      }
      return json({ ok: true, ...plan, seed: String(plan.seed), context: plan.context ?? null });
    } catch (e) {
      if (e instanceof Refusal) return error(422, e.code, e.message);
      throw e;
    }
  }

  async function start(request) {
    if (!(await limiter.allow(`activate:${clientKey(request)}`, limits.activationsPerMinute, 60_000))) {
      return error(429, "rate-limited", undefined, { "retry-after": "60" });
    }
    const body = await readJson(request);
    const started = await accounts.startActivation({
      clientKey: clientKey(request),
      machine: typeof body?.machine === "string" ? body.machine.slice(0, 64) : undefined,
    });
    return json(started);
  }

  async function poll(request) {
    const body = await readJson(request);
    const code = body?.device_code;
    if (typeof code !== "string" || !/^[0-9a-f]{64}$/.test(code)) {
      return error(400, "bad-request");
    }
    if (!(await limiter.allow(`poll:${code}`, limits.pollsPerMinute, 60_000))) {
      return error(429, "rate-limited", undefined, { "retry-after": "10" });
    }
    const result = await accounts.pollActivation(code);
    if (result.status === "expired") return json({ status: "expired" }, 410);
    if (result.status === "ready") {
      return json({ status: "ready", token: result.token, name: result.name ?? null });
    }
    return json({ status: "pending" });
  }

  async function read(request, kind, rest) {
    const handler = Object.hasOwn(handlers, kind) ? handlers[kind] : null;
    if (!handler || typeof handler.read !== "function") return error(404, "not-found");
    const token = bearer(request);
    const player = token ? await accounts.player(token) : null;
    const answer = await handler.read({ request, kind, path: rest, player });
    if (answer instanceof Response) return answer;
    if (answer === undefined) return error(404, "not-found");
    return json(answer);
  }

  async function route(request) {
    const url = new URL(request.url);
    let path = url.pathname;
    if (base) {
      if (path !== base && !path.startsWith(`${base}/`)) return error(404, "not-found");
      path = path.slice(base.length);
    }
    const parts = path.split("/").filter(Boolean);
    const method = request.method.toUpperCase();
    if (method === "POST" && parts.length === 1 && parts[0] === "runs") return submit(request, url);
    if (method === "POST" && parts.length === 1 && parts[0] === "start") return begin(request, url);
    if (method === "POST" && parts.length === 2 && parts[0] === "activate") {
      if (parts[1] === "start") return start(request);
      if (parts[1] === "poll") return poll(request);
    }
    if (method === "GET" && parts.length >= 1 && KIND_PATTERN.test(parts[0])) {
      return read(request, parts[0], parts.slice(1));
    }
    return error(404, "not-found");
  }

  return {
    async fetch(request) {
      try {
        return await route(request);
      } catch (e) {
        log({ error: String(e?.stack ?? e) });
        return error(503, "unavailable");
      }
    },
  };
}
