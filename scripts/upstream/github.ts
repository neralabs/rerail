/**
 * What the nightly report (scripts/upstream/watch.ts) reads of an upstream repository on GitHub: the blobs under a
 * directory at a commit (the trees API on `<commit>:<dir>`, so a release's whole change set is seen, where the compare API
 * stops at 300 files) and a file's text (raw.githubusercontent.com, outside the API's rate limit). One client per run
 * caches every answer and bounds the requests in flight. Tests pass a fake GitHubClient instead.
 */
import { z } from "zod";

import { pathGlob } from "../s3/pack-fixtures.mjs";
import { GITHUB_API, githubHeaders, type Sources } from "./bump.js";

/** How the report reads an upstream repository (`<owner>/<name>`) at a commit. */
export interface GitHubClient {
  /**
   * The blobs under `directory` (repository-relative, `""` for the root) at `commit`: path (repository-relative) to blob
   * sha, every depth when `recursive`, direct children otherwise; empty when the directory does not exist there.
   */
  tree(repository: string, commit: string, directory: string, recursive: boolean): Promise<ReadonlyMap<string, string>>;
  /** A file's text at `commit`, or null when it does not exist there. */
  file(repository: string, commit: string, path: string): Promise<string | null>;
}

/** How long one request may take. */
const REQUEST_TIMEOUT_MS = 60_000;
/** The most requests in flight at once. */
const CONCURRENCY = 8;

const treeSchema = z.object({
  truncated: z.boolean(),
  tree: z.array(z.object({ path: z.string(), type: z.string(), sha: z.string() })),
});

/** A path's segments, each URL-encoded, joined by `/`. */
function encodePath(path: string): string {
  return path.split("/").map(encodeURIComponent).join("/");
}

/** A GitHubClient over `sources`, caching every answer for the run. */
export function githubClient(sources: Sources): GitHubClient {
  const cache = new Map<string, Promise<unknown>>();
  let active = 0;
  const waiting: (() => void)[] = [];
  const limited = async <T>(work: () => Promise<T>): Promise<T> => {
    if (active >= CONCURRENCY) await new Promise<void>((resolve) => waiting.push(resolve));
    active += 1;
    try {
      return await work();
    } finally {
      active -= 1;
      waiting.shift()?.();
    }
  };
  const once = <T>(key: string, work: () => Promise<T>): Promise<T> => {
    if (!cache.has(key)) cache.set(key, limited(work));
    return cache.get(key) as Promise<T>;
  };
  const get = async (url: string, headers: Record<string, string>): Promise<string | null> => {
    const response = await sources.fetch(url, { headers, signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
    if (response.status === 404) return null;
    if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
    return await response.text();
  };
  return {
    tree: (repository, commit, directory, recursive) => once(`tree ${repository} ${commit}:${directory} ${recursive}`, async () => {
      const url = `${GITHUB_API}/repos/${repository}/git/trees/${commit}${directory === "" ? "" : `:${encodePath(directory)}`}${recursive ? "?recursive=1" : ""}`;
      const body = await get(url, githubHeaders(sources));
      const blobs = new Map<string, string>();
      if (body === null) return blobs;
      const tree = treeSchema.parse(JSON.parse(body));
      if (tree.truncated) throw new Error(`${url}: GitHub truncated the tree; watch narrower directories`);
      for (const entry of tree.tree) if (entry.type === "blob") blobs.set(directory === "" ? entry.path : `${directory}/${entry.path}`, entry.sha);
      return blobs;
    }),
    file: (repository, commit, path) => once(`file ${repository} ${commit}:${path}`, () => get(`https://raw.githubusercontent.com/${repository}/${commit}/${encodePath(path)}`, {})),
  };
}

/** A pattern's static directory and whether it reaches below that directory's direct children. */
export function patternDirectory(pattern: string): { directory: string; recursive: boolean } {
  const segments = pattern.split("/");
  const wild = segments.findIndex((segment) => segment.includes("*"));
  if (wild === -1) return { directory: segments.slice(0, -1).join("/"), recursive: false };
  const rest = segments.slice(wild);
  return { directory: segments.slice(0, wild).join("/"), recursive: rest.length > 1 || rest.some((segment) => segment.includes("**")) };
}

/**
 * The directory reads that cover `patterns`: one per directory, and a directory below another one read (the root aside)
 * folded into a recursive read of that one, so `tests/rules/{rule}.test.ts` and `tests/rules/{rule}/**` over sixty rules
 * are one read, not a hundred and twenty.
 */
export function directoryReads(patterns: readonly string[]): Map<string, boolean> {
  const wanted = new Map<string, boolean>();
  for (const { directory, recursive } of patterns.map(patternDirectory)) wanted.set(directory, (wanted.get(directory) ?? false) || recursive);
  const depth = (directory: string): number => (directory === "" ? 0 : directory.split("/").length);
  const reads = new Map<string, boolean>();
  for (const [directory, recursive] of [...wanted].sort(([a], [b]) => depth(a) - depth(b))) {
    const ancestor = [...reads.keys()].find((other) => other !== "" && directory.startsWith(`${other}/`));
    if (ancestor !== undefined) reads.set(ancestor, true);
    else if (!(reads.get("") ?? false) || directory === "") reads.set(directory, recursive);
  }
  return reads;
}

/** The blobs at `commit` matching any of `patterns` (repository-relative paths or `pathGlob` globs), read per directoryReads. */
export async function blobsMatching(github: GitHubClient, repository: string, commit: string, patterns: readonly string[]): Promise<Map<string, string>> {
  const trees = await Promise.all([...directoryReads(patterns)].map(([directory, recursive]) => github.tree(repository, commit, directory, recursive)));
  const literals = new Set(patterns.filter((pattern) => !pattern.includes("*")));
  const globs = patterns.filter((pattern) => pattern.includes("*")).map((pattern) => pathGlob(pattern));
  const blobs = new Map<string, string>();
  for (const tree of trees) for (const [path, sha] of tree) if (literals.has(path) || globs.some((glob) => glob.test(path))) blobs.set(path, sha);
  return blobs;
}

/** One file that differs between two commits. */
export interface FileChange {
  path: string;
  change: "added" | "modified" | "deleted";
}

/** The files whose blob differs between `before` and `after`, byte-sorted by path; a rename is a deletion and an addition. */
export function changedFiles(before: ReadonlyMap<string, string>, after: ReadonlyMap<string, string>): FileChange[] {
  const paths = [...new Set([...before.keys(), ...after.keys()])].sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
  return paths.flatMap((path): FileChange[] => {
    const [old, fresh] = [before.get(path), after.get(path)];
    if (old === fresh) return [];
    return [{ path, change: old === undefined ? "added" : fresh === undefined ? "deleted" : "modified" }];
  });
}
