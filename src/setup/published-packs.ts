/**
 * The rule packs published on npm, for the daemon's `GET /api/rule-packs/published` (the dashboard's Add rule packs):
 * every pack `discoverPacks` offers, installed or not, held for a day. Discovery runs on a worker thread of its own
 * (`published-packs-worker.ts`): checking the fetched packs compiles every rule, seconds of CPU the thread that answers
 * requests must not spend. npm runs only when a person asks (docs/privacy.md): the daemon never reads it on its own,
 * and a coding agent's read gets what is held. Each npm call is stopped after `PUBLISHED_FETCH_TIMEOUT_MS`, so a
 * registry that never answers ends the read with why (#633).
 */
import { Worker } from "node:worker_threads";
import type { PublishedRulePack, PublishedRulePackReader, PublishedRulePacks } from "../engine/catalog/rule-packs.js";

/** How long a list read from npm is shown before a person's read asks npm again. */
export const PUBLISHED_PACKS_MAX_AGE_MS = 24 * 60 * 60_000;

/** Each npm call of a read is stopped at this: a pack is a few MB, seconds on most lines. */
export const PUBLISHED_FETCH_TIMEOUT_MS = 60_000;

/** What `discoverPublished` posts back (`published-packs-worker.ts`): the packs, or why npm gave none. */
export type PublishedPacksMessage = { readonly packs: readonly PublishedRulePack[] } | { readonly unread: string };

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
}

/**
 * The reader the daemon serves the published packs from. A read that fails is not held: the next person's read asks
 * npm again, while a list read earlier stays shown. Reads never overlap: one asked while another runs waits for it.
 */
export function publishedPackReader(options: PublishedPackReaderOptions): PublishedRulePackReader {
  const discover = options.discover ?? discoverPublished;
  const now = options.now ?? Date.now;
  const maxAgeMs = options.maxAgeMs ?? PUBLISHED_PACKS_MAX_AGE_MS;
  let held: Extract<PublishedRulePacks, { packs: unknown }> | undefined;
  let reading: Promise<PublishedRulePacks> | undefined;

  const readNpm = (): Promise<PublishedRulePacks> => {
    reading ??= (async (): Promise<PublishedRulePacks> => {
      try {
        const answer = await discover(options.home);
        if ("unread" in answer) return answer;
        held = { packs: answer.packs, readAt: now() };
        return held;
      } catch (error) {
        return { unread: error instanceof Error ? error.message.split("\n")[0]! : String(error) };
      } finally {
        reading = undefined;
      }
    })();
    return reading;
  };

  return {
    async read(refresh) {
      if (!refresh) return held ?? { unread: "The published rule packs are read when a person opens Add rule packs in the dashboard, and none has yet." };
      if (held === undefined) return await readNpm();
      // An old list is shown while the next one is read, so a person never waits on npm twice.
      if (now() - held.readAt >= maxAgeMs) void readNpm();
      return held;
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
