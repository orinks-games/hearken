// Rooms against Pebbles' real WebAssembly module (two seats, built by
// `cargo build -p hearken-replay --example pebbles --target wasm32-unknown-unknown --release`).
// Skipped when the module is not built, unless HEARKEN_REQUIRE_WASM is set.

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, resolve } from "node:path";
import {
  cleanText,
  createService,
  loadRules,
  memoryAccounts,
  memoryLimiter,
  memoryRooms,
  memoryRuns,
  normalizeRoomCode,
  publishedRules,
  transition,
} from "../src/index.js";

const root = fileURLToPath(new URL("../..", import.meta.url));
const target = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(root, "target");
const wasmPath = join(target, "wasm32-unknown-unknown/release/examples/pebbles.wasm");
const fixturePath = join(target, "hearken-replay-fixtures/pebbles.json");
const ready = existsSync(wasmPath) && existsSync(fixturePath);
if (!ready && process.env.HEARKEN_REQUIRE_WASM) {
  throw new Error(`build the Pebbles module and fixtures first (${wasmPath})`);
}
const skip = ready ? false : "the Pebbles module or fixtures are not built";
const wasm = ready ? readFileSync(wasmPath) : null;
const fixture = ready ? JSON.parse(readFileSync(fixturePath, "utf8")) : null;

const base = "https://example.test/api/games/pebbles";

function pebblesHandler(extra = {}) {
  const results = [];
  return {
    results,
    handler: {
      rules: "pebbles",
      seats: { min: 2, max: 2 },
      progress: true,
      start: ({ context }) => ({ seed: "7", context: { pile: context?.pile ?? 10 } }),
      accept: ({ verified, players }) => {
        results.push({ verified, players });
        return { winner: players.find((p) => p.seat === verified.outcome.winner)?.name ?? null };
      },
      ...extra,
    },
  };
}

async function setup(handlerExtra = {}, serviceExtra = {}) {
  const clock = { t: 1_700_000_000_000 };
  const { handler, results } = pebblesHandler(handlerExtra);
  const accounts = memoryAccounts();
  const service = createService({
    rules: publishedRules([{ bytes: wasm }]),
    accounts,
    runs: memoryRuns(),
    rooms: memoryRooms(),
    limiter: memoryLimiter(),
    handlers: { match: handler },
    basePath: "/api/games/pebbles",
    log: () => {},
    now: () => clock.t,
    ...serviceExtra,
  });
  const ada = await accounts.tokenFor("Ada");
  const bo = await accounts.tokenFor("Bo");
  return { service, accounts, results, clock, ada, bo };
}

async function call(service, method, path, token, body) {
  const res = await service.fetch(
    new Request(`${base}/${path}`, {
      method,
      headers: {
        ...(token ? { authorization: `Bearer ${token}` } : {}),
        ...(body !== undefined ? { "content-type": "application/json" } : {}),
      },
      body: body !== undefined ? JSON.stringify(body) : undefined,
    }),
  );
  return { status: res.status, body: await res.json() };
}

async function match(service, ada, bo, context = { pile: 10 }) {
  const made = await call(service, "POST", "rooms?kind=match", ada, { context });
  assert.equal(made.status, 200, JSON.stringify(made.body));
  const id = made.body.room.id;
  const joined = await call(service, "POST", "rooms/join", bo, { code: made.body.room.code.toLowerCase() });
  assert.equal(joined.status, 200, JSON.stringify(joined.body));
  return { id, code: made.body.room.code, made, joined };
}

test("the phase machine refuses a jump it does not have", () => {
  const room = { phase: "waiting" };
  transition(room, "playing", 1);
  assert.equal(room.phase, "playing");
  assert.throws(() => transition(room, "waiting", 2), /cannot go from playing to waiting/);
  transition(room, "finished", 3);
  assert.throws(() => transition(room, "playing", 4), /cannot go/);
});

