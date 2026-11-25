// Rooms: matches between players, chat channels and direct messages, all
// one thing to the server. A room has a kind, members in seats, a phase,
// and an ordered log of events (turns, chat, seats leaving). A match is a
// room whose kind has rules: the server orders the turns, stamps each with
// the seat that sent it, and replays them with the game's published rules
// module (`hearken_play`) to learn where the match stands and, at the end,
// what it came to. Nothing the clients say about the outcome is read.
//
// A kind with seats but no rules is a channel: a room that is open as soon
// as it is made, where members come and go and only chat is posted. A
// channel joined by name (`rooms/find?kind=K&name=N`) is made the first
// time anyone asks for it and stays open. A direct room (`rooms/direct`)
// is a channel of two, found or made by the other player's handle.
//
// Routes, relative to the base URL (all with a bearer token):
//   POST rooms?kind=<kind>        { context?, open? }: a new room, with an invite code
//   POST rooms/join               { code }: take the next seat
//   POST rooms/find?kind=<kind>   join an open room of that kind, or make one;
//                                 &name=<name> names a channel to join or make
//   POST rooms/direct?kind=<kind> { handle }: the two-member room with that player
//   GET  rooms                    my rooms
//   GET  rooms/<id>?after=<seq>   the room and its events after seq
//   GET  rooms/<id>?last=<n>      the room and its last n events (&before=<seq> pages back)
//   POST rooms/<id>/start         the host starts with enough seats filled
//   POST rooms/<id>/events        { type: "turn" | "chat" | "leave" | "finish", ... }
//                                 (leave on a room that is over drops it from my rooms)
//   POST rooms/<id>/report        { handle, note? }: hand a member's recent lines to the host
//   GET  rooms/mutes              who I have muted or blocked
//   POST rooms/mutes              { handle, mode: "mute" | "block" | "none" }
//   POST rooms/pulse              { state, game?, place?, show?, heard? }: I am here;
//                                 answers who is here, my friends, invites, rooms
//                                 and open tables (lobby.js)
//   POST rooms/invites            { handle, kind, context? }: invite a player to a match
//   POST rooms/invites/<id>       { answer: "accept" | "decline" | "withdraw" }
//   POST rooms/friends            { handle, friend: true | false }
//
// Players are known to each other by handle: a short stable id made from
// the account, never the account id itself. Mutes and blocks are kept per
// handle pair and never shown to the other side: a muted player's lines
// are left out of what the muter reads; a block does that both ways and
// keeps the two out of each other's direct rooms and Play-with-anyone games.
//
// The room's phase is an explicit state machine (`PHASES`, `transition`):
//   waiting   -> playing   start, or the last seat filled
//   waiting   -> closed    the last member left, or idle past the limit
//   playing   -> finished  the rules ended (a turn, a seat leaving, a forfeit)
//   playing   -> abandoned idle past the handler's idle_limit
//   open      -> closed    a channel emptied or idle past its limit (a named
//                          channel stays open), or a direct room left
// finished, closed and abandoned are final. An event on a room in the
// wrong phase is refused as 409 wrong-phase, naming the phase.
//
// docs/online.md in the Hearken repository is the protocol in full.

import { Refusal, refuse } from "./refusal.js";
import { sha256Hex } from "./rules.js";
import {
  FINAL_INVITE_STATES,
  INVITE_ANSWERS,
  LOBBY_LIMITS,
  inviteTransition,
  memoryPresence,
  presenceOf,
  publicInvite,
  publicPresence,
  visibilityOf,
  visibleTo,
} from "./lobby.js";

export const PHASES = ["waiting", "playing", "finished", "closed", "abandoned", "open"];
export const FINAL_PHASES = ["finished", "closed", "abandoned"];

/** The phase changes a room may make. Anything else is a bug, not a state. */
const TRANSITIONS = {
  waiting: ["playing", "closed"],
  playing: ["finished", "abandoned"],
  open: ["closed"],
  finished: [],
  closed: [],
  abandoned: [],
};

/** `room` moved to `phase`, or throws: a room never jumps a phase it cannot. */
export function transition(room, phase, now) {
  if (!TRANSITIONS[room.phase]?.includes(phase)) {
    throw new Error(`a room cannot go from ${room.phase} to ${phase}`);
  }
  room.phase = phase;
  room.updatedAt = now;
  if (phase === "playing") room.turnStartedAt = now;
  return room;
}

// Invite codes: six letters a voice does not blur (no O/0, I/1/L, S/5, Z/2),
// the alphabet the connect codes use.
export const ROOM_CODE_ALPHABET = "ABCDEFGHJKMNPQRTUVWXY346789";
export const ROOM_CODE_LENGTH = 6;

export function mintRoomCode() {
  const bytes = new Uint8Array(ROOM_CODE_LENGTH);
  crypto.getRandomValues(bytes);
  let code = "";
  for (const b of bytes) code += ROOM_CODE_ALPHABET[b % ROOM_CODE_ALPHABET.length];
  return code;
}

/** What a player typed, as a room code: any case, spaces and dashes dropped. */
export function normalizeRoomCode(text) {
  const raw = String(text ?? "").toUpperCase().replace(/[\s-]+/g, "");
  if (raw.length !== ROOM_CODE_LENGTH || [...raw].some((c) => !ROOM_CODE_ALPHABET.includes(c))) return "";
  return raw;
}

/** Seats in a match: the seat rides in the top four bits of each input code. */
export const MAX_SEATS = 16;
/** Members of a channel, which has no input codes to fit them in. */
export const MAX_CHANNEL_SEATS = 500;
export const MAX_SEATED_CODE = 0xffe;
export const MAX_INPUTS_PER_TURN = 256;
export const MAX_CHAT_CHARS = 200;
export const MAX_TAG_CHARS = 64;
export const MAX_NAME_CHARS = 32;
export const MAX_EVENTS = 65_536;
export const PAGE = 500;
/** How many lines a history read hands back at most. */
export const MAX_HISTORY = 100;
/** The lines a report carries: the reported member's most recent ones. */
export const REPORT_LINES = 20;
export const HANDLE_LENGTH = 16;
export const MUTE_MODES = ["mute", "block", "none"];

/** A line as it may be spoken: no control or invisible characters, one space between words. */
export function cleanText(text, max = MAX_CHAT_CHARS) {
  let out = "";
  for (const ch of String(text ?? "")) {
    const cp = ch.codePointAt(0);
    const control = cp < 0x20 || (cp >= 0x7f && cp <= 0x9f);
    const invisible =
      (cp >= 0x200b && cp <= 0x200f) ||
      (cp >= 0x202a && cp <= 0x202e) ||
      (cp >= 0x2060 && cp <= 0x2064) ||
      cp === 0xfeff ||
      (cp >= 0xfff9 && cp <= 0xfffb);
    if (control || invisible) continue;
    out += ch;
  }
  out = out.replace(/\s+/g, " ").trim();
  return [...out].slice(0, max).join("");
}

/** A channel name as the store keys it: lower case, one space between words. */
export function normalizeChannelName(text) {
  return cleanText(text, MAX_NAME_CHARS).toLowerCase();
}

/**
 * A player's handle: what other players know them by. Stable for the
 * account, short enough to compare, and not the account id, which never
 * leaves the server.
 */
