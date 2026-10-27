// A game's verified runs on your own machine or a VPS: Node's own HTTP
// server, modules from a folder, and replays in a worker thread with a time
// limit. Put it behind a TLS proxy (Caddy, nginx) so the game reaches it
// over HTTPS, and set trustProxy so the proxy's X-Forwarded-For is kept.
//
//   node examples/node-server.js
//
// The in-memory stores forget everything on restart; swap in SQLite or
// Postgres behind the same three interfaces for real use.

import { createService, memoryAccounts, memoryLimiter, memoryRuns } from "../src/index.js";
import { rulesFromDir, serve, workerReplay } from "../src/node.js";
import { createHandlers } from "./handlers.js";

const service = createService({
  rules: await rulesFromDir(process.env.RULES_DIR ?? "./rules"),
  accounts: memoryAccounts({ verifyUrl: "https://games.example.org/activate" }),
  runs: memoryRuns(),
  limiter: memoryLimiter(),
  handlers: createHandlers(),
  basePath: "/api/games/coin-dash",
  replay: workerReplay({ timeoutMs: 5000 }),
});

await serve(service.fetch, { port: Number(process.env.PORT ?? 8787), trustProxy: true });
console.log("Serving /api/games/coin-dash");
