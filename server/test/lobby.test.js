// The lobby (presence, invites, friends), against Pebbles' real WebAssembly module (two seats, built by
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
  inviteTransition,
  memoryPresence,
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
    handlers: {
      match: handler,
      lobby: { seats: { min: 1, max: 50 }, history: 7 * 86_400, chat_limit: { per_minute: 20, gap: 3 } },
      dm: { seats: { min: 2, max: 2 }, history: 86_400 },
    },
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


async function pulse(service, token, body = {}) {
  const res = await call(service, "POST", "rooms/pulse", token, { state: "here", ...body });
  assert.equal(res.status, 200, JSON.stringify(res.body));
  return res.body;
}

async function three() {
  const s = await setup();
  const cy = await s.accounts.tokenFor("Cy");
  return { ...s, cy };
}

test("the invite machine refuses a jump it does not have", () => {
  const invite = { state: "sent" };
  inviteTransition(invite, "heard", 1);
  assert.throws(() => inviteTransition(invite, "sent", 2), /cannot go from heard to sent/);
  inviteTransition(invite, "declined", 3);
  assert.throws(() => inviteTransition(invite, "accepted", 4), /cannot go/);
});

test("presence forgets a player once their pulses stop", () => {
  const presence = memoryPresence();
  presence.set({ id: "a", at: 0 });
  presence.set({ id: "b", at: 100_000 });
  assert.deepEqual(presence.present(110_000, 120_000).map((p) => p.id), ["a", "b"]);
  assert.deepEqual(presence.present(200_000, 120_000).map((p) => p.id), ["b"]);
  assert.equal(presence.size(), 1);
});

test("a pulse lists who is here, where they are, and goes away when they stop", async () => {
  const { service, clock, ada, bo, cy } = await three();
  const first = await pulse(service, ada, { game: "larkfield", place: "Larkfield", state: "playing" });
  assert.equal(first.you.name, "Ada");
  assert.deepEqual(first.here, []);
  assert.equal(first.every, 60);
  await pulse(service, bo);
  const seen = await pulse(service, cy);
  assert.deepEqual(
    seen.here.map((p) => [p.name, p.state, p.place]),
    [["Ada", "playing", "Larkfield"], ["Bo", "here", null]],
  );
  // Ada's pulses stop: two minutes on she is away.
  clock.t += 121_000;
  await pulse(service, bo);
  const later = await pulse(service, cy);
  assert.deepEqual(later.here.map((p) => p.name), ["Bo"]);
  // Leaving says so at once.
  await call(service, "POST", "rooms/pulse", bo, { state: "away" });
  assert.deepEqual((await pulse(service, cy)).here, []);
});

test("show me as here: nobody hides, friends shows only to the players I added", async () => {
  const { service, ada, bo, cy } = await three();
  const adaHandle = (await pulse(service, ada)).you.handle;
  const boHandle = (await pulse(service, bo)).you.handle;
  const cyHandle = (await pulse(service, cy)).you.handle;
  // Ada shows herself to friends only, and has added Bo.
  assert.equal((await call(service, "POST", "rooms/friends", ada, { handle: boHandle, friend: true })).status, 200);
  await pulse(service, ada, { show: "friends" });
  assert.deepEqual((await pulse(service, bo)).here.map((p) => p.name), ["Ada", "Cy"]);
  assert.deepEqual((await pulse(service, cy)).here.map((p) => p.name), ["Bo"]);
  // Nobody: gone from every list, friends' too.
  await pulse(service, ada, { show: "nobody" });
  assert.deepEqual((await pulse(service, bo)).here.map((p) => p.name), ["Cy"]);
  // Bo adds Ada: she is listed with his friends, away while hidden.
  await call(service, "POST", "rooms/friends", bo, { handle: adaHandle, friend: true });
  const bos = await pulse(service, bo);
  assert.deepEqual(bos.friends.map((p) => [p.name, p.state]), [["Ada", "away"]]);
  await pulse(service, ada, { show: "everyone" });
  const again = await pulse(service, bo);
  assert.deepEqual(again.friends.map((p) => [p.name, p.state]), [["Ada", "here"]]);
  assert.deepEqual(again.here.map((p) => [p.name, p.friend]), [["Ada", true], ["Cy", false]]);
  // Removing a friend is one-sided too.
  await call(service, "POST", "rooms/friends", bo, { handle: adaHandle, friend: false });
  assert.deepEqual((await pulse(service, bo)).friends, []);
  assert.equal((await call(service, "POST", "rooms/friends", bo, { handle: boHandle, friend: true })).status, 422);
  assert.equal((await call(service, "POST", "rooms/friends", bo, { handle: cyHandle, friend: "yes" })).status, 400);
});