export async function handleOf(playerId, key = "") {
  // With a key, a handle cannot be worked out from an account id someone
  // has seen elsewhere (a public profile page, say).
  const input = key ? `hearken-handle:${key}:${playerId}` : `hearken-handle:${playerId}`;
  const hex = await sha256Hex(new TextEncoder().encode(input));
  return hex.slice(0, HANDLE_LENGTH);
}

export function isHandle(text) {
  return typeof text === "string" && new RegExp(`^[0-9a-f]{${HANDLE_LENGTH}}$`).test(text);
}

function randomId() {
  const bytes = new Uint8Array(12);
  crypto.getRandomValues(bytes);
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

/**
 * Rooms in memory: for the dev server, tests, and as the reference for a
 * real store. What a store backed by Convex, SQL or a key-value store must
 * do the same way:
 *
 * - `append` is atomic: one `seq` per event, in order, with no gaps. Two
 *   appends to one room at once get different seqs and both are kept.
 * - `create` gives the room an id and keeps its code unique among rooms
 *   that are not in a final phase, and a named channel unique for its kind.
 * - `save` writes the room record as a whole; the service reads, changes
 *   and saves a room within one request, and a store that can should do
 *   that under the room's lock (a Convex mutation does).
 * - `prune` drops a room's events older than a moment, keeping `seq`
 *   numbering as it was: a client's `after` cursor still means the same.
 * - Players are remembered by handle (`rememberPlayer`, `playerByHandle`)
 *   so a direct room can seat someone who is not in the room yet, and
 *   `relations` answers who a player has muted or blocked, and who has
 *   blocked them, in one read.
 * - The lobby keeps invites (`saveInvite`, `getInvite`, `invitesFor`,
 *   `dropInvite`) and one-sided friends (`setFriend`, `friendsOf`). Presence
 *   is not stored: it lives in memory (lobby.js `memoryPresence`).
 */
export function memoryRooms() {
  const rooms = new Map(); // id -> room
  const logs = new Map(); // id -> events
  const players = new Map(); // handle -> { id, name, guest, handle }
  const mutes = new Map(); // player id -> Map(handle -> "mute" | "block")
  const blockedBy = new Map(); // player id -> Set(handle of who blocked them)
  const invites = new Map(); // id -> invite
  const friends = new Map(); // player id -> Set(handle)
  return {
    async create(room) {
      const id = randomId();
      let code = room.code;
      while (!code || [...rooms.values()].some((r) => r.code === code && !FINAL_PHASES.includes(r.phase))) {
        code = mintRoomCode();
      }
      const made = { ...room, id, code, seq: 0 };
      rooms.set(id, made);
      logs.set(id, []);
      return made;
    },
    async get(id) {
      return rooms.get(id) ?? null;
    },
    async save(room) {
      rooms.set(room.id, room);
    },
    async byCode(code) {
      for (const room of rooms.values()) {
        if (room.code === code && !FINAL_PHASES.includes(room.phase)) return room;
      }
      return null;
    },
    /** The named channel of `kind` that is not over, if any. */
    async byName(kind, name) {
      for (const room of rooms.values()) {
        if (room.kind === kind && room.name === name && !FINAL_PHASES.includes(room.phase)) return room;
      }
      return null;
    },
    /** The room of `kind` between exactly these two players that is not over. */
    async directBetween(kind, a, b) {
      for (const room of rooms.values()) {
        if (room.kind !== kind || FINAL_PHASES.includes(room.phase) || !room.direct) continue;
        const ids = room.members.map((m) => m.id);
        if (ids.length === 2 && ids.includes(a) && ids.includes(b)) return room;
      }
      return null;
    },
    /** Every room `playerId` has a seat in, newest first. */
    async listFor(playerId) {
      return [...rooms.values()]
        .filter((r) => r.members.some((m) => m.id === playerId))
        .sort((a, b) => b.updatedAt - a.updatedAt);
    },
    /** Open rooms of `kind` still waiting, with a free seat, oldest first. */
    async openRooms(kind) {
      return [...rooms.values()]
        .filter((r) => r.kind === kind && r.open && r.phase === "waiting")
        .sort((a, b) => a.createdAt - b.createdAt);
    },
    async append(id, event) {
      const room = rooms.get(id);
      const log = logs.get(id);
      if (!room || !log) throw new Error("no such room");
      const seq = room.seq + 1;
      room.seq = seq;
      log.push({ ...event, seq });
      return seq;
    },
    async eventsAfter(id, seq, limit = PAGE) {
      const log = logs.get(id) ?? [];
      const out = [];
      for (const e of log) {
        if (e.seq <= seq) continue;
        out.push(e);
        if (out.length >= limit) break;
      }
      return out;
    },
    /** The last `limit` events with a seq below `seq` (or the last ones at all), oldest first. */
    async eventsBefore(id, seq, limit = PAGE) {
      const log = logs.get(id) ?? [];
      const out = [];
      for (let i = log.length - 1; i >= 0 && out.length < limit; i -= 1) {
        if (seq !== null && log[i].seq >= seq) continue;
        out.push(log[i]);
      }
      return out.reverse();
    },
    async prune(id, beforeAt) {
      const log = logs.get(id);
      if (!log) return;
      let keep = 0;
      while (keep < log.length && log[keep].at < beforeAt) keep += 1;
      if (keep > 0) log.splice(0, keep);
    },
    async rememberPlayer(player) {
      players.set(player.handle, { id: player.id, name: player.name, guest: Boolean(player.guest), handle: player.handle });
    },
    async playerByHandle(handle) {
      return players.get(handle) ?? null;
    },
    async relations(playerId) {
      const mine = mutes.get(playerId) ?? new Map();
      return {
        muted: new Set([...mine].filter(([, mode]) => mode === "mute").map(([h]) => h)),
        blocked: new Set([...mine].filter(([, mode]) => mode === "block").map(([h]) => h)),
        blockedBy: new Set(blockedBy.get(playerId) ?? []),
      };
    },
    /** `mode` is "mute", "block" or "none" (forget). `them` is the other player, if known. */
    async setMute(player, handle, mode, them) {
      const mine = mutes.get(player.id) ?? new Map();
      mutes.set(player.id, mine);
      if (mode === "none") mine.delete(handle);
      else mine.set(handle, mode);
      if (them) {
        const set = blockedBy.get(them.id) ?? new Set();
        blockedBy.set(them.id, set);
        if (mode === "block") set.add(player.handle);
        else set.delete(player.handle);
      }
    },
    // The lobby (lobby.js): invites and friends.
    /** Writes an invite as a whole, new or changed. */
    async saveInvite(invite) {
      invites.set(invite.id, structuredClone(invite));
    },
    async getInvite(id) {
      const invite = invites.get(id);
      return invite ? structuredClone(invite) : null;
    },
    /** Every invite `playerId` sent or was sent, newest first. */
    async invitesFor(playerId) {
      return [...invites.values()]
        .filter((i) => i.from.id === playerId || i.to.id === playerId)
        .sort((a, b) => b.at - a.at)
        .map((i) => structuredClone(i));
    },
    async dropInvite(id) {
      invites.delete(id);
    },
    /** Adds (`on`) or removes the player with `handle` from `playerId`'s friends. */
    async setFriend(playerId, handle, on) {
      const set = friends.get(playerId) ?? new Set();
      friends.set(playerId, set);
      if (on) set.add(handle);
      else set.delete(handle);
    },
    /** The handles `playerId` has added as friends. */
    async friendsOf(playerId) {
      return [...(friends.get(playerId) ?? [])];
    },
  };
}

function json(body, status = 200, headers = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store", ...headers },
  });
}

