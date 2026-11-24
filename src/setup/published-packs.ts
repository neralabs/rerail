/**
 * The rule packs published on npm, for the daemon's `GET /api/rule-packs/published` (the dashboard's Add rule packs):
 * every pack `discoverPacks` offers, installed or not, held for a day. Discovery runs on a worker thread of its own
 * (`published-packs-worker.ts`): checking the fetched packs compiles every rule, seconds of CPU the thread that answers
 * requests must not spend. npm runs only when a person asks (docs/privacy.md): the daemon never reads it on its own,
 * and a coding agent's read gets what is held. Each npm call is stopped after `PUBLISHED_FETCH_TIMEOUT_MS`, so a
 * registry that never answers ends the read with why (#633). A failed read, and each pack a read leaves out, is
 * logged to the daemon log, as the update check logs its failures.
 */
import { Worker } from "node:worker_threads";
import type { PublishedRulePack, PublishedRulePackReader, PublishedRulePacks } from "../engine/catalog/rule-packs.js";
import type { PackDiscovery } from "./rule-pack-store.js";

/** How long a list read from npm is shown before a person's read asks npm again. */
export const PUBLISHED_PACKS_MAX_AGE_MS = 24 * 60 * 60_000;

/**
 * After a read that failed, a person's read asks npm again only once this has passed: offline, a read still checks
 * every installed pack, seconds of CPU, and a person who comes back to the page should not start one each time.
 */
export const PUBLISHED_RETRY_MS = 5 * 60_000;

/** Each npm call of a read is stopped at this: a pack is a few MB, seconds on most lines. */
export const PUBLISHED_FETCH_TIMEOUT_MS = 60_000;

/**
 * What `discoverPublished` posts back (`published-packs-worker.ts`): the packs, with each one left out and why (it
 * could not be fetched, or failed a check), or why npm gave none.
 */
export type PublishedPacksMessage =
  | { readonly packs: readonly PublishedRulePack[]; readonly problems: readonly string[] }
  | { readonly unread: string };

/** What a discovery for no repository offers: the packs it found, or why npm gave none (offline, no access). */
export function publishedPacksOf(discovery: Pick<PackDiscovery, "available" | "problems" | "downloadFailures">): PublishedPacksMessage {
  if (discovery.downloadFailures !== undefined) return { unread: discovery.downloadFailures.join("; ") };
  return { packs: discovery.available.map(({ pack, packageName, version, ruleCount }) => ({ pack, packageName, version, ruleCount })), problems: discovery.problems };
}

/** What the worker is started with. */
export interface PublishedPacksWorkerData {
  readonly home: string;
  readonly fetchTimeoutMs: number;
}

export interface PublishedPackReaderOptions {
  readonly home: string;
  /** Reads npm once; the default runs discovery on a worker thread. */
  readonly discover?: (home: string) => Promise<PublishedPacksMessage>;
  readonly now?: () => number;
  readonly maxAgeMs?: number;
  readonly retryMs?: number;
  /** Where failures go: the daemon's stderr, which is its log, by default. */
  readonly log?: (line: string) => void;
}

const NOT_READ = { unread: "The published rule packs are read when a person opens Add rule packs in the dashboard, and none has yet." } as const;

/** The first line of what a failed read threw. */
const firstLine = (error: unknown): string => (error instanceof Error ? error.message.split("\n")[0]! : String(error));

/**
 * The reader the daemon serves the published packs from. A list read is held for `maxAgeMs`; a failed read is not, but
 * the next read waits `retryMs`, and until then reads get the list read earlier, else why it failed. Reads never
 * overlap: one asked while another runs waits for it, or gets the list held.
 */
export function publishedPackReader(options: PublishedPackReaderOptions): PublishedRulePackReader {
  const discover = options.discover ?? discoverPublished;
  const now = options.now ?? Date.now;
  const maxAgeMs = options.maxAgeMs ?? PUBLISHED_PACKS_MAX_AGE_MS;
  const retryMs = options.retryMs ?? PUBLISHED_RETRY_MS;
  const log = options.log ?? (line => process.stderr.write(line));
  let held: Extract<PublishedRulePacks, { packs: unknown }> | undefined;
  let failed: { readonly at: number; readonly reason: string } | undefined;
  let reading: Promise<PublishedRulePacks> | undefined;

  const fail = (reason: string): PublishedRulePacks => {
    failed = { at: now(), reason };
    log(`rerail daemon: reading the published rule packs from npm failed, and a person's read asks again after ${Math.round(retryMs / 60_000)} min: ${reason}\n`);
    return { unread: reason };
  };
  const keep = (packs: readonly PublishedRulePack[], problems: readonly string[]): PublishedRulePacks => {
    for (const problem of problems) log(`rerail daemon: Add rule packs leaves out a published rule pack: ${problem}\n`);
    failed = undefined;
    held = { packs, readAt: now() };
    return held;
  };
  const readNpm = (): Promise<PublishedRulePacks> => {
    reading ??= discover(options.home)
      .then(answer => ("unread" in answer ? fail(answer.unread) : keep(answer.packs, answer.problems)), (error: unknown) => fail(firstLine(error)))
      .finally(() => { reading = undefined; });
    return reading;
  };
  // Nothing held or an old list, and no read failed lately.
  const due = (): boolean => (held === undefined || now() - held.readAt >= maxAgeMs) && (failed === undefined || now() - failed.at >= retryMs);
  const shown = (): PublishedRulePacks => held ?? (failed === undefined ? NOT_READ : { unread: failed.reason });

  return {
    async read(refresh) {
      if (refresh && (reading !== undefined || due())) {
        const read = readNpm();
        // An old list is shown while the next one is read, so a person never waits on npm twice.
        if (held === undefined) return await read;
      }
      return shown();
    },
  };
}

/**
 * The worker beside this module: its JavaScript as the package ships it, or, where this module runs from its
 * TypeScript source as the tests run it, that source through `tsx`.
 */
function publishedPacksWorker(data: PublishedPacksWorkerData): Worker {
  const fromSource = new URL(import.meta.url).pathname.endsWith(".ts");
  const entry = new URL(`./published-packs-worker.${fromSource ? "ts" : "js"}`, import.meta.url);
  return new Worker(entry, { workerData: data, ...(fromSource ? { execArgv: ["--import", "tsx"] } : {}) });
}

/** One discovery on a worker thread, which ends with it. Unreferenced, it never keeps a closing daemon alive. */
async function discoverPublished(home: string): Promise<PublishedPacksMessage> {
  const worker = publishedPacksWorker({ home, fetchTimeoutMs: PUBLISHED_FETCH_TIMEOUT_MS });
  worker.unref();
  return await new Promise((resolve, reject) => {
    let answer: PublishedPacksMessage | undefined;
    worker.once("message", (message: PublishedPacksMessage) => { answer = message; });
    worker.once("error", reject);
    worker.once("exit", code => {
      if (answer !== undefined) resolve(answer);
      else reject(new Error(`reading the published rule packs ended with code ${code} and no answer`));
    });
  });
}
