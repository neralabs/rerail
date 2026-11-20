/**
 * The `.rerail` runner's child process (`rr-runner.ts`): `node rr-child.js <request file> <result file>`. It loads the
 * addon, answers the request with `describeRr` or `runRr`, writes the answer to the result file and exits. While it
 * runs it reports its resident memory to its parent, which kills it past its budget, and it ends itself when its
 * parent is gone. It runs in the checkout it reads: the engine keeps one store per working directory.
 */
import { readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
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

/**
 * Where the child was started with an IPC channel (`runRerailPolicies`; `describeRerailPoliciesSync` starts it
 * without one): its memory reported now and every `MEMORY_REPORT_MS`, and its end when the parent is gone.
 */
function connectToParent(directory: string): void {
  if (process.send === undefined) return;
  reportMemory();
  setInterval(reportMemory, MEMORY_REPORT_MS).unref();
  // The parent is gone (it was killed, and ran no exit handler): nothing will read the answer, so the run stops here,
  // and the temporary directory the parent made for it (`directory`) goes with it.
  process.once("disconnect", () => {
    rmSync(directory, { recursive: true, force: true, maxRetries: 5 });
    process.kill(process.pid, "SIGKILL");
  });
}

/** Why `error` was thrown, in words. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function main([requestFile, resultFile]: string[]): Promise<void> {
  if (requestFile === undefined || resultFile === undefined) throw new Error("usage: rr-child <request file> <result file>");
  connectToParent(dirname(requestFile));
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