test("room codes read any case and chat lines are cleaned", () => {
  assert.equal(normalizeRoomCode(" m4k7-t8 "), "M4K7T8");
  assert.equal(normalizeRoomCode("M4K7T"), "");
  assert.equal(normalizeRoomCode("O00000"), "");
  assert.equal(cleanText("  hi​ there\u0007\n friend "), "hi there friend");
  assert.equal(cleanText("x".repeat(300)).length, 200);
});

test("a match fills, starts on its own and is replayed after every turn", { skip }, async () => {
  const { service, results, ada, bo } = await setup();
  const { id, made, joined } = await match(service, ada, bo);
  assert.equal(made.body.room.phase, "waiting");
  assert.equal(made.body.seat, 0);
  assert.equal(joined.body.seat, 1);
  assert.equal(joined.body.room.phase, "playing");
  assert.equal(joined.body.room.seed, "7");
  assert.deepEqual(joined.body.room.context, { pile: 10 });
  assert.equal(joined.body.room.snapshot.outcome.next_seat, 0);

  // Bo tries to move first: ordered and stored, ignored by the rules.
  const early = await call(service, "POST", `rooms/${id}/events`, bo, { type: "turn", inputs: [[1, 3]] });
  assert.equal(early.status, 200);
  assert.equal(early.body.room.snapshot.outcome.pile, 10);
  assert.equal(early.body.room.snapshot.outcome.next_seat, 0);

  for (const [token, take] of [[ada, 3], [bo, 3], [ada, 3]]) {
    const res = await call(service, "POST", `rooms/${id}/events`, token, { type: "turn", inputs: [[1, take]] });
    assert.equal(res.status, 200, JSON.stringify(res.body));
    assert.equal(res.body.room.phase, "playing");
  }
  const last = await call(service, "POST", `rooms/${id}/events`, bo, { type: "turn", inputs: [[1, 1]] });
  assert.equal(last.body.room.phase, "finished");
  assert.equal(last.body.room.snapshot.outcome.winner, 0);
  assert.deepEqual(last.body.room.result, { winner: "Ada" });
  assert.equal(results.length, 1);
  assert.equal(results[0].verified.rules, "pebbles");
  assert.equal(results[0].verified.ticks, 5);
  assert.deepEqual(results[0].verified.outcome, fixture.outcome);

  // The log holds everything in order, with the seat on each turn.
  const read = await call(service, "GET", `rooms/${id}?after=0`, ada);
  assert.equal(read.status, 200);
  const types = read.body.events.map((e) => e.type);
  assert.deepEqual(types, ["join", "start", "turn", "turn", "turn", "turn", "turn"]);
  assert.deepEqual(read.body.events.slice(2).map((e) => e.seat), [1, 0, 1, 0, 1]);
  assert.ok(read.body.events.every((e) => !("player" in e)));

  // Nothing more can happen in a finished room, and the client is told why.
  const late = await call(service, "POST", `rooms/${id}/events`, ada, { type: "turn", inputs: [[1, 1]] });
  assert.equal(late.status, 409);
  assert.deepEqual(late.body, { error: "wrong-phase", phase: "finished" });
});

test("a client reads from where it left off, and sees who is here", { skip }, async () => {
  const { service, clock, ada, bo } = await setup();
  const { id } = await match(service, ada, bo);
  const first = await call(service, "GET", `rooms/${id}?after=0`, ada);
  assert.equal(first.body.events.length, 2);
  // Bo reads the room once, then goes away before Ada moves.
  await call(service, "GET", `rooms/${id}?after=0`, bo);
  await call(service, "POST", `rooms/${id}/events`, ada, { type: "turn", inputs: [[1, 2]] });
  const next = await call(service, "GET", `rooms/${id}?after=${first.body.room.seq}`, ada);
  assert.equal(next.body.events.length, 1);
  assert.equal(next.body.events[0].type, "turn");
  assert.deepEqual(next.body.events[0].inputs, [[1, 2]]);
  assert.equal(next.body.room.members[0].here, true);
  // Bo has not polled for a minute: away, not left.
  clock.t += 60_000;
  const later = await call(service, "GET", `rooms/${id}?after=${next.body.room.seq}`, ada);
  assert.deepEqual(
    later.body.room.members.map((m) => [m.name, m.here, m.left]),
    [["Ada", true, false], ["Bo", false, false]],
  );
  // My games lists the room with what Bo has not read.
  const list = await call(service, "GET", "rooms", bo);
  assert.equal(list.body.rooms.length, 1);
  assert.equal(list.body.rooms[0].unread, 1);
  assert.equal(list.body.rooms[0].snapshot.outcome.next_seat, 1);
});

