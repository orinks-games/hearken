// Guests: players with no account, named by the server.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createService, isGuestName, memoryAccounts, memoryLimiter, memoryRuns } from "../src/index.js";

const base = "https://example.test/api/games/g";

function setup(options = {}) {
  const accounts = memoryAccounts();
  const links = [];
  const service = createService({
    rules: { get: async () => null },
    accounts,
    runs: memoryRuns(),
    limiter: memoryLimiter(),
    handlers: {},
    basePath: "/api/games/g",
    log: () => {},
    onLink: async (link) => links.push(link),
    ...options,
  });
  return { service, accounts, links };
}

function post(service, path, { token, body, address } = {}) {
  const headers = {};
  if (token) headers.authorization = `Bearer ${token}`;
  if (address) headers["x-forwarded-for"] = address;
  return service.fetch(
    new Request(`${base}/${path}`, { method: "POST", headers, body: JSON.stringify(body ?? {}) }),
  );
}

test("a guest gets a token and a name from the word lists", async () => {
  const { service, accounts } = setup();
  const res = await post(service, "activate/guest");
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.status, "ready");
  assert.equal(body.guest, true);
  assert.ok(isGuestName(body.name), body.name);
  const player = await accounts.player(body.token);
  assert.equal(player.name, body.name);
  assert.equal(player.guest, true);
});

test("a guest takes no name from the request", async () => {
  const { service } = setup();
  const res = await post(service, "activate/guest", { body: { name: "Rude Word" } });
  const body = await res.json();
  assert.notEqual(body.name, "Rude Word");
  assert.ok(isGuestName(body.name));
});

test("new guests are limited per address per day", async () => {
  const { service } = setup({ limits: { guestsPerDay: 2 } });
  const statuses = [];
  for (let i = 0; i < 3; i++) statuses.push((await post(service, "activate/guest", { address: "203.0.113.9" })).status);
  assert.deepEqual(statuses, [200, 200, 429]);
  assert.equal((await post(service, "activate/guest", { address: "198.51.100.4" })).status, 200);
});

test("a guest can ask for another name, and only a guest", async () => {
  const { service, accounts } = setup();
  const { token } = await (await post(service, "activate/guest")).json();
  const renamed = await (await post(service, "activate/rename", { token })).json();
  assert.equal(renamed.ok, true);
  assert.ok(isGuestName(renamed.name));
  assert.equal((await accounts.player(token)).name, renamed.name);

  const account = await accounts.tokenFor("Ada");
  assert.equal((await post(service, "activate/rename", { token: account })).status, 422);
  assert.equal((await post(service, "activate/rename")).status, 401);
});

test("renames are limited", async () => {
  const { service } = setup({ limits: { renamesPerHour: 1 } });
  const { token } = await (await post(service, "activate/guest")).json();
  assert.equal((await post(service, "activate/rename", { token })).status, 200);
  assert.equal((await post(service, "activate/rename", { token })).status, 429);
});

test("a guest who connects an account is linked once, and its token stops working", async () => {
  const { service, accounts, links } = setup();
  const guest = await (await post(service, "activate/guest")).json();
  const guestId = (await accounts.player(guest.token)).id;
  const start = await (await post(service, "activate/start", { token: guest.token })).json();
  assert.ok(await accounts.approve(start.user_code, "Ada"));
  const ready = await (await post(service, "activate/poll", { body: { device_code: start.device_code } })).json();
  assert.equal(ready.status, "ready");
  assert.equal(ready.name, "Ada");
  assert.equal(links.length, 1);
  assert.equal(links[0].guest, guestId);
  assert.equal(links[0].player.name, "Ada");
  assert.equal(await accounts.player(guest.token), null);
  assert.ok(await accounts.player(ready.token));
});

test("connecting without a guest links nothing", async () => {
  const { service, accounts, links } = setup();
  const start = await (await post(service, "activate/start")).json();
  await accounts.approve(start.user_code, "Ada");
  await post(service, "activate/poll", { body: { device_code: start.device_code } });
  assert.equal(links.length, 0);
});

test("a store with no guests answers that there are none", async () => {
  const accounts = memoryAccounts();
  delete accounts.createGuest;
  const { service } = setup({ accounts });
  assert.equal((await post(service, "activate/guest")).status, 404);
});
