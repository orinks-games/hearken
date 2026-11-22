// Synced files: a sealed copy of a save kept with the player's account.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createService, memoryAccounts, memoryLimiter, memoryRuns, memorySaves, sha256Hex } from "../src/index.js";

const base = "https://example.test/api/games/g";
const SEALED = new Uint8Array([0x48, 0x4b, 0x53, 0x4c, 1, 2, 3, 4]);

function setup({ files, guests, limits, keep } = {}) {
  const accounts = memoryAccounts();
  const store = memorySaves({ keep });
  const saved = [];
  const service = createService({
    rules: { get: async () => null },
    accounts,
    runs: memoryRuns(),
    limiter: memoryLimiter(),
    handlers: {},
    basePath: "/api/games/g",
    log: () => {},
    limits,
    saves: {
      store,
      guests,
      files: files ?? {
        records: {
          sealed: true,
          maxBytes: 64,
          card: (card) => {
            if (typeof card.best !== "number") throw Object.assign(new Error(), { code: "bad-card" });
            return { best: card.best };
          },
        },
      },
      onSave: async (event) => saved.push(event),
    },
  });
  return { service, accounts, store, saved };
}

const b64 = (bytes) => btoa(String.fromCharCode(...bytes));

async function upload(service, token, { base: from = 0, content = SEALED, format = 1, card, hash, name = "records" } = {}) {
  const body = { base: from, format, hash: hash ?? (await sha256Hex(content)), content: b64(content) };
  if (card !== undefined) body.card = card;
  return service.fetch(
    new Request(`${base}/saves/${name}`, {
      method: "POST",
      headers: token ? { authorization: `Bearer ${token}` } : {},
      body: JSON.stringify(body),
    }),
  );
}

function get(service, token, path) {
  return service.fetch(
    new Request(`${base}/${path}`, { headers: token ? { authorization: `Bearer ${token}` } : {} }),
  );
}

test("a save goes up, and comes back on another computer", async () => {
  const { service, accounts, saved } = setup();
  const token = await accounts.tokenFor("Ada");
  const empty = await (await get(service, token, "saves/records/head")).json();
  assert.equal(empty.ok, true);
  assert.equal(empty.save, null);
  assert.ok(empty.account);

  const up = await upload(service, token, { card: { best: 12 } });
  assert.equal(up.status, 200);
  assert.deepEqual(await up.json(), { ok: true, revision: 1 });
  assert.equal(saved.length, 1);
  assert.deepEqual(saved[0].card, { best: 12 });

  // Another computer: another token for the same player.
  const other = await accounts.tokenForPlayer(await accounts.player(token));
  const head = await (await get(service, other, "saves/records/head")).json();
  assert.equal(head.account, empty.account);
  assert.equal(head.save.revision, 1);
  assert.equal(head.save.bytes, SEALED.length);
  assert.equal(head.save.hash, await sha256Hex(SEALED));
  const down = await (await get(service, other, "saves/records")).json();
  assert.equal(down.revision, 1);
  assert.equal(down.content, b64(SEALED));
});

test("an upload from an old revision is refused as stale", async () => {
  const { service, accounts } = setup();
  const token = await accounts.tokenFor("Ada");
  assert.equal((await upload(service, token)).status, 200);
  const second = new Uint8Array([...SEALED, 9]);
  assert.equal((await upload(service, token, { base: 1, content: second })).status, 200);
  const stale = await upload(service, token, { base: 1, content: new Uint8Array([...SEALED, 7]) });
  assert.equal(stale.status, 409);
  assert.deepEqual(await stale.json(), { error: "stale", revision: 2 });
  // The same bytes again are answered with the revision they already are.
  const repeat = await upload(service, token, { base: 1, content: second });
  assert.equal(repeat.status, 200);
  assert.equal((await repeat.json()).revision, 2);
});

test("guests keep no saves unless the game allows them", async () => {
  const { service } = setup();
  const guest = await (await service.fetch(new Request(`${base}/activate/guest`, { method: "POST", body: "{}" }))).json();
  assert.equal((await upload(service, guest.token)).status, 403);
  assert.equal((await get(service, guest.token, "saves/records/head")).status, 403);
  const open = setup({ guests: true }).service;
  const g2 = await (await open.fetch(new Request(`${base}/activate/guest`, { method: "POST", body: "{}" }))).json();
  assert.equal((await upload(open, g2.token)).status, 200);
});

test("damaged, plain, oversized and unknown saves are refused", async () => {
  const { service, accounts } = setup();
  const token = await accounts.tokenFor("Ada");
  assert.equal((await upload(service, null)).status, 401);
  assert.equal((await upload(service, token, { hash: "0".repeat(64) })).status, 422);
  const plain = await upload(service, token, { content: new TextEncoder().encode("{}") });
  assert.equal(plain.status, 422);
  assert.equal((await plain.json()).error, "not-sealed");
  assert.equal((await upload(service, token, { content: new Uint8Array(65).fill(0x48) })).status, 413);
  assert.equal((await upload(service, token, { name: "other" })).status, 404);
  assert.equal((await get(service, token, "saves/records")).status, 404);
  const badCard = await upload(service, token, { card: { best: "lots" } });
  assert.equal(badCard.status, 503, "a card check that throws a plain error is a server fault");
});

test("a card refused by the game is refused with its code", async () => {
  const { refuse } = await import("../src/index.js");
  const { service, accounts, store } = setup({
    files: { records: { card: () => { throw refuse("bad-card", "That card is not one this game makes."); } } },
  });
  const token = await accounts.tokenFor("Ada");
  const res = await upload(service, token, { card: { x: 1 } });
  assert.equal(res.status, 422);
  assert.equal((await res.json()).error, "bad-card");
  assert.equal(await store.head("Ada", "records"), null);
});

test("only the latest revisions are kept, and uploads are limited", async () => {
  const { service, accounts, store } = setup({ keep: 3, limits: { savesPerHour: 5 } });
  const token = await accounts.tokenFor("Ada");
  const player = await accounts.player(token);
  const statuses = [];
  for (let i = 0; i < 6; i++) {
    statuses.push((await upload(service, token, { base: i, content: new Uint8Array([...SEALED, i]) })).status);
  }
  assert.deepEqual(statuses, [200, 200, 200, 200, 200, 429]);
  assert.deepEqual(await store.revisions(player.id, "records"), [3, 4, 5]);
});

test("a service with no saves answers 404, and no kind may be called saves", async () => {
  const accounts = memoryAccounts();
  const service = createService({
    rules: { get: async () => null },
    accounts,
    runs: memoryRuns(),
    limiter: memoryLimiter(),
    handlers: {},
    log: () => {},
  });
  const token = await accounts.tokenFor("Ada");
  assert.equal((await get(service, token, "saves/records/head")).status, 404);
  assert.throws(() =>
    createService({ rules: {}, accounts, runs: {}, limiter: {}, handlers: { saves: { expect() {}, accept() {} } } }),
  );
});
