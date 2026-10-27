# Verified online runs

A Hearken game can send a player's runs to a server that checks them by
playing them again. The game sends the run's seed and inputs, never its
result. The server replays the inputs with its own copy of the game's
rules, built for WebAssembly, and only the result it computes counts. What
that result is for is the game's choice: a leaderboard, a shared goal for
everyone, a ghost to race, an unlock. Nothing in the engine or the server
library knows any of these.

It is opt-in per game. A game that never turns on the `online` feature or
names no rules crate gets none of it.

There are three parts:

| Part | Where | What it does |
|------|-------|--------------|
| `hearken-replay` | `crates/hearken-replay` | The run file, the `Replayable` trait a game's rules implement, `Recording`, and `export_rules!`, which builds the rules into a WebAssembly module. |
| `ctx.online` | `hearken-app`, feature `online` | Connects the player's account from Settings and sends runs in the background. |
| `@hearken/online` | `server/` | The server: replays runs with a game's published modules and hands the verified outcome to the game's handler. One web-standard `fetch(Request)` function, so it runs on Vercel, Netlify, Cloudflare Workers, Deno, Bun or a plain Node server. |

## Putting a game online

### 1. Split the rules into their own crate

The rules are the part of the game that decides the outcome: the
simulation, scoring and randomness, with no screen, sound, speech or clock.
They go in a crate that depends on `hearken-replay` (not on `hearken`), so
it builds for `wasm32-unknown-unknown`:

```toml
# my-game-rules/Cargo.toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
hearken-replay = { git = "https://github.com/orinks-games/hearken" }
serde = { version = "1", features = ["derive"] }
```

The rules implement `Replayable`:

```rust
use hearken_replay::{InputCode, Replayable};

impl Replayable for Dive {
    const RULES: &'static str = "sonar-dive"; // the name runs carry
    const MAX_TICKS: u64 = 20 * 60 * 120;     // the longest run a server replays
    type Input = Control;                      // the game's inputs, as InputCode
    type Context = Setup;                      // a mode, a difficulty: any serde type
    type Outcome = Result;                     // what a run comes to: any serde type

    fn new(seed: u64, setup: &Setup) -> Self { /* ... */ }
    fn step(&mut self, inputs: &[Control]) { /* one fixed step */ }
    fn finished(&self) -> bool { /* ... */ }
    fn outcome(&self) -> Result { /* ... */ }
}

hearken_replay::export_rules!(Dive);
```

`crates/hearken-replay/examples/coin_dash.rs` is a complete small example.

Rules must be deterministic: the same seed, context and inputs give the
same outcome on every platform. In practice:

- Run the simulation in fixed steps, one `step` per tick.
- Take randomness only from the seed.
- Read no clock. Time is ticks.
- Do not iterate a `HashMap` where the order matters; use a `BTreeMap` or a `Vec`.
- Keep `sin`, `cos`, `exp`, `powf` and similar out of anything that decides
  the outcome; they can differ between platforms. Plain `+ - * /` and
  `sqrt` on `f64` are the same everywhere.

The rules are also the referee. A run file can say anything, so `step` must
accept any input at any tick and ignore one the game would not allow then,
instead of relying on the menus to prevent it.

`hearken_replay::assert_replays(&recording)` is the determinism check: it
writes the run to a file, reads it back, replays it and compares outcomes.
Run it in the game's tests for each set of rules before turning them on.

### 2. Record runs in the game

```rust
use hearken::replay::Recording;

let mut run = Recording::<Dive>::new(seed, &setup)?;
// Each fixed step, with the inputs that arrived on it:
clock.advance(dt, |_| { run.step(&inputs_this_step); });
// Read state for speech and sound through run.sim(), which is read-only:
let depth = run.sim().depth();
```

`Recording` steps the rules and records the inputs at once, so what the
player hears is exactly what will be replayed.

### 3. Turn on the online feature and name the server

```toml
# the game's Cargo.toml
[dependencies]
hearken = { git = "https://github.com/orinks-games/hearken", features = ["online"] }

[package.metadata.hearken]
rules = "my-game-rules"
```

```rust
impl Game for MyGame {
    fn online() -> Option<OnlineConfig> {
        Some(OnlineConfig::new("https://orinks.net/api/games/my-game", "orinks.net"))
    }
}
```

Settings now has an **Online account** row. Choosing it asks the server for
a short code and speaks it, letter by letter; the player enters the code on
the site while signed in, and the game keeps the token it is given. Nothing
is sent until the player has connected an account.

