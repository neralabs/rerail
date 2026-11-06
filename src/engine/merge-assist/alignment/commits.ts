/**
 * One commit as alignment reads it: its message, author and the lines each file gained and lost against its first
 * parent, from one `git show --unified=0`. Lock files, build output and generated files are skipped, and a file's
 * lines are capped, so a vendored or generated change cannot swamp a ledger. Nothing here writes to the repository.
 */
import { readGitBlobs, runGit } from "../../git.js";

export type FileStatus = "added" | "modified" | "deleted" | "renamed";

/** A run of consecutive lines one hunk adds or removes, from line `start` (1-based) of the new or old file. */
export interface LineBlock {
  readonly start: number;
  readonly lines: string[];
}

export interface FileChange {
  readonly path: string;
  /** The path before a rename; the path otherwise. */
  readonly oldPath: string;
  readonly status: FileStatus;
  /** Lines the commit adds, in order, numbered in the new file. */
  readonly added: LineBlock[];
  /** Lines the commit removes, in order, numbered in the old file. */
  readonly removed: LineBlock[];
}

export interface CommitChange {
  readonly sha: string;
  /** The first parent, or null for a root commit. */
  readonly parent: string | null;
  readonly merge: boolean;
  readonly author: string;
  readonly authoredAt: string;
  readonly committedAt: string;
  readonly subject: string;
  readonly body: string;
  readonly files: FileChange[];
}

/** Paths whose diffs say nothing a teammate needs: lock files, build output, generated and vendored code. */
const SKIPPED_PATH = /(?:^|\/)(?:[^/]*\.lock|package-lock\.json|pnpm-lock\.yaml|yarn\.lock|go\.sum|dist\/.*|build\/.*|vendor\/.*|node_modules\/.*|[^/]*\.min\.(?:js|css)|[^/]*\.generated\.[^/]+|[^/]*\.snap|[^/]*\.svg|[^/]*\.map)$/u;
/** The most lines kept per file, added and removed together. */
const MAX_FILE_LINES = 8_000;
const SHOW_OPTIONS = { maxBuffer: 256 * 1024 * 1024, timeout: 120_000 };
const FIELD = "\u001f";
const RECORD = "\u001e";
const FORMAT = `--format=%H${FIELD}%P${FIELD}%an${FIELD}%aI${FIELD}%cI${FIELD}%s${FIELD}%b${RECORD}`;

/** Whether alignment reads `path`'s changes at all. */
export function readablePath(path: string): boolean {
  return !SKIPPED_PATH.test(path);
}

/** Parses `git show <FORMAT> --unified=0 -M` output: the header fields, then each file's added and removed runs. */
export function parseShow(sha: string, output: string): CommitChange {
  const headerEnd = output.indexOf(RECORD);
  const header = (headerEnd === -1 ? output : output.slice(0, headerEnd)).split(FIELD);
  const patch = headerEnd === -1 ? "" : output.slice(headerEnd + 1);
  const parents = (header[1] ?? "").trim().split(/\s+/u).filter(Boolean);
  const parsed: Array<{ path: string; oldPath: string; status: FileStatus; added: LineBlock[]; removed: LineBlock[]; kept: number }> = [];
  let hunk: { old: number; new: number; removed: LineBlock | null; added: LineBlock | null } | null = null;
  for (const line of patch.split("\n")) {
    if (line.startsWith("diff --git ")) {
      const paths = /^diff --git "?a\/(.+?)"? "?b\/(.+?)"?$/u.exec(line);
      parsed.push({ path: paths?.[2] ?? "", oldPath: paths?.[1] ?? "", status: "modified", added: [], removed: [], kept: 0 });
      hunk = null;
      continue;
    }
    const file = parsed.at(-1);
    if (file === undefined) continue;
    const hunkHeader = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/u.exec(line);
    if (hunkHeader !== null) {
      hunk = { old: Number(hunkHeader[1]), new: Number(hunkHeader[2]), removed: null, added: null };
    } else if (hunk === null) {
      if (line.startsWith("new file mode")) file.status = "added";
      else if (line.startsWith("deleted file mode")) file.status = "deleted";
      else if (line.startsWith("rename from ")) {
        file.status = "renamed";
        file.oldPath = line.slice("rename from ".length);
      } else if (line.startsWith("rename to ")) file.path = line.slice("rename to ".length);
    } else if ((line.startsWith("+") || line.startsWith("-")) && file.kept < MAX_FILE_LINES) {
      // With --unified=0 a hunk is one run of removed lines, then one of added lines.
      if (line.startsWith("+")) {
        if (hunk.added === null) file.added.push(hunk.added = { start: hunk.new, lines: [] });
        hunk.added.lines.push(line.slice(1));
      } else {
        if (hunk.removed === null) file.removed.push(hunk.removed = { start: hunk.old, lines: [] });
        hunk.removed.lines.push(line.slice(1));
      }
      file.kept += 1;
    }
  }
  return {
    sha,
    parent: parents[0] ?? null,
    merge: parents.length > 1,
    author: header[2] ?? "",
    authoredAt: header[3] ?? "",
    committedAt: header[4] ?? "",
    subject: header[5] ?? "",
    body: (header[6] ?? "").trim(),
    files: parsed.filter(file => file.path !== "" && readablePath(file.path)).map(({ kept: _kept, ...file }) => file),
  };
}

/**
 * `sha` against its first parent; a merge commit's files are left empty (what it brings in belongs to the commits
 * it merges). Undefined when Git cannot show the commit.
 */
export function readCommit(root: string, sha: string): CommitChange | undefined {
  const meta = runGit(root, ["show", "-s", FORMAT, sha], SHOW_OPTIONS);
  if (meta.status !== 0) return undefined;
  if (/\S+\s+\S+/u.test(meta.stdout.split(FIELD)[1] ?? "")) return parseShow(sha, meta.stdout);
  const result = runGit(root, ["show", FORMAT, "--unified=0", "-M", "--no-color", "--no-ext-diff", sha], SHOW_OPTIONS);
  return result.status === 0 ? parseShow(sha, result.stdout) : undefined;
}

/** The blobs of `paths` at `revision`, as text; a missing, binary or over-large file is absent. */
export async function readTexts(root: string, revision: string, paths: readonly string[], maxBytes = 512 * 1024): Promise<Map<string, string>> {
  const blobs = await readGitBlobs(root, paths.map(path => `${revision}:${path}`), { maxBlobBytes: maxBytes, timeout: 60_000 });
  const texts = new Map<string, string>();
  for (const path of paths) {
    const blob = blobs.get(`${revision}:${path}`);
    if (blob !== undefined && !blob.includes(0)) texts.set(path, blob.toString("utf8"));
  }
  return texts;
}

/** The commits `git rev-list --reverse <base>..<head>` lists, oldest first; empty when either is unknown. */
export function commitsBetween(root: string, base: string | null, head: string): string[] {
  const result = runGit(root, ["rev-list", "--reverse", "--topo-order", base === null ? head : `${base}..${head}`], SHOW_OPTIONS);
  return result.status === 0 ? result.stdout.split("\n").filter(Boolean) : [];
}

/** Every file path in `revision`'s tree. */
export function treePaths(root: string, revision: string): string[] {
  const result = runGit(root, ["ls-tree", "-r", "-z", "--name-only", revision], SHOW_OPTIONS);
  return result.status === 0 ? result.stdout.split("\0").filter(Boolean) : [];
}
