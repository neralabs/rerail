/**
 * The daemon serves this page, so every request is same-origin and relative, and carries the daemon's access
 * capability as `x-rerail-access`, from this origin's storage (index.html keeps it there from the link that opened
 * the page). A custom header cannot cross origins without a preflight the daemon never grants, so no other site can
 * send it. A conditional GET that answers 304 reports no body; the caller keeps what it has.
 */

/** What a request without the capability is told: the page has to be opened from its link once. */
export const ACCESS_HINT = "Open the dashboard with rerail manage: this browser does not have its access link yet.";

/** Where this origin's storage keeps the capability; index.html writes it there under the same name. */
export const ACCESS_KEY = "rerail-access";

/** A `401`: the daemon is up and this browser lacks its capability, which is not a lost connection. */
function accessRequired() {
  return Object.assign(new Error(ACCESS_HINT), { name: "AccessRequired" });
}

/** Resolve `path` against this page and refuse anything that leaves the daemon's `/api/` surface or `/healthz`. */
function apiUrl(path) {
  const url = new URL(path, location.origin);
  if (url.origin !== location.origin || !(url.pathname.startsWith("/api/") || url.pathname === "/healthz")) throw new Error(`refusing request outside /api/: ${path}`);
  return url;
}

function accessHeaders() {
  let access = null;
  try {
    access = localStorage.getItem(ACCESS_KEY);
  } catch {}
  return access === null ? {} : { "x-rerail-access": access };
}

/**
 * The API's error `code` and its own `apiMessage`, the reason for a person, in a refusal's body (`{ error, message }`);
 * each `null` when the body holds none.
 */
async function refusal(response) {
  try {
    const body = await response.json();
    return {
      code: typeof body?.error === "string" ? body.error : null,
      apiMessage: typeof body?.message === "string" && body.message.trim() !== "" ? body.message : null,
    };
  } catch {
    return { code: null, apiMessage: null };
  }
}

/**
 * What a read that did not answer OK throws: the 401's `AccessRequired`, else an Error carrying the answer's `status`,
 * and the API's error `code` and `apiMessage` (each `null` without one), its `message` "<path> answered <status>".
 */
async function refused(path, response) {
  if (response.status === 401) return accessRequired();
  return Object.assign(new Error(`${path} answered ${response.status}`), { status: response.status, ...await refusal(response) });
}

/** A failed read throws as `refused` says; callers match its `message`. */
export async function get(path, { etag } = {}) {
  const response = await fetch(apiUrl(path), {
    headers: { ...accessHeaders(), ...(etag === undefined ? {} : { "if-none-match": etag }) },
  });
  const nextEtag = response.headers.get("etag") ?? undefined;
  if (response.status === 304) return { status: 304, etag: nextEtag, body: undefined };
  if (!response.ok) throw await refused(path, response);
  return { status: response.status, etag: nextEtag, body: await response.json() };
}

/** One server-sent event's lines, up to the blank line that ends it, as `{ event, data }`; `event` "message" by default. */
export function parseMessage(text) {
  let event = "message";
  const data = [];
  for (const line of text.split("\n")) {
    if (line.startsWith("event: ")) event = line.slice("event: ".length);
    else if (line.startsWith("data: ")) data.push(line.slice("data: ".length));
  }
  return { event, data: data.join("\n") };
}

/** Hands on each whole message in `text`, in order; returns the start of a message not whole yet. */
function deliver(text, onMessage) {
  let rest = text;
  for (let end = rest.indexOf("\n\n"); end !== -1; end = rest.indexOf("\n\n")) {
    onMessage(parseMessage(rest.slice(0, end)));
    rest = rest.slice(end + 2);
  }
  return rest;
}

/**
 * Follow a stream of server-sent events the daemon holds open (its change stream, `GET /api/changes`), through
 * `fetch` because `EventSource` cannot send the access header; `onMessage` gets each message as it arrives. Resolves
 * when the daemon ends it, rejects when it was refused (as `get` rejects) or broke, when `onMessage` throws, or once
 * `signal` ends it here. However it ends, the stream is let go of, so its connection closes.
 */
export async function follow(path, onMessage, signal) {
  const response = await fetch(apiUrl(path), { headers: accessHeaders(), signal });
  if (!response.ok) throw await refused(path, response);
  const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
  let rest = "";
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) return;
      rest = deliver(rest + value, onMessage);
    }
  } finally {
    reader.cancel().catch(() => {});
  }
}

/**
 * What a write that did not answer OK throws: an Error "<method> <path> answered <status>: <body>", or for a change the
 * daemon made but could not confirm on the disk (`503 not-on-disk`), a `NotOnDisk` Error in the daemon's own words,
 * which say the change was made: the page must not tell a person it failed.
 */
function writeRefused(method, path, status, text) {
  let body = null;
  try {
    body = JSON.parse(text);
  } catch {}
  if (body?.error === "not-on-disk" && typeof body.message === "string") return Object.assign(new Error(body.message), { name: "NotOnDisk" });
  return new Error(`${method} ${path} answered ${status}: ${text.slice(0, 200)}`);
}

/**
 * Whether a write's error is a change the daemon made but could not confirm on the disk (`writeRefused`'s
 * `NotOnDisk`): the page keeps what it shows of the change and sends the writes after it.
 */
export const madeButUnconfirmed = error => error?.name === "NotOnDisk";

/** `signal`, when given, aborts the request: `send` then rejects with the signal's reason. */
export async function send(method, path, body, { signal } = {}) {
  const response = await fetch(apiUrl(path), {
    method,
    headers: { ...accessHeaders(), "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  const etag = response.headers.get("etag") ?? undefined;
  const text = await response.text();
  if (response.status === 401) throw accessRequired();
  if (!response.ok) throw writeRefused(method, path, response.status, text);
  return { status: response.status, etag, body: text === "" ? undefined : JSON.parse(text) };
}
