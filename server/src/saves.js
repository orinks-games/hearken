// Synced files: a copy of a game's save kept with the player's account, so
// their progress follows them to another computer. The server never reads
// the file: the game seals it before it leaves, and opens and merges what
// comes back. Beside the file the game may send a small "card" of public
// facts (bests, counts) that the game's own `card` check shapes; the
// service hands it to `onSave` and does not store it.
//
// Routes, relative to the base URL (all with an account's bearer token):
//   GET  saves/<name>/head   { ok, account, save: { revision, hash, bytes, format, at } | null }
//   GET  saves/<name>        { ok, revision, hash, format, content: <base64> }
//   POST saves/<name>        { base, format, hash, content: <base64>, card? }
//
// An upload names the revision it was made from (`base`, 0 for none). When
// the account has moved on since, it is refused as 409 stale, naming the
// newer revision: the game downloads it, merges, and tries again. So two
// computers can never overwrite each other's progress unseen.
//
// docs/online.md in the Hearken repository is the protocol in full.

import { sha256Hex } from "./rules.js";
import { Refusal } from "./refusal.js";

export const SAVE_NAME_PATTERN = /^[a-z0-9][a-z0-9-]{0,31}$/;
/** The largest save a file may set, before base64. */
export const MAX_SAVE_BYTES = 1024 * 1024;
/** Revisions a store keeps of each file by default. */
export const KEEP_REVISIONS = 10;
/** The largest card, as JSON. */
export const MAX_CARD_BYTES = 8 * 1024;
const SEALED_MAGIC = [0x48, 0x4b, 0x53, 0x4c]; // "HKSL"

