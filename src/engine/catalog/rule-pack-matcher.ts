import { lstatSync, readdirSync, readFileSync, realpathSync, type Dirent } from "node:fs";
import { dirname, join, posix, relative, sep } from "node:path";
import { withinCheckout } from "./checkout-control.js";
import { readTomlStringLists } from "./lint-config/lint-config-formats.js";
import { IGNORED_DIRECTORIES } from "../code-rule/code-rule-enforcer.js";
import { GitUnavailableError, runGit } from "../git.js";
import { globPattern, type GlobMatcher } from "../glob-pattern.js";
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
  /** The folders the repository has that the scan does not enter: under Git those holding most files first, else by name. */
  readonly notScanned: readonly NotScannedFolder[];
  /** Whether `files` is Git's listing, which leaves out what Git ignores, rather than a walk of the folder. */
  readonly git: boolean;
  /** Whether the walk outside Git stopped at `MAX_WALKED_FILES` with folders left unread. */
  readonly stoppedEarly: boolean;
}

/** A dependency, build-output or test-data folder the scan does not enter, by name (`node_modules`, `fixtures`). */
export interface NotScannedFolder {
  readonly folder: string;
  /** How many files Git lists under folders of that name; undefined outside Git, where the walk never enters them. */
  readonly fileCount?: number;
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

const notScannedFolder = (name: string): boolean => IGNORED_DIRECTORIES.has(name) || FIXTURE_DIRECTORIES.has(name);
/** Version control's and Rerail's own folders hold no project code: skipped like the others, but never named. */
const UNNAMED_FOLDERS = new Set([".git", ".hg", ".svn", ".rerail"]);

interface Walked {
  readonly files: string[];
  /** The names of the folders the walk did not enter. */
  readonly notEntered: Set<string>;
  /** The folders still to read. */
  readonly pending: string[];
}

/** Adds the files in `directory` to `walked`, and its folders to those still to read. */
function walkFolder(root: string, directory: string, walked: Walked): void {
  for (const entry of readableEntries(directory)) {
    if (entry.isFile()) walked.files.push(relative(root, join(directory, entry.name)).split(sep).join("/"));
    else if (!entry.isDirectory()) continue;
    else if (notScannedFolder(entry.name)) walked.notEntered.add(entry.name);
    else walked.pending.push(join(directory, entry.name));
  }
}

/** The files under `root`, up to `MAX_WALKED_FILES`, the folders it did not enter, and those it had yet to read. */
function walk(root: string): Walked {
  const walked: Walked = { files: [], notEntered: new Set(), pending: [root] };
  while (walked.pending.length > 0 && walked.files.length < MAX_WALKED_FILES) walkFolder(root, walked.pending.pop()!, walked);
  return walked;
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
 * a bounded walk elsewhere. Files under a folder the scan does not enter are left out and counted per folder name.
 */
function listRepositoryFiles(root: string): Pick<RepositoryScan, "files" | "notScanned" | "git" | "stoppedEarly"> {
  // `-t` prefixes each entry with its status tag and a space: `H` tracked, `S` skip-worktree, `?` untracked, ….
  const tagged = gitListing(root, ["-t", "--cached", "--others", "--exclude-standard"]);
  const listed = tagged?.filter(entry => !entry.startsWith(`${SKIP_WORKTREE_TAG} `)).map(entry => entry.slice(2));
  const deleted = new Set(listed === undefined ? [] : gitListing(root, ["--deleted"]) ?? []);
  const walked = listed === undefined ? walk(root) : undefined;
  const paths = walked?.files ?? listed!.filter(path => !deleted.has(path));
  const counts = new Map<string, number>();
  const files = [...new Set(paths)]
    .filter(path => {
      const folder = path.split("/").slice(0, -1).find(notScannedFolder);
      if (folder !== undefined) counts.set(folder, (counts.get(folder) ?? 0) + 1);
      return folder === undefined;
    })
    .sort(compareDeterministicStrings);
  const notScanned = walked === undefined
    ? [...counts]
      .sort(([leftFolder, leftCount], [rightFolder, rightCount]) => rightCount - leftCount || compareDeterministicStrings(leftFolder, rightFolder))
      .map(([folder, fileCount]) => ({ folder, fileCount }))
    : [...walked.notEntered].sort(compareDeterministicStrings).map(folder => ({ folder }));
  return {
    files,
    notScanned: notScanned.filter(({ folder }) => !UNNAMED_FOLDERS.has(folder)),
    git: walked === undefined,
    stoppedEarly: (walked?.pending.length ?? 0) > 0,
  };
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

const POETRY_DEPENDENCIES = /^tool\.poetry\.(?:dev-dependencies|(?:group\.(?:[A-Za-z0-9_-]+|"(?:[^"\\]|\\.)*")\.)?dependencies)\.(.+)$/u;

/** The first key of a dotted TOML path: bare, or quoted as `readTomlStringLists` writes it. */
function firstKey(path: string): string | undefined {
  if (!path.startsWith("\"")) return path.split(".")[0];
  const quoted = /^"(?:[^"\\]|\\.)*"/u.exec(path)?.[0];
  return quoted === undefined ? undefined : JSON.parse(quoted) as string;
}

/**
 * `pyproject.toml`: PEP 621's `[project] dependencies` and `optional-dependencies`, and Poetry's
 * `[tool.poetry.dependencies]`, `[tool.poetry.group.<name>.dependencies]` and, from Poetry before 1.2,
 * `[tool.poetry.dev-dependencies]` (Poetry's `python` entry is the interpreter, not a dependency). A file the TOML
 * reader cannot read names nothing.
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
  const listing = listRepositoryFiles(root);
  const { files } = listing;
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
  return { ...listing, dependencies };
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

/**
 * A file glob as a person reads it, `.gitignore` style: `**\/tests/**` is `tests/` and `**\/*.py` is `*.py`, at any
 * depth; a glob anchored at the repository root keeps a leading `/` (`tests/**` is `/tests/`).
 */
function shownGlob(glob: string): string {
  const name = glob.startsWith("**/") ? glob.slice(3) : `/${glob}`;
  return name.endsWith("/**") ? name.slice(0, -2) : name;
}

/** A pack that looks for files the repository has but excludes every one of them. */
export interface NearMiss {
  /** How many files it looks for, all excluded. */
  readonly fileCount: number;
  /** Each exclude glob that left a file out (as `shownGlob` writes it), with how many, most first, then in the pack's order. */
  readonly excludedBy: readonly { readonly glob: string; readonly fileCount: number }[];
}

interface FileSignalPatterns {
  readonly includes: readonly { readonly length: number; readonly pattern: GlobMatcher }[];
  readonly excludes: readonly { readonly glob: string; readonly pattern: GlobMatcher }[];
}

/**
 * The exclude glob that leaves `path` out, taken from the file signal whose include glob names it most closely, the
 * longest. Two signals of one pack can look for one file, each excluding it for a different reason: the Complexity
 * pack's PHP signal excludes `*.blade.php`, which its Blade signal looks for, so `tests/view.blade.php` is out because
 * of the Blade signal's `tests/`. Undefined for a file no signal looks for, or one a signal does not exclude.
 */
function excludingGlob(path: string, signals: readonly FileSignalPatterns[]): string | undefined {
  let closest: { readonly length: number; readonly signal: FileSignalPatterns } | undefined;
  for (const signal of signals) {
    for (const include of signal.includes) {
      if (include.pattern.test(path) && include.length > (closest?.length ?? 0)) closest = { length: include.length, signal };
    }
  }
  return closest?.signal.excludes.find(exclude => exclude.pattern.test(path))?.glob;
}

/**
 * How close `match`, which does not fit the scan, came to fitting: undefined when the repository has none of the files
 * it looks for. For a pack that fits, only its excluded files are counted. Dependency signals never come close: a
 * dependency is in a manifest or not.
 */
export function nearMiss(match: RulePackMatch, scan: RepositoryScan): NearMiss | undefined {
  const signals = match.any.flatMap((signal): FileSignalPatterns[] => "dependencies" in signal ? [] : [{
    includes: signal.files.map(glob => ({ length: glob.length, pattern: globPattern([glob]) })),
    excludes: (signal.exclude ?? []).map(glob => ({ glob: shownGlob(glob), pattern: globPattern([glob]) })),
  }]);
  const counts = new Map<string, number>();
  for (const path of scan.files) {
    const glob = excludingGlob(path, signals);
    if (glob !== undefined) counts.set(glob, (counts.get(glob) ?? 0) + 1);
  }
  if (counts.size === 0) return undefined;
  const order = signals.flatMap(signal => signal.excludes.map(exclude => exclude.glob));
  const excludedBy = [...counts]
    .map(([glob, fileCount]) => ({ glob, fileCount }))
    .sort((left, right) => right.fileCount - left.fileCount || order.indexOf(left.glob) - order.indexOf(right.glob));
  return { fileCount: excludedBy.reduce((total, { fileCount }) => total + fileCount, 0), excludedBy };
}

/** What a pack's match definition looks for: its file globs (as `shownGlob` writes them) and dependency names, each once. */
export interface PackLooksFor {
  readonly files: readonly string[];
  readonly dependencies: readonly string[];
}

export function packLooksFor(match: RulePackMatch): PackLooksFor {
  const files = new Set<string>();
  const dependencies = new Set<string>();
  for (const signal of match.any) {
    if ("dependencies" in signal) for (const name of signal.dependencies.names) dependencies.add(name);
    else for (const glob of signal.files) files.add(shownGlob(glob));
  }
  return { files: [...files], dependencies: [...dependencies] };
}

/** How many scanned files end in `ending` (`.py`, lowercased). */
export interface FileEnding {
  readonly ending: string;
  readonly fileCount: number;
}

/** The endings of the scanned files, most files first, then alphabetically; a file with no ending is not counted. */
export function fileEndings(scan: RepositoryScan): readonly FileEnding[] {
  const counts = new Map<string, number>();
  for (const path of scan.files) {
    const ending = posix.extname(path).toLowerCase();
    if (ending.length > 1) counts.set(ending, (counts.get(ending) ?? 0) + 1);
  }
  return [...counts]
    .sort(([leftEnding, leftCount], [rightEnding, rightCount]) => rightCount - leftCount || compareDeterministicStrings(leftEnding, rightEnding))
    .map(([ending, fileCount]) => ({ ending, fileCount }));
}
