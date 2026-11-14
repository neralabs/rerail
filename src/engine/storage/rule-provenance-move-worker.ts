/**
 * The thread `moveRuleProvenancesInWorker` starts (`rule-provenance-move.ts`). It moves the provenance of the database
 * file named in `workerData` (`MoveWorkerData`) on a connection of its own, stops before its next step once the
 * starting thread posts `MOVE_STOP`, and posts back how many audits it moved. The thread ends with the move.
 */
import { parentPort, workerData } from "node:worker_threads";
import { MOVE_STOP, moveRuleProvenances, type MoveWorkerData } from "./rule-provenance-move.js";

// `moveRuleProvenancesInWorker` starts this thread with a `MoveWorkerData`.
const { path, olderWindowMs } = workerData as MoveWorkerData;
const stop = new AbortController();
// Unreferenced, the port does not keep the thread alive once the move has ended; the move's own pauses do until then.
parentPort?.on("message", (message: unknown) => {
  if (message === MOVE_STOP) stop.abort();
}).unref();
parentPort?.postMessage(await moveRuleProvenances(path, { signal: stop.signal, olderWindowMs }));
