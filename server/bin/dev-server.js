#!/usr/bin/env node
// A local server for trying a game's online play before it has a real one:
// published modules from a folder, accounts and runs in memory, and a page
// at /activate to approve a connection.
//
//   node server/bin/dev-server.js --rules dist --handlers ./my-handlers.js
//
// Then run the game with <PREFIX>_ONLINE_URL=http://127.0.0.1:8787 (a
// build that has no rules ID of its own also needs HEARKEN_RULES_ID).

import { parseArgs } from "node:util";
import { pathToFileURL } from "node:url";
import { resolve } from "node:path";
import { createService, memoryAccounts, memoryLimiter, memoryRuns } from "../src/index.js";
import { rulesFromDir, serve, workerReplay } from "../src/node.js";

const { values } = parseArgs({
  options: {
    rules: { type: "string", default: "dist" },
    handlers: { type: "string" },
    port: { type: "string", default: "8787" },
    "auto-approve": { type: "string" },
  },
});

const port = Number(values.port);
const origin = `http://127.0.0.1:${port}`;
const rules = await rulesFromDir(resolve(values.rules));
const builds = await rules.builds();
const handlersUrl = values.handlers
  ? pathToFileURL(resolve(values.handlers)).href
  : new URL("../examples/handlers.js", import.meta.url).href;
const handlerModule = await import(handlersUrl);
const handlers = handlerModule.createHandlers ? handlerModule.createHandlers() : handlerModule.default;
const accounts = memoryAccounts({ verifyUrl: `${origin}/activate`, autoApprove: values["auto-approve"] ?? null });
const service = createService({
  rules,
  accounts,
  runs: memoryRuns(),
  limiter: memoryLimiter(),
  handlers,
  replay: workerReplay(),
});

const page = (message = "") => `<!doctype html><html lang="en"><head><meta charset="utf-8">
<title>Connect a game</title></head><body><main><h1>Connect a game</h1>
${message ? `<p role="status">${message}</p>` : ""}
<form method="post" action="/activate"><p><label>Code from the game <input name="code" autocomplete="off" required></label></p>
<p><label>Your name <input name="name" required maxlength="32"></label></p><p><button>Connect</button></p></form></main></body></html>`;

await serve(async (request) => {
  const url = new URL(request.url);
  if (url.pathname === "/activate") {
    if (request.method === "POST") {
      const form = await request.formData();
      const ok = await accounts.approve(form.get("code"), form.get("name"));
      const message = ok ? "Connected. You can go back to the game." : "That code is not one the game is waiting on.";
      return new Response(page(message), { headers: { "content-type": "text/html; charset=utf-8" } });
    }
    return new Response(page(), { headers: { "content-type": "text/html; charset=utf-8" } });
  }
  return service.fetch(request);
}, { port });

console.log(`Hearken dev server on ${origin}`);
console.log(`Published builds: ${builds.length ? builds.join(", ") : "none (put .wasm files in the rules folder)"}`);
console.log(`Kinds: ${Object.keys(handlers).join(", ")}`);
