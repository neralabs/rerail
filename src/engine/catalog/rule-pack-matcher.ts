import { lstatSync, readdirSync, readFileSync, realpathSync, type Dirent } from "node:fs";
import { dirname, join, posix, relative, sep } from "node:path";
import { withinCheckout } from "./checkout-control.js";
import { readTomlStringLists } from "./lint-config-formats.js";
import { IGNORED_DIRECTORIES } from "../code-rule/code-rule-enforcer.js";
import { GitUnavailableError, runGit } from "../git.js";
import { compareDeterministicStrings } from "../stable-json.js";

/**
 * The local half of rule pack matching: list a repository's files once, read its dependency manifests
 * (`package.json`, `requirements*.txt`, `pyproject.toml`, `composer.json`), and test each pack's match definition
 * against them. No model, no network, no source text, and no code from the pack: a match is data.
 */

export const DEPENDENCY_ECOSYSTEMS = ["npm", "pypi", "composer"] as const;
export type DependencyEcosystem = (typeof DEPENDENCY_ECOSYSTEMS)[number];

/** One thing in a repository that makes a rule pack fit it. */
export type RulePackSignal =
  /** Files whose repository-relative path matches one of `files` and none of `exclude`; `label` names them in the reason. */
  | { files: readonly string[]; exclude?: readonly string[] | undefined; label: string }
  /** Any manifest of `ecosystem` in the repository that depends on one of `names`. */
  | { dependencies: { ecosystem: DependencyEcosystem; names: readonly string[] } };

/** A pack fits a repository when any of its signals is present. */
export interface RulePackMatch {
  readonly any: readonly RulePackSignal[];
}

/** What the matcher read from a repository. */
export interface RepositoryScan {
  /** Repository-relative paths with `/` separators, outside dependency and build-output directories. */
  readonly files: readonly string[];
  /** Per ecosystem, package name (normalised) to every manifest that depends on it, shallowest first, then by path. */
  readonly dependencies: ReadonlyMap<DependencyEcosystem, ReadonlyMap<string, readonly string[]>>;
}

const GIT_LIST_BUFFER_BYTES = 256 * 1024 * 1024;
/** Outside Git the walk stops here; a folder this large is not one project. */
const MAX_WALKED_FILES = 200_000;
/** Test data says nothing about a repository's own stack: a PHP fixture does not make a repo a PHP repo. */
const FIXTURE_DIRECTORIES = new Set(["fixtures", "__fixtures__", "testdata", "test-fixtures"]);
/** A reason names at most this many findings, so it stays one short line. */
const MAX_REASON_PARTS = 2;
const DEPENDENCY_FIELDS = ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"] as const;
const COMPOSER_FIELDS = ["require", "require-dev"] as const;
/** A manifest larger than this is not one a person wrote; it is skipped unread. */
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
  let listed;
  try {
    listed = runGit(root, ["ls-files", "-z", ...flags], { maxBuffer: GIT_LIST_BUFFER_BYTES });
  } catch (error) {
    if (error instanceof GitUnavailableError) return undefined;
    throw error;
  }
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

/** The manifest's text, or undefined when it may not be read (see {@link readableManifest}) or cannot be. */
function manifestText(root: string, path: string): string | undefined {
  if (!readableManifest(root, path)) return undefined;
  try {
    return readFileSync(path, "utf8");
  } catch {
    // A manifest that vanished between listing and reading says nothing about the stack.
    return undefined;
  }
}

/** The object a JSON manifest holds, or undefined for one that is unreadable, half-written or not an object. */
function jsonManifest(root: string, path: string): Record<string, unknown> | undefined {
  const text = manifestText(root, path);
  if (text === undefined) return undefined;
  try {
    const value: unknown = JSON.parse(text);
    return typeof value === "object" && value !== null ? value as Record<string, unknown> : undefined;
  } catch {
    // An unreadable or half-written manifest says nothing about the stack; the other signals still count.
    return undefined;
  }
}

function objectKeys(manifest: Record<string, unknown>, fields: readonly string[]): string[] {
  return fields.flatMap(field => {
    const value = manifest[field];
    return typeof value === "object" && value !== null && !Array.isArray(value) ? Object.keys(value) : [];
  });
}

/** A Python distribution name as PyPI compares it (PEP 503): lowercase, each run of `-`, `_` and `.` one `-`. */
export function normalizePythonName(name: string): string {
  return name.toLowerCase().replace(/[-_.]+/gu, "-");
}

/** The distribution a PEP 508 requirement names (`Django>=4.2; python_version >= "3.10"` gives `django`). */
function requirementName(requirement: string): string | undefined {
  const name = /^\s*([A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?)/u.exec(requirement)?.[1];
  return name === undefined ? undefined : normalizePythonName(name);
}

/** `requirements*.txt`: one requirement per line; comments, blank lines and pip options (`-r`, `-e`, `--index-url`) name nothing. */
function requirementsNames(text: string): string[] {
  return text.split(/\r?\n/u).flatMap(line => {
    const requirement = line.replace(/(^|\s)#.*$/u, "").trim();
    if (requirement === "" || requirement.startsWith("-")) return [];
    return requirementName(requirement) ?? [];
  });
}

const POETRY_DEPENDENCIES = /^tool\.poetry\.(?:group\.(?:[A-Za-z0-9_-]+|"(?:[^"\\]|\\.)*")\.)?dependencies\.(.+)$/u;

/** The first key of a dotted TOML path: bare, or quoted as `readTomlStringLists` writes it. */
function firstKey(path: string): string | undefined {
  if (!path.startsWith("\"")) return path.split(".")[0];
  const quoted = /^"(?:[^"\\]|\\.)*"/u.exec(path)?.[0];
  return quoted === undefined ? undefined : JSON.parse(quoted) as string;
}

