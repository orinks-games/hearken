// Chat safety: blocks everywhere and never told, reports with a marked
// line, ready lines, what guests may send, and the pulse's channel and
// standing answers. Needs no rules module: no match here is played.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createService, memoryAccounts, memoryLimiter, memoryRooms, memoryRuns, publishedRules } from "../src/index.js";

const base = "https://example.test/api/games/pebbles";

async function setup(serviceExtra = {}, lobbyExtra = {}) {
  const clock = { t: 1_700_000_000_000 };
  const accounts = memoryAccounts();
  const reports = [];
  const screened = [];
  const service = createService({
    rules: publishedRules([]),
    accounts,
    runs: memoryRuns(),
    rooms: memoryRooms(),
    limiter: memoryLimiter({ now: () => clock.t }),
    handlers: {
      match: { rules: "pebbles", seats: { min: 2, max: 2 }, start: () => ({ seed: "1" }), accept: () => ({}), quick: { gg: "Good game." }, guest_chat: "quick" },
      lobby: {
        seats: { min: 1, max: 50 },
        quick: { hello: "Hello." },
        guest_chat: "none",
        chat_limit: { per_minute: 20, max_chars: 12 },
        ...lobbyExtra,
      },
    },
    basePath: "/api/games/pebbles",
    log: () => {},
    now: () => clock.t,
    report: async (r) => reports.push(r),
    screen: async (line) => {
      screened.push(line);
      return line.text.includes("rude") ? false : undefined;
    },
    ...serviceExtra,
  });
  const ada = await accounts.tokenFor("Ada");
  const bo = await accounts.tokenFor("Bo");
  const cy = await accounts.tokenFor("Cy");
  return { service, accounts, clock, reports, screened, ada, bo, cy };
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

async function pulse(service, token, body = {}) {
  const res = await call(service, "POST", "rooms/pulse", token, { state: "here", ...body });
  assert.equal(res.status, 200, JSON.stringify(res.body));
  return res.body;
}

async function handle(service, token) {
  return (await pulse(service, token)).you.handle;
}

async function guest(service) {
  const res = await call(service, "POST", "activate/guest", null, {});
  assert.equal(res.status, 200, JSON.stringify(res.body));
  return res.body.token;
}

async function lobby(service, token) {
  const res = await call(service, "POST", "rooms/find?kind=lobby&name=parlor", token, {});
  assert.equal(res.status, 200, JSON.stringify(res.body));
  return res.body.room.id;
}

async function say(service, token, id, body) {
  return call(service, "POST", `rooms/${id}/events`, token, { type: "chat", ...body });
}

test("a block takes the friend away and the blocker's invites back; theirs is kept as sent and never heard", async () => {
  const { service, ada, bo } = await setup();
  const adaHandle = await handle(service, ada);
  const boHandle = await handle(service, bo);
  await call(service, "POST", "rooms/friends", ada, { handle: boHandle, friend: true });
  await call(service, "POST", "rooms/friends", bo, { handle: adaHandle, friend: true });
  const mine = (await call(service, "POST", "rooms/invites", ada, { handle: boHandle, kind: "match" })).body.invite;
  assert.equal(mine.state, "sent");

  assert.equal((await call(service, "POST", "rooms/mutes", ada, { handle: boHandle, mode: "block" })).status, 200);
  const adas = await pulse(service, ada);
  assert.deepEqual(adas.friends, []);
  assert.deepEqual(adas.here, []);
  // Bo still has Ada as a friend, always away, and her invite is gone from his list.
  const bos = await pulse(service, bo);
  assert.deepEqual(bos.friends.map((f) => [f.name, f.state]), [["Ada", "away"]]);
  assert.deepEqual(bos.invites, []);

  // Bo's invite to Ada answers as sent, and she never hears it.
  const theirs = await call(service, "POST", "rooms/invites", bo, { handle: adaHandle, kind: "match" });
  assert.equal(theirs.status, 200, JSON.stringify(theirs.body));
  assert.equal(theirs.body.invite.state, "sent");
  assert.deepEqual((await pulse(service, ada)).invites, []);
  assert.deepEqual((await pulse(service, bo)).invites.map((i) => [i.to.name, i.state]), [["Ada", "sent"]]);

  // The Settings list has Bo, and unblocking brings both back, friendship not.
  const list = await call(service, "GET", "rooms/mutes", ada);
  assert.deepEqual(list.body.mutes.map((m) => [m.name, m.mode]), [["Bo", "block"]]);
  await call(service, "POST", "rooms/mutes", ada, { handle: boHandle, mode: "none" });
  const back = await pulse(service, ada);
  assert.deepEqual(back.here.map((p) => p.name), ["Bo"]);
  assert.deepEqual(back.friends, []);
});

test("a code to a table across a block: no such game for the blocked, a reason for the blocker", async () => {
  const { service, ada, bo, cy } = await setup();
  const boHandle = await handle(service, bo);
  const cyHandle = await handle(service, cy);
  await handle(service, ada);
  const made = await call(service, "POST", "rooms?kind=match", ada, {});
  assert.equal(made.status, 200, JSON.stringify(made.body));
  const code = made.body.room.code;
  // Ada blocked Bo: Bo hears there is no such game.
  await call(service, "POST", "rooms/mutes", ada, { handle: boHandle, mode: "block" });
  const bos = await call(service, "POST", "rooms/join", bo, { code });
  assert.equal(bos.status, 404);
  assert.equal(bos.body.error, "unknown-code");
  // Cy blocked Ada: Cy is told why.
  await call(service, "POST", "rooms/mutes", cy, { handle: (await handle(service, ada)), mode: "block" });
  const cys = await call(service, "POST", "rooms/join", cy, { code });
  assert.equal(cys.status, 403);
  assert.equal(cys.body.error, "blocked-at-table");
  assert.equal(cys.body.message, "A player you blocked is at that table.");
  void cyHandle;
});

test("in a channel a blocked player's lines, comings and goings and seat are left out, both ways", async () => {
  const { service, ada, bo, cy } = await setup();
  const boHandle = await handle(service, bo);
  const id = await lobby(service, ada);
  await call(service, "POST", "rooms/mutes", ada, { handle: boHandle, mode: "block" });
  await lobby(service, bo);
  await lobby(service, cy);
  assert.equal((await say(service, bo, id, { text: "hi all" })).status, 200);
  assert.equal((await say(service, cy, id, { text: "hey" })).status, 200);
  const adas = (await call(service, "GET", `rooms/${id}?after=0`, ada)).body;
  assert.deepEqual(adas.events.map((e) => [e.type, e.name]), [["join", "Ada"], ["join", "Cy"], ["chat", "Cy"]]);
  assert.deepEqual(adas.room.members.map((m) => m.name), ["Ada", "Cy"]);
  const bos = (await call(service, "GET", `rooms/${id}?after=0`, bo)).body;
  assert.deepEqual(bos.room.members.map((m) => m.name), ["Bo", "Cy"]);
  assert.ok(!bos.events.some((e) => e.name === "Ada"));
  // Cy sees everyone.
  const cys = (await call(service, "GET", `rooms/${id}?after=0`, cy)).body;
  assert.deepEqual(cys.room.members.map((m) => m.name), ["Ada", "Bo", "Cy"]);
});

test("ready lines go by id and are screened as such; typed lines keep the kind's length", async () => {
  const { service, ada, bo, screened } = await setup();
  const id = await lobby(service, ada);
  await lobby(service, bo);
  const quick = await say(service, ada, id, { quick: "hello" });
  assert.equal(quick.status, 200, JSON.stringify(quick.body));
  assert.equal((await say(service, ada, id, { quick: "nope" })).status, 400);
  assert.equal(screened.at(-1).quick, "hello");
  assert.equal((await say(service, ada, id, { text: "a long line that runs on" })).status, 200);
  assert.equal((await say(service, ada, id, { text: "rude" })).status, 422);
  const events = (await call(service, "GET", `rooms/${id}?after=0`, bo)).body.events.filter((e) => e.type === "chat");
  assert.deepEqual(events.map((e) => [e.text, e.quick ?? null]), [["Hello.", "hello"], ["a long line", null]]);
});

test("guests send only what the kind allows", async () => {
  const { service, ada } = await setup();
  const g = await guest(service);
  const id = await lobby(service, ada);
  await lobby(service, g);
  const typed = await say(service, g, id, { text: "hi" });
  assert.equal(typed.status, 403);
  assert.equal(typed.body.error, "guests-quick");
  assert.equal((await say(service, g, id, { quick: "hello" })).status, 403);
  // At a table guests may send the ready lines only.
  const made = await call(service, "POST", "rooms?kind=match", g, {});
  const table = made.body.room.id;
  assert.equal((await say(service, g, table, { quick: "gg" })).status, 200);
  assert.equal((await say(service, g, table, { text: "hello" })).status, 403);
});

test("a report carries the marked line and the talk around it, and mutes the player for the reporter", async () => {
  const { service, ada, bo, cy, reports } = await setup();
  const boHandle = await handle(service, bo);
  const id = await lobby(service, ada);
  await lobby(service, bo);
  await lobby(service, cy);
  await say(service, cy, id, { text: "hello" });
  const bad = await say(service, bo, id, { text: "mean thing" });
  await say(service, ada, id, { text: "stop" });
  const res = await call(service, "POST", `rooms/${id}/report`, ada, { handle: boHandle, seq: bad.body.seq, note: "he was mean" });
  assert.equal(res.status, 200, JSON.stringify(res.body));
  assert.deepEqual(res.body, { ok: true, lines: 1, marked: true, muted: true });
  const r = reports[0];
  assert.equal(r.about.name, "Bo");
  assert.equal(r.marked.text, "mean thing");
  assert.deepEqual(r.context.map((e) => e.text), ["hello", "mean thing", "stop"]);
  assert.equal(r.note, "he was mean");
  assert.ok(!("player" in r.marked));
  // Bo is muted for Ada now: his next line is not hers to hear.
  await say(service, bo, id, { text: "again" });
  const adas = (await call(service, "GET", `rooms/${id}?after=0`, ada)).body.events;
  assert.ok(!adas.some((e) => e.text === "again"));
  const list = (await call(service, "GET", "rooms/mutes", ada)).body.mutes;
  assert.deepEqual(list.map((m) => [m.name, m.mode]), [["Bo", "mute"]]);
});

test("a name can be reported without a room, five reports an hour at most", async () => {
  const { service, ada, bo, reports, clock } = await setup();
  const boHandle = await handle(service, bo);
  await handle(service, ada);
  for (let i = 0; i < 5; i += 1) {
    const res = await call(service, "POST", "rooms/report", ada, { handle: boHandle });
    assert.equal(res.status, 200, JSON.stringify(res.body));
  }
  assert.equal(reports[0].room, null);
  assert.deepEqual(reports[0].lines, []);
  assert.equal((await call(service, "POST", "rooms/report", ada, { handle: boHandle })).status, 429);
  clock.t += 3_600_001;
  assert.equal((await call(service, "POST", "rooms/report", ada, { handle: boHandle })).status, 200);
  assert.equal((await call(service, "POST", "rooms/report", ada, { handle: await handle(service, ada) })).status, 400);
});

test("a pulse answers how many are in a channel, its last line, muted players, and the player's standing", async () => {
  const notes = new Map();
  const { service, ada, bo, cy } = await setup({
    standing: async (player) => (notes.has(player.name) ? { chat: "off", message: notes.get(player.name) } : null),
  });
  const cyHandle = await handle(service, cy);
  await handle(service, bo);
  const id = await lobby(service, bo);
  await lobby(service, cy);
  await say(service, bo, id, { text: "anyone?" });
  const asked = await pulse(service, ada, { channels: [{ kind: "lobby", name: "parlor" }, { kind: "match", name: "x" }, { kind: "lobby", name: "empty" }] });
  assert.deepEqual(asked.channels.map((c) => [c.name, c.here, c.last?.name ?? null]), [["parlor", 2, "Bo"], ["empty", 0, null]]);
  assert.equal(asked.standing, null);
  await call(service, "POST", "rooms/mutes", ada, { handle: cyHandle, mode: "mute" });
  await say(service, cy, id, { text: "me too" });
  const again = await pulse(service, ada, { channels: [{ kind: "lobby", name: "parlor" }] });
  assert.equal(again.channels[0].last.name, "Bo");
  assert.deepEqual(again.here.map((p) => [p.name, p.muted]), [["Bo", false], ["Cy", true]]);
  notes.set("Ada", "Your chat is off until tomorrow.");
  assert.deepEqual((await pulse(service, ada)).standing, { chat: "off", message: "Your chat is off until tomorrow." });
});
