// Rooms: matches between players, chat channels and direct messages, all
// one thing to the server. A room has a kind, members in seats, a phase,
// and an ordered log of events (turns, chat, seats leaving). A match is a
// room whose kind has rules: the server orders the turns, stamps each with
// the seat that sent it, and replays them with the game's published rules
// module (`hearken_play`) to learn where the match stands and, at the end,
// what it came to. Nothing the clients say about the outcome is read.
//
// Routes, relative to the base URL (all with a bearer token):
//   POST rooms?kind=<kind>        { context?, open? }: a new room, with an invite code
//   POST rooms/join               { code }: take the next seat
//   POST rooms/find?kind=<kind>   join an open room of that kind, or make one
//   GET  rooms                    my rooms
//   GET  rooms/<id>?after=<seq>   the room and its events after seq
//   POST rooms/<id>/start         the host starts with enough seats filled
//   POST rooms/<id>/events        { type: "turn" | "chat" | "leave" | "finish", ... }
//                                 (leave on a room that is over drops it from my rooms)
//
// The room's phase is an explicit state machine (`PHASES`, `transition`):
//   waiting   -> playing   start, or the last seat filled
//   waiting   -> closed    the last member left, or idle past the limit
//   playing   -> finished  the rules ended (a turn, a seat leaving, a forfeit)
//   playing   -> abandoned idle past the handler's idle_limit
// finished, closed and abandoned are final. An event on a room in the
// wrong phase is refused as 409 wrong-phase, naming the phase.
//
// docs/online.md in the Hearken repository is the protocol in full.

import { Refusal, refuse } from "./refusal.js";

export const PHASES = ["waiting", "playing", "finished", "closed", "abandoned"];
export const FINAL_PHASES = ["finished", "closed", "abandoned"];

