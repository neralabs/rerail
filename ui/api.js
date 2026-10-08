/**
 * The daemon serves this page, so every request is same-origin and relative.
 * A conditional GET that answers 304 reports no body; the caller keeps what it has.
 */

/** Resolve `path` against this page and refuse anything that leaves the daemon's `/api/` surface. */
function apiUrl(path) {
  const url = new URL(path, location.origin);
  if (url.origin !== location.origin || !url.pathname.startsWith("/api/")) throw new Error(`refusing request outside /api/: ${path}`);
  return url;
}

export async function get(path, { etag } = {}) {
  const response = await fetch(apiUrl(path), {
    headers: etag === undefined ? {} : { "if-none-match": etag },
  });
  const nextEtag = response.headers.get("etag") ?? undefined;
  if (response.status === 304) return { status: 304, etag: nextEtag, body: undefined };
  if (!response.ok) throw new Error(`${path} answered ${response.status}`);
  return { status: response.status, etag: nextEtag, body: await response.json() };
}

export async function send(method, path, body) {
  const response = await fetch(apiUrl(path), {
    method,
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const etag = response.headers.get("etag") ?? undefined;
  const text = await response.text();
  if (!response.ok) throw new Error(`${method} ${path} answered ${response.status}: ${text.slice(0, 200)}`);
  return { status: response.status, etag, body: text === "" ? undefined : JSON.parse(text) };
}