test("chat rides in the same log and is cleaned and screened", { skip }, async () => {
  const { service, ada, bo } = await setup({}, {
    screen: ({ text }) => (text.includes("rude") ? "Keep it kind." : undefined),
  });
  const { id } = await match(service, ada, bo);
  const said = await call(service, "POST", `rooms/${id}/events`, bo, { type: "chat", text: " good\u0000 luck " });
  assert.equal(said.status, 200);
  const refused = await call(service, "POST", `rooms/${id}/events`, bo, { type: "chat", text: "rude" });
  assert.equal(refused.status, 422);
  assert.deepEqual(refused.body, { error: "not-allowed", message: "Keep it kind." });
  const empty = await call(service, "POST", `rooms/${id}/events`, bo, { type: "chat", text: "​" });
  assert.equal(empty.status, 400);
  const read = await call(service, "GET", `rooms/${id}?after=2`, ada);
  assert.deepEqual(read.body.events.map((e) => [e.type, e.seat, e.name, e.text]), [["chat", 1, "Bo", "good luck"]]);
});

test("leaving a waiting room frees the seat; leaving a match forfeits it", { skip }, async () => {
  const { service, ada, bo, results } = await setup();
  const made = await call(service, "POST", "rooms?kind=match", ada, {});
  const id = made.body.room.id;
  const gone = await call(service, "POST", `rooms/${id}/events`, ada, { type: "leave" });
  assert.equal(gone.body.room.phase, "closed");
  // The code is free again, and no longer joins.
  const stale = await call(service, "POST", "rooms/join", bo, { code: made.body.room.code });
  assert.equal(stale.status, 404);

  const live = await match(service, ada, bo);
  await call(service, "POST", `rooms/${live.id}/events`, ada, { type: "turn", inputs: [[1, 1]] });
  const left = await call(service, "POST", `rooms/${live.id}/events`, bo, { type: "leave" });
  assert.equal(left.body.room.phase, "finished");
  assert.equal(left.body.room.snapshot.outcome.winner, 0);
  assert.deepEqual(left.body.room.members.map((m) => m.left), [false, true]);
  assert.equal(results.length, 1);
  // Leaving again is harmless.
  const again = await call(service, "POST", `rooms/${live.id}/events`, bo, { type: "leave" });
  assert.equal(again.status, 200);
});

test("play with anyone joins the open room that is waiting, or makes one", { skip }, async () => {
  const { service, ada, bo } = await setup();
  const first = await call(service, "POST", "rooms/find?kind=match", ada);
  assert.equal(first.status, 200, JSON.stringify(first.body));
  assert.equal(first.body.room.phase, "waiting");
  assert.equal(first.body.room.open, true);
  // The same player asking again lands in their own room, not a second one.
  const same = await call(service, "POST", "rooms/find?kind=match", ada);
  assert.notEqual(same.body.room.id, first.body.room.id);
  const second = await call(service, "POST", "rooms/find?kind=match", bo);
  assert.equal(second.body.room.id, first.body.room.id);
  assert.equal(second.body.room.phase, "playing");
});

test("a seat that holds its turn too long forfeits", { skip }, async () => {
  const { service, clock, ada, bo } = await setup({ turn_limit: 3 * 86_400 });
  const { id } = await match(service, ada, bo);
  await call(service, "POST", `rooms/${id}/events`, ada, { type: "turn", inputs: [[1, 1]] });
  clock.t += 2 * 86_400_000;
  let read = await call(service, "GET", `rooms/${id}`, ada);
  assert.equal(read.body.room.phase, "playing");
  clock.t += 2 * 86_400_000;
  read = await call(service, "GET", `rooms/${id}`, ada);
  assert.equal(read.body.room.phase, "finished");
  assert.equal(read.body.room.snapshot.outcome.winner, 0);
  const log = await call(service, "GET", `rooms/${id}?after=0`, ada);
  const left = log.body.events.find((e) => e.type === "left");
  assert.equal(left.seat, 1);
  assert.equal(left.reason, "turn-limit");
});

