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
async function checkoutFiles(stores: RerailApiStores, path: string | undefined): Promise<string> {
  const checkout = recordedCheckout(stores, path);
  if (checkout === undefined) return "";
  const [rules, branches] = await Promise.all([worktreeRulesStamp(checkout.checkoutPath), branchFilesStamp(checkout.checkoutPath)]);
  return `${rules}\u0000${branches}`;
}

/** The token of one stream: `now` says it, `refresh` reads the checkout's files again and says whether they moved. */
interface StreamToken {
  now(notice: ChangeNotice | "start"): string;
  refresh(): Promise<boolean>;
}

/**
 * What the last read of each scoped checkout's files found (`checkoutFiles`), so a new stream's first token starts
 * from it and waits for no read; one entry per checkout a page has followed.
 */
const lastFiles = new Map<string, string>();

/**
 * The token a page compares, for one stream scoped to `checkout`: it moves when the database commits (`historyVersion`,
 * which creates no database), the installed packs' catalog changes (read again only at a `files` notice: their folders
 * are watched) or the checkout's own files change (`checkoutFiles`, read again by `refresh`, one read at a time, at the
 * stream's start and at each beat). `salt` is the daemon's, so a restarted daemon never repeats a token.
 */
function streamToken(stores: RerailApiStores, salt: string, checkout: string | undefined): StreamToken {
  let installed: RuleCatalog | undefined;
  let files = checkout === undefined ? "" : lastFiles.get(checkout) ?? "";
  /** The digest of the catalog and the checkout's files, and what it was taken of: hashed again only when that moved. */
  let hashed = { of: "", digest: "" };
  const digestOf = (input: string): string => {
    if (input !== hashed.of) hashed = { of: input, digest: createHash("sha256").update(input).digest("hex").slice(0, 32) };
    return hashed.digest;
  };
  let reading: Promise<boolean> | undefined;
  const read = async (): Promise<boolean> => {
    const before = files;
    files = await checkoutFiles(stores, checkout);
    if (checkout !== undefined) lastFiles.set(checkout, files);
    return files !== before;
  };
  return {
    now(notice) {
      if (installed === undefined || notice === "files") installed = stores.ruleCatalog();
      return `${salt}.${stores.historyVersion() ?? "none"}.${digestOf(`${installed.digest}\u0000${files}`)}`;
    },
    refresh() {
      reading ??= read().finally(() => { reading = undefined; });
      return reading;
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
 * token once more when a read of the checkout's files, at the start and at each beat, finds them moved. No message
 * waits for a file read, which shares Node's few I/O threads with the daemon's other work. It ends only when the client
 * leaves (the body is cancelled, and the stream stops listening) or the daemon closes its connections at stop, which
 * the page reads as the daemon gone. A token it cannot read (the database closing) ends it too.
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
        void token.refresh().then(moved => { if (moved) send(() => `data: ${token.now("beat")}\n\n`); });
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
