# @hearken/online

The server side of Hearken's verified runs. A game sends a run file (its
seed and inputs); this library replays it with the game's own rules,
published as a WebAssembly module, and hands the outcome it computed to the
game's handler for that kind of run. The handler decides what the outcome
is for: a leaderboard, a shared total, a ghost, anything.

It has no dependencies and uses only web-standard APIs (WebAssembly,
`Request`, `Response`, Web Crypto), so the same code runs on Vercel,
Netlify, Cloudflare Workers, Deno, Bun and Node.

```js
import { createService, publishedRules } from "@hearken/online";

const service = createService({ rules, accounts, runs, limiter, handlers });
export default { fetch: (request) => service.fetch(request) };
```

- `src/rules.js`: loading and calling a rules module.
- `src/service.js`: the HTTP protocol, as one `fetch` function.
- `src/memory.js`: accounts, run claims and rate limits in memory, and what
  a real store must guarantee.
- `src/node.js`: `serve()` for Node's HTTP server, `rulesFromDir()`, and
  `workerReplay()` for a hard time limit per replay.
- `bin/dev-server.js`: a local server for trying a game before it has a
  real one.
- `examples/`: handlers for the example rules, and setups for Vercel with
  Convex, a Cloudflare Worker and a self-hosted Node server.

`docs/online.md` in the Hearken repository covers the protocol, the rules
module, writing handlers and hosting.

## Tests

```sh
cargo test -p hearken-replay
cargo build -p hearken-replay --example coin-dash --target wasm32-unknown-unknown --release
cd server && npm test
```

The tests verify the runs the Rust tests recorded against the real Coin
Dash module, so build both first.