test("a room nobody touches is closed or abandoned", { skip }, async () => {
  const { service, clock, ada, bo } = await setup({ idle_limit: 60 });
  const made = await call(service, "POST", "rooms?kind=match", ada, {});
  clock.t += 61_000;
  const list = await call(service, "GET", "rooms", ada);
  assert.equal(list.body.rooms[0].phase, "closed");
  const { id } = await match(service, ada, bo);
  clock.t += 61_000;
  const read = await call(service, "GET", `rooms/${id}`, bo);
  assert.equal(read.body.room.phase, "abandoned");
  assert.equal((await call(service, "POST", `rooms/${made.body.room.id}/events`, ada, { type: "chat", text: "hi" })).status, 409);
});

test("the host starts with enough players, and only the host", { skip }, async () => {
  const { service, accounts, ada, bo } = await setup({ seats: { min: 2, max: 3 } });
  const made = await call(service, "POST", "rooms?kind=match", ada, {});
  const id = made.body.room.id;
  const short = await call(service, "POST", `rooms/${id}/start`, ada);
  assert.equal(short.status, 422);
  assert.equal(short.body.error, "not-enough-players");
  await call(service, "POST", "rooms/join", bo, { code: made.body.room.code });
  const notHost = await call(service, "POST", `rooms/${id}/start`, bo);
  assert.equal(notHost.status, 403);
  const started = await call(service, "POST", `rooms/${id}/start`, ada);
  assert.equal(started.body.room.phase, "playing");
  // A third player can no longer join.
  const cy = await accounts.tokenFor("Cy");
  const late = await call(service, "POST", "rooms/join", cy, { code: made.body.room.code });
  assert.equal(late.status, 409);
  assert.equal(late.body.error, "started");
  // Someone who is not in the room cannot read it.
  assert.equal((await call(service, "GET", `rooms/${id}`, cy)).status, 403);
  assert.equal((await call(service, "GET", `rooms/${id}`, null)).status, 401);
});

test("bad turns are refused before they are stored", { skip }, async () => {
  const { service, ada, bo } = await setup();
  const { id } = await match(service, ada, bo);
  for (const inputs of [[[4095, 1]], [[1, 1.5]], [[1]], "x", [[-1, 0]]]) {
    const res = await call(service, "POST", `rooms/${id}/events`, ada, { type: "turn", inputs });
    assert.equal(res.status, 400, JSON.stringify(inputs));
  }
  const read = await call(service, "GET", `rooms/${id}?after=2`, ada);
  assert.equal(read.body.events.length, 0);
});

test("a kind without rooms gets no room", { skip }, async () => {
  const { service, ada } = await setup();
  assert.equal((await call(service, "POST", "rooms?kind=other", ada, {})).status, 404);
  assert.throws(
    () =>
      createService({
        rules: publishedRules([]),
        accounts: memoryAccounts(),
        runs: memoryRuns(),
        rooms: memoryRooms(),
        limiter: memoryLimiter(),
        handlers: { duel: { seats: { min: 2 }, start: () => ({}), accept: () => null } },
      }),
    /needs rules/,
  );
});

test("the module itself plays a match from turns", { skip }, async () => {
  const rules = await loadRules(wasm);
  assert.equal(rules.abi, 2);
  assert.equal(rules.plays, true);
  const answer = rules.play({ rules: "pebbles", seed: fixture.seed, context: fixture.context, turns: fixture.turns, finish: true });
  assert.equal(answer.ok, true, JSON.stringify(answer));
  assert.deepEqual(answer.outcome, fixture.outcome);
  const partial = rules.play({ rules: "pebbles", seed: fixture.seed, context: fixture.context, turns: fixture.turns.slice(0, 1) });
  assert.equal(partial.finished, false);
  assert.equal(partial.outcome.next_seat, 1);
});