/** The phase changes a room may make. Anything else is a bug, not a state. */
const TRANSITIONS = {
  waiting: ["playing", "closed"],
  playing: ["finished", "abandoned"],
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

export const MAX_SEATS = 16;
export const MAX_SEATED_CODE = 0xffe;
export const MAX_INPUTS_PER_TURN = 256;
export const MAX_CHAT_CHARS = 200;
export const MAX_EVENTS = 65_536;
export const PAGE = 500;

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
 *   that are not in a final phase.
 * - `save` writes the room record as a whole; the service reads, changes
 *   and saves a room within one request, and a store that can should do
 *   that under the room's lock (a Convex mutation does).
 */
export function memoryRooms() {
  const rooms = new Map(); // id -> room
  const logs = new Map(); // id -> events
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
      const from = Math.max(0, Math.min(seq, log.length));
      return log.slice(from, from + limit);
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

/** A room as a client sees it: no player ids, seats instead. */
function publicRoom(room, player, now, presentMs) {
  const mine = room.members.find((m) => m.id === player.id);
  return {
    id: room.id,
    kind: room.kind,
    code: room.code,
    phase: room.phase,
    open: room.open,
    seats: room.seats,
    host: room.members.find((m) => m.host)?.seat ?? 0,
    members: room.members.map((m) => ({
      seat: m.seat,
      name: m.name,
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
 * `accounts`, `limiter`, `rules`, `handlers`, `log`, `now`, `limits`, plus
 * `bearer(request)` and `readJson(request)` from the service.
 */
export function roomRoutes(deps) {
  const { rooms, accounts, limiter, rules, handlers, log, bearer, readJson } = deps;
  const now = deps.now ?? (() => Date.now());
  const limits = {
    openRoomsPerPlayer: 20,
    roomEventsPerMinute: 60,
    chatPerMinute: 10,
    roomsPerHour: 30,
    presentMs: 30_000,
    idleMs: 30 * 86_400_000,
    ...deps.limits,
  };

  function roomHandler(kind) {
    if (!Object.hasOwn(handlers, kind)) return null;
    const handler = handlers[kind];
    if (!handler.seats) return null;
    return handler;
  }

  function seatsOf(handler) {
    const min = Math.max(1, Number(handler.seats.min ?? 2));
    const max = Math.min(MAX_SEATS, Math.max(min, Number(handler.seats.max ?? min)));
    return { min, max };
  }

  async function who(request) {
    const token = bearer(request);
    return token ? await accounts.player(token) : null;
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
      guest: Boolean(player.guest),
      seat: seatNo,
      state: "taken",
      host,
      lastSeen: now(),
      lastRead: 0,
    });
    return seatNo;
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

  async function create(request, url) {
    const kind = url.searchParams.get("kind") ?? "";
    const handler = roomHandler(kind);
    if (!handler) return error(404, "unknown-kind");
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    if (!(await limiter.allow(`rooms:${player.id}`, limits.roomsPerHour, 3_600_000))) {
      return error(429, "rate-limited", undefined, undefined);
    }
    const mine = (await rooms.listFor(player.id)).filter((r) => !FINAL_PHASES.includes(r.phase));
    if (mine.length >= limits.openRoomsPerPlayer) return error(422, "too-many-rooms", "You have too many open games.");
    const body = (await readJson(request, 16 * 1024)) ?? {};
    const seats = seatsOf(handler);
    const t = now();
    const room = await rooms.create({
      kind,
      phase: "waiting",
      open: Boolean(body.open),
      seats,
      members: [],
      context: body.context ?? null,
      createdAt: t,
      updatedAt: t,
    });
    const seatNo = await seat(room, player, { host: true });
    await rooms.save(room);
    return json({ ok: true, room: publicRoom(room, player, t, limits.presentMs), seat: seatNo });
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
      await rooms.save(room);
      return json({ ok: true, room: publicRoom(room, player, t, limits.presentMs), seat: already.seat });
    }
    if (room.phase !== "waiting") return error(409, room.phase === "playing" ? "started" : "wrong-phase", undefined, { phase: room.phase });
    if (room.members.filter((m) => m.state === "taken").length >= room.seats.max) return error(409, "full");
    const seatNo = await seat(room, player);
    room.updatedAt = t;
    await rooms.append(room.id, { type: "join", seat: seatNo, name: player.name, at: t, phase: room.phase });
    if (room.members.filter((m) => m.state === "taken").length >= room.seats.max) {
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
    for (const room of await rooms.openRooms(kind)) {
      if (room.members.some((m) => m.id === player.id)) continue;
      if (room.members.filter((m) => m.state === "taken").length >= room.seats.max) continue;
      return joinRoom(room, player);
    }
    const made = new URL(url);
    made.searchParams.set("kind", kind);
    return create(new Request(made, { method: "POST", headers: request.headers, body: JSON.stringify({ open: true }) }), made);
  }

  async function list(request) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const t = now();
    const out = [];
    for (const room of await rooms.listFor(player.id)) {
      await sweep(room, roomHandler(room.kind));
      const me = room.members.find((m) => m.id === player.id);
      if (me?.dismissed) continue;
      out.push({
        ...publicRoom(room, player, t, limits.presentMs),
        unread: Math.max(0, room.seq - (me?.lastRead ?? 0)),
      });
    }
    return json({ ok: true, rooms: out });
  }

  async function get(request, url, id) {
    const player = await who(request);
    if (!player) return error(401, "signed-out");
    const room = await rooms.get(id);
    if (!room) return error(404, "not-found");
    const me = room.members.find((m) => m.id === player.id);
    if (!me) return error(403, "not-a-member");
    const handler = roomHandler(room.kind);
    await sweep(room, handler);
    const after = Math.max(0, Number(url.searchParams.get("after") ?? "0") || 0);
    const events = await rooms.eventsAfter(room.id, after, PAGE);
    const t = now();
    me.lastSeen = t;
    if (events.length > 0) me.lastRead = Math.max(me.lastRead ?? 0, events.at(-1).seq);
    else me.lastRead = Math.max(me.lastRead ?? 0, Math.min(after, room.seq));
    await rooms.save(room);
    return json({
      ok: true,
      room: publicRoom(room, player, t, limits.presentMs),
      events: events.map(publicEvent),
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
    const taken = room.members.filter((m) => m.state === "taken").length;
    if (taken < room.seats.min) return error(422, "not-enough-players", `This game needs ${room.seats.min} players.`);
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
    await sweep(room, handler);
    const t = now();
    me.lastSeen = t;
    if (room.seq >= MAX_EVENTS && body.type !== "leave") return error(422, "too-long", "This game has gone on too long.");
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
        case "chat": {
          if (FINAL_PHASES.includes(room.phase)) return error(409, "wrong-phase", undefined, { phase: room.phase });
          if (!(await limiter.allow(`chat:${player.id}:${room.id}`, limits.chatPerMinute, 60_000))) {
            return error(429, "rate-limited");
          }
          const text = cleanText(body.text);
          if (!text) return error(400, "bad-request");
          if (typeof deps.screen === "function") {
            const verdict = await deps.screen({ text, player, room: publicRoom(room, player, t, limits.presentMs) });
            if (verdict === false) return error(422, "not-allowed", "That line was not sent.");
            if (typeof verdict === "string") return error(422, "not-allowed", verdict);
          }
          const seq = await rooms.append(room.id, { type: "chat", seat: me.seat, name: me.name, player: player.id, text, at: t, phase: room.phase });
          await rooms.save(room);
          return json({ ok: true, seq });
        }
        case "leave": {
          if (FINAL_PHASES.includes(room.phase)) {
            // Leaving a room that is over takes it off the player's list.
            me.dismissed = true;
            await rooms.save(room);
            return json({ ok: true, seq: room.seq, room: publicRoom(room, player, t, limits.presentMs) });
          }
          if (room.phase === "waiting") {
            room.members = room.members.filter((m) => m.id !== player.id);
            const seq = await rooms.append(room.id, { type: "leave", seat: me.seat, at: t, phase: room.phase });
            room.updatedAt = t;
            if (room.members.length === 0) transition(room, "closed", t);
            else if (me.host) room.members[0].host = true;
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

  /** Answers a rooms request, or null when `parts` is not a rooms route. */
  return async function route(request, method, parts, url) {
    if (parts[0] !== "rooms") return null;
    if (method === "POST" && parts.length === 1) return create(request, url);
    if (method === "GET" && parts.length === 1) return list(request);
    if (method === "POST" && parts.length === 2 && parts[1] === "join") return join(request);
    if (method === "POST" && parts.length === 2 && parts[1] === "find") return find(request, url);
    if (parts.length >= 2 && /^[0-9a-f]{24}$/.test(parts[1])) {
      if (method === "GET" && parts.length === 2) return get(request, url, parts[1]);
      if (method === "POST" && parts.length === 3 && parts[2] === "start") return start(request, parts[1]);
      if (method === "POST" && parts.length === 3 && parts[2] === "events") return post(request, parts[1]);
    }
    return error(404, "not-found");
  };
}
