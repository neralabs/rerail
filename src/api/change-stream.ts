import { createHash } from "node:crypto";
import type { OpenAPIHono } from "@hono/zod-openapi";
import type { CheckoutContext, RuleCatalog } from "../engine/catalog/repository-model.js";
import { worktreeRulesStamp } from "../engine/catalog/worktree-pack.js";
import { branchFilesStamp } from "../engine/merge-assist/branch-git.js";
import type { ChangeFeed, ChangeNotice } from "./change-feed.js";
import type { RerailApiStores } from "./operation.js";
import { checkoutAt } from "./repository-operations.js";

const STREAM_HEADERS = { "content-type": "text/event-stream", "cache-control": "no-store" } as const;
const encoder = new TextEncoder();

/** The checkout Rerail recorded at `path`, or none; recording one is a commit, which moves the token anyway. */
function recordedCheckout(stores: RerailApiStores, path: string | undefined): CheckoutContext | undefined {
  if (path === undefined) return undefined;
  try {
    return checkoutAt(stores, path, false).checkout;
  } catch {
    return undefined;
  }
}

/**
 * What the checkout's own files say that no commit tells: its rules in `.rerail/rules`, as the catalog reads them, and
 * what its branches depend on, as Merge Assist reads them (`branchFilesStamp`). Read without holding the daemon's thread.
 */
async function checkoutFiles(checkout: CheckoutContext): Promise<string> {
  const [rules, branches] = await Promise.all([worktreeRulesStamp(checkout.checkoutPath), branchFilesStamp(checkout.checkoutPath)]);
  return `${rules}\u0000${branches}`;
}

/**
 * `gh`'s sign-in as the checkout's Merge Assist status compares it (`signInStamp`: one stat of `gh`'s hosts file, no
 * process, nothing while Merge Assist is off there). A login, logout or switch made with `gh` moves it, so an open
 * Merge Assist page reads the status, which reads `gh` because the file moved.
 */
async function signInStamp(stores: RerailApiStores, checkout: CheckoutContext): Promise<string> {
  try {
    return await stores.mergeAssist().signInStamp(checkout.checkoutPath);
  } catch {
    // The database closing: the stream's next token ends the stream.
    return "";
  }
}

/**
 * The parts of a stream's token that reads of files find, by name. Each is read on its own, so a stat that never
 * answers (a share that dropped) holds only its own part.
 */
const FILE_READS: Readonly<Record<string, (stores: RerailApiStores, checkout: CheckoutContext) => Promise<string>>> = {
  files: (_stores, checkout) => checkoutFiles(checkout),
  "sign-in": signInStamp,
};

/**
 * What the last read of each part of each scoped checkout found (`FILE_READS`), so a new stream's first token starts
 * from it and waits for no read; one entry per part and checkout a page has followed.
 */
const lastRead = new Map<string, string>();

/** One part of a stream's token: what its last read found, and a read again, one at a time. */
interface ReadPart {
  value(): string;
  /** Read with `read` unless a read of this part runs, which is shared; resolves to whether the part moved. */
  refresh(read: () => Promise<string>): Promise<boolean>;
}

/** A part kept in `lastRead` under `key`; one with no key (a stream with no checkout) stays empty. */
function readPart(key: string | undefined): ReadPart {
  let value = key === undefined ? "" : lastRead.get(key) ?? "";
  let reading: Promise<boolean> | undefined;
  const take = async (read: () => Promise<string>): Promise<boolean> => {
    const before = value;
    value = await read();
    if (key !== undefined) lastRead.set(key, value);
    return value !== before;
  };
  return {
    value: () => value,
    refresh(read) {
      reading ??= take(read).finally(() => { reading = undefined; });
      return reading;
    },
  };
}

/**
 * The token of one stream: `now` says it, `refresh` reads each part of the checkout's files again and says, part by
 * part, whether it moved.
 */
interface StreamToken {
  now(notice: ChangeNotice | "start"): string;
  refresh(): readonly Promise<boolean>[];
}

