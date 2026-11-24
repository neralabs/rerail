/**
 * The thread `publishedPackReader` starts for one read (`published-packs.ts`). It runs `discoverPacks` for no
 * repository over the home named in `workerData` (`PublishedPacksWorkerData`), posts back the packs it offers, or why
 * npm gave none, and ends.
 */
import { parentPort, workerData } from "node:worker_threads";
import { runNpm, type NpmRunner } from "../runtime/npm.js";
import { publishedPacksOf, type PublishedPacksWorkerData } from "./published-packs.js";
import { discoverPacks, npmPackStore } from "./rule-pack-store.js";

const { home, fetchTimeoutMs } = workerData as PublishedPacksWorkerData;
// npm run by the Node running the daemon, which finds the npm installed beside it first on PATH, as the update check
// runs it; each call is stopped after `fetchTimeoutMs`.
const npm: NpmRunner = async (args, options) => await runNpm(args, { timeoutMs: fetchTimeoutMs, ...options, node: process.execPath });
parentPort?.postMessage(publishedPacksOf(await discoverPacks(undefined, npmPackStore(home, npm))));