test("a block keeps two players out of each other's lists and invites", async () => {
  const { service, ada, bo } = await three();
  const boHandle = (await pulse(service, bo)).you.handle;
  await pulse(service, ada);
  await call(service, "POST", "rooms/mutes", ada, { handle: boHandle, mode: "block" });
  assert.deepEqual((await pulse(service, ada)).here, []);
  assert.deepEqual((await pulse(service, bo)).here, []);
  const sent = await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "match" });
  assert.equal(sent.status, 403);
});

test("an invite is sent, heard, accepted, and the match starts with both seated", { skip }, async () => {
  const { service, ada, bo } = await three();
  const adaHandle = (await pulse(service, ada)).you.handle;
  const boHandle = (await pulse(service, bo)).you.handle;
  const sent = await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "match", context: { pile: 7 } });
  assert.equal(sent.status, 200, JSON.stringify(sent.body));
  assert.equal(sent.body.invite.state, "sent");
  assert.equal(sent.body.invite.mine, true);
  assert.equal(sent.body.invite.to.name, "Bo");
  // The same invite again is the one waiting.
  const again = await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "match" });
  assert.equal(again.body.invite.id, sent.body.invite.id);
  const id = sent.body.invite.id;

  const bos = await pulse(service, bo);
  assert.deepEqual(bos.invites.map((i) => [i.from.name, i.state, i.mine]), [["Ada", "sent", false]]);
  // Bo's client took the notice: heard.
  const heard = await pulse(service, bo, { heard: [id] });
  assert.equal(heard.invites[0].state, "heard");
  assert.equal((await pulse(service, ada)).invites[0].state, "heard");

  // Only Bo answers; only Ada takes it back.
  assert.equal((await call(service, "POST", `rooms/invites/${id}`, ada, { answer: "accept" })).status, 403);
  assert.equal((await call(service, "POST", `rooms/invites/${id}`, bo, { answer: "withdraw" })).status, 403);
  const yes = await call(service, "POST", `rooms/invites/${id}`, bo, { answer: "accept" });
  assert.equal(yes.status, 200, JSON.stringify(yes.body));
  assert.equal(yes.body.invite.state, "accepted");
  assert.equal(yes.body.room.phase, "playing");
  assert.deepEqual(yes.body.room.context, { pile: 7 });
  assert.equal(yes.body.room.you, 1);
  assert.deepEqual(yes.body.room.members.map((m) => [m.name, m.seat]), [["Ada", 0], ["Bo", 1]]);
  assert.equal(yes.body.invite.room, yes.body.room.id);

  // Ada hears it once, then it is gone from her pulse; the match is in both.
  const told = await pulse(service, ada);
  assert.deepEqual(told.invites.map((i) => i.state), ["accepted"]);
  assert.deepEqual(told.rooms.map((r) => [r.phase, r.snapshot.outcome.next_seat, r.you]), [["playing", 0, 0]]);
  assert.deepEqual((await pulse(service, ada)).invites, []);
  assert.deepEqual((await pulse(service, bo)).invites, []);
  assert.equal((await call(service, "POST", `rooms/invites/${id}`, bo, { answer: "decline" })).status, 409);
  void adaHandle;
});

