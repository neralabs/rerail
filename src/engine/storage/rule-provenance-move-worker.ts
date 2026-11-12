/**
 * The thread `moveRuleProvenancesInWorker` starts (`rule-provenance-move.ts`). It moves the provenance of the database
 * file named in `workerData` on a connection of its own, stops before its next step once the starting thread posts a
 * message, and posts back how many audits it moved. The thread ends with the move.
 */
import { parentPort, workerData } from "node:worker_threads";
import { moveRuleProvenances } from "./rule-provenance-move.js";

const stop = new AbortController();
// Unreferenced, the port does not keep the thread alive once the move has ended; the move's own pauses do until then.
parentPort?.once("message", () => stop.abort()).unref();
parentPort?.postMessage(await moveRuleProvenances(String(workerData), { signal: stop.signal }));