function error(status, code, message, extra) {
  return json({ error: code, ...(message ? { message } : {}), ...extra }, status);
}

/** A room as a client sees it: no player ids, seats and handles instead. */
function publicRoom(room, player, now, presentMs) {
  const mine = room.members.find((m) => m.id === player.id);
  return {
    id: room.id,
    kind: room.kind,
    code: room.code,
    name: room.name ?? null,
    direct: Boolean(room.direct),
    phase: room.phase,
    open: room.open,
    seats: room.seats,
    host: room.members.find((m) => m.host)?.seat ?? 0,
    members: room.members.map((m) => ({
      seat: m.seat,
      name: m.name,
      handle: m.handle ?? null,
      guest: Boolean(m.guest),
      here: m.state === "taken" && now - m.lastSeen < presentMs,
      left: m.state === "left",
    })),
    you: mine?.seat ?? null,
    seed: room.seed ?? null,
    context: room.context ?? null,
    seq: room.seq,
    snapshot: room.snapshot ?? null,
    result: room.result ?? null,
    updated_at: room.updatedAt,
    turn_started_at: room.turnStartedAt ?? null,
  };
}

/**
 * The rooms routes. `deps` is the service's: `rooms` (the store),
 * `accounts`, `limiter`, `rules`, `handlers`, `log`, `now`, `limits`,
 * `screen`, `report`, `handleKey`, plus `bearer(request)` and
 * `readJson(request)` from the service.
 */
