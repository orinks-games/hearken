// A game's rules module: the WebAssembly file `hearken release` builds from
// the game's rules crate. Only the WebAssembly API is used here, so this
// runs on Node, Deno, Bun, Cloudflare Workers and Vercel alike.

export const ABI = 1;
export const MAGIC = [0x48, 0x4b, 0x52, 0x4e]; // "HKRN"
export const FORMAT_VERSION = 1;
export const BUILD_OFFSET = 5;
export const MAX_RUN_BYTES = 4 << 20;
// No answer from a module is this long; one that claims to be is broken.
const MAX_ANSWER_BYTES = 1 << 20;

const REQUIRED = [
  "memory",
  "hearken_abi",
  "hearken_alloc",
  "hearken_free",
  "hearken_rules",
  "hearken_inspect",
  "hearken_verify",
];

export function toHex(bytes) {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

export async function sha256Hex(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return toHex(new Uint8Array(digest));
}

/** The build ID a run file names, or null when it is not a run file. */
export function buildOf(run) {
  const bytes = run instanceof Uint8Array ? run : new Uint8Array(run);
  if (bytes.length < BUILD_OFFSET + 32) return null;
  for (let i = 0; i < 4; i++) if (bytes[i] !== MAGIC[i]) return null;
  if (bytes[4] !== FORMAT_VERSION) return null;
  return toHex(bytes.subarray(BUILD_OFFSET, BUILD_OFFSET + 32));
}

export class RulesError extends Error {
  constructor(code, message) {
    super(message ?? code);
    this.code = code;
  }
}

/**
 * Loads a rules module from its bytes, or from a WebAssembly.Module already
 * compiled (Cloudflare Workers import .wasm files that way). Refuses a module
 * that imports anything: rules that read a clock, randomness or the host
 * could not be replayed exactly. `build` is the module's SHA-256; it is
 * computed from `bytes` when they are given, and must be passed with a
 * precompiled module.
 */
export async function loadRules(source, { build } = {}) {
  let module;
  let id = build;
  if (source instanceof WebAssembly.Module) {
    module = source;
    if (!/^[0-9a-f]{64}$/.test(id ?? "")) {
      throw new RulesError("bad-module", "a precompiled module needs its build ID");
    }
  } else {
    const bytes = source instanceof Uint8Array ? source : new Uint8Array(source);
    const computed = await sha256Hex(bytes);
    if (id && id !== computed) {
      throw new RulesError("bad-module", `the module's SHA-256 is ${computed}, not ${id}`);
    }
    id = computed;
    module = await WebAssembly.compile(bytes);
  }
  const imports = WebAssembly.Module.imports(module);
  if (imports.length > 0) {
    const names = imports.map((i) => `${i.module}.${i.name}`).join(", ");
    throw new RulesError("bad-module", `rules may not import anything, and this module imports ${names}`);
  }
  const exported = new Set(WebAssembly.Module.exports(module).map((e) => e.name));
  const missing = REQUIRED.filter((name) => !exported.has(name));
  if (missing.length > 0) {
    throw new RulesError("bad-module", `the module lacks ${missing.join(", ")}`);
  }
  const rules = new Rules(module, id);
  const abi = rules.call((x) => x.hearken_abi());
  if (abi !== ABI) {
    throw new RulesError("bad-module", `the module speaks ABI ${abi}, not ${ABI}`);
  }
  const listed = rules.call((x) => rules.answer(x, x.hearken_rules()));
  if (!Array.isArray(listed.rules)) {
    throw new RulesError("bad-module", "the module does not list its rules");
  }
  rules.names = listed.rules;
  return rules;
}

const decoder = new TextDecoder("utf-8", { fatal: true });
const encoder = new TextEncoder();

/** One loaded rules module. Each call runs in a fresh instance. */
export class Rules {
  constructor(module, build) {
    this.module = module;
    this.build = build;
    this.names = [];
  }

  /** Runs `f` against a fresh instance, so no run leaves state for the next. */
  call(f) {
    const instance = new WebAssembly.Instance(this.module, {});
    return f(instance.exports);
  }

  /** Reads and frees an answer: its pointer in the high half, length in the low. */
  answer(x, packed) {
    const value = BigInt.asUintN(64, BigInt(packed));
    const ptr = Number(value >> 32n);
    const len = Number(value & 0xffffffffn);
    if (len > MAX_ANSWER_BYTES || ptr + len > x.memory.buffer.byteLength) {
      throw new RulesError("bad-module", "the module's answer is out of bounds");
    }
    const text = decoder.decode(new Uint8Array(x.memory.buffer, ptr, len).slice());
    x.hearken_free(ptr, len);
    return JSON.parse(text);
  }

  /** Copies `bytes` into the instance; returns [ptr, len]. */
  put(x, bytes) {
    const ptr = x.hearken_alloc(bytes.length) >>> 0;
    if (bytes.length > 0) {
      if (ptr === 0 || ptr + bytes.length > x.memory.buffer.byteLength) {
        throw new RulesError("bad-module", "the module gave no room for the run");
      }
      new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
    }
    return [ptr, bytes.length];
  }

  /** What a run file says about itself. Nothing in it is verified. */
  inspect(run) {
    const bytes = asBytes(run);
    try {
      return this.call((x) => {
        const [p, n] = this.put(x, bytes);
        return this.answer(x, x.hearken_inspect(p, n));
      });
    } catch (error) {
      if (error instanceof WebAssembly.RuntimeError) {
        return { ok: false, error: "rules-crashed" };
      }
      throw error;
    }
  }

  /**
   * Checks the run against `expected` ({ rules, seed, context }) and replays
   * it. Gives the module's answer: { ok: true, outcome, run_id, ... } or
   * { ok: false, error }. A module that traps (a panic in the rules) gives
   * { ok: false, error: "rules-crashed" }.
   */
  verify(run, expected) {
    const bytes = asBytes(run);
    if (typeof expected.seed === "number" && !Number.isSafeInteger(expected.seed)) {
      // Past 2^53 a JavaScript number is already rounded: send a string or a BigInt.
      throw new RulesError("bad-seed", "a seed above 2^53 must be a string or a BigInt");
    }
    const request = encoder.encode(
      JSON.stringify({
        rules: expected.rules,
        seed: String(expected.seed),
        context: expected.context ?? null,
      }),
    );
    try {
      return this.call((x) => {
        const [rp, rn] = this.put(x, bytes);
        const [qp, qn] = this.put(x, request);
        return this.answer(x, x.hearken_verify(rp, rn, qp, qn));
      });
    } catch (error) {
      if (error instanceof WebAssembly.RuntimeError) {
        return { ok: false, error: "rules-crashed" };
      }
      throw error;
    }
  }
}

function asBytes(run) {
  const bytes = run instanceof Uint8Array ? run : new Uint8Array(run);
  if (bytes.length > MAX_RUN_BYTES) {
    throw new RulesError("too-large", "the run is larger than any run can be");
  }
  return bytes;
}

/**
 * The published builds, from a list of { bytes } or { module, build }: the
 * only modules runs may name. Loaded once, on first use.
 */
export function publishedRules(list) {
  let loading = null;
  // A load that failed (a broken module in the list) is tried again on the
  // next call, rather than refusing every run until a restart.
  const loaded = () => {
    loading ??= load().catch((error) => {
      loading = null;
      throw error;
    });
    return loading;
  };
  const load = async () => {
    const map = new Map();
    for (const item of list) {
      const rules = await loadRules(item.module ?? item.bytes ?? item, { build: item.build });
      map.set(rules.build, rules);
    }
    return map;
  };
  return {
    async get(build) {
      return (await loaded()).get(build) ?? null;
    },
    async builds() {
      return [...(await loaded()).keys()];
    },
  };
}
