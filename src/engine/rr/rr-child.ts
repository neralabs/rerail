/**
 * The `.rerail` runner's child process (`rr-runner.ts`): `node rr-child.js <request file> <result file>`. It loads the
 * addon, answers the request with `describeRr` or `runRr`, writes the answer to the result file and exits. While it
 * runs it reports its resident memory to its parent, which kills it past its budget, and it ends itself when its
 * parent is gone. It runs in the checkout it reads.
 */
import { readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { Worker } from "node:worker_threads";
import { loadNativeBinding } from "../native/native.js";
import type { RrChildRequest, RrChildResult, RrMemoryReport } from "./rr-messages.js";

/** How often the child reports its resident memory: often enough that a run growing by gigabytes a second is caught. */
const MEMORY_REPORT_MS = 200;

/** The addon's answer to `request`, parsed. */
async function answer(request: RrChildRequest): Promise<unknown> {
  const runtime = loadNativeBinding().CodeRuleDslRuntime;
  if (request.kind === "describe") return JSON.parse(runtime.describeRr(JSON.stringify(request.paths)));
  return JSON.parse(await runtime.runRr(JSON.stringify(request.run)));
}

/** The process's peak resident memory so far (`maxRSS` is in kibibytes). */
function peakRssBytes(): number {
  return process.resourceUsage().maxRSS * 1024;
}

function reportMemory(): void {
  const report: RrMemoryReport = { rssBytes: process.memoryUsage.rss() };
  process.send?.(report);
}

/** The temporary directory the parent made for this child removed, and the child ended: nothing will read its answer. */
function end(directory: string): void {
  rmSync(directory, { recursive: true, force: true, maxRetries: 5 });
  process.kill(process.pid, "SIGKILL");
}

/**
 * Where the child was started with an IPC channel (`runRerailPolicies`): its memory reported now and every
 * `MEMORY_REPORT_MS`, and its end as soon as the channel closes, when the parent is gone (it was killed, and ran no
 * exit handler, which would have killed this child: `rr-child-process.ts`).
 */
function reportToParent(directory: string): void {
  if (process.send === undefined) return;
  reportMemory();
  setInterval(reportMemory, MEMORY_REPORT_MS).unref();
  process.once("disconnect", () => end(directory));
}

/** How often the watch thread looks for the parent. */
const PARENT_WATCH_MS = 1000;

/**
 * The watch thread's code: every `PARENT_WATCH_MS`, it looks for the parent, by signal 0 and, on Linux, by its state
 * (a parent killed and not reaped yet answers signal 0 as a zombie). Once it is gone, the temporary directory goes and
 * the child ends. A thread of its own, for a child with no IPC channel (`describeRerailPoliciesSync`), whose main
 * thread the synchronous `describeRr` holds for as long as it runs.
 */
const PARENT_WATCH = `
const { workerData } = require("node:worker_threads");
const { readFileSync, rmSync } = require("node:fs");
function present() {
  try {
    process.kill(workerData.parent, 0);
  } catch {
    return false;
  }
  try {
    return !/^\\d+ \\(.*\\) Z/s.test(readFileSync("/proc/" + workerData.parent + "/stat", "utf8"));
  } catch {
    return true;
  }
}
setInterval(() => {
  if (present()) return;
  rmSync(workerData.directory, { recursive: true, force: true, maxRetries: 5 });
  process.kill(process.pid, "SIGKILL");
}, workerData.every);
`;

/** The watch thread started (`PARENT_WATCH`); `directory` is the temporary directory the parent made for this child. */
function endWithParent(directory: string): void {
  new Worker(PARENT_WATCH, { eval: true, workerData: { parent: process.ppid, directory, every: PARENT_WATCH_MS } }).unref();
}

/** Why `error` was thrown, in words. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function main([requestFile, resultFile]: string[]): Promise<void> {
  if (requestFile === undefined || resultFile === undefined) throw new Error("usage: rr-child <request file> <result file>");
  reportToParent(dirname(requestFile));
  endWithParent(dirname(requestFile));
  const request = JSON.parse(readFileSync(requestFile, "utf8")) as RrChildRequest;
  const result: RrChildResult = await answer(request).then(
    value => ({ ok: true, value, peakRssBytes: peakRssBytes() }),
    (error: unknown) => ({ ok: false, message: messageOf(error), peakRssBytes: peakRssBytes() }),
  );
  writeFileSync(resultFile, JSON.stringify(result));
}

main(process.argv.slice(2)).then(
  () => process.exit(0),
  (error: unknown) => {
    process.stderr.write(`${messageOf(error)}\n`);
    process.exit(1);
  },
);