test("a declined invite is told to the sender once; a withdrawn one to nobody", async () => {
  const { service, ada, bo, cy } = await three();
  await pulse(service, ada);
  const boHandle = (await pulse(service, bo)).you.handle;
  const cyHandle = (await pulse(service, cy)).you.handle;
  const toBo = (await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "match" })).body.invite;
  const toCy = (await call(service, "POST", "rooms/invites", ada, { handle: cyHandle, kind: "match" })).body.invite;
  assert.equal((await call(service, "POST", `rooms/invites/${toBo.id}`, bo, { answer: "decline" })).body.invite.state, "declined");
  assert.equal((await call(service, "POST", `rooms/invites/${toCy.id}`, ada, { answer: "withdraw" })).body.invite.state, "withdrawn");
  assert.deepEqual((await pulse(service, cy)).invites, []);
  const told = await pulse(service, ada);
  assert.deepEqual(told.invites.map((i) => [i.to.name, i.state]), [["Bo", "declined"]]);
  assert.deepEqual((await pulse(service, ada)).invites, []);
});

test("an invite nobody answers lapses after three days", async () => {
  const { service, clock, ada, bo } = await three();
  await pulse(service, ada);
  const boHandle = (await pulse(service, bo)).you.handle;
  const invite = (await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "match" })).body.invite;
  clock.t += 3 * 86_400_000 + 1;
  assert.deepEqual((await pulse(service, bo)).invites, []);
  const told = await pulse(service, ada);
  assert.deepEqual(told.invites.map((i) => i.state), ["lapsed"]);
  const late = await call(service, "POST", `rooms/invites/${invite.id}`, bo, { answer: "accept" });
  assert.equal(late.status, 409);
  assert.equal(late.body.state, "lapsed");
});

test("a muted player's invites are never heard, and they are not told", async () => {
  const { service, ada, bo } = await three();
  const adaHandle = (await pulse(service, ada)).you.handle;
  const boHandle = (await pulse(service, bo)).you.handle;
  await call(service, "POST", "rooms/mutes", bo, { handle: adaHandle, mode: "mute" });
  const sent = await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "match" });
  assert.equal(sent.status, 200);
  assert.deepEqual((await pulse(service, bo)).invites, []);
  assert.equal((await pulse(service, ada)).invites[0].state, "sent");
});

test("invites go only to matches, and not to yourself", async () => {
  const { service, ada, bo } = await three();
  const adaHandle = (await pulse(service, ada)).you.handle;
  const boHandle = (await pulse(service, bo)).you.handle;
  assert.equal((await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "lobby" })).status, 422);
  assert.equal((await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "nope" })).status, 404);
  assert.equal((await call(service, "POST", "rooms/invites", ada, { handle: adaHandle, kind: "match" })).status, 422);
  assert.equal((await call(service, "POST", "rooms/invites", ada, { handle: "0123456789abcdef", kind: "match" })).status, 404);
  assert.equal((await call(service, "POST", "rooms/pulse", null, {})).status, 401);
});

test("a pulse carries my waiting rooms and the open tables of others", { skip }, async () => {
  const { service, ada, bo, cy } = await three();
  await pulse(service, ada);
  const open = await call(service, "POST", "rooms/find?kind=match", ada, {});
  assert.equal(open.status, 200);
  const adas = await pulse(service, ada);
  assert.deepEqual(adas.rooms.map((r) => r.phase), ["waiting"]);
  assert.deepEqual(adas.open, []);
  const bos = await pulse(service, bo);
  assert.deepEqual(bos.open.map((o) => [o.kind, o.host, o.room]), [["match", "Ada", open.body.room.id]]);
  // Bo sits down: Ada's room is in play, and no longer open to Cy.
  await call(service, "POST", "rooms/find?kind=match", bo, {});
  assert.deepEqual((await pulse(service, ada)).rooms.map((r) => [r.phase, r.members.length]), [["playing", 2]]);
  assert.deepEqual((await pulse(service, cy)).open, []);
});
