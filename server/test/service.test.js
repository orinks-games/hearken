// The server side against Coin Dash's real WebAssembly module and runs the
// Rust tests recorded. Build both first:
//
//   cargo test -p hearken-replay
//   cargo build -p hearken-replay --example coin-dash --target wasm32-unknown-unknown --release
//
// Missing either, these tests are skipped, unless HEARKEN_REQUIRE_WASM is set.

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, resolve } from "node:path";
import {
  buildOf,
  createService,
  loadRules,
  memoryAccounts,
  memoryLimiter,
  memoryRuns,
  publishedRules,
  refuse,
  sha256Hex,
} from "../src/index.js";
import { workerReplay } from "../src/node.js";

const root = fileURLToPath(new URL("../..", import.meta.url));
const target = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(root, "target");
const wasmPath = join(target, "wasm32-unknown-unknown/release/examples/coin_dash.wasm");
const fixtureDir = join(target, "hearken-replay-fixtures");
const ready = existsSync(wasmPath) && existsSync(join(fixtureDir, "fixtures.json"));
if (!ready && process.env.HEARKEN_REQUIRE_WASM) {
  throw new Error(`build the Coin Dash module and fixtures first (${wasmPath})`);
}
const skip = ready ? false : "the Coin Dash module or fixtures are not built";

const wasm = ready ? readFileSync(wasmPath) : null;
const fixtures = ready ? JSON.parse(readFileSync(join(fixtureDir, "fixtures.json"), "utf8")) : [];

/** A fixture's run file, stamped with `build`. */
function runFile(fixture, build) {
  const bytes = new Uint8Array(readFileSync(join(fixtureDir, fixture.file)));
  for (let i = 0; i < 32; i++) bytes[5 + i] = parseInt(build.slice(i * 2, i * 2 + 2), 16);
  return bytes;
}

async function setup(handlers, options = {}) {
  const rules = publishedRules([{ bytes: wasm }]);
  const build = (await rules.builds())[0];
  const accounts = memoryAccounts({ autoApprove: options.autoApprove ?? null });
  const service = createService({
    rules,
    accounts,
    runs: memoryRuns(),
    limiter: memoryLimiter(),
    handlers,
    basePath: "/api/games/coin",
    log: () => {},
    ...options.service,
  });
  return { service, accounts, build };
}

const base = "https://example.test/api/games/coin";

function submit(service, kind, token, body) {
  return service.fetch(
    new Request(`${base}/runs?kind=${kind}`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/octet-stream" },
      body,
    }),
  );
}

const practice = {
  copies: "allow",
  expect: ({ run }) => ({ rules: "coin-dash", seed: run.seed, context: run.context }),
  accept: ({ verified }) => ({ coins: verified.outcome.coins }),
};

test("the module verifies every recorded run to the score native replay gave", { skip }, async () => {
  const rules = await loadRules(wasm);
  assert.deepEqual(rules.names, ["coin-dash"]);
  for (const f of fixtures) {
    const file = runFile(f, rules.build);
    assert.equal(buildOf(file), rules.build);
    const answer = rules.verify(file, { rules: "coin-dash", seed: f.seed, context: f.context });
    assert.equal(answer.ok, true, JSON.stringify(answer));
    assert.deepEqual(answer.outcome, f.outcome);
    assert.equal(answer.run_id, await sha256Hex(file));
    assert.equal(answer.seed, f.seed);
  }
});

test("a module that imports anything is refused", { skip }, async () => {
  // (module (import "env" "now" (func)))
  const importing = new Uint8Array([
    0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 2, 11, 1, 3, 101, 110, 118, 3, 110, 111, 119, 0, 0,
  ]);
  await assert.rejects(loadRules(importing), /may not import/);
  await assert.rejects(loadRules(wasm, { build: "00".repeat(32) }), /SHA-256/);
});

test("a run is answered with what its handler made of the verified score", { skip }, async () => {
  const { service, accounts, build } = await setup({ practice });
  const token = await accounts.tokenFor("Ada");
  const f = fixtures[0];
  const res = await submit(service, "practice", token, runFile(f, build));
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.deepEqual(body.result, { coins: f.outcome.coins });
  // The same run again is answered from what was stored.
  const again = await (await submit(service, "practice", token, runFile(f, build))).json();
  assert.equal(again.repeat, true);
  assert.deepEqual(again.result, body.result);
});

