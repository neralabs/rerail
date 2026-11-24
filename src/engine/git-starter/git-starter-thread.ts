/**
 * A starter thread's entry: `../git-thread.ts` starts it as a worker, and it serves that thread's starts for the life of
 * the process (`git-starter.ts`).
 */
import { parentPort } from "node:worker_threads";
import { serveGitStarts } from "./git-starter.js";

// Run as a worker, which always has a port to the thread that started it.
if (parentPort !== null) serveGitStarts(parentPort);
