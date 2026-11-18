import type { OpenAPIHono } from "@hono/zod-openapi";
import type { ChangeFeed, ChangeNotice } from "./change-feed.js";
import type { RerailApiStores } from "./operation.js";
import { checkoutAt, checkoutRuleCatalog } from "./repository-operations.js";

const STREAM_HEADERS = { "content-type": "text/event-stream", "cache-control": "no-store" } as const;
const encoder = new TextEncoder();

/**
 * The digest of the rule catalog a page scoped to `checkout` reads: the installed packs, and that checkout's own rules
 * (`.rerail/rules`). A checkout Rerail has not recorded yet has the installed packs alone; recording it is a commit,
 * which moves the token anyway.
 */
function catalogDigest(stores: RerailApiStores, checkout: string | undefined): string {
  if (checkout === undefined) return stores.ruleCatalog().digest;
  try {
    return checkoutRuleCatalog(stores, checkoutAt(stores, checkout, false).checkout).digest;
  } catch {
    return stores.ruleCatalog().digest;
  }
}

/**
 * The token a page compares, for one stream: it moves when the database commits (`historyVersion`, which creates no
 * database) or the scope's rule catalog changes. `salt` is the daemon's, so a restarted daemon never repeats a token.
 * The catalog is read again only at the stream's start and its beats: it is files, which no notice covers.
 */
function streamToken(stores: RerailApiStores, salt: string, checkout: string | undefined): (notice: ChangeNotice | "start") => string {
  let digest = "";
  return notice => {
    if (notice === "start" || notice === "beat") digest = catalogDigest(stores, checkout);
    return `${salt}.${stores.historyVersion() ?? "none"}.${digest}`;
  };
}

/** The message a notice sends, or none: a beat always (the daemon is alive), a data notice only once the token moved. */
function message(notice: ChangeNotice, token: string, last: string): string | undefined {
  if (notice === "conflicts") return `event: conflicts\ndata: ${token}\n\n`;
  return notice === "beat" || token !== last ? `data: ${token}\n\n` : undefined;
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
 * The held response of `GET /api/changes`, as server-sent events: the token at once, then a message whenever it
 * moves, a `conflicts` message on each Merge Assist conflict event, and the token again at each beat. It ends only
 * when the client leaves (the body is cancelled, and the stream stops listening) or the daemon closes its connections
 * at stop, which the page reads as the daemon gone. A token it cannot read (the database closing) ends it too.
 */
function changeStream(feed: ChangeFeed, token: (notice: ChangeNotice | "start") => string): Response {
  let stop = (): void => {};
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      let last = token("start");
      controller.enqueue(encoder.encode(`data: ${last}\n\n`));
      stop = feed.listen(notice => {
        try {
          const now = token(notice);
          const text = message(notice, now, last);
          last = now;
          if (text !== undefined) controller.enqueue(encoder.encode(text));
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
