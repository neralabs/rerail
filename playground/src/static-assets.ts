import { readdir, stat } from "node:fs/promises";
import { extname, join, posix } from "node:path";

const CONTENT_TYPE_BY_EXTENSION: Record<string, string> = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".ico": "image/x-icon",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
};
const ENTRY_PATH = "/index.html";

export interface StaticAsset {
  path: string;
  contentType: string;
}

/**
 * Every file the built client may serve, keyed by its URL path.
 *
 * The supervisor's origin also answers the control socket, so anything it serves is same-origin with
 * a worker-spawning API. Requests therefore never contribute to a filesystem path: the build is
 * enumerated once, and a request can only select an entry that enumeration already produced.
 * Symlinks and file types the client never emits are dropped while building the manifest.
 */
export async function loadAssetManifest(root: string): Promise<Record<string, StaticAsset>> {
  const manifest: Record<string, StaticAsset> = {};
  const pending = [{ directory: root, urlPrefix: "" }];
  while (pending.length > 0) {
    const { directory, urlPrefix } = pending.pop()!;
    const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
    for (const entry of entries) {
      const urlPath = `${urlPrefix}/${entry.name}`;
      if (entry.isDirectory()) {
        pending.push({ directory: join(directory, entry.name), urlPrefix: urlPath });
        continue;
      }
      // Regular files only: a symlink in the build could point anywhere on the host.
      if (!entry.isFile()) continue;
      const contentType = CONTENT_TYPE_BY_EXTENSION[extname(entry.name).toLowerCase()];
      if (contentType !== undefined) manifest[urlPath] = { path: join(directory, entry.name), contentType };
    }
  }
  return manifest;
}

/**
 * Resolve one request against the manifest. `/` and client-side routes (no file extension) answer
 * the entry document so a reload works; a missing asset stays a miss.
 */
export function selectAsset(manifest: Record<string, StaticAsset>, pathname: string): StaticAsset | undefined {
  let decoded: string;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return undefined;
  }
  const normalized = posix.normalize(decoded);
  if (extname(normalized) === "") return manifest[ENTRY_PATH];
  return manifest[normalized];
}

/** The client is built before the supervisor starts; a rebuilt bundle is picked up by its new mtime. */
export function assetManifestLoader(root: string): () => Promise<Record<string, StaticAsset>> {
  let cached: { stamp: number; manifest: Record<string, StaticAsset> } | undefined;
  return async () => {
    const stamp = await stat(join(root, "index.html")).then((entry) => entry.mtimeMs, () => 0);
    if (cached?.stamp !== stamp) cached = { stamp, manifest: await loadAssetManifest(root) };
    return cached.manifest;
  };
}
