// One verification in its own thread, so `workerReplay` can stop it.
import { parentPort, workerData } from "node:worker_threads";
import { Rules } from "./rules.js";

const { module, build, run, expected } = workerData;
const rules = new Rules(module, build);
try {
  parentPort.postMessage({ answer: rules.verify(new Uint8Array(run), expected) });
} catch (error) {
  parentPort.postMessage({ failed: String(error?.message ?? error) });
}
