// In-memory accounts, run claims and rate limits: for the dev server, tests,
// and as the reference for a real store. Each says what a store backed by
// Convex, a key-value store or SQL must do the same way.

import { toHex, sha256Hex } from "./rules.js";

function randomHex(bytes) {
  const out = new Uint8Array(bytes);
  crypto.getRandomValues(out);
  return toHex(out);
}

/**
 * Run claims. `claim` must be atomic: of two claims of one key at once,
 * exactly one gets { claimed: true } (a Convex mutation, an SQL insert with
 * a unique key, or a key-value "set if absent" each do this). A claim with
 * no reply after `staleMs` (a request that died partway) may be taken again.
 */
export function memoryRuns({ staleMs = 120_000, now = () => Date.now() } = {}) {
  const map = new Map();
  return {
    async claim(key, record) {
      const existing = map.get(key);
      if (existing && (existing.reply !== undefined || now() - existing.at < staleMs)) {
        return { claimed: false, existing };
      }
      const mine = { ...record, at: now(), nonce: randomHex(8) };
      map.set(key, mine);
      return { claimed: true, nonce: mine.nonce };
    },
    async complete(key, claim, reply) {
      const row = map.get(key);
      if (claim.claimed && row?.nonce === claim.nonce) row.reply = reply;
    },
    async release(key, claim) {
      const row = map.get(key);
      if (claim.claimed && row?.nonce === claim.nonce && row.reply === undefined) map.delete(key);
    },
  };
}

/** A fixed-window rate limit: at most `limit` calls per key per window. */
export function memoryLimiter({ now = () => Date.now() } = {}) {
  const windows = new Map();
  return {
    async allow(key, limit, windowMs) {
      const t = now();
      const slot = Math.floor(t / windowMs);
      const row = windows.get(key);
      if (!row || row.slot !== slot) {
        windows.set(key, { slot, count: 1 });
        if (windows.size > 100_000) {
          for (const [k, v] of windows) if (v.slot !== slot) windows.delete(k);
        }
        return true;
      }
      row.count += 1;
      return row.count <= limit;
    },
  };
}

// Letters a synthesized voice does not blur together (no O/0, I/1/L, S/5, Z/2),
// as the Freight Fate driver board uses.
export const CODE_ALPHABET = "ABCDEFGHJKMNPQRTUVWXY346789";

export function mintUserCode(length = 8) {
  const bytes = new Uint8Array(length);
  crypto.getRandomValues(bytes);
  let code = "";
  for (const b of bytes) code += CODE_ALPHABET[b % CODE_ALPHABET.length];
  return `${code.slice(0, 4)}-${code.slice(4)}`;
}

/** What a player typed, as a code: any case, any dashes and spaces. */
export function normalizeUserCode(text) {
  const raw = String(text ?? "").toUpperCase().replace(/[\s-]+/g, "");
  if (raw.length !== 8 || [...raw].some((c) => !CODE_ALPHABET.includes(c))) return "";
  return `${raw.slice(0, 4)}-${raw.slice(4)}`;
}

/**
 * Players, tokens and account connection, in memory. Tokens and device codes
 * are kept only as SHA-256 hashes, as a real store should keep them.
 *
 * Connecting works like a TV's sign-in: the game asks `startActivation` for
 * a short code and speaks it, the player enters it on the site's page while
 * signed in (`approve`), and the game's `pollActivation` then collects a
 * token. `autoApprove` approves every code as the named player at once, for
 * tests.
 */
export function memoryAccounts({
  verifyUrl = "http://localhost:8787/activate",
  ttlMs = 10 * 60_000,
  autoApprove = null,
  now = () => Date.now(),
} = {}) {
  const tokens = new Map(); // token hash -> player
  const activations = new Map(); // device code hash -> row
  const byCode = new Map(); // user code -> device code hash
  let nextId = 1;

  async function issue(player) {
    const token = randomHex(32);
    tokens.set(await sha256Hex(new TextEncoder().encode(token)), player);
    return token;
  }

  const accounts = {
    async player(token) {
      return tokens.get(await sha256Hex(new TextEncoder().encode(token))) ?? null;
    },
    async startActivation() {
      for (const [hash, row] of activations) {
        if (now() > row.expires) {
          activations.delete(hash);
          byCode.delete(row.userCode);
        }
      }
      const deviceCode = randomHex(32);
      const hash = await sha256Hex(new TextEncoder().encode(deviceCode));
      let userCode = mintUserCode();
      while (byCode.has(userCode)) userCode = mintUserCode();
      activations.set(hash, { userCode, expires: now() + ttlMs, player: null, collected: false });
      byCode.set(userCode, hash);
      if (autoApprove) await accounts.approve(userCode, autoApprove);
      return {
        device_code: deviceCode,
        user_code: userCode,
        verify_url: verifyUrl,
        expires_in: Math.round(ttlMs / 1000),
      };
    },
    async pollActivation(deviceCode) {
      const hash = await sha256Hex(new TextEncoder().encode(deviceCode));
      const row = activations.get(hash);
      if (!row || row.collected || now() > row.expires) return { status: "expired" };
      if (!row.player) return { status: "pending" };
      // A token is handed out once; the row is spent.
      row.collected = true;
      byCode.delete(row.userCode);
      return { status: "ready", token: await issue(row.player), name: row.player.name };
    },
    /** The site's side: a signed-in player entered `userCode`. */
    async approve(userCode, name) {
      const code = normalizeUserCode(userCode);
      const hash = byCode.get(code);
      const row = hash && activations.get(hash);
      if (!row || row.player || now() > row.expires) return false;
      row.player = { id: `p${nextId++}`, name: String(name).slice(0, 32) || "Player" };
      return true;
    },
    /** A token for a new player, for tests. */
    async tokenFor(name) {
      return issue({ id: `p${nextId++}`, name });
    },
  };
  return accounts;
}