function fromBase64(text) {
  if (typeof text !== "string" || !/^[A-Za-z0-9+/]*={0,2}$/.test(text) || text.length % 4 !== 0) {
    return null;
  }
  try {
    const binary = atob(text);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}

function toBase64(bytes) {
  let binary = "";
  const step = 0x8000;
  for (let i = 0; i < bytes.length; i += step) {
    binary += String.fromCharCode(...bytes.subarray(i, i + step));
  }
  return btoa(binary);
}

function isSealed(bytes) {
  return SEALED_MAGIC.every((b, i) => bytes[i] === b);
}

/**
 * The saves routes. `deps.saves` is `{ store, files, guests?, onSave? }`:
 * - `store`: where revisions live (see `memorySaves`).
 * - `files`: `{ "<name>": { maxBytes?, sealed?, card?(card, { player }) } }`,
 *   the files a game keeps. `sealed: true` refuses a file that is not
 *   sealed, so a developer build cannot put a plain file on a live
 *   account. `card` returns the card to hand on, or throws `refuse(...)`.
 * - `guests`: whether guests may keep files (default false: 403
 *   account-needed).
 * - `onSave({ player, name, revision, card })`: called after each new
 *   revision, as a board's profile might want.
 */
export function saveRoutes(deps) {
  const { saves, accounts, limiter, bearer, readBody, json, error, log, now } = deps;
  const { store, files = {}, guests = false, onSave = async () => {} } = saves;
  for (const name of Object.keys(files)) {
    if (!SAVE_NAME_PATTERN.test(name)) throw new Error(`"${name}" cannot name a synced file`);
  }
  const limits = { savesPerHour: 30, saveReadsPerHour: 120, ...deps.limits };

  async function who(request) {
    const token = bearer(request);
    const player = token ? await accounts.player(token) : null;
    if (!player) return { refused: error(401, "signed-out") };
    if (player.guest && !guests) return { refused: error(403, "account-needed") };
    return { player };
  }

  async function head(player, name) {
    const latest = await store.head(player.id, name);
    return json({
      ok: true,
      account: String(player.id),
      save: latest
        ? { revision: latest.revision, hash: latest.hash, bytes: latest.bytes, format: latest.format, at: latest.at }
        : null,
    });
  }

  async function download(player, name) {
    const latest = await store.read(player.id, name);
    if (!latest) return error(404, "no-save");
    return json({
      ok: true,
      revision: latest.revision,
      hash: latest.hash,
      format: latest.format,
      content: toBase64(latest.content),
    });
  }

  async function upload(request, player, name, file) {
    if (!(await limiter.allow(`saves:${player.id}`, limits.savesPerHour, 3_600_000))) {
      return error(429, "rate-limited", undefined, { "retry-after": "600" });
    }
    const maxBytes = Math.min(file.maxBytes ?? MAX_SAVE_BYTES, MAX_SAVE_BYTES);
    // Base64 is a third longer, plus room for the card and the fields.
    const raw = await readBody(request, Math.ceil(maxBytes / 3) * 4 + MAX_CARD_BYTES + 1024);
    if (raw === null) return error(413, "too-large");
    let body;
    try {
      body = JSON.parse(new TextDecoder().decode(raw));
    } catch {
      return error(400, "bad-request");
    }
    const { base, format, hash } = body ?? {};
    if (!Number.isSafeInteger(base) || base < 0 || !Number.isSafeInteger(format) || format < 0) {
      return error(400, "bad-request");
    }
    if (typeof hash !== "string" || !/^[0-9a-f]{64}$/.test(hash)) return error(400, "bad-request");
    const content = fromBase64(body.content);
    if (!content || content.length === 0) return error(422, "bad-save");
    if (content.length > maxBytes) return error(413, "too-large");
    if ((await sha256Hex(content)) !== hash) return error(422, "bad-save", "The save was damaged on the way.");
    if (file.sealed && !isSealed(content)) return error(422, "not-sealed");

    let card = null;
    if (body.card !== undefined && body.card !== null) {
      if (JSON.stringify(body.card).length > MAX_CARD_BYTES) return error(413, "too-large");
      if (typeof file.card === "function") {
        try {
          card = await file.card(body.card, { player });
        } catch (e) {
          if (e instanceof Refusal) return error(422, e.code, e.message);
          throw e;
        }
      }
    }

    const latest = await store.head(player.id, name);
    if (latest && latest.hash === hash) {
      return json({ ok: true, revision: latest.revision, repeat: true });
    }
    const written = await store.write(player.id, name, {
      base,
      format,
      hash,
      content,
      bytes: content.length,
      at: now(),
    });
    if (written.stale) return json({ error: "stale", revision: written.revision }, 409);
    try {
      await onSave({ player, name, revision: written.revision, card });
    } catch (e) {
      log({ error: String(e?.stack ?? e), note: "a save was kept but onSave failed" });
    }
    return json({ ok: true, revision: written.revision });
  }

  return async function route(request, method, parts) {
    // parts[0] === "saves"
    const name = parts[1] ?? "";
    if (!SAVE_NAME_PATTERN.test(name) || !Object.hasOwn(files, name) || parts.length > 3) {
      return error(404, "not-found");
    }
    const { player, refused } = await who(request);
    if (refused) return refused;
    if (method === "GET" && (parts.length === 2 || parts[2] === "head")) {
      if (!(await limiter.allow(`save-reads:${player.id}`, limits.saveReadsPerHour, 3_600_000))) {
        return error(429, "rate-limited", undefined, { "retry-after": "600" });
      }
      return parts.length === 2 ? download(player, name) : head(player, name);
    }
    if (method === "POST" && parts.length === 2) return upload(request, player, name, files[name]);
    return error(404, "not-found");
  };
}

/**
 * Saves in memory, for tests and the dev server. A real store must make
 * `write`'s base check and the new revision one step (a transaction), or two
 * computers uploading at once could both win.
 */
export function memorySaves({ keep = KEEP_REVISIONS } = {}) {
  const slots = new Map();
  const slot = (player, name) => `${player}\u0000${name}`;
  return {
    async head(player, name) {
      const list = slots.get(slot(player, name));
      if (!list?.length) return null;
      const { content, ...latest } = list[list.length - 1];
      return latest;
    },
    async read(player, name) {
      const list = slots.get(slot(player, name));
      return list?.length ? { ...list[list.length - 1] } : null;
    },
    async write(player, name, save) {
      const key = slot(player, name);
      const list = slots.get(key) ?? [];
      const current = list.length ? list[list.length - 1].revision : 0;
      if (save.base !== current) return { stale: true, revision: current };
      const revision = current + 1;
      const { base, ...kept } = save;
      list.push({ ...kept, content: save.content.slice(), revision });
      while (list.length > keep) list.shift();
      slots.set(key, list);
      return { revision };
    },
    async revisions(player, name) {
      return (slots.get(slot(player, name)) ?? []).map((s) => s.revision);
    },
    async purge(player) {
      for (const key of [...slots.keys()]) {
        if (key.startsWith(`${player}\u0000`)) slots.delete(key);
      }
    },
  };
}