Then, when a run ends:

```rust
let ticket = ctx.online.submit("daily-challenge", &run.to_run())?;
// Later, in update():
if let Some(answer) = ctx.online.result(ticket) {
    match answer {
        Ok(result) => { /* whatever the handler returned, as JSON */ }
        Err(e) => ctx.say(e.spoken(ctx.online.site())),
    }
}
```

`submit` refuses at once, without sending anything, when no account is
connected, when the build has no rules ID, or when eight runs are already on
their way. Every error has a `spoken` line for the player.

### 4. Release

`hearken release` builds the rules crate for WebAssembly first, checks the
module imports nothing and exports what a server needs, and puts it in
`dist/` beside the archive as `<game>-<version>-rules-<id>.wasm`. Its
SHA-256 is the release's **rules ID**, which goes into the game so every run
it records names the module that can replay it.

Publish that module on the server. Runs from a build whose module is not
published are refused, so an old or modified build cannot submit. Each
platform's release builds its own module; the build strips local paths so
they usually match, but publish every one that differs.

The rules target must be installed: `rustup target add
wasm32-unknown-unknown`, or `targets = ["wasm32-unknown-unknown"]` in the
game's `rust-toolchain.toml`.

### Trying it locally

```sh
cargo build -p my-game-rules --target wasm32-unknown-unknown --release
mkdir -p rules && cp target/wasm32-unknown-unknown/release/my_game_rules.wasm rules/
node <hearken>/server/bin/dev-server.js --rules rules --handlers ./handlers.js
```

Then run a developer build with `<PREFIX>_ONLINE_URL=http://127.0.0.1:8787`
and `HEARKEN_RULES_ID` set to the module's SHA-256 (`sha256sum`). A
developer build reads both when it runs; a release reads neither, so a
player's copy always talks to the game's own server with its own rules ID.
The dev server's page at `http://127.0.0.1:8787/activate` approves a code.

## The run file

A compact binary file, usually a few kilobytes:

```text
"HKRN"               magic
u8                   format version, 1
[u8; 32]             build: the SHA-256 of the rules module that made it
varint + UTF-8       rules: the name the rules go by
u64 little-endian    seed
varint + UTF-8       context, as canonical JSON (sorted keys, no spaces)
varint               ticks: how many steps the run took
varint               how many inputs follow
each input:          varint ticks since the previous input, varint code,
                     varint value (zigzag)
```

Every run has exactly one encoding (no padded varints, no trailing bytes,
canonical JSON), so the SHA-256 of the file is the run's ID. The build is at
a fixed offset, byte 5, so a server picks the module before parsing.

Limits: 4 MiB per file, 1,048,576 inputs, a 64-byte rules name and a 16 KiB
context. A replay also refuses a run longer than the rules' `MAX_TICKS`, a
run whose rules end before its last tick, and one whose inputs run out
before the rules end.

## The rules module

Built by `export_rules!`. It imports nothing and exports (ABI 1):

| Export | What it does |
|--------|--------------|
| `memory` | The module's memory, capped at 256 MiB by `hearken release`. |
| `hearken_abi() -> u32` | The ABI version, 1. |
| `hearken_alloc(len) -> ptr` / `hearken_free(ptr, len)` | Room for the host to write a run or request into. |
| `hearken_rules() -> u64` | `{"rules":["sonar-dive"]}`: the rules the module has. |
| `hearken_inspect(run, len) -> u64` | What a run file says about itself: ID, build, rules, seed, context, ticks. Nothing in it is verified. |
| `hearken_verify(run, len, request, len) -> u64` | The run checked against the request, then replayed. |

An answer is a `u64` with a pointer in the high half and a length in the
low half, JSON, freed by the host. The request is
`{"rules":"sonar-dive","seed":"123","context":{...}}`, the seed as a decimal
string. A verified answer is
`{"ok":true,"run_id":"...","rules":"...","build":"...","seed":"...","ticks":N,"outcome":{...}}`;
a refused one is `{"ok":false,"error":"wrong-seed"}`. The server library
runs each call in a fresh instance, so one run can leave nothing behind for
the next.

## The HTTP protocol

What `ctx.online` speaks, relative to the game's base URL. Any server that
speaks it works; `@hearken/online` is one.

### Connecting an account

`POST activate/start` with `{}` answers:

