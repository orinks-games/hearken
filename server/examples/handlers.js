// Handlers for Coin Dash, the replay crate's example rules: three kinds of
// run, to show that what a verified result is for is the game's own choice.
//
//   daily     a leaderboard for the day's seed, best score per player
//   practice  any seed, just answered with the verified score
//   ghost     the best run kept for others to race (copies refused)
//
// A handler has `expect` (what the run must have started from: the server
// decides, never the run) and `accept` (what to do with the verified
// outcome, and what to tell the game). `read` is optional: a GET route.

import { refuse } from "../src/index.js";

/** The day's seed, the same in the game and here: sha256("coin-dash:<day>")'s first 8 bytes. */
export async function dailySeed(day) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`coin-dash:${day}`));
  return new DataView(digest).getBigUint64(0, true).toString();
}

export function today(now = new Date()) {
  return now.toISOString().slice(0, 10);
}

export function createHandlers({ now = () => new Date() } = {}) {
  const boards = new Map(); // day -> Map(player id -> { name, coins })
  let ghost = null;

  return {
    daily: {
      async expect({ run }) {
        // Yesterday's run still counts in its first minutes past midnight.
        const day = today(now());
        const yesterday = today(new Date(now().getTime() - 15 * 60_000));
        for (const d of new Set([day, yesterday])) {
          if (run.seed === (await dailySeed(d))) {
            return { rules: "coin-dash", seed: run.seed, context: { seconds: 60 }, day: d };
          }
        }
        throw refuse("wrong-day", "That run is not today's challenge.");
      },
      async accept({ player, verified }) {
        const day = today(now());
        const board = boards.get(day) ?? new Map();
        boards.set(day, board);
        const best = board.get(player.id);
        if (!best || verified.outcome.coins > best.coins) {
          board.set(player.id, { name: player.name, coins: verified.outcome.coins });
        }
        const ranked = [...board.values()].sort((a, b) => b.coins - a.coins);
        const rank = ranked.findIndex((r) => r.name === player.name) + 1;
        return { coins: verified.outcome.coins, rank, of: ranked.length };
      },
      async read({ path }) {
        const board = boards.get(path[0] ?? today(now())) ?? new Map();
        return { top: [...board.values()].sort((a, b) => b.coins - a.coins).slice(0, 10) };
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
    ghost: {
      async expect({ run }) {
        return { rules: "coin-dash", seed: run.seed, context: run.context };
      },
      async accept({ player, verified }) {
        if (!ghost || verified.outcome.coins > ghost.coins) {
          ghost = { name: player.name, coins: verified.outcome.coins, run_id: verified.run_id };
          return { kept: true };
        }
        return { kept: false, best: ghost.coins };
      },
      async read() {
        return ghost ?? {};
      },
    },
  };
}
