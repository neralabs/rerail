import { spawnSync } from "node:child_process";
import { lstatSync, readdirSync, readFileSync, realpathSync, type Dirent } from "node:fs";
import { dirname, join, posix, relative, sep } from "node:path";
import { withinCheckout } from "./checkout-control.js";
import { IGNORED_DIRECTORIES } from "./executable-policy-enforcer.js";
import { GIT_EXECUTABLE } from "./repository-identity.js";
import { compareDeterministicStrings } from "./stable-json.js";

/**
 * The local half of rule pack matching: list a repository's files once, read its `package.json`
 * dependencies, and test each pack's match definition against them. No model, no network, and no source text.
 */

/** One thing in a repository that makes a rule pack fit it. */
export type RulePackSignal =
  /** Files whose repository-relative path matches one of `files` and none of `exclude`; `label` names them in the reason. */
  | { files: readonly string[]; exclude?: readonly string[] | undefined; label: string }
  /** Any `package.json` in the repository that depends on one of these npm packages. */
  | { npmDependencies: readonly string[] };

/** A pack fits a repository when any of its signals is present. */
export interface RulePackMatch {
  readonly any: readonly RulePackSignal[];
}

/** What the matcher read from a repository. */
export interface RepositoryScan {
  /** Repository-relative paths with `/` separators, outside dependency and build-output directories. */
  readonly files: readonly string[];
  /** npm package name to the first manifest (shallowest, then by path) that depends on it. */
  readonly npmDependencies: ReadonlyMap<string, string>;
}

const GIT_LIST_BUFFER_BYTES = 256 * 1024 * 1024;
/** Outside Git the walk stops here; a folder this large is not one project. */
const MAX_WALKED_FILES = 200_000;
/** Test data says nothing about a repository's own stack: a PHP fixture does not make a repo a PHP repo. */
const FIXTURE_DIRECTORIES = new Set(["fixtures", "__fixtures__", "testdata", "test-fixtures"]);
/** A reason names at most this many findings, so it stays one short line. */
const MAX_REASON_PARTS = 2;
const DEPENDENCY_FIELDS = ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"] as const;
/** A `package.json` larger than this is not a manifest a person wrote; it is skipped unread. */
const MAX_MANIFEST_BYTES = 1024 * 1024;

/** A folder's entries, or none when the user cannot read it: such a folder says nothing about the stack. */
function readableEntries(directory: string): Dirent[] {
  try {
    return readdirSync(directory, { withFileTypes: true });
  } catch (error) {
    if (typeof error === "object" && error !== null && "code" in error && (error.code === "EACCES" || error.code === "EPERM")) return [];
    throw error;
  }
}

function walk(root: string): string[] {
  const files: string[] = [];
  const pending = [root];
  while (pending.length > 0 && files.length < MAX_WALKED_FILES) {
    const directory = pending.pop()!;
    for (const entry of readableEntries(directory)) {
      const path = join(directory, entry.name);
      if (entry.isFile()) files.push(relative(root, path).split(sep).join("/"));
      else if (entry.isDirectory() && !IGNORED_DIRECTORIES.has(entry.name) && !FIXTURE_DIRECTORIES.has(entry.name)) pending.push(path);
    }
  }
  return files;
}

function gitListing(root: string, flags: readonly string[]): string[] | undefined {
  const listed = spawnSync(GIT_EXECUTABLE, ["-C", root, "ls-files", "-z", ...flags], { encoding: "utf8", maxBuffer: GIT_LIST_BUFFER_BYTES });
  return listed.status === 0 ? listed.stdout.split("\0").filter(path => path.length > 0) : undefined;
}

/** `git ls-files -t` tag of an entry the sparse checkout left off disk (the skip-worktree bit). */
const SKIP_WORKTREE_TAG = "S";

/**
 * Files on disk under Git: tracked, less those deleted from the worktree and those a sparse checkout leaves out
 * (skip-worktree entries, which `--cached` lists and `--deleted` does not report), plus untracked-but-not-ignored;
 * a bounded walk elsewhere.
 */
export function listRepositoryFiles(root: string): string[] {
  // `-t` prefixes each entry with its status tag and a space: `H` tracked, `S` skip-worktree, `?` untracked, ….
  const tagged = gitListing(root, ["-t", "--cached", "--others", "--exclude-standard"]);
  const listed = tagged?.filter(entry => !entry.startsWith(`${SKIP_WORKTREE_TAG} `)).map(entry => entry.slice(2));
  const deleted = new Set(listed === undefined ? [] : gitListing(root, ["--deleted"]) ?? []);
  const paths = listed === undefined ? walk(root) : listed.filter(path => !deleted.has(path));
  return [...new Set(paths)]
    .filter(path => !path.split("/").slice(0, -1).some(segment => IGNORED_DIRECTORIES.has(segment) || FIXTURE_DIRECTORIES.has(segment)))
    .sort(compareDeterministicStrings);
}

/**
 * Whether `path` is a manifest the matcher may read: a regular file (not a symlink, which Git lists as tracked and
 * could point outside the repository or at a device), whose folder resolves inside `root`, and no larger than
 * `MAX_MANIFEST_BYTES`.
 */
