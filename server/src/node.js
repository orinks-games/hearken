// Node's side: serve the service over node:http (a VPS, a home server, the
// dev server), load published modules from a folder, and run verifications
// in a worker thread with a hard time limit.

import { createServer } from "node:http";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { Worker } from "node:worker_threads";
import { publishedRules } from "./rules.js";

/**
 * Every `.wasm` file in `dir`, as the published builds. A module joins by
 * being put there; a server restart picks it up.
 */
export async function rulesFromDir(dir) {
  const names = (await readdir(dir)).filter((n) => n.endsWith(".wasm"));
  const list = await Promise.all(names.map(async (n) => ({ bytes: await readFile(join(dir, n)) })));
  return publishedRules(list);
}

/**
 * Runs each verification in a worker thread and stops it after `timeoutMs`,
 * answering { ok: false, error: "too-slow" }. Pass as `replay` to
 * `createService`.
 */
export function workerReplay({ timeoutMs = 5000 } = {}) {
  return (rules, run, expected) =>
    new Promise((resolve, reject) => {
      const worker = new Worker(new URL("./replay-worker.js", import.meta.url), {
        workerData: { module: rules.module, build: rules.build, run: new Uint8Array(run), expected },
      });
      const timer = setTimeout(() => {
        worker.terminate();
        resolve({ ok: false, error: "too-slow" });
      }, timeoutMs);
      worker.once("message", (message) => {
        clearTimeout(timer);
        worker.terminate();
        if (message.failed) reject(new Error(message.failed));
        else resolve(message.answer);
      });
      worker.once("error", (error) => {
        clearTimeout(timer);
        reject(error);
      });
    });
}

/** Turns a node:http request into a web Request. */
export function toRequest(req, origin) {
  const headers = new Headers();
  for (const [name, value] of Object.entries(req.headers)) {
    if (Array.isArray(value)) value.forEach((v) => headers.append(name, v));
    else if (value !== undefined) headers.set(name, value);
  }
  const method = req.method ?? "GET";
  const body =
    method === "GET" || method === "HEAD"
      ? undefined
      : new ReadableStream({
          start(controller) {
            req.on("data", (chunk) => controller.enqueue(new Uint8Array(chunk)));
            req.on("end", () => controller.close());
            req.on("error", (e) => controller.error(e));
          },
          cancel() {
            req.destroy();
          },
        });
  return new Request(new URL(req.url ?? "/", origin), { method, headers, body, duplex: "half" });
}

/** Writes a web Response to a node:http response. */
export async function sendResponse(res, response) {
  res.statusCode = response.status;
  response.headers.forEach((value, name) => res.setHeader(name, value));
  res.end(Buffer.from(await response.arrayBuffer()));
}

/**
 * Serves `fetch` (a service's, or any Request -> Response function) on
 * `port`. Behind a proxy, set `trustProxy` so X-Forwarded-For is kept;
 * otherwise it is replaced with the socket's own address.
 */
export function serve(fetch, { port = 8787, host = "127.0.0.1", trustProxy = false } = {}) {
  const server = createServer(async (req, res) => {
    try {
      if (!trustProxy) req.headers["x-forwarded-for"] = req.socket.remoteAddress ?? "unknown";
      const request = toRequest(req, `http://${req.headers.host ?? `${host}:${port}`}`);
      await sendResponse(res, await fetch(request));
    } catch {
      res.statusCode = 500;
      res.end();
    }
  });
  return new Promise((resolve) => server.listen(port, host, () => resolve(server)));
}