/**
 * The token a page compares, for one stream scoped to `checkout`: it moves when the database commits (`historyVersion`,
 * which creates no database), the installed packs' catalog changes (read again only at a `files` notice: their folders
 * are watched), or a part of `FILE_READS` changes: the checkout's own files, or `gh`'s sign-in while Merge Assist is on
 * there (each read again by `refresh`, one read at a time, at the stream's start and at each beat). `salt` is the
 * daemon's, so a restarted daemon never repeats a token.
 */
function streamToken(stores: RerailApiStores, salt: string, checkout: string | undefined): StreamToken {
  let installed: RuleCatalog | undefined;
  const parts = Object.entries(FILE_READS).map(([name, read]) => ({ read, part: readPart(checkout === undefined ? undefined : `${name}\u0000${checkout}`) }));
  /** The digest of the catalog and the parts, and what it was taken of: hashed again only when that moved. */
  let hashed = { of: "", digest: "" };
  const digestOf = (input: string): string => {
    if (input !== hashed.of) hashed = { of: input, digest: createHash("sha256").update(input).digest("hex").slice(0, 32) };
    return hashed.digest;
  };
  return {
    now(notice) {
      if (installed === undefined || notice === "files") installed = stores.ruleCatalog();
      const input = [installed.digest, ...parts.map(({ part }) => part.value())].join("\u0000");
      return `${salt}.${stores.historyVersion() ?? "none"}.${digestOf(input)}`;
    },
    refresh() {
      const found = recordedCheckout(stores, checkout);
      return parts.map(({ read, part }) => part.refresh(() => (found === undefined ? Promise.resolve("") : read(stores, found))));
    },
  };
}

/** The message a notice sends: the feed tells `data` only once a commit shows, and a beat says the daemon is alive. */
function message(notice: ChangeNotice, token: string): string {
  return notice === "conflicts" ? `event: conflicts\ndata: ${token}\n\n` : `data: ${token}\n\n`;
}

/** Close a stream that may already be closed or cancelled. */
function closeQuietly(controller: ReadableStreamDefaultController<Uint8Array>): void {
  try {
    controller.close();
  } catch {
    // Already closed: the client left first.
  }
}

/**
 * The held response of `GET /api/changes`, as server-sent events: the token at once, then the token again at each
 * commit the change feed tells and at each beat, a `conflicts` message on each Merge Assist conflict event, and the
 * token once more when a read of a part of `FILE_READS` (the checkout's files, `gh`'s sign-in), at the start and at
 * each beat, finds it moved. No message waits for a file read, which shares Node's few I/O threads with the daemon's
 * other work. It ends only when the client leaves (the body is cancelled, and the stream stops listening) or the
 * daemon closes its connections at stop, which the page reads as the daemon gone. A token it cannot read (the database closing) ends it too.
 */
function changeStream(feed: ChangeFeed, token: StreamToken): Response {
  let stop = (): void => {};
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      const send = (text: () => string): void => {
        try {
          controller.enqueue(encoder.encode(text()));
        } catch {
          stop();
          closeQuietly(controller);
        }
      };
      const readFiles = (): void => {
        for (const read of token.refresh()) void read.then(moved => { if (moved) send(() => `data: ${token.now("beat")}\n\n`); });
      };
      send(() => `data: ${token.now("start")}\n\n`);
      stop = feed.listen(notice => {
        send(() => message(notice, token.now(notice)));
        if (notice === "beat") readFiles();
      });
      readFiles();
    },
    cancel() {
      stop();
    },
  });
  return new Response(body, { headers: STREAM_HEADERS });
}

/**
 * `GET /api/changes[?checkout=<path>]`: what the dashboard follows instead of reading everything every 5 s. Not an
 * operation: it is HTTP's own (MCP has no stream), so neither the OpenAPI document nor the MCP tools list it. Stores
 * without a change feed answer 404, and the page then reads again only on what the person does.
 */
export function mountChangeStream(app: OpenAPIHono, stores: RerailApiStores, salt: string): void {
  app.get("/api/changes", context => {
    const feed = stores.changes?.();
    if (feed === undefined) return context.json({ error: "not-found", message: "These stores have no change stream." }, 404);
    return changeStream(feed, streamToken(stores, salt, context.req.query("checkout")));
  });
}