```json
{ "device_code": "<64 hex>", "user_code": "ABCD-EF34",
  "verify_url": "https://orinks.net/activate", "expires_in": 600, "interval": 5 }
```

The game speaks the user code and the URL, then sends
`POST activate/poll` with `{"device_code": "..."}` every `interval` seconds:

- `200 {"status":"pending"}`: not yet.
- `200 {"status":"ready","token":"...","name":"Ada"}`: connected. The token
  is handed out once; the device code is spent.
- `410 {"status":"expired"}`: the code ran out.
- `429`: poll more slowly.

The site's own page, where a signed-in player enters the code, is the
site's business. User codes should use letters a voice does not blur (the
library's alphabet leaves out O, 0, I, 1, L, S, 5, Z and 2), and the page
should rate limit attempts.

### Asking what to play

`POST start?kind=<kind>`, with the bearer token and `{}`, for a kind whose
handler hands out seeds. It answers
`200 {"ok":true,"seed":"<decimal>","context":{...}}` plus any fields of the
handler's own, `404 {"error":"unknown-kind"}` when the handler has no
`start`, and `401` for an unknown token. `ctx.online.start(kind)` sends it;
`Plan::from_answer` reads the answer. See "Choosing seeds" for why a game
would ask instead of working the seed out itself.

### Sending a run

`POST runs?kind=<kind>`, with `Authorization: Bearer <token>`,
`Content-Type: application/octet-stream` and the run file as the body. A
kind is 1 to 32 of `a-z`, `0-9` and `-`, and names one of the game's
handlers.

| Answer | Meaning |
|--------|---------|
| `200 {"ok":true,"run_id":"...","result":...}` | Verified and accepted. `result` is whatever the handler returned. A repeat of the same run by the same player gets the stored answer with `"repeat":true`. |
| `401 {"error":"signed-out"}` | The token is unknown or revoked. The game forgets it. |
| `404 {"error":"unknown-kind"}` | No handler for that kind. |
| `409 {"error":"already-submitted"}` | Another player already sent this exact run for this kind. |
| `413 {"error":"too-large"}` | Over 4 MiB. |
| `422 {"error":"<code>","message"?:"..."}` | Refused: `unknown-build`, a verification code (`wrong-seed`, `wrong-context`, `unfinished`, `too-long`, ...), or a handler's own refusal with a line for the player. |
| `429 {"error":"rate-limited"}` | Too many runs; the game says so. |
| `503 {"error":"unavailable"}` | The server is broken or down. |

### Reading

`GET <kind>/<path...>` reaches the handler's `read`, when it has one: a
leaderboard's top ten, a ghost to download, an event's total. The token is
optional there.

## Writing a server

```js
import { createService, publishedRules, refuse } from "@hearken/online";

const service = createService({
  rules: publishedRules([{ bytes: moduleBytes }]), // the published builds
  accounts,  // players and tokens
  runs,      // run claims, for idempotency
  limiter,   // rate limits
  basePath: "/api/games/my-game",
  handlers: {
    "daily-challenge": {
      // What the run must have started from. The server decides, never the run.
      async expect({ run, player }) {
        const seed = await seedFor(today());
        return { rules: "sonar-dive", seed, context: { mode: "daily" } };
      },
      // The verified outcome. Store it, and answer the game.
      async accept({ player, verified }) {
        const rank = await board.record(player, verified.outcome.depth);
        return { rank, of: await board.size() };
      },
      async read({ path }) { return board.top(10); },
    },
  },
});

export default { fetch: (request) => service.fetch(request) };
```

A handler can also have `start({ kind, player })`, which answers
`POST start` with the seed and context to play (see "Choosing seeds").

By default a handler refuses a run file another player already sent for the
same kind (`409 already-submitted`). That stops only a byte-for-byte copy:
adding an input that changes nothing (a key let go that was never pressed)
makes a different file with the same outcome. Binding runs to players is
done with seeds, below. A handler that refuses nothing can set
`copies: "allow"`, as a turn-based game where two players can honestly make
the same moves from the same seed may want.

### Choosing seeds

The handler decides what a run must have started from, and how it does
decides what a cheat can do:

- **A seed anyone can compute** (a hash of the date, as Wicket's offline
  challenges use): simple, and the game works offline. But anyone can work
  out any day's course ahead of time and have a program find the perfect
  run, and a run one player made can be resubmitted by another with a
  harmless input added.
