# Hearken online play

Design, 2026-11-09. The decisions here were made in the Wicket's project
thread "Add online peer play to Hearken"; the first pull request (rooms and
turns) was built from it. Later pull requests (text chat, voice) follow the
plan at the end, subject to the open questions.

## Goal
Hearken gets online play that any game can turn on: matches between players, turn-based or played over days, text chat, and push-to-talk voice. It is built on the verified-runs server Hearken already has, so a match is checked the same way a run is, and no game writes networking code.
First users: the Space Colony Defense redesign (its first multiplayer mode) and Freight Fate V2's text and voice chat. Neither game's name, kinds or rules appear in Hearken: the engine knows rooms, seats, events and voice frames, and a game gives them meaning.
Done when:
- Two copies of the template game, on two computers, play a match by invite code with a text chat beside it, and the server's verified outcome reaches the game's handler.
- A match left mid-way is finished days later from the game's "My games" list.
- Two players hold a push-to-talk conversation inside a room with a screen reader running on both sides.
- Freight Fate can use the chat and voice crates without hearken-app, with its own tokens.
- No frame of any game waits on the network.

## Decisions
The one that needs Orinks' call is the first row: a relay through the server first, peer-to-peer WebRTC later, when a game needs it.
| Decision | Why |
|---|---|
| Relay through the server first; WebRTC later, behind the same API | WebRTC gives the lowest latency, but it needs NAT traversal, a paid TURN relay for the players it cannot connect, a tokio runtime inside the engine (the `webrtc` crate 0.21), and a mesh once a room has more than two people. A WebSocket relay works through every firewall, needs no TURN, carries chat, turns, presence and voice on one connection, and a Cloudflare Durable Object per room hosts it on orinks.net as it is today. Push-to-talk voice tolerates the 100 to 300 ms a relay adds. Games call `Room`, never a transport, so WebRTC can replace the relay for voice or real-time inputs later without a game changing. |
| Rooms are the one primitive | A room has a kind, members (with seats when it is a match), and an ordered event log. A match, a chat channel and a direct message are all rooms. One store, one set of routes, one client. |
| Turns are ordered by the server and verified by replay | A client posts its move; the server appends it with a sequence number and the player's seat; every client applies turns in server order, its own included. At the end the server rebuilds the run file and replays it with the published rules module, exactly as a solo run. Nothing new to trust. |
| Seats ride in the input code | The top 4 bits of a run file's 16-bit input code carry the seat (16 seats, 4,096 codes each). The server stamps the seat, so a player cannot move for another. The run format and `Replayable` trait do not change; multiplayer rules use `Seated<Move>` as their input type. |
| Polling for async play, a live channel for the rest | The HTTP routes alone give turn-based and days-long matches with text chat on any host, polling every 2 s while a room screen is open. The live WebSocket is optional: it pushes events, carries presence and voice. Where a host has no live channel, everything but voice works. |
| Two standalone crates, no `Ctx` | `hearken-rooms` (client, protocol, Room handle) and `hearken-talk` (microphone, Opus, push-to-talk, playback feed) depend on nothing above `hearken-audio`, so Freight Fate uses them with its own screens and tokens, as it uses `hearken-update`. `hearken-app` wraps them as `ctx.rooms` and ready-made screens. |
| Auth is `accounts.player(token)` and nothing more | The server library already resolves a bearer token to a player through the host's `accounts` store. Rooms and the live channel use only that call, so Freight Fate's driver tokens and Wicket's online accounts both work. |
| Opus through `opusic-sys`, already in the tree | Hearken decodes Opus for radio through `symphonia-adapter-libopus`, which builds libopus from source. The encoder is the same library, so voice adds no new C dependency. 48 kHz mono, 20 ms frames, 24 kbps. |
| Push-to-talk by default, one speaker at a time | A player who runs a screen reader through speakers would send it to the room with an open microphone. Push-to-talk keeps speech out of the channel. The relay grants the floor to one speaker per room (CB radio), which keeps a mesh and mixing out of version one. Open mic stays a Settings option for headset players. |
| Blocking threads, no async runtime | As the radio and `ctx.online` do: `ureq` for HTTP, `tungstenite` for the live socket, each on a worker thread that talks to the game loop through channels. A stalled server blocks only its own thread. |
| Text chat goes through the event voice | A chat line is spoken as a gameplay event with its own category in the verbosity table, so a player can turn chat down or off, and it never cuts an urgent line. A key reads the last lines back. |

