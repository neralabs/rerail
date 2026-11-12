/**
 * The thread `moveRuleProvenancesInWorker` starts (`rule-provenance-move.ts`). It moves the provenance of the database
 * file named in `workerData` on a connection of its own, stops before its next step once the starting thread posts
 * `MOVE_STOP`, and posts back how many audits it moved. Its `VACUUM` gives way to the starting thread, which answers the
 * pings this thread posts while it runs (`watchThread`). The thread ends with the move.
 */
import { parentPort, workerData } from "node:worker_threads";
import { MOVE_STOP, moveRuleProvenances } from "./rule-provenance-move.js";
import { watchThread } from "./vacuum-giving-way.js";

const stop = new AbortController();
// Unreferenced, the port does not keep the thread alive once the move has ended; the move's own pauses do until then.
parentPort?.on("message", (message: unknown) => {
  if (message === MOVE_STOP) stop.abort();
}).unref();
const watch = parentPort === null ? {} : { watch: watchThread(parentPort) };
parentPort?.postMessage(await moveRuleProvenances(String(workerData), { signal: stop.signal, ...watch }));
