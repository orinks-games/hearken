// Handlers for Coin Dash, the replay crate's example rules: three kinds of
// run, to show that what a verified result is for is the game's own choice.
//
//   daily     a leaderboard on the day's seed, shared by every player and
//             handed out by `start` only once the day has begun
//   practice  any seed, just answered with the verified score
//   personal  a course of the player's own (a seed only they are given), best
//             score kept: another player's run started elsewhere, so it
//             cannot be passed off as theirs
//
// A handler has `expect` (what the run must have started from: the server
// decides, never the run) and `accept` (what to do with the verified
// outcome, and what to tell the game). `start` (what to play, before the
// run) and `read` (a GET route) are optional.
//
// `pebbles` is a kind with rooms, for the replay crate's two-player Pebbles
// rules: `seats` makes it one, `start` picks the match's seed when the room
// starts, and `accept` gets the verified outcome when the rules end.
//
// `lobby` and `dm` are channels: seats with no rules. A lobby is joined by
// name (`rooms/find?kind=lobby&name=general`) and keeps a week of lines; a
// dm is the two-member room `rooms/direct?kind=dm` finds or makes.

import { refuse } from "../src/index.js";

/**
 * A seed only this server can make: the first 8 bytes of HMAC-SHA-256 of
 * `label` under `secret`. A seed anyone can compute (a hash of the date)
 * lets a bot work out the perfect run for any day ahead of time; this one
 * is unknown until `start` hands it out.
 */
export async function secretSeed(secret, label) {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const mac = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(label));
  return new DataView(mac).getBigUint64(0, true).toString();
}

export function today(now = new Date()) {
  return now.toISOString().slice(0, 10);
}

export function createHandlers({ now = () => new Date(), secret = "dev only: set a real secret" } = {}) {
  const boards = new Map(); // day -> Map(player id -> { name, coins })
  const bests = new Map(); // player id -> best coins
  const wins = new Map(); // player id -> { name, won, played }

  return {
    daily: {
      async start() {
        return { seed: await secretSeed(secret, `daily:${today(now())}`), context: { seconds: 60 } };
      },
      async expect({ run }) {
        // Yesterday's run still counts in its first minutes past midnight.
        const day = today(now());
        const yesterday = today(new Date(now().getTime() - 15 * 60_000));
        for (const d of new Set([day, yesterday])) {
          if (run.seed === (await secretSeed(secret, `daily:${d}`))) {
            return { rules: "coin-dash", seed: run.seed, context: { seconds: 60 }, day: d };
          }
        }
        throw refuse("wrong-day", "That run is not today's challenge.");
      },
      async accept({ player, verified, expected }) {
        // The day the run's seed belongs to, not the day it arrived.
        const day = expected.day;
        const board = boards.get(day) ?? new Map();
        boards.set(day, board);
        const best = board.get(player.id);
        if (!best || verified.outcome.coins > best.coins) {
          board.set(player.id, { id: player.id, name: player.name, coins: verified.outcome.coins });
        }
        const ranked = [...board.values()].sort((a, b) => b.coins - a.coins);
        const rank = ranked.findIndex((r) => r.id === player.id) + 1;
        return { coins: verified.outcome.coins, rank, of: ranked.length };
      },
      async read({ path }) {
        const board = boards.get(path[0] ?? today(now())) ?? new Map();
        const top = [...board.values()].sort((a, b) => b.coins - a.coins).slice(0, 10);
        return { top: top.map(({ name, coins }) => ({ name, coins })) };
      },
    },
    practice: {
      copies: "allow",
      async expect({ run }) {
        // Any seed: practice proves nothing beyond the score itself.
        return { rules: "coin-dash", seed: run.seed, context: { seconds: 30 } };
      },
      async accept({ verified }) {
        return { coins: verified.outcome.coins };
      },
    },
    personal: {
      async start({ player }) {
        return { seed: await secretSeed(secret, `personal:${player.id}`), context: { seconds: 60 } };
      },
      async expect({ player }) {
        return { rules: "coin-dash", seed: await secretSeed(secret, `personal:${player.id}`), context: { seconds: 60 } };
      },
      async accept({ player, verified }) {
        const best = bests.get(player.id) ?? 0;
        const coins = verified.outcome.coins;
        if (coins > best) bests.set(player.id, coins);
        return { coins, best: Math.max(best, coins) };
      },
    },
    pebbles: {
      rules: "pebbles",
      seats: { min: 2, max: 2 },
      // Replay after each turn, so My games can say whose move it is.
      progress: true,
      // A seat that sits on its move for three days has left.
      turn_limit: 3 * 86_400,
      async start({ context }) {
        const seed = new DataView(crypto.getRandomValues(new Uint8Array(8)).buffer).getBigUint64(0, true).toString();
        const pile = Number.isInteger(context?.pile) ? Math.min(60, Math.max(5, context.pile)) : 10;
        return { seed, context: { pile } };
      },
      async accept({ players, verified }) {
        const winner = players.find((p) => p.seat === verified.outcome.winner);
        for (const p of players) {
          const row = wins.get(p.id) ?? { name: p.name, won: 0, played: 0 };
          row.played += 1;
          if (p.id === winner?.id) row.won += 1;
          wins.set(p.id, row);
        }
        return { winner: winner?.name ?? null };
      },
      async read() {
        return { players: [...wins.values()].sort((a, b) => b.won - a.won).slice(0, 10) };
      },
    },
    lobby: {
      seats: { min: 1, max: 200 },
      // Lines are kept a week; one line every three seconds, twenty a minute.
      history: 7 * 86_400,
      chat_limit: { per_minute: 20, gap: 3 },
    },
    dm: {
      seats: { min: 2, max: 2 },
      history: 7 * 86_400,
      chat_limit: { per_minute: 20, gap: 1 },
    },
  };
}