test("the handler's seed wins over the run's", { skip }, async () => {
  let accepted = 0;
  const { service, accounts, build } = await setup({
    daily: {
      expect: () => ({ rules: "coin-dash", seed: "1", context: { seconds: 10 } }),
      accept: () => ++accepted,
    },
  });
  const token = await accounts.tokenFor("Ada");
  const res = await submit(service, "daily", token, runFile(fixtures[0], build));
  assert.equal(res.status, 422);
  assert.equal((await res.json()).error, "wrong-seed");
  assert.equal(accepted, 0);
});

test("a handler can refuse a run in its own words", { skip }, async () => {
  const { service, accounts, build } = await setup({
    event: {
      expect: () => {
        throw refuse("event-over", "The festival has ended.");
      },
      accept: () => null,
    },
  });
  const token = await accounts.tokenFor("Ada");
  const res = await submit(service, "event", token, runFile(fixtures[0], build));
  assert.equal(res.status, 422);
  assert.deepEqual(await res.json(), { error: "event-over", message: "The festival has ended." });
});

test("unknown builds, kinds, tokens and broken files are refused", { skip }, async () => {
  const { service, accounts, build } = await setup({ practice });
  const token = await accounts.tokenFor("Ada");
  const file = runFile(fixtures[0], build);
  assert.equal((await submit(service, "practice", "x".repeat(40), file)).status, 401);
  assert.equal((await submit(service, "nope", token, file)).status, 404);
  assert.equal((await submit(service, "__proto__", token, file)).status, 404);
  const other = runFile(fixtures[0], "11".repeat(32));
  assert.equal((await (await submit(service, "practice", token, other)).json()).error, "unknown-build");
  const cut = file.slice(0, file.length - 1);
  assert.equal((await (await submit(service, "practice", token, cut)).json()).error, "truncated");
  const big = new Uint8Array((4 << 20) + 1);
  assert.equal((await submit(service, "practice", token, big)).status, 413);
});

test("another player's copy of a run is refused unless the handler allows copies", { skip }, async () => {
  const ghost = { expect: practice.expect, accept: () => ({ kept: true }) };
  const { service, accounts, build } = await setup({ ghost, practice });
  const ada = await accounts.tokenFor("Ada");
  const bo = await accounts.tokenFor("Bo");
  const file = runFile(fixtures[1], build);
  assert.equal((await submit(service, "ghost", ada, file)).status, 200);
  const copied = await submit(service, "ghost", bo, file);
  assert.equal(copied.status, 409);
  assert.equal((await copied.json()).error, "already-submitted");
  assert.equal((await submit(service, "practice", bo, file)).status, 200);
});

test("two copies sent at once are accepted once", { skip }, async () => {
  let accepted = 0;
  const { service, accounts, build } = await setup({
    ghost: {
      expect: practice.expect,
      accept: async () => {
        accepted += 1;
        return { kept: true };
      },
    },
  });
  const ada = await accounts.tokenFor("Ada");
  const bo = await accounts.tokenFor("Bo");
  const file = runFile(fixtures[2], build);
  const answers = await Promise.all([submit(service, "ghost", ada, file), submit(service, "ghost", bo, file)]);
  assert.deepEqual(answers.map((r) => r.status).sort(), [200, 409]);
  assert.equal(accepted, 1);
});

test("a player is rate limited", { skip }, async () => {
  const { service, accounts, build } = await setup({ practice }, { service: { limits: { runsPerMinute: 2 } } });
  const token = await accounts.tokenFor("Ada");
  const file = runFile(fixtures[0], build);
  const statuses = [];
  for (let i = 0; i < 3; i++) statuses.push((await submit(service, "practice", token, file)).status);
  assert.deepEqual(statuses, [200, 200, 429]);
});

test("connecting an account hands the game a token once", { skip }, async () => {
  const { service } = await setup({ practice }, { autoApprove: "Ada" });
  const start = await (await service.fetch(new Request(`${base}/activate/start`, { method: "POST" }))).json();
  assert.match(start.user_code, /^[A-Z0-9]{4}-[A-Z0-9]{4}$/);
  const poll = () =>
    service.fetch(
      new Request(`${base}/activate/poll`, {
        method: "POST",
        body: JSON.stringify({ device_code: start.device_code }),
      }),
    );
  const ready = await (await poll()).json();
  assert.equal(ready.status, "ready");
  assert.equal(ready.name, "Ada");
  assert.equal((await poll()).status, 410);
});

