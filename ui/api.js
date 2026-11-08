/**
 * The daemon serves this page, so every request is same-origin and relative, and carries the daemon's access
 * capability as `x-rerail-access`, from this origin's storage (index.html keeps it there from the link that opened
 * the page). A custom header cannot cross origins without a preflight the daemon never grants, so no other site can
 * send it. A conditional GET that answers 304 reports no body; the caller keeps what it has.
 */

/** What a request without the capability is told: the page has to be opened from its link once. */
export const ACCESS_HINT = "Open the dashboard with rerail manage: this browser does not have its access link yet.";

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
    access = localStorage.getItem("rerail-access");
  } catch {}
  return access === null ? {} : { "x-rerail-access": access };
}

export async function get(path, { etag } = {}) {
  const response = await fetch(apiUrl(path), {
    headers: { ...accessHeaders(), ...(etag === undefined ? {} : { "if-none-match": etag }) },
  });
  const nextEtag = response.headers.get("etag") ?? undefined;
  if (response.status === 304) return { status: 304, etag: nextEtag, body: undefined };
  if (response.status === 401) throw accessRequired();
  if (!response.ok) throw new Error(`${path} answered ${response.status}`);
  return { status: response.status, etag: nextEtag, body: await response.json() };
}

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
  if (!response.ok) throw new Error(`${method} ${path} answered ${response.status}: ${text.slice(0, 200)}`);
  return { status: response.status, etag, body: text === "" ? undefined : JSON.parse(text) };
}