function readableManifest(root: string, path: string): boolean {
  try {
    const info = lstatSync(path);
    return info.isFile() && info.size <= MAX_MANIFEST_BYTES && withinCheckout(realpathSync(root), realpathSync(dirname(path)));
  } catch {
    // A manifest that vanished or cannot be inspected says nothing about the stack.
    return false;
  }
}

function readManifestDependencies(root: string, path: string): string[] {
  if (!readableManifest(root, path)) return [];
  let manifest: unknown;
  try {
    manifest = JSON.parse(readFileSync(path, "utf8"));
  } catch {
    // An unreadable or half-written manifest says nothing about the stack; the other signals still count.
    return [];
  }
  if (typeof manifest !== "object" || manifest === null) return [];
  return DEPENDENCY_FIELDS.flatMap(field => {
    const dependencies = (manifest as Record<string, unknown>)[field];
    return typeof dependencies === "object" && dependencies !== null ? Object.keys(dependencies) : [];
  });
}

export function scanRepository(root: string): RepositoryScan {
  const files = listRepositoryFiles(root);
  const npmDependencies = new Map<string, string>();
  const manifests = files
    .filter(path => posix.basename(path) === "package.json")
    .sort((left, right) => left.split("/").length - right.split("/").length || compareDeterministicStrings(left, right));
  for (const manifest of manifests) {
    for (const dependency of readManifestDependencies(root, join(root, manifest))) {
      if (!npmDependencies.has(dependency)) npmDependencies.set(dependency, manifest);
    }
  }
  return { files, npmDependencies };
}

/** How many characters the glob token at `index` spans: `**\/`, `**`, a whole `{…}` group, or one character. */
function globTokenLength(glob: string, index: number): number {
  if (glob.startsWith("**/", index)) return 3;
  if (glob.startsWith("**", index)) return 2;
  if (glob[index] !== "{") return 1;
  const close = glob.indexOf("}", index);
  if (close === -1) throw new Error(`Unclosed { in glob ${glob}`);
  const nested = glob.indexOf("{", index + 1);
  if (nested !== -1 && nested < close) throw new Error(`Nested { in glob ${glob}: braces do not nest`);
  return close + 1 - index;
}

function translateGlobToken(token: string, index: number, glob: string): string {
  const segmentStart = index === 0 || glob[index - 1] === "/";
  if (token === "**/") return segmentStart ? "(?:[^/]*/)*" : "[^/]*/";
  if (token === "**") return segmentStart && index + token.length === glob.length ? ".*" : "[^/]*";
  if (token === "*") return "[^/]*";
  if (token === "?") return "[^/]";
  if (token.startsWith("{")) return `(?:${token.slice(1, -1).split(",").map(translateGlob).join("|")})`;
  return token.replace(/[.+^$(){}|[\]\\]/gu, String.raw`\$&`);
}

function translateGlob(glob: string): string {
  let pattern = "";
  let index = 0;
  while (index < glob.length) {
    const length = globTokenLength(glob, index);
    pattern += translateGlobToken(glob.slice(index, index + length), index, glob);
    index += length;
  }
  return pattern;
}

/**
 * One expression matching any of `globs`: `**` spans directories when it is a whole segment (`**\/x` also matches
 * `x`) and acts as `*` otherwise (`**.ts` matches `a.ts`, not `src/a.ts`), as in minimatch; `*` and `?` stay inside
 * one segment; `{a,b}` alternates, without nesting. A malformed glob throws.
 */
export function globPattern(globs: readonly string[]): RegExp {
  return new RegExp(`^(?:${globs.map(translateGlob).join("|")})$`, "u");
}

function commonDirectory(paths: readonly string[]): string {
  const [first, ...rest] = paths.map(path => posix.dirname(path).split("/"));
  if (first === undefined) return "";
  let length = first.length;
  for (const segments of rest) {
    let shared = 0;
    while (shared < length && segments[shared] === first[shared]) shared += 1;
    length = shared;
  }
  const common = first.slice(0, length).join("/");
  return common === "." ? "" : common;
}

function signalReason(signal: RulePackSignal, scan: RepositoryScan): string | undefined {
  if ("npmDependencies" in signal) {
    for (const dependency of signal.npmDependencies) {
      const manifest = scan.npmDependencies.get(dependency);
      if (manifest !== undefined) return `the ${dependency} dependency in ${manifest}`;
    }
    return undefined;
  }
  const include = globPattern(signal.files);
  const exclude = signal.exclude === undefined || signal.exclude.length === 0 ? undefined : globPattern(signal.exclude);
  const matched = scan.files.filter(path => include.test(path) && exclude?.test(path) !== true);
  if (matched.length === 0) return undefined;
  const directory = commonDirectory(matched);
  return directory === "" ? signal.label : `${signal.label} in ${directory}`;
}

/**
 * Why `match` fits the scanned repository, built from what was found ("matches the react dependency in
 * web/package.json and *.tsx files in web/src"), or undefined when it does not fit.
 */
export function matchReason(match: RulePackMatch, scan: RepositoryScan): string | undefined {
  const parts = match.any.flatMap(signal => signalReason(signal, scan) ?? []);
  return parts.length === 0 ? undefined : `matches ${parts.slice(0, MAX_REASON_PARTS).join(" and ")}`;
}