## How a match works
A match is a room whose kind has rules. A player creates it and gets a six-letter invite code from the same voice-safe alphabet as the connect codes (no O, 0, I, 1, L, S, 5, Z, 2). Others join by code, or "Play with anyone" puts a player into the first open public room of that kind and makes one when none is waiting. Each member takes a seat, 0 upward, in order of joining.
Starting. When the seats the handler asks for are filled, or the host presses Start with at least the minimum, the server asks the handler's `start({ room, players })` for the seed and context, as it does for a solo run. The seed is the server's, never a client's, so no player can prepare the match.
Turns. A move is a list of inputs, as the game's `InputCode` encodes them. The client posts it; the server stamps each code with the player's seat, appends the turn as the next event with a sequence number, and answers with that number. Every client, the mover included, applies turns in server order, one `Recording::step` per turn. The rules are the referee: a move from the wrong seat, or one the rules do not allow, is stepped and ignored, exactly as a hand-written run file would be. A client that is behind reads the log from its last sequence number and catches up; a client that has been closed for a week does the same.
Ending. When the rules report `finished`, any client posts `finish`. The server rebuilds the run file from the log (seed, context and every stamped input, one tick per turn), replays it with the published rules module, and hands the verified outcome and the seat list to the handler's `accept`. The handler decides what it means: a win recorded for seat 1, a shared colony that survived, ratings. Players see the result the handler returns. A match nobody touches for 30 days (the handler can set its own limit) is closed as abandoned, and the handler hears that too.
Asynchronous play. Nothing in this needs both players online. `GET rooms` lists a player's open rooms with whose turn it is and when the last move was made, so a game can offer "My games: Dots, your move, 2 days ago" from its title screen. While a room screen is open the client polls every 2 s, backing off to 10 s after a minute without a change; with the live channel it is told at once.
Leaving. A player who leaves a match in progress forfeits; the rules see a `left` input for that seat and decide what that means. A player who merely disconnects stays seated.
Real time, later. The same room and the same seated run carry a real-time co-op game: each client sends its inputs every tick over the live channel instead of a turn at a time, and the final run is verified the same way. That needs lockstep timing and input delay handling in `hearken-rooms`, which waits until a game asks for it; nothing in this design closes the door.