- **A secret shared seed, revealed when its period begins**: the handler's
  `start` returns HMAC(secret, "daily:<day>") and `expect` checks it. Every
  player still plays the same course, and nobody can prepare for a day
  before it starts. Copying a run within the day is still possible.
- **A seed of the player's own**: `start` returns HMAC(secret,
  "<kind>:<player id>:<period>"). A run made by anyone else started
  somewhere else, so it can never pass as this player's. Players no longer
  share a course, which suits personal bests and unlocks more than a
  ranked board.

`server/examples/handlers.js` has one kind of each.

`accounts`, `runs` and `limiter` are small interfaces, so any storage
works. `server/src/memory.js` implements all three in memory and says what
each method must guarantee; a real store mirrors it:

| Interface | Methods | Guarantee |
|-----------|---------|-----------|
| `accounts` | `player(token)`, `startActivation()`, `pollActivation(deviceCode)` | Tokens and device codes stored only as SHA-256 hashes; a token handed out once. |
| `runs` | `claim(key, record)`, `complete(key, claim, reply)`, `release(key, claim)` | `claim` is atomic: of two claims of one key at once, one wins. |
| `limiter` | `allow(key, limit, windowMs)` | A count per key per window. |

### Hosting

- **Vercel or Next.js** (the Freight Fate driver board's setup):
  `server/examples/vercel-next/route.js` is a catch-all route,
  `app/api/games/[game]/[...path]/route.js`, with Convex behind the three
  interfaces. Use the Node runtime and pass `workerReplay()` from
  `@hearken/online/node` for a hard time limit on each replay.
- **Cloudflare Workers**: `server/examples/cloudflare/worker.js` imports the
  `.wasm` modules directly and keeps state in a Durable Object, D1 or KV.
- **Netlify, Deno, Bun**: each takes a `fetch(Request)` function as is.
- **Your own server or a VPS**: `serve()` from `@hearken/online/node` runs
  the service on Node's own HTTP server; `rulesFromDir()` loads every
  module in a folder.

The activation rate limit is per client address. By default that is the
last `X-Forwarded-For` entry, the one the nearest proxy added; any earlier
entry is whatever the client sent. Vercel and Netlify set the header
themselves. On Cloudflare pass `clientKey: (r) =>
r.headers.get("cf-connecting-ip")`, and behind a chain of proxies pass a
`clientKey` that picks the hop you trust.

A published module keeps accepting runs until it is taken out of the
published list. To retire a build whose rules had a bug, unpublish its
module; its players' runs are then refused as `unknown-build` until they
update.

A replay is usually milliseconds, but a long run costs more. `MAX_TICKS`
bounds it; on Node, `workerReplay({ timeoutMs })` stops one that runs over.
Hosts without worker threads (Cloudflare Workers) rely on the platform's own
CPU limit.

## What it protects against

| Cheat | Result |
|-------|--------|
| Edit a result in a save or the request | Ignored: the server never reads a result from the game. |
| Hand-craft a run file | Its outcome is whatever the inputs really produce: that is playing the game. |
| Use a modified build | The server replays with its own published copy of the rules, so the build's changes do nothing. A run naming an unpublished module is refused; a modified build can still name a published one, which is harmless for the same reason. |
| Replay an old day's seed for today | The handler supplies the seed, not the run. |
| Prepare a day's perfect run in advance | Stopped when the handler's seed comes from a secret and `start` reveals it only when the day begins. |
| Send the same run twice, or flood | Run IDs make repeats harmless; tokens are rate limited. |
| Submit another player's run | Stopped when each player is given a seed of their own. With a shared seed, a byte-for-byte copy is refused, but a copy with a harmless input added is not. |
| A run that takes forever to replay | Refused above `MAX_TICKS`; stopped by the time limit on Node. |

Accepted limits, which no replay can close:

- **Bots and tool-assisted play.** A replay proves the inputs produce the
  result, not that a person typed them. Input timing checks in a handler,
  reports and moderation are the answer for a small community.
- **Shared accounts.** An account policy matter.
- **Copied runs on a shared seed.** Nothing in a run proves who played it.
  When every player gets the same seed, a player can send another's run
  (with a harmless input added) and tie their score. Seeds of each player's
  own close this, at the cost of a shared course.
- **Phished codes.** An attacker could talk a player into entering the
  attacker's connect code, which connects the player's account to the
  attacker's computer. This is the nature of code sign-in, as on a TV; the
  site's connect page should say which game is asking.