test("a slow verification is stopped", { skip }, async () => {
  const rules = await loadRules(wasm);
  const replay = workerReplay({ timeoutMs: 1 });
  const answer = await replay(rules, runFile(fixtures[1], rules.build), {
    rules: "coin-dash",
    seed: fixtures[1].seed,
    context: fixtures[1].context,
  });
  // One millisecond is too short to start a worker, let alone replay a minute.
  assert.deepEqual(answer, { ok: false, error: "too-slow" });
  const roomy = await workerReplay({ timeoutMs: 10_000 })(rules, runFile(fixtures[0], rules.build), {
    rules: "coin-dash",
    seed: fixtures[0].seed,
    context: fixtures[0].context,
  });
  assert.deepEqual(roomy.outcome, fixtures[0].outcome);
});

test("start hands out the handler's seed, and the run must use it", { skip }, async () => {
  const f = fixtures[0];
  const { service, accounts, build } = await setup({
    mine: {
      start: ({ player }) => ({ seed: player.id === "p1" ? f.seed : "7", context: f.context }),
      expect: ({ player }) => ({ rules: "coin-dash", seed: player.id === "p1" ? f.seed : "7", context: f.context }),
      accept: ({ verified }) => verified.outcome,
    },
  });
  const ada = await accounts.tokenFor("Ada");
  const bo = await accounts.tokenFor("Bo");
  const start = (token) =>
    service.fetch(
      new Request(`${base}/start?kind=mine`, { method: "POST", headers: { authorization: `Bearer ${token}` } }),
    );
  assert.deepEqual(await (await start(ada)).json(), { ok: true, seed: f.seed, context: f.context });
  assert.equal((await (await start(bo)).json()).seed, "7");
  // A run on Ada's seed is no use to Bo, even sent first: it is not his.
  const copied = await submit(service, "mine", bo, runFile(f, build));
  assert.equal((await copied.json()).error, "wrong-seed");
  assert.equal((await submit(service, "mine", ada, runFile(f, build))).status, 200);
  assert.equal((await start("x".repeat(40))).status, 401);
  const none = await service.fetch(
    new Request(`${base}/start?kind=practice`, { method: "POST", headers: { authorization: `Bearer ${ada}` } }),
  );
  assert.equal(none.status, 404);
});

test("routes answer only under the base path", { skip }, async () => {
  const { service, accounts, build } = await setup({ practice });
  const token = await accounts.tokenFor("Ada");
  const file = runFile(fixtures[0], build);
  for (const url of ["https://example.test/runs?kind=practice", "https://example.test/api/games/coinruns?kind=practice"]) {
    const res = await service.fetch(
      new Request(url, { method: "POST", headers: { authorization: `Bearer ${token}` }, body: file }),
    );
    assert.equal(res.status, 404, url);
  }
});

test("the activation limit counts the address the nearest proxy saw", { skip }, async () => {
  const { service } = await setup({ practice }, { service: { limits: { activationsPerMinute: 2 } } });
  const statuses = [];
  for (let i = 0; i < 3; i++) {
    const res = await service.fetch(
      new Request(`${base}/activate/start`, {
        method: "POST",
        headers: { "x-forwarded-for": `10.0.0.${i}, 203.0.113.9` },
      }),
    );
    statuses.push(res.status);
  }
  assert.deepEqual(statuses, [200, 200, 429]);
});

test("a copy stamped with another build of the same rules is still a copy", { skip }, async () => {
  // The same module with a custom section added: another build ID, same rules.
  const name = new TextEncoder().encode("x");
  const other = new Uint8Array([...wasm, 0, name.length + 1, name.length, ...name]);
  const rules = publishedRules([{ bytes: wasm }, { bytes: other }]);
  const [first, second] = await rules.builds();
  assert.notEqual(first, second);
  const accounts = memoryAccounts();
  const service = createService({
    rules,
    accounts,
    runs: memoryRuns(),
    limiter: memoryLimiter(),
    handlers: { ghost: { expect: practice.expect, accept: () => ({ kept: true }) } },
    basePath: "/api/games/coin",
    log: () => {},
  });
  const ada = await accounts.tokenFor("Ada");
  const bo = await accounts.tokenFor("Bo");
  assert.equal((await submit(service, "ghost", ada, runFile(fixtures[0], first))).status, 200);
  const copied = await submit(service, "ghost", bo, runFile(fixtures[0], second));
  assert.equal((await copied.json()).error, "already-submitted");
});

test("a seed JavaScript has already rounded is refused", { skip }, async () => {
  const rules = await loadRules(wasm);
  assert.throws(
    () => rules.verify(runFile(fixtures[0], rules.build), { rules: "coin-dash", seed: 2 ** 60, context: {} }),
    /2\^53/,
  );
});