## State machines
Following Orinks' rule (2026-11-09): every state that was a flag or a nested check is a named state with named transitions, spoken to the player, readable by a developer in one enum, and printed by the agent server's `screen` tool so a play test reads the state instead of guessing it from speech. Five machines, each explicit in code: a `phase` on the room record with one `transition()` in the server library, and Rust enums on the client.
Room phase (server, stored on the room; every event carries the phase it left the room in):
| State | Enters on | Leaves on | The player hears |
|---|---|---|---|
| `waiting` | create, find with no open room | start, full, host leaves alone, idle limit | "Waiting for players. Code M4K7 T8. 1 of 2." |
| `playing` | start by host, last seat filled | rules finish, idle limit | whose move, the rules' summary line |
| `finished` | rules report finished (after a turn, a left, a forfeit) | never; read-only | "Match over. You won." plus the handler's result |
| `closed` | waiting room emptied, or idle 30 days | never | "That game closed." (only from My games) |
| `abandoned` | playing room idle past `idle_limit` | never | "Abandoned, no move in 30 days." |
A `turn`, `chat` or `finish` on a room that is not `playing` is refused with `409 wrong-phase` and the phase it is in, never silently dropped.
Seat (server, per member): `open` → `taken` on join; `taken` → `open` on leave while waiting; `taken` → `left` on leave, forfeit or turn limit while playing (one `Seated::Left` input, never two). Presence is a separate flag on a taken seat, `here` or `away`, from the live socket or the last poll, so leaving and dropping offline are never confused.
Match sync (client, `Match<R>`): `Idle` (my recording matches the server's log) → `Posting` (my turn sent, not yet ordered; input is locked and the screen says "Sending" if it takes over a second) → `Idle` when the ack's `seq` arrives and the turn is applied in order, or `Refused(reason)` spoken once and input unlocked. `Behind` whenever the server's `seq` is ahead of the recording: the client fetches events after its seq and applies them, speaking each other seat's turn as it goes; a game can hold `Behind` and replay at its own pace. The agent server prints `match: Behind 3` so a test knows a turn is still arriving.
Connection (client, `Rooms`): `Polling(interval)` is the base state; `Live` once the WebSocket is open; `Reconnecting(n)` after a live drop, falling back to polling after three tries and saying so once; `Offline` after two failed polls, spoken once as "Offline. Trying again." and cleared silently when a poll succeeds. No state here ever blocks a frame or changes the room's phase.
Voice (client, `hearken-talk`, PR 3): `Quiet` → `Asking` on the talk key → `Talking` when the relay grants the floor (open tone) or `Busy` when it refuses (tick, back to `Quiet` on release) → `Quiet` on release (close tone). `Listening(seat)` runs beside them while another seat holds the floor. `NoMicrophone` replaces `Asking` when there is no input device, spoken once per session.

## Text chat
A chat line is a room event like a turn: `{"type":"chat","text":"..."}`, appended in order, read back with the rest of the log. It works in a match room, in a standing channel (a room of kind `lobby` with open membership that a game creates once), and in a direct message (a two-member room). Freight Fate's public channel and DMs are these.
What the player does. A game binds one key (Settings, movable) that opens a one-line text entry; Enter sends, Escape drops it. Another key reads the last lines back, newest first, one per press. Lines arrive through the event voice as "Ada: nice move" under their own verbosity category, so a player can turn chat to short, full or off, and an urgent game line is never held behind it.
Limits and cleaning. 200 characters a line, 10 lines a minute per player, the last 20 lines handed to a member who joins (more by paging). The server cleans every line as `ctx.online` already cleans server text: no control or invisible characters, so nothing can speak a screen reader into silence. A host can pass `screen(text, player)` to the service; orinks.net has an obscenity matcher to put there, and a line it refuses is answered with a spoken reason and not stored.
Mute and block. A player can mute another from the room's member list; the server keeps the list, so it holds across games and devices and the muted player's lines are left out of what this player reads. Block is mute plus no direct messages and no shared matches from Play with anyone. A report sends the last 20 lines of that room to the host's `report` hook, for a human to read.
Presence. A member list speaks who is in the room and who is here now. Over the live channel presence is exact; over polling, a member is "here" if they have polled in the last 30 s.

## Voice chat
Voice is push-to-talk over the room's live channel. Holding the talk key asks the relay for the floor; when it is granted, the player hears a short open tone and the microphone goes out as Opus frames until the key is released, which ends with a close tone. Everyone else in the room hears the speaker on the voice bus, and the relay refuses a second speaker until the first lets go (the asker hears a short "busy" tick). That is the CB radio model Freight Fate asked for, and it suits a two-seat match as well.
On the wire. 48 kHz mono, 20 ms frames, 24 kbps, encoded by the same libopus Hearken already builds for radio. Each frame is one binary WebSocket message of about 60 bytes plus a 4-byte header (seat, sequence). The relay forwards frames and never stores them. A talk of one minute is about 180 KB.
Hearing it. `hearken-talk` keeps a small jitter buffer per speaker (60 ms, growing to 200 ms under loss) and plays it as a `Feed` on `Bus::VOICE`, so the voice volume slider, ducking and the spatial model apply. A game may place a speaker at their seat's position. Lost frames are concealed by the Opus decoder. While someone speaks, music and ambience duck as they do for speech.
Screen readers. Push-to-talk is the default because a screen reader heard through speakers would otherwise go to the room. Settings offers open mic with a simple level gate for headset players, and "mute my voice while the game speaks", which holds the microphone while the built-in voice or the event voice is speaking (Hearken knows when its own speech plays; it cannot know about the player's screen reader, which is why push-to-talk stays the default). Help says once that headphones keep the game out of your microphone.
The microphone. `hearken-talk` opens the default input device through cpal, follows a device change as the output does, and says plainly when there is none: the talk key then speaks "No microphone" and the room still works for text. Settings lists input devices; the choice is saved. A per-player local mute silences one speaker on this computer alone; the server-side mute above also stops their frames reaching this player.
Later: peer to peer. For a game that wants lower latency than a relay, `hearken-talk` does not change: it hands frames to whatever carries them. A WebRTC transport would use the room's event log for signalling and a `GET ice` route for STUN and short-lived TURN credentials. That is the upgrade path, not version one.

## Protocol and server
The routes join the ones in `docs/online.md`, under the game's base URL, with the bearer token. Any server that speaks them works; `@hearken/online` gains them, with the in-memory stores and the dev server, so a game can be tried on one computer.
| Route | What it does |
|---|---|
| `POST rooms?kind=K` `{context?, open?}` | Makes a room. Answers `{room, code, seat, seats:{min,max}}`. `open` lists it for Play with anyone. |
| `POST rooms/join` `{code}` | Takes the next seat. `404 unknown-code`, `409 full`, `409 started`. |
| `POST rooms/find?kind=K` | Joins the first open room of that kind with a free seat, or makes one. |
| `GET rooms` | My rooms: kind, members, phase, whose turn, last move time, unread count, the progress snapshot. |
| `GET rooms/<id>?after=<seq>` | Members, seats, presence, phase, and every event after `seq`. The client's poll. |
| `POST rooms/<id>/start` | The host starts with at least `seats.min` filled. The server asks the handler's `start` for seed and context. |
| `POST rooms/<id>/events` | One event: `turn {inputs:[[code,value],...]}`, `chat {text}`, `leave`, or `finish`. Answers `{seq}`. |
| `POST rooms/<id>/mute` `{player, on}` | Mute or unmute a member, kept server side; `block` the same with `{block:true}`. |
| `GET rooms/<id>/live` | Upgraded to a WebSocket by the host adapter. Pushes `{event}` as text, carries presence, relays binary voice frames with floor control. Optional. |
| Events are `{seq, at, seat?, player, type, ...}`. A `turn` is stored with its codes seat-stamped. A `chat` is stored cleaned. `finish` makes the server rebuild the run file (format unchanged: seed, context, one tick per turn, the stamped inputs) and replay it with the published build the room named when it started; the handler's `accept({ room, players, verified })` gets the outcome. With `progress: true` on the handler, the server also replays after each turn through a new rules export, `hearken_progress`, which answers the outcome of an unfinished run; its result is the room's snapshot, so "whose turn" costs no client a replay. Rules ABI goes to 2 for that export; ABI 1 modules still verify. | Handler fields for a kind with rooms: `seats: {min, max}` (max 16), `start`, `accept`, optional `progress`, `turn_limit` (seconds a seat may hold a turn before it is given a `left` input; the rules decide what that means), `idle_limit` (30 days by default), `read`. |
| Store interface. One new store beside `accounts`, `runs` and `limiter`, with the same in-memory reference and the same rule that a Convex, SQL or KV store mirrors it: | Method |
| Guarantee | `create(room)`, `get(id)`, `byCode(code)`, `listFor(player)` |
| Codes unique among open rooms. | `append(id, event)` |
| Atomic: one `seq` per event, in order, no gaps. A Convex mutation or an SQL insert under the room's row lock does this. | `eventsAfter(id, seq, limit)` |
| In `seq` order. | `setSnapshot(id, phase, snapshot)`, `setMembers(id, members)` |
| Last write wins. | `mutes(player)`, `setMute(player, other, mode)` |
| Per player, across rooms. | Hosting the live channel. The service stays one `fetch(Request)` function. The live channel is a `RoomLive` object the host attaches sockets to: `live.attach(socket, { player, room })`, where `socket` has `send(data)` and `close()` and the host forwards messages to `live.receive(socket, data)`. `server/examples/cloudflare/room.js` is a Durable Object per room with the Hibernation API; `server/examples/node-server.js` gains `ws`. orinks.net has the Worker already, so its part is the Durable Object binding and the Convex store (its own thread, after this one). A host with no live channel answers `GET rooms/<id>/live` with 404 and games fall back to polling; voice then says "Voice is not available on this server". |
| Limits. Rooms: 20 open per player. Events: 60 a minute per player per room, chat 10 a minute. A room log: 65,536 events or the rules' `MAX_TICKS`, whichever is less; the run file limits (4 MiB, 1,048,576 inputs) still apply at finish. Live: 1 socket per player per room, voice 60 frames a second from the floor holder only, anything else dropped. | ## What a game writes |

A game with rules already split out for verified runs adds three things: seated inputs, `with_rooms()` on its online config, and a screen that drives a `Match`. `docs/online.md`, "Play between players", is the reference; `template/src/online.rs` is the whole of it for Pebbles.

Rules. `hearken_replay::Seated<I>` wraps the game's input with a `Seat` (0 to 15) and implements `InputCode` by putting the seat in the top 4 bits. Rules that want multiplayer use `type Input = Seated<Move>` and check the seat against whose turn it is. Nothing else in `Replayable` changes.

Config. `OnlineConfig::new(url, site).with_rooms()` offers `ctx.rooms` and the screens. Each kind's seats, limits and handler live on the server, as a run kind's do.

Client API (`hearken-rooms`, as `ctx.rooms` in hearken-app). Calls return a `Ticket` and the answer arrives in `update`, as `ctx.online` works now; nothing blocks a frame.

```rust
let mut game = Match::<Pebbles>::new(&room.info)?;   // from the room's seed and context
game.post(&mut ctx.rooms, &id, &[Take(2)])?;         // my move, applied when the server orders it
game.update(&mut ctx.rooms);                          // the answer: ordered, or refused once
for turn in game.apply_all(ctx.rooms.room(&id)?) {    // every turn read but not applied, in order
    speak(turn.seat, &turn.inputs, game.sim());
}
```

Ready-made screens in `hearken-app`: `OnlinePlayScreen` (Create a game, Join with a code, Play with anyone, My games) pushed with a `RoomKind` (the kind, its label, the context Create sends, and how to open a match in play); `RoomScreen`, which speaks the invite code, the players as they arrive, Start for the host and the result; `MyGamesScreen`, reading `next_seat` and `summary` from the snapshot when the rules provide them. Chat and talk keys, and the Voice chat settings, come with the second and third pull requests.

Dustfall (the Space Colony Defense redesign). Two seats with roles set by the rules; the defender's turn is many inputs, the attacker's a short order; `progress: true` on its handler gives the rules' own summary line in My games; a 3-day `turn_limit`; later a 2 to 4 seat mode on one seed. Nothing of this is in Hearken: it is a kind, a context and an outcome.

Freight Fate V2. Uses `hearken-rooms` and `hearken-talk` directly, with its own screens and driver tokens, the way it uses `hearken-update`: a `lobby` room for the public channel, two-member rooms for direct messages, voice over the lobby's live channel. Its server implements `accounts.player(token)` over driver tokens and the rooms store in its Convex functions, and passes orinks.net's obscenity matcher as `screen`.

Template. The template game has the two-seat example (Pebbles: take 1 to 3 from a pile, last take loses) behind its `online` feature, so the whole path is tested end to end and a new game has it to copy.

## Build plan
Three Hearken pull requests, each played before it merges, in this order. Dustfall's online mode starts once the first is up; Freight Fate's chat once the third is.
- Rooms and turns (`hearken-replay` Seated inputs and `hearken_progress`; `@hearken/online` rooms routes, store, memory store, dev server; `hearken-rooms` client with polling; `ctx.rooms`, the Online play, Room and My games screens; Pebbles in the template). Tests: unit tests on the store and routes in `server/test`, a two-client match through the dev server in Rust, `assert_replays` on seated runs, a Headless test of the screens. Play test: two agent-driven copies of the template play Pebbles by code on one dev server, with speech recorded, through tutorial, a full match, a leave, and a match resumed after a restart.
- Text chat (chat events, cleaning, mute and block, the chat keys and screens, the verbosity category, `screen` and `report` hooks). Play test: both copies chatting mid-match, chat turned down and off, a muted player, lines read back.
- Voice (`hearken-talk`: cpal input, Opus, push-to-talk, jitter buffer and Feed; the live channel in `@hearken/online` with the Node adapter and the Durable Object example; `hearken-rooms` live socket with reconnect; Settings rows). Tests: encode and decode round trip, floor control in `server/test`, a loopback through the Node relay with a synthesized microphone. Play test: two copies on one machine, each with its own virtual microphone, talking in a room with speech-dispatcher running; the busy tick; no microphone; the relay missing.
Out of this thread: orinks.net's Convex store and Durable Object binding (its own thread after PR 1), the Dustfall and Freight Fate game code, WebRTC peer-to-peer and real-time lockstep (when a game needs them).
Open questions (for the project owner)
- Relay first, WebRTC later, as the Decisions table proposes? The alternative is peer-to-peer voice in version one, with Cloudflare's TURN service as the fallback for players who cannot connect directly, about two weeks more work and a tokio runtime inside the engine.
- Push-to-talk as the default for everyone, with open mic as a Settings option?
- Chat, chat history and talk as three engine-wide keys, like F1 and F2, so every Hearken game shares them? Proposed defaults: T, Shift+T and V, all movable.
- Guests allowed in matches and chat, as on the boards? A guest's chat lines carry the guest name; blocking a guest blocks the device's token.

## Sources
- Hearken `docs/online.md` and `server/` (the verified-runs server and protocol this extends), `crates/hearken-app/src/online.rs`, `crates/hearken-audio/src/feed.rs`.
- Freight Fate chat needs: the thread Add text and voice chat to Freight Fate V2. Dustfall's needs: the Space Colony Defense redesign thread.
- webrtc crate 0.21.1 (Tokio runtime) and str0m 0.25 (no TURN client, no codec), read 2026-11-09.
- Cloudflare Durable Objects pricing: free plan 100,000 requests a day, incoming WebSocket messages counted 20 to 1, read 2026-11-09.
