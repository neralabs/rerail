import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import type { OpenAPIHono } from "@hono/zod-openapi";
import type { CheckoutContext } from "../engine/catalog/repository-model.js";
import type { ChangeFeed, ChangeNotice } from "./change-feed.js";
import type { RerailApiStores } from "./operation.js";
import { checkoutAt, checkoutRuleCatalog } from "./repository-operations.js";

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
 * The checkout's `HEAD` file as it reads now: the branch Merge Assist's status names. Git writes it with no commit to
 * the database, so the beat reads it. A worktree's `.git` is a file naming its folder; anything unreadable is empty.
 */
function headOf(checkout: CheckoutContext | undefined): string {
  if (checkout === undefined) return "";
  try {
    const dotGit = join(checkout.checkoutPath, ".git");
    const named = statSync(dotGit).isDirectory() ? dotGit : /^gitdir: (.+)/u.exec(readFileSync(dotGit, "utf8"))?.[1]?.trim();
    return named === undefined ? "" : readFileSync(join(resolve(checkout.checkoutPath, named), "HEAD"), "utf8");
  } catch {
    return "";
  }
}

/**
 * What files say for a page scoped to `path`, as one digest: the rule catalog it reads (the installed packs, and the
 * checkout's own rules in `.rerail/rules`) and the checkout's branch.
 */
function filesStamp(stores: RerailApiStores, path: string | undefined): string {
  const checkout = recordedCheckout(stores, path);
  const digest = createHash("sha256").update(checkoutRuleCatalog(stores, checkout).digest).update("\u0000").update(headOf(checkout));
  return digest.digest("hex").slice(0, 32);
}

/**
 * The token a page compares, for one stream: it moves when the database commits (`historyVersion`, which creates no
 * database) or what files say for the scope changes (`filesStamp`). `salt` is the daemon's, so a restarted daemon
 * never repeats a token. The files are read again only at the stream's start and its beats: no notice covers them.
 */
function streamToken(stores: RerailApiStores, salt: string, checkout: string | undefined): (notice: ChangeNotice | "start") => string {
  let files = "";
  return notice => {
    if (notice === "start" || notice === "beat") files = filesStamp(stores, checkout);
    return `${salt}.${stores.historyVersion() ?? "none"}.${files}`;
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
 * commit the change feed tells and at each beat, and a `conflicts` message on each Merge Assist conflict event. It ends only
 * when the client leaves (the body is cancelled, and the stream stops listening) or the daemon closes its connections
 * at stop, which the page reads as the daemon gone. A token it cannot read (the database closing) ends it too.
 */
function changeStream(feed: ChangeFeed, token: (notice: ChangeNotice | "start") => string): Response {
  let stop = (): void => {};
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(`data: ${token("start")}\n\n`));
      stop = feed.listen(notice => {
        try {
          controller.enqueue(encoder.encode(message(notice, token(notice))));
        } catch {
          stop();
          closeQuietly(controller);
        }
      });
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
