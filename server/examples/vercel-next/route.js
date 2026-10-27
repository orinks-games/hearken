// app/api/games/[game]/[...path]/route.js in a Next.js app on Vercel: one
// route for every game's verified runs, as the Freight Fate driver board
// keeps its routes under app/api/. Each game gets its own service, with its
// own published modules and handlers.
//
// The three stores here are Convex calls; any database works the same way
// (see docs/online.md, "Writing a server").

import { createService, publishedRules } from "@hearken/online";
import { workerReplay } from "@hearken/online/node";
import { ConvexHttpClient } from "convex/browser";
import { api } from "@/convex/_generated/api";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { wicketsHandlers } from "@/lib/games/wickets";

export const runtime = "nodejs";

const convex = new ConvexHttpClient(process.env.NEXT_PUBLIC_CONVEX_URL);
// The server's own key for Convex functions only it may call.
const serverKey = process.env.HEARKEN_SERVER_KEY;

// Players, tokens and connecting a game, kept in Convex. Each mutation
// hashes what it stores, as freightFate.provisionDriver does.
const accounts = {
  player: (token) => convex.query(api.hearken.playerForToken, { token, serverKey }),
  startActivation: ({ clientKey }) =>
    convex.mutation(api.hearken.startActivation, { clientKey, serverKey, now: Date.now() }),
  pollActivation: (deviceCode) =>
    convex.mutation(api.hearken.pollActivation, { deviceCode, serverKey, now: Date.now() }),
};

// A Convex mutation is a serializable transaction, so a claim is atomic.
const runs = {
  claim: (key, record) => convex.mutation(api.hearken.claimRun, { key, ...record, serverKey, now: Date.now() }),
  complete: (key, claim, reply) => convex.mutation(api.hearken.completeRun, { key, claim, reply, serverKey }),
  release: (key, claim) => convex.mutation(api.hearken.releaseRun, { key, claim, serverKey }),
};

const limiter = {
  allow: (key, limit, windowMs) =>
    convex.mutation(api.hearken.consumeRateLimit, { key, limit, windowMs, serverKey, now: Date.now() }),
};

const games = {
  wickets: () =>
    createService({
      rules: rulesFolder("wickets"),
      accounts,
      runs,
      limiter,
      handlers: wicketsHandlers(convex, serverKey),
      basePath: "/api/games/wickets",
      replay: workerReplay({ timeoutMs: 8000 }),
    }),
};

// Published modules ship with the deployment, in hearken-rules/<game>/.
function rulesFolder(game) {
  let loaded = null;
  return {
    async get(build) {
      loaded ??= (async () => {
        const dir = join(process.cwd(), "hearken-rules", game);
        const names = (await readdir(dir)).filter((n) => n.endsWith(".wasm"));
        return publishedRules(await Promise.all(names.map(async (n) => ({ bytes: await readFile(join(dir, n)) }))));
      })();
      return (await loaded).get(build);
    },
  };
}

const services = new Map();

async function handle(request, { params }) {
  const { game } = await params;
  if (!Object.hasOwn(games, game)) {
    return Response.json({ error: "not-found" }, { status: 404 });
  }
  if (!services.has(game)) services.set(game, games[game]());
  return services.get(game).fetch(request);
}

export { handle as GET, handle as POST };