export function roomRoutes(deps) {
  const { rooms, accounts, limiter, rules, handlers, log, bearer, readJson } = deps;
  const now = deps.now ?? (() => Date.now());
  const handleKey = deps.handleKey ?? "";
  const limits = {
    openRoomsPerPlayer: 20,
    roomEventsPerMinute: 60,
    chatPerMinute: 10,
    chatGapMs: 0,
    roomsPerHour: 30,
    presentMs: 30_000,
    idleMs: 30 * 86_400_000,
    // A channel keeps its lines this long.
    historyMs: 7 * 86_400_000,
    mutesPerPlayer: 500,
    // A channel member not seen this long has gone: their seat is freed,
    // so a busy channel does not fill with players who stopped listening.
    channelAwayMs: 15 * 60_000,
    ...deps.limits,
  };

  function roomHandler(kind) {
    if (!Object.hasOwn(handlers, kind)) return null;
    const handler = handlers[kind];
    if (!handler.seats) return null;
    return handler;
  }

  /** A channel: seats, but no rules to play. */
  function isChannel(handler) {
    return Boolean(handler) && typeof handler.rules !== "string";
  }

  function seatsOf(handler) {
    const cap = isChannel(handler) ? MAX_CHANNEL_SEATS : MAX_SEATS;
    const min = Math.max(1, Number(handler.seats.min ?? 2));
    const max = Math.min(cap, Math.max(min, Number(handler.seats.max ?? min)));
    return { min, max };
  }

  function historyMsOf(handler) {
    return handler?.history === undefined ? limits.historyMs : Math.max(0, Number(handler.history)) * 1000;
  }

  async function who(request) {
    const token = bearer(request);
    const player = token ? await accounts.player(token) : null;
    if (!player) return null;
    const handle = await handleOf(player.id, handleKey);
    const known = { ...player, handle };
    await rooms.rememberPlayer(known);
    return known;
  }

  /** A turn's inputs as the rules module takes them, from the whole log. */
  async function turnsOf(room) {
    const turns = [];
    let after = 0;
    for (;;) {
      const page = await rooms.eventsAfter(room.id, after, PAGE);
      for (const e of page) {
        if (e.type === "turn") turns.push({ seat: e.seat, inputs: e.inputs });
        if (e.type === "left") turns.push({ seat: e.seat, left: true });
      }
      if (page.length < PAGE) break;
      after = page.at(-1).seq;
    }
    return turns;
  }

  /** Replays the match so far; finishes the room when the rules have ended. */
  async function settle(room, handler, { finish = false } = {}) {
    const published = await rules.get(room.build);
    if (!published) throw refuse("unknown-build", "This match's rules are no longer on the server.");
    const turns = await turnsOf(room);
    const played = published.play({ rules: handler.rules, seed: room.seed, context: room.context, turns, finish });
    if (!played.ok) throw refuse(played.error);
    room.snapshot = { ticks: played.ticks, finished: played.finished, outcome: played.outcome };
    if (played.finished) {
      transition(room, "finished", now());
      const verified = {
        rules: handler.rules,
        seed: String(room.seed),
        build: room.build,
        ticks: played.ticks,
        outcome: played.outcome,
      };
      try {
        room.result =
          (await handler.accept({ kind: room.kind, room: publicRoom(room, { id: null }, now(), limits.presentMs), players: room.members, verified })) ??
          null;
      } catch (e) {
        log({ error: String(e?.stack ?? e), note: "a match finished but its handler failed" });
        room.result = null;
      }
    }
    return played;
  }

  /** Idle and turn limits, applied when a room is read. */
  async function sweep(room, handler) {
    const t = now();
    if (room.phase === "open") {
      // A channel forgets lines past its history, and a channel nobody
      // named closes once idle. A named channel stays, however quiet.
      const history = historyMsOf(handler);
      if (history > 0) await rooms.prune(room.id, t - history);
      // Members who stopped listening give up their seats. A direct room
      // keeps both: it stays on each player's list until one leaves.
      if (!room.direct && limits.channelAwayMs > 0) {
        const away = room.members.filter((m) => t - m.lastSeen > limits.channelAwayMs);
        for (const m of away) {
          await rooms.append(room.id, { type: "leave", seat: m.seat, name: m.name, handle: m.handle, at: t, reason: "away", phase: room.phase });
        }
        if (away.length > 0) {
          const wasHost = away.some((m) => m.host);
          room.members = room.members.filter((m) => !away.includes(m));
          if (wasHost && room.members.length > 0 && !room.members.some((m) => m.host)) room.members[0].host = true;
          if (room.members.length === 0 && !room.name) transition(room, "closed", t);
          await rooms.save(room);
          if (room.phase !== "open") return;
        }
      }
      // A named channel never closes, whatever the kind's idle limit.
      if (room.name) return;
      const idle = handler?.idle_limit !== undefined ? handler.idle_limit * 1000 : limits.idleMs;
      if (idle > 0 && t - room.updatedAt > idle) {
        transition(room, "closed", t);
        await rooms.save(room);
      }
      return;
    }
    const idle = handler?.idle_limit ? handler.idle_limit * 1000 : limits.idleMs;
    if (room.phase === "waiting" && t - room.updatedAt > idle) {
      transition(room, "closed", t);
      await rooms.save(room);
      return;
    }
    if (room.phase === "playing" && t - room.updatedAt > idle) {
      transition(room, "abandoned", t);
      await rooms.save(room);
      return;
    }
    if (room.phase === "playing" && handler?.turn_limit && handler.progress) {
      const next = room.snapshot?.outcome?.next_seat;
      if (Number.isInteger(next) && t - room.turnStartedAt > handler.turn_limit * 1000) {
        const member = room.members.find((m) => m.seat === next && m.state === "taken");
        if (member) {
          member.state = "left";
          await rooms.append(room.id, { type: "left", seat: next, player: member.id, at: t, reason: "turn-limit" });
          room.updatedAt = t;
          room.turnStartedAt = t;
          await settle(room, handler);
          await rooms.save(room);
        }
      }
    }
  }

  async function seat(room, player, { host = false } = {}) {
    const taken = new Set(room.members.filter((m) => m.state !== "open").map((m) => m.seat));
    let seatNo = 0;
    while (taken.has(seatNo)) seatNo += 1;
    room.members.push({
      id: player.id,
      name: player.name,
      handle: player.handle ?? (await handleOf(player.id, handleKey)),
      guest: Boolean(player.guest),
      seat: seatNo,
      state: "taken",
      host,
      lastSeen: now(),
      lastRead: 0,
      lastChat: 0,
    });
    return seatNo;
  }

  function taken(room) {
    return room.members.filter((m) => m.state === "taken").length;
  }

  async function begin(room, handler) {
    const published = await rules.forRules(handler.rules);
    if (!published) throw refuse("no-rules", "This server has no rules for that game yet.");
    const players = room.members.map((m) => ({ id: m.id, name: m.name, seat: m.seat, guest: m.guest }));
    const plan = await handler.start({ kind: room.kind, room: publicRoom(room, { id: null }, now(), limits.presentMs), players, context: room.context });
    if (!plan || plan.seed === undefined) throw new Error(`the handler for "${room.kind}" started no seed`);
    room.seed = String(plan.seed);
    room.context = plan.context ?? room.context ?? null;
    room.build = published.build;
    transition(room, "playing", now());
    await rooms.append(room.id, { type: "start", at: now(), seed: room.seed, context: room.context, build: room.build, phase: "playing" });
    if (handler.progress) await settle(room, handler);
  }

  /** Whether `a` and `b` have blocked each other, either way. */
  function blockedPair(relationsA, memberB) {
    return relationsA.blocked.has(memberB.handle) || relationsA.blockedBy.has(memberB.handle);
  }

  /** A new room of `kind` for `player`, hosted by them; a channel is open at once. */
  async function makeRoom(kind, handler, player, { open = false, context = null, name = null, direct = false, members = [] } = {}) {
    const seats = seatsOf(handler);
    const t = now();
    const room = await rooms.create({
      kind,
      phase: isChannel(handler) ? "open" : "waiting",
      open: Boolean(open),
      seats,
      members: [],
      context,
      name,
      direct,
      createdAt: t,
      updatedAt: t,
    });
    const seatNo = await seat(room, player, { host: true });
    // In a channel, members come and go, so everyone's coming is logged.
    if (room.phase === "open") {
      await rooms.append(room.id, { type: "join", seat: seatNo, name: player.name, handle: player.handle, at: t, phase: room.phase });
    }
    for (const other of members) {
      const seatNo = await seat(room, other);
      await rooms.append(room.id, { type: "join", seat: seatNo, name: other.name, handle: other.handle, at: t, phase: room.phase });
    }
    await rooms.save(room);
    return { room, seat: seatNo };
  }

  async function allowedToMake(player) {
    if (!(await limiter.allow(`rooms:${player.id}`, limits.roomsPerHour, 3_600_000))) return error(429, "rate-limited");
    const mine = (await rooms.listFor(player.id)).filter((r) => !FINAL_PHASES.includes(r.phase));
    if (mine.length >= limits.openRoomsPerPlayer) return error(422, "too-many-rooms", "You have too many open games.");
    return null;
  }

  async function create(request, url) {
    const kind = url.searchParams.get("kind") ?? "";
    const handler = roomHandler(kind);
    if (!handler) return error(404, "unknown-kind");
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const refused = await allowedToMake(player);
    if (refused) return refused;
    const body = (await readJson(request, 16 * 1024)) ?? {};
    const { room, seat: seatNo } = await makeRoom(kind, handler, player, { open: body.open, context: body.context ?? null });
    return json({ ok: true, room: publicRoom(room, player, now(), limits.presentMs), seat: seatNo });
  }

  async function join(request) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const body = (await readJson(request)) ?? {};
    const code = normalizeRoomCode(body.code);
    if (!code) return error(404, "unknown-code");
    if (!(await limiter.allow(`join:${player.id}`, limits.roomEventsPerMinute, 60_000))) {
      return error(429, "rate-limited");
    }
    const room = await rooms.byCode(code);
    if (!room) return error(404, "unknown-code");
    return joinRoom(room, player);
  }

  async function joinRoom(room, player) {
    const handler = roomHandler(room.kind);
    if (!handler) return error(404, "unknown-kind");
    await sweep(room, handler);
    const t = now();
    const already = room.members.find((m) => m.id === player.id);
    if (already) {
      already.lastSeen = t;
      already.name = player.name;
      await rooms.save(room);
      return json({ ok: true, room: publicRoom(room, player, t, limits.presentMs), seat: already.seat });
    }
    if (room.phase !== "waiting" && room.phase !== "open") {
      return error(409, room.phase === "playing" ? "started" : "wrong-phase", undefined, { phase: room.phase });
    }
    if (taken(room) >= room.seats.max) return error(409, "full");
    const seatNo = await seat(room, player);
    room.updatedAt = t;
    await rooms.append(room.id, { type: "join", seat: seatNo, name: player.name, handle: player.handle, at: t, phase: room.phase });
    if (room.phase === "waiting" && taken(room) >= room.seats.max) {
      try {
        await begin(room, handler);
      } catch (e) {
        if (e instanceof Refusal) {
          await rooms.save(room);
          return error(422, e.code, e.message);
        }
        throw e;
      }
    }
    await rooms.save(room);
    return json({ ok: true, room: publicRoom(room, player, t, limits.presentMs), seat: seatNo });
  }

  async function find(request, url) {
    const kind = url.searchParams.get("kind") ?? "";
    const handler = roomHandler(kind);
    if (!handler) return error(404, "unknown-kind");
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const name = normalizeChannelName(url.searchParams.get("name") ?? "");
    if (name) {
      // A channel by name: everyone who asks for it lands in the same room.
      if (!isChannel(handler)) return error(422, "not-a-channel", "That kind is played, not joined by name.");
      if (!(await limiter.allow(`join:${player.id}`, limits.roomEventsPerMinute, 60_000))) return error(429, "rate-limited");
      const existing = await rooms.byName(kind, name);
      if (existing) return joinRoom(existing, player);
      const { room, seat: seatNo } = await makeRoom(kind, handler, player, { open: true, name });
      return json({ ok: true, room: publicRoom(room, player, now(), limits.presentMs), seat: seatNo });
    }
    const relations = await rooms.relations(player.id);
    for (const room of await rooms.openRooms(kind)) {
      if (room.members.some((m) => m.id === player.id)) continue;
      if (taken(room) >= room.seats.max) continue;
      // Neither side of a block is paired with the other.
      if (room.members.some((m) => blockedPair(relations, m))) continue;
      return joinRoom(room, player);
    }
    const refused = await allowedToMake(player);
    if (refused) return refused;
    const { room, seat: seatNo } = await makeRoom(kind, handler, player, { open: true });
    return json({ ok: true, room: publicRoom(room, player, now(), limits.presentMs), seat: seatNo });
  }

  /** The two-member room of `kind` with another player, found or made. */
  async function direct(request, url) {
    const kind = url.searchParams.get("kind") ?? "";
    const handler = roomHandler(kind);
    if (!handler) return error(404, "unknown-kind");
    if (!isChannel(handler) || seatsOf(handler).max !== 2) return error(422, "not-direct", "That kind is not for two.");
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const body = (await readJson(request)) ?? {};
    if (!isHandle(body.handle)) return error(400, "bad-request");
    if (body.handle === player.handle) return error(422, "not-direct", "That is you.");
    const them = await rooms.playerByHandle(body.handle);
    if (!them) return error(404, "unknown-player", "No player by that handle.");
    const relations = await rooms.relations(player.id);
    if (blockedPair(relations, them)) return error(403, "blocked");
    const existing = await rooms.directBetween(kind, player.id, them.id);
    if (existing) {
      await sweep(existing, handler);
      if (existing.phase === "open") return joinRoom(existing, player);
    }
    const refused = await allowedToMake(player);
    if (refused) return refused;
    const { room, seat: seatNo } = await makeRoom(kind, handler, player, { direct: true, members: [them] });
    return json({ ok: true, room: publicRoom(room, player, now(), limits.presentMs), seat: seatNo });
  }

  async function list(request) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const t = now();
    const out = [];
    for (const room of await rooms.listFor(player.id)) {
      await sweep(room, roomHandler(room.kind));
      const me = room.members.find((m) => m.id === player.id);
      // Gone from a channel they stopped listening to, or done with a room.
      if (!me || me.dismissed) continue;
      out.push({
        ...publicRoom(room, player, t, limits.presentMs),
        unread: Math.max(0, room.seq - (me?.lastRead ?? 0)),
      });
    }
    return json({ ok: true, rooms: out });
  }

  /** The handles whose chat `player` does not hear in `room`: muted, blocked, or blocking them. */
  async function silenced(player) {
    const relations = await rooms.relations(player.id);
    return new Set([...relations.muted, ...relations.blocked, ...relations.blockedBy]);
  }

  async function get(request, url, id) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const room = await rooms.get(id);
    if (!room) return error(404, "not-found");
    const me = room.members.find((m) => m.id === player.id);
    if (!me) return error(403, "not-a-member");
    const handler = roomHandler(room.kind);
    // Reading is being here: seen before the sweep, so it keeps the seat.
    me.lastSeen = now();
    await sweep(room, handler);
    const t = now();
    const quiet = await silenced(player);
    const audible = (e) => !(e.type === "chat" && e.handle && quiet.has(e.handle));
    const lastParam = url.searchParams.get("last");
    if (lastParam !== null) {
      // History: the last events the reader hears, or the ones before a
      // seq, oldest first. `only=chat` counts lines and leaves out joins
      // and leaves, so a busy channel's arrivals do not crowd out what
      // was said.
      const last = Math.min(MAX_HISTORY, Math.max(1, Number(lastParam) || 0));
      const beforeParam = url.searchParams.get("before");
      const before = beforeParam === null ? null : Math.max(0, Number(beforeParam) || 0);
      const onlyChat = url.searchParams.get("only") === "chat";
      const wanted = (e) => audible(e) && (!onlyChat || e.type === "chat");
      const events = [];
      // Where the scan stopped: the earliest seq looked at, so the next
      // page back starts there even when it held nothing the reader hears.
      let cursor = before;
      let more = false;
      // A bounded scan: a reader who has muted a whole channel still gets
      // an answer, and pages back from where it stopped.
      for (let pages = 0; pages < 4 && events.length < last; pages += 1) {
        const page = await rooms.eventsBefore(room.id, cursor, PAGE);
        if (page.length === 0) {
          more = false;
          break;
        }
        let i = page.length - 1;
        for (; i >= 0 && events.length < last; i -= 1) {
          if (wanted(page[i])) events.unshift(page[i]);
        }
        cursor = page[i + 1].seq;
        more = i >= 0 || page.length === PAGE;
        if (!more) break;
      }
      if (before === null) me.lastRead = room.seq;
      await rooms.save(room);
      return json({
        ok: true,
        room: publicRoom(room, player, t, limits.presentMs),
        events: events.map(publicEvent),
        first_seq: cursor,
        // Where the reader now stands: the latest line, read or left out.
        read_to: before === null ? room.seq : null,
        more,
      });
    }
    const after = Math.max(0, Number(url.searchParams.get("after") ?? "0") || 0);
    const events = await rooms.eventsAfter(room.id, after, PAGE);
    if (events.length > 0) me.lastRead = Math.max(me.lastRead ?? 0, events.at(-1).seq);
    else me.lastRead = Math.max(me.lastRead ?? 0, Math.min(after, room.seq));
    await rooms.save(room);
    return json({
      ok: true,
      room: publicRoom(room, player, t, limits.presentMs),
      events: events.filter(audible).map(publicEvent),
      // The last seq this page covered, lines left out included, so a
      // client whose last line was one it does not hear is not behind.
      read_to: events.length > 0 ? events.at(-1).seq : Math.min(after, room.seq),
      more: events.length === PAGE,
    });
  }

  function publicEvent(e) {
    const { player: _player, ...rest } = e;
    return rest;
  }

  async function start(request, id) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const room = await rooms.get(id);
    if (!room) return error(404, "not-found");
    const me = room.members.find((m) => m.id === player.id);
    if (!me) return error(403, "not-a-member");
    if (!me.host) return error(403, "not-the-host");
    const handler = roomHandler(room.kind);
    if (!handler) return error(404, "unknown-kind");
    if (room.phase !== "waiting") return error(409, "wrong-phase", undefined, { phase: room.phase });
    if (taken(room) < room.seats.min) return error(422, "not-enough-players", `This game needs ${room.seats.min} players.`);
    try {
      await begin(room, handler);
    } catch (e) {
      if (e instanceof Refusal) {
        await rooms.save(room);
        return error(422, e.code, e.message);
      }
      throw e;
    }
    await rooms.save(room);
    return json({ ok: true, room: publicRoom(room, player, now(), limits.presentMs) });
  }

  /** One player's leaving: a member out of a room that has not ended. */
  async function leave(room, handler, me, player, t) {
    if (FINAL_PHASES.includes(room.phase)) {
      // Leaving a room that is over takes it off the player's list.
      me.dismissed = true;
      await rooms.save(room);
      return json({ ok: true, seq: room.seq, room: publicRoom(room, player, t, limits.presentMs) });
    }
    if (room.phase === "waiting" || room.phase === "open") {
      room.members = room.members.filter((m) => m.id !== player.id);
      const seq = await rooms.append(room.id, { type: "leave", seat: me.seat, name: me.name, handle: me.handle, at: t, phase: room.phase });
      room.updatedAt = t;
      // A match that empties closes; so does a direct room either side
      // leaves, and a channel nobody named. A named channel waits.
      const closes = room.direct || (room.members.length === 0 && !room.name);
      if (closes) transition(room, "closed", t);
      else if (me.host && room.members.length > 0) room.members[0].host = true;
      await rooms.save(room);
      return json({ ok: true, seq, room: publicRoom(room, player, t, limits.presentMs) });
    }
    if (me.state === "left") return json({ ok: true, seq: room.seq, room: publicRoom(room, player, t, limits.presentMs) });
    me.state = "left";
    const seq = await rooms.append(room.id, { type: "left", seat: me.seat, player: player.id, at: t, reason: "left", phase: room.phase });
    room.updatedAt = t;
    await settle(room, handler);
    await rooms.save(room);
    return json({ ok: true, seq, room: publicRoom(room, player, t, limits.presentMs) });
  }

  /** A chat line: cleaned, paced, screened, then logged. */
  async function chat(room, handler, me, player, body, t) {
    if (FINAL_PHASES.includes(room.phase)) return error(409, "wrong-phase", undefined, { phase: room.phase });
    const perMinute = handler.chat_limit?.per_minute ?? limits.chatPerMinute;
    const gapMs = handler.chat_limit?.gap !== undefined ? handler.chat_limit.gap * 1000 : limits.chatGapMs;
    if (gapMs > 0 && t - (me.lastChat ?? 0) < gapMs) return error(429, "too-fast", "Wait a moment before the next line.");
    if (!(await limiter.allow(`chat:${player.id}:${room.id}`, perMinute, 60_000))) {
      return error(429, "too-fast", "Wait a moment before the next line.");
    }
    if (room.direct) {
      // A block stops a direct room, from either side.
      const relations = await rooms.relations(player.id);
      if (room.members.some((m) => m.id !== player.id && blockedPair(relations, m))) return error(403, "blocked");
    }
    const text = cleanText(body.text);
    if (!text) return error(400, "bad-request");
    const tag = body.tag === undefined || body.tag === null ? null : cleanText(body.tag, MAX_TAG_CHARS) || null;
    if (typeof deps.screen === "function") {
      const verdict = await deps.screen({ text, tag, player, room: publicRoom(room, player, t, limits.presentMs) });
      if (verdict === false) return error(422, "screened", "That line was not sent.");
      if (typeof verdict === "string") return error(422, "screened", verdict);
      if (verdict && typeof verdict === "object" && typeof verdict.code === "string") {
        return error(422, verdict.code, verdict.message);
      }
    }
    me.lastChat = t;
    const seq = await rooms.append(room.id, {
      type: "chat",
      seat: me.seat,
      name: me.name,
      handle: me.handle,
      player: player.id,
      text,
      ...(tag ? { tag } : {}),
      at: t,
      phase: room.phase,
    });
    // A line is activity in a channel, where nothing else is.
    if (room.phase === "open") room.updatedAt = t;
    await rooms.save(room);
    return json({ ok: true, seq });
  }

  async function post(request, id) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const room = await rooms.get(id);
    if (!room) return error(404, "not-found");
    const me = room.members.find((m) => m.id === player.id);
    if (!me) return error(403, "not-a-member");
    const handler = roomHandler(room.kind);
    if (!handler) return error(404, "unknown-kind");
    if (!(await limiter.allow(`events:${player.id}:${room.id}`, limits.roomEventsPerMinute, 60_000))) {
      return error(429, "rate-limited");
    }
    const body = await readJson(request, 64 * 1024);
    if (!body || typeof body.type !== "string") return error(400, "bad-request");
    me.lastSeen = now();
    await sweep(room, handler);
    const t = now();
    me.name = player.name;
    if (room.phase !== "open" && room.seq >= MAX_EVENTS && body.type !== "leave") {
      return error(422, "too-long", "This game has gone on too long.");
    }
    try {
      switch (body.type) {
        case "turn": {
          if (room.phase !== "playing") return error(409, "wrong-phase", undefined, { phase: room.phase });
          if (me.state !== "taken") return error(409, "left");
          const inputs = Array.isArray(body.inputs) ? body.inputs : null;
          if (!inputs || inputs.length > MAX_INPUTS_PER_TURN) return error(400, "bad-request");
          for (const input of inputs) {
            if (!Array.isArray(input) || input.length !== 2) return error(400, "bad-request");
            const [code, value] = input;
            if (!Number.isInteger(code) || code < 0 || code > MAX_SEATED_CODE) return error(400, "bad-request");
            if (!Number.isInteger(value) || value < -2147483648 || value > 2147483647) return error(400, "bad-request");
          }
          const seq = await rooms.append(room.id, { type: "turn", seat: me.seat, player: player.id, inputs, at: t, phase: room.phase });
          room.updatedAt = t;
          room.turnStartedAt = t;
          if (handler.progress) await settle(room, handler);
          await rooms.save(room);
          return json({ ok: true, seq, room: publicRoom(room, player, t, limits.presentMs) });
        }
        case "chat":
          return chat(room, handler, me, player, body, t);
        case "leave":
          return leave(room, handler, me, player, t);
        case "finish": {
          if (room.phase !== "playing") return error(409, "wrong-phase", undefined, { phase: room.phase });
          await settle(room, handler, { finish: true });
          await rooms.save(room);
          return json({ ok: true, seq: room.seq, room: publicRoom(room, player, t, limits.presentMs) });
        }
        default:
          return error(400, "bad-request");
      }
    } catch (e) {
      if (e instanceof Refusal) {
        await rooms.save(room);
        return error(422, e.code, e.message);
      }
      throw e;
    }
  }

  /** A member's recent lines, handed to the host's `report` hook. */
  async function report(request, id) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const room = await rooms.get(id);
    if (!room) return error(404, "not-found");
    const me = room.members.find((m) => m.id === player.id);
    if (!me) return error(403, "not-a-member");
    if (!(await limiter.allow(`report:${player.id}`, limits.roomEventsPerMinute, 60_000))) return error(429, "rate-limited");
    const body = (await readJson(request)) ?? {};
    if (!isHandle(body.handle)) return error(400, "bad-request");
    const about = room.members.find((m) => m.handle === body.handle) ?? (await rooms.playerByHandle(body.handle));
    if (!about) return error(404, "unknown-player", "No player by that handle.");
    const lines = [];
    let before = null;
    while (lines.length < REPORT_LINES) {
      const page = await rooms.eventsBefore(room.id, before, PAGE);
      if (page.length === 0) break;
      for (let i = page.length - 1; i >= 0 && lines.length < REPORT_LINES; i -= 1) {
        const e = page[i];
        if (e.type === "chat" && e.handle === body.handle) lines.unshift(publicEvent(e));
      }
      before = page[0].seq;
      if (page.length < PAGE) break;
    }
    const note = cleanText(body.note, MAX_CHAT_CHARS) || null;
    if (typeof deps.report === "function") {
      try {
        await deps.report({
          reporter: { id: player.id, name: player.name, handle: player.handle },
          about: { id: about.id, name: about.name, handle: about.handle },
          room: publicRoom(room, player, now(), limits.presentMs),
          lines,
          note,
        });
      } catch (e) {
        log({ error: String(e?.stack ?? e), note: "a report was made but its handler failed" });
      }
    } else {
      log({ note: "a player was reported, and the service has no report hook", room: room.id, about: about.handle, lines, reporter: player.handle });
    }
    return json({ ok: true, lines: lines.length });
  }

  async function mutes(request, method) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    if (method === "GET") {
      const relations = await rooms.relations(player.id);
      const named = async (handle, mode) => {
        const them = await rooms.playerByHandle(handle);
        return { handle, mode, name: them?.name ?? null };
      };
      const out = [];
      for (const h of relations.muted) out.push(await named(h, "mute"));
      for (const h of relations.blocked) out.push(await named(h, "block"));
      return json({ ok: true, mutes: out });
    }
    if (!(await limiter.allow(`mutes:${player.id}`, limits.roomEventsPerMinute, 60_000))) return error(429, "rate-limited");
    const body = (await readJson(request)) ?? {};
    if (!isHandle(body.handle) || !MUTE_MODES.includes(body.mode)) return error(400, "bad-request");
    if (body.handle === player.handle) return error(422, "not-allowed", "That is you.");
    const them = await rooms.playerByHandle(body.handle);
    if (body.mode !== "none") {
      const relations = await rooms.relations(player.id);
      if (relations.muted.size + relations.blocked.size >= limits.mutesPerPlayer && !relations.muted.has(body.handle) && !relations.blocked.has(body.handle)) {
        return error(422, "too-many", "You have muted too many players.");
      }
    }
    await rooms.setMute(player, body.handle, body.mode, them);
    if (body.mode === "block" && them) {
      // A block ends any direct room between the two.
      const t = now();
      for (const [kind, handler] of Object.entries(handlers)) {
        if (!handler?.seats || !isChannel(handler) || seatsOf(handler).max !== 2) continue;
        const room = await rooms.directBetween(kind, player.id, them.id);
        if (room && room.phase === "open") {
          transition(room, "closed", t);
          await rooms.save(room);
        }
      }
    }
    return json({ ok: true, handle: body.handle, mode: body.mode, name: them?.name ?? null });
  }

  // -- The lobby (lobby.js): presence, invites, friends ----------------------

  const presence = deps.presence ?? memoryPresence();
  const lobbyLimits = { ...LOBBY_LIMITS, ...deps.lobbyLimits };

  /** Whether `invite` is past its time, moved to lapsed if so. Says whether it changed. */
  function lapse(invite, t) {
    if ((invite.state === "sent" || invite.state === "heard") && t - invite.at > lobbyLimits.inviteMs) {
      inviteTransition(invite, "lapsed", t);
      return true;
    }
    return false;
  }

  /**
   * POST rooms/pulse { state, game?, place?, show?, heard? }: the player is
   * here (or `state: "away"`, leaving). Answers everything the lobby shows:
   * who is here, the player's friends, invites to and from them, their
   * rooms in play or waiting, and open tables anyone may take.
   */
  async function pulse(request) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    if (!(await limiter.allow(`pulse:${player.id}`, lobbyLimits.pulsesPerMinute, 60_000))) return error(429, "rate-limited");
    const body = (await readJson(request, 8 * 1024)) ?? {};
    const t = now();
    if (body.state === "away") {
      presence.drop(player.id);
      return json({ ok: true, state: "away" });
    }
    const friends = await rooms.friendsOf(player.id);
    const friendSet = new Set(friends);
    const entry = {
      id: player.id,
      handle: player.handle,
      name: player.name,
      guest: Boolean(player.guest),
      state: presenceOf(body.state),
      game: cleanText(body.game, MAX_NAME_CHARS) || null,
      place: cleanText(body.place, MAX_TAG_CHARS) || null,
      show: visibilityOf(body.show),
      friends: friendSet,
      at: t,
    };
    presence.set(entry);
    const relations = await rooms.relations(player.id);
    const present = presence.present(t, lobbyLimits.awayMs);
    const here = present
      .filter((p) => visibleTo(p, player, relations))
      .map((p) => publicPresence(p, friendSet.has(p.handle)))
      .sort((a, b) => Number(b.friend) - Number(a.friend) || a.name.localeCompare(b.name))
      .slice(0, lobbyLimits.hereListed);
    const byHandle = new Map(present.map((p) => [p.handle, p]));
    const friendList = [];
    for (const handle of friends) {
      const known = await rooms.playerByHandle(handle);
      if (!known) continue;
      const live = byHandle.get(handle);
      const seen = live && visibleTo(live, player, relations);
      friendList.push(
        seen
          ? publicPresence(live, true)
          : { handle, name: known.name, guest: Boolean(known.guest), state: "away", game: null, place: null, friend: true },
      );
    }
    friendList.sort((a, b) => Number(b.state !== "away") - Number(a.state !== "away") || a.name.localeCompare(b.name));

    // Invites: take the ones the client heard, lapse the old, hand out the rest.
    const heard = new Set(Array.isArray(body.heard) ? body.heard.slice(0, 50).map(String) : []);
    const invites = [];
    for (const invite of await rooms.invitesFor(player.id)) {
      let changed = lapse(invite, t);
      const toMe = invite.to.id === player.id;
      if (toMe && invite.state === "sent" && heard.has(invite.id)) {
        inviteTransition(invite, "heard", t);
        changed = true;
      }
      if (FINAL_INVITE_STATES.includes(invite.state) && t - (invite.changedAt ?? invite.at) > lobbyLimits.keepInviteMs) {
        await rooms.dropInvite(invite.id);
        continue;
      }
      let show = false;
      if (toMe) {
        // A muted or blocked sender's invites are never heard; they are not told.
        const quiet = relations.muted.has(invite.from.handle) || relations.blocked.has(invite.from.handle) || relations.blockedBy.has(invite.from.handle);
        show = !quiet && (invite.state === "sent" || invite.state === "heard");
      } else if (invite.state === "sent" || invite.state === "heard") {
        show = true;
      } else if (!invite.toldFrom && invite.state !== "withdrawn") {
        // The sender hears how it ended, once.
        show = true;
        invite.toldFrom = true;
        changed = true;
      }
      if (changed) await rooms.saveInvite(invite);
      if (show) invites.push(publicInvite(invite, player));
    }

    // The player's own matches, waiting or in play: whose move, who sat down.
    const mine = [];
    for (const room of await rooms.listFor(player.id)) {
      if (room.phase !== "waiting" && room.phase !== "playing") continue;
      const member = room.members.find((m) => m.id === player.id);
      if (!member || member.state === "left") continue;
      mine.push(publicRoom(room, player, t, limits.presentMs));
    }

    // Tables anyone may take, oldest first, across every kind with rules.
    const open = [];
    for (const kind of Object.keys(handlers)) {
      const handler = roomHandler(kind);
      if (!handler || isChannel(handler)) continue;
      for (const room of await rooms.openRooms(kind)) {
        if (open.length >= lobbyLimits.openListed) break;
        if (room.members.some((m) => m.id === player.id)) continue;
        if (taken(room) >= room.seats.max) continue;
        if (room.members.some((m) => blockedPair(relations, m))) continue;
        const host = room.members.find((m) => m.host) ?? room.members[0];
        open.push({ room: room.id, kind, code: room.code, host: host?.name ?? null, since: room.createdAt });
      }
    }
    open.sort((a, b) => a.since - b.since);

    return json({
      ok: true,
      you: { handle: player.handle, name: player.name, guest: Boolean(player.guest) },
      state: entry.state,
      here,
      friends: friendList,
      invites,
      rooms: mine,
      open,
      every: lobbyLimits.pulseEvery,
    });
  }

  /** POST rooms/invites { handle, kind, context? }: invite a player to a match. */
  async function sendInvite(request) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const body = (await readJson(request, 16 * 1024)) ?? {};
    const kind = String(body.kind ?? "");
    const handler = roomHandler(kind);
    if (!handler) return error(404, "unknown-kind");
    if (isChannel(handler)) return error(422, "not-a-match", "That kind is joined, not played.");
    if (!isHandle(body.handle)) return error(400, "bad-request");
    if (body.handle === player.handle) return error(422, "not-allowed", "That is you.");
    const them = await rooms.playerByHandle(body.handle);
    if (!them) return error(404, "unknown-player", "No player by that handle.");
    const relations = await rooms.relations(player.id);
    if (blockedPair(relations, them)) return error(403, "blocked");
    const t = now();
    const mine = await rooms.invitesFor(player.id);
    let outstanding = 0;
    for (const invite of mine) {
      if (lapse(invite, t)) await rooms.saveInvite(invite);
      if (invite.from.id !== player.id || FINAL_INVITE_STATES.includes(invite.state)) continue;
      // The same invite again is the one already waiting.
      if (invite.to.id === them.id && invite.kind === kind) return json({ ok: true, invite: publicInvite(invite, player) });
      outstanding += 1;
    }
    if (outstanding >= lobbyLimits.outstandingInvites) return error(422, "too-many", "You have too many invites waiting.");
    if (!(await limiter.allow(`invites:${player.id}`, lobbyLimits.invitesPerHour, 3_600_000))) return error(429, "rate-limited");
    const side = (p) => ({ id: p.id, handle: p.handle, name: p.name, guest: Boolean(p.guest) });
    const invite = {
      id: randomId(),
      kind,
      from: side(player),
      to: side(them),
      state: "sent",
      at: t,
      changedAt: t,
      room: null,
      toldFrom: false,
      context: body.context ?? null,
    };
    await rooms.saveInvite(invite);
    return json({ ok: true, invite: publicInvite(invite, player) });
  }

  /** POST rooms/invites/<id> { answer: accept | decline | withdraw }. */
  async function answerInvite(request, id) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const body = (await readJson(request)) ?? {};
    if (!INVITE_ANSWERS.includes(body.answer)) return error(400, "bad-request");
    const invite = await rooms.getInvite(id);
    const party = invite && (invite.from.id === player.id || invite.to.id === player.id);
    if (!party) return error(404, "unknown-invite", "No such invite.");
    const t = now();
    if (lapse(invite, t)) await rooms.saveInvite(invite);
    if (FINAL_INVITE_STATES.includes(invite.state)) {
      return error(409, "invite-over", "That invite is over.", { state: invite.state, invite: publicInvite(invite, player) });
    }
    const toMe = invite.to.id === player.id;
    if (body.answer === "withdraw") {
      if (toMe) return error(403, "not-yours", "Only the sender takes an invite back.");
      inviteTransition(invite, "withdrawn", t);
      await rooms.saveInvite(invite);
      return json({ ok: true, invite: publicInvite(invite, player) });
    }
    if (!toMe) return error(403, "not-yours", "Only the invited player answers.");
    if (body.answer === "decline") {
      inviteTransition(invite, "declined", t);
      await rooms.saveInvite(invite);
      return json({ ok: true, invite: publicInvite(invite, player) });
    }
    const handler = roomHandler(invite.kind);
    if (!handler || isChannel(handler)) return error(404, "unknown-kind");
    const relations = await rooms.relations(player.id);
    if (blockedPair(relations, invite.from)) return error(403, "blocked");
    const refused = await allowedToMake(player);
    if (refused) return refused;
    const sender = (await rooms.playerByHandle(invite.from.handle)) ?? invite.from;
    const { room } = await makeRoom(invite.kind, handler, { ...invite.from, name: sender.name ?? invite.from.name }, {
      context: invite.context ?? null,
      members: [player],
    });
    if (taken(room) >= room.seats.max) {
      try {
        await begin(room, handler);
      } catch (e) {
        if (e instanceof Refusal) {
          await rooms.save(room);
          return error(422, e.code, e.message);
        }
        throw e;
      }
      await rooms.save(room);
    }
    invite.room = room.id;
    inviteTransition(invite, "accepted", t);
    await rooms.saveInvite(invite);
    return json({ ok: true, invite: publicInvite(invite, player), room: publicRoom(room, player, now(), limits.presentMs) });
  }

  /** POST rooms/friends { handle, friend }: add or remove a friend. */
  async function setFriend(request) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    if (!(await limiter.allow(`friends:${player.id}`, limits.roomEventsPerMinute, 60_000))) return error(429, "rate-limited");
    const body = (await readJson(request)) ?? {};
    if (!isHandle(body.handle) || typeof body.friend !== "boolean") return error(400, "bad-request");
    if (body.handle === player.handle) return error(422, "not-allowed", "That is you.");
    const them = await rooms.playerByHandle(body.handle);
    if (!them) return error(404, "unknown-player", "No player by that handle.");
    if (body.friend) {
      const friends = await rooms.friendsOf(player.id);
      if (!friends.includes(body.handle) && friends.length >= lobbyLimits.friendsPerPlayer) {
        return error(422, "too-many", "You have too many friends listed.");
      }
    }
    await rooms.setFriend(player.id, body.handle, body.friend);
    const entry = presence.get(player.id);
    if (entry) {
      if (body.friend) entry.friends.add(body.handle);
      else entry.friends.delete(body.handle);
    }
    return json({ ok: true, handle: body.handle, friend: body.friend, name: them.name });
  }

  /** Answers a rooms request, or null when `parts` is not a rooms route. */
  return async function route(request, method, parts, url) {
    if (parts[0] !== "rooms") return null;
    if (method === "POST" && parts.length === 1) return create(request, url);
    if (method === "GET" && parts.length === 1) return list(request);
    if (method === "POST" && parts.length === 2 && parts[1] === "join") return join(request);
    if (method === "POST" && parts.length === 2 && parts[1] === "find") return find(request, url);
    if (method === "POST" && parts.length === 2 && parts[1] === "direct") return direct(request, url);
    if ((method === "GET" || method === "POST") && parts.length === 2 && parts[1] === "mutes") return mutes(request, method);
    if (method === "POST" && parts.length === 2 && parts[1] === "pulse") return pulse(request);
    if (method === "POST" && parts.length === 2 && parts[1] === "invites") return sendInvite(request);
    if (method === "POST" && parts.length === 3 && parts[1] === "invites" && /^[0-9a-f]{24}$/.test(parts[2])) return answerInvite(request, parts[2]);
    if (method === "POST" && parts.length === 2 && parts[1] === "friends") return setFriend(request);
    if (parts.length >= 2 && /^[0-9a-f]{24}$/.test(parts[1])) {
      if (method === "GET" && parts.length === 2) return get(request, url, parts[1]);
      if (method === "POST" && parts.length === 3 && parts[2] === "start") return start(request, parts[1]);
      if (method === "POST" && parts.length === 3 && parts[2] === "events") return post(request, parts[1]);
      if (method === "POST" && parts.length === 3 && parts[2] === "report") return report(request, parts[1]);
    }
    return error(404, "not-found");
  };
}