/**
 * `pyproject.toml`: PEP 621's `[project] dependencies` and `optional-dependencies`, and Poetry's
 * `[tool.poetry.dependencies]` and `[tool.poetry.group.<name>.dependencies]` (Poetry's `python` entry is the
 * interpreter, not a dependency). A file the TOML reader cannot read names nothing.
 */
function pyprojectNames(text: string): string[] {
  let read;
  try {
    read = readTomlStringLists(text, ["project.dependencies"], ["project.optional-dependencies"]);
  } catch {
    return [];
  }
  const requirements = [...(read.lists.get("project.dependencies") ?? []), ...[...read.tables.get("project.optional-dependencies")?.values() ?? []].flat()];
  const poetry = read.paths.flatMap(path => {
    const rest = POETRY_DEPENDENCIES.exec(path)?.[1];
    const key = rest === undefined ? undefined : firstKey(rest);
    return key === undefined || key === "python" ? [] : [normalizePythonName(key)];
  });
  return [...requirements.flatMap(requirement => requirementName(requirement) ?? []), ...poetry];
}

/** The ecosystem whose manifest `path` is, or undefined for a file that is no dependency manifest. */
function manifestEcosystem(path: string): DependencyEcosystem | undefined {
  const name = posix.basename(path);
  if (name === "package.json") return "npm";
  if (name === "composer.json") return "composer";
  if (name === "pyproject.toml" || /^requirements.*\.txt$/u.test(name)) return "pypi";
  return undefined;
}

/** The packages the manifest at repository-relative `path` depends on, named as its ecosystem compares them. */
function manifestDependencies(root: string, path: string, ecosystem: DependencyEcosystem): string[] {
  const absolute = join(root, path);
  if (ecosystem === "npm") {
    const manifest = jsonManifest(root, absolute);
    return manifest === undefined ? [] : objectKeys(manifest, DEPENDENCY_FIELDS);
  }
  if (ecosystem === "composer") {
    const manifest = jsonManifest(root, absolute);
    return manifest === undefined ? [] : objectKeys(manifest, COMPOSER_FIELDS).map(name => name.toLowerCase());
  }
  const text = manifestText(root, absolute);
  if (text === undefined) return [];
  return posix.basename(path) === "pyproject.toml" ? pyprojectNames(text) : requirementsNames(text);
}

/** A signal's package name as its ecosystem compares names. */
export function normalizeDependencyName(ecosystem: DependencyEcosystem, name: string): string {
  if (ecosystem === "pypi") return normalizePythonName(name);
  return ecosystem === "composer" ? name.toLowerCase() : name;
}

export function scanRepository(root: string): RepositoryScan {
  const files = listRepositoryFiles(root);
  const dependencies = new Map<DependencyEcosystem, Map<string, string[]>>(DEPENDENCY_ECOSYSTEMS.map(ecosystem => [ecosystem, new Map()]));
  const manifests = files
    .flatMap(path => {
      const ecosystem = manifestEcosystem(path);
      return ecosystem === undefined ? [] : [{ path, ecosystem }];
    })
    .sort((left, right) => left.path.split("/").length - right.path.split("/").length || compareDeterministicStrings(left.path, right.path));
  for (const { path, ecosystem } of manifests) {
    const byName = dependencies.get(ecosystem)!;
    for (const name of new Set(manifestDependencies(root, path, ecosystem))) byName.set(name, [...(byName.get(name) ?? []), path]);
  }
  return { files, dependencies };
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

/**
 * The repository-relative files that make `signal` present: every manifest that depends on one of its packages, or
 * every file its globs match. Empty when the signal is absent.
 */
export function signalEvidence(signal: RulePackSignal, scan: RepositoryScan): readonly string[] {
  if ("dependencies" in signal) {
    const { ecosystem, names } = signal.dependencies;
    const byName = scan.dependencies.get(ecosystem);
    return [...new Set(names.flatMap(name => byName?.get(normalizeDependencyName(ecosystem, name)) ?? []))];
  }
  const include = globPattern(signal.files);
  const exclude = signal.exclude === undefined || signal.exclude.length === 0 ? undefined : globPattern(signal.exclude);
  return scan.files.filter(path => include.test(path) && exclude?.test(path) !== true);
}

function signalReason(signal: RulePackSignal, scan: RepositoryScan): string | undefined {
  if ("dependencies" in signal) {
    const { ecosystem, names } = signal.dependencies;
    for (const name of names) {
      const manifest = scan.dependencies.get(ecosystem)?.get(normalizeDependencyName(ecosystem, name))?.[0];
      if (manifest !== undefined) return `the ${name} dependency in ${manifest}`;
    }
    return undefined;
  }
  const matched = signalEvidence(signal, scan);
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

/**
 * The folder a matched pack belongs in: the deepest folder holding all of its evidence, across every signal
 * (`''` is the whole repository). Two manifests in `web/a` and `web/b` give `web`.
 */
export function matchFolder(match: RulePackMatch, scan: RepositoryScan): string {
  return commonDirectory(match.any.flatMap(signal => signalEvidence(signal, scan)));
}
