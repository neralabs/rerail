/**
 * The packages Clippy lints in a repository, as Cargo 1.99 finds them (cargo 5f94df47 `src/workspace/workspace.rs`:
 * `find_members`, `find_path_deps`, `members_paths`, `is_excluded`), read as data: the root manifest; with a
 * `[workspace]`, each folder its `members` names, a glob expanded as the `glob` crate expands it, less what `exclude`
 * leaves out (unless a member names it), and each package a member depends on by `path` in the repository, followed
 * from member to member. Without a `[workspace]`, the root package is the only one.
 *
 * A member Rerail cannot read, or that Cargo refuses, a glob it does not expand, and a package below the root that is
 * no member (Clippy lints it only when run in its folder) are kept as folders Rerail cannot tell, with why.
 */
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { LintConfigFormatError, readFormat, readTomlTrees, type TomlValue } from "./formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile, type LinterNote } from "./linter-config.js";
import { repositoryFiles } from "./repository-files.js";
import { repositoryPath } from "./repository-paths.js";

export type CargoTable = ReadonlyMap<string, TomlValue>;

/** A manifest, read: the tables Rerail reads from it. */
export interface CargoManifest {
  /** Its folder, relative to the repository root; `""` for the root. */
  readonly folder: string;
  /** `package`, `lints`, `workspace`, `target` and the dependency tables, each the file sets. */
  readonly tables: ReadonlyMap<string, CargoTable>;
}

/** A folder whose packages Clippy may lint otherwise than Rerail can tell, and why, for the person. */
export interface UnreadFolder {
  readonly folder: string;
  readonly because: string;
}

export interface CargoWorkspace {
  readonly root: CargoManifest;
  /** The members other than the root, each read. */
  readonly members: readonly CargoManifest[];
  readonly unread: readonly UnreadFolder[];
}

/** The tables of a package's dependencies, at the top level or under `[target.<platform>]`. */
const DEPENDENCY_TABLES = ["dependencies", "dev-dependencies", "dev_dependencies", "build-dependencies", "build_dependencies"];
const MANIFEST_TABLES = ["package", "lints", "workspace", "target", ...DEPENDENCY_TABLES];

export const manifestFile = (folder: string): string => (folder === "" ? "Cargo.toml" : `${folder}/Cargo.toml`);

const isTable = (value: unknown): value is Map<string, TomlValue> => value instanceof Map;

/** The manifest in `folder`, or why it is not read; undefined when there is none. */
export function readManifest(root: string, folder: string): CargoManifest | { readonly why: string } | undefined {
  const read = readRootFile(root, manifestFile(folder));
  if (!read.present) return undefined;
  if (read.text === undefined) return { why: NOT_A_FILE.replace(/\.$/u, "") };
  const text = read.text;
  const parsed = readFormat(() => readTomlTrees(text, MANIFEST_TABLES).trees);
  return parsed.ok ? { folder, tables: parsed.value } : { why: parsed.why };
}

/** `[workspace] <key>`, a list of strings; Cargo refuses anything else. */
function stringList(workspace: CargoTable, key: string): string[] {
  const value = workspace.get(key) ?? [];
  if (!Array.isArray(value) || !value.every(item => typeof item === "string")) throw new LintConfigFormatError(`[workspace] ${key} is not a list of strings`);
  return value as string[];
}

/** `written` (relative to the root) as Rust's `Path` components: no empty or `.` part; undefined when absolute. */
function components(written: string): string[] | undefined {
  if (/^(?:\/|[A-Za-z]:[\\/])/u.test(written)) return undefined;
  return written.split("/").filter(part => part !== "" && part !== ".");
}

/** Whether manifest `file`'s path starts with `written`'s, part by part, as Rust's `Path::starts_with`. */
function startsWith(file: readonly string[], written: string): boolean {
  const prefix = components(written);
  return prefix !== undefined && prefix.every((part, index) => file[index] === part);
}

/** A glob component's `[…]` class from `start` as a regular expression's class, and the index after it. */
function globClass(part: string, start: number): { source: string; end: number } {
  let index = start + 1;
  const negated = part[index] === "!";
  if (negated) index += 1;
  const close = part.indexOf("]", index + 1);
  if (close < 0) throw new LintConfigFormatError("is not a glob Cargo takes: a [ is not closed");
  const inside = part.slice(index, close).replace(/[\\\]^[]/gu, "\\$&");
  return { source: `[${negated ? "^" : ""}${inside}]`, end: close + 1 };
}

const GLOB_TOKENS: Readonly<Record<string, string>> = { "*": "[^/]*", "?": "[^/]" };

/** Glob component `part` (`*`, `?`, `[ab]`, `[!a-z]`) as a regular expression matching a whole name. */
function globPattern(part: string): RegExp {
  let source = "";
  for (let index = 0; index < part.length;) {
    const character = part[index]!;
    const token = character === "[" ? globClass(part, index) : { source: GLOB_TOKENS[character] ?? character.replace(/[$()*+.?[\\\]^{|}]/gu, "\\$&"), end: index + 1 };
    source += token.source;
    index = token.end;
  }
  return new RegExp(`^${source}$`, "u");
}

const isGlob = (part: string): boolean => /[*?[]/u.test(part);

/** Whether `path` (relative to the root) exists; with `folder`, whether it is a folder, as Rust's `is_dir`. */
function exists(root: string, path: string, folder = false): boolean {
  try {
    const info = statSync(join(root, path));
    return !folder || info.isDirectory();
  } catch {
    return false;
  }
}

/** The names in folder `path` matching `pattern`, as paths; none when it is no folder. */
function matching(root: string, path: string, pattern: RegExp): string[] {
  try {
    return readdirSync(join(root, path)).filter(name => pattern.test(name)).map(name => (path === "" ? name : `${path}/${name}`));
  } catch {
    return [];
  }
}

/** What a member's path or glob names: folders, or the folder of what Rerail cannot tell; undefined outside the repository. */
type MemberPaths = { readonly folders: readonly string[] } | UnreadFolder;

/** A member glob's matches, part by part: a literal part where it exists, a glob part each name it matches. */
function expandGlob(root: string, pattern: string, parts: readonly string[], base: readonly string[]): MemberPaths {
  const folder = base.join("/");
  const because = `Cargo.toml [workspace] members ${JSON.stringify(pattern)}`;
  if (parts.some(part => part === ".." || part.includes("**"))) return { folder, because: `${because} is not expanded: Rerail expands *, ? and [...] in a folder's name only` };
  let paths = [folder];
  for (const part of parts.slice(base.length)) {
    const glob = isGlob(part) ? globPattern(part) : undefined;
    paths = paths.flatMap(path => (glob === undefined ? [`${path}/${part}`].filter(next => exists(root, next)) : matching(root, path, glob)));
  }
  // Matching nothing, Cargo looks for the glob itself as a folder; matching only files, it takes none.
  if (paths.length === 0) return { folder, because: `${because} matches nothing, which Cargo refuses` };
  return { folders: paths.filter(path => exists(root, path, true)).sort() };
}

/** Member glob `pattern`'s folders, its `parts` from `firstGlob` on expanded; its base folder when Cargo refuses it. */
function globPaths(root: string, pattern: string, parts: readonly string[], firstGlob: number): MemberPaths {
  const base = parts.slice(0, firstGlob);
  const read = readFormat(() => expandGlob(root, pattern, parts, base));
  return read.ok ? read.value : { folder: base.join("/"), because: `Cargo.toml [workspace] members ${JSON.stringify(pattern)} ${read.why}` };
}

/** The folders member `pattern` names, as Cargo's `members_paths` expands it; undefined outside the repository. */
function memberPaths(root: string, pattern: string): MemberPaths | undefined {
  const parts = components(pattern);
  if (parts === undefined) return undefined;
  const firstGlob = parts.findIndex(isGlob);
  if (firstGlob >= 0) return globPaths(root, pattern, parts, firstGlob);
  const folder = repositoryPath("", pattern);
  if (folder === undefined) return undefined;
  // A path that is there but no folder is no member; one that is not there, Cargo refuses (`#visit`).
  return { folders: exists(root, folder) && !exists(root, folder, true) ? [] : [folder] };
}

/** Each `path` dependency `manifest` has, as a folder of the repository; `workspace = true` takes the workspace's. */
function pathDependencies(manifest: CargoManifest, workspaceDependencies: CargoTable | undefined): string[] {
  const target = manifest.tables.get("target");
  const platforms = target === undefined ? [] : [...target.values()].filter(isTable);
  const sources: readonly ReadonlyMap<string, unknown>[] = [manifest.tables, ...platforms];
  const tables = sources.flatMap(table => DEPENDENCY_TABLES.map(name => table.get(name)).filter(isTable));
  return tables.flatMap(table => [...table].flatMap(([name, value]) => {
    if (!isTable(value)) return [];
    const inherited = value.has("workspace") ? workspaceDependencies?.get(name) : undefined;
    const [from, path] = isTable(inherited) ? ["", inherited.get("path")] : [manifest.folder, value.get("path")];
    const folder = typeof path === "string" ? repositoryPath(from, path) : undefined;
    return folder === undefined ? [] : [folder];
  }));
}

/** Why Cargo refuses `manifest` as a member of the root's workspace; undefined when it does not. */
function refusedMember(manifest: CargoManifest): string | undefined {
  if (!manifest.tables.has("package")) return "it has no [package], which Cargo refuses for a member";
  if (manifest.tables.has("workspace")) return "it is the root of a workspace of its own, which Cargo refuses for a member";
  const named = manifest.tables.get("package")!.get("workspace");
  if (named !== undefined && (typeof named !== "string" || repositoryPath(manifest.folder, named) !== "")) return "its package.workspace names another workspace root, which Cargo refuses";
  return undefined;
}

/** The member in `folder`, or why Rerail does not read it or Cargo refuses it. */
function readMember(root: string, folder: string): CargoManifest | { readonly why: string } {
  const manifest = readManifest(root, folder) ?? { why: "there is none, which Cargo refuses" };
  if ("why" in manifest) return manifest;
  const why = refusedMember(manifest);
  return why === undefined ? manifest : { why };
}

/** The workspace's members as Cargo finds them, read, and the folders Rerail cannot tell. */
class MemberFinder {
  readonly members: CargoManifest[] = [];
  readonly unread: UnreadFolder[] = [];
  readonly #visited = new Set<string>([""]);
  readonly #patterns: readonly string[];
  readonly #exclude: readonly string[];
  readonly #dependencies: CargoTable | undefined;

  constructor(readonly root: string, workspace: CargoTable) {
    this.#patterns = stringList(workspace, "members");
    this.#exclude = stringList(workspace, "exclude");
    const dependencies = workspace.get("dependencies");
    this.#dependencies = isTable(dependencies) ? dependencies : undefined;
  }

  /** Every member: each `members` names, then the packages they and the root depend on by path. */
  find(rootManifest: CargoManifest, notes: LinterNote[]): void {
    for (const pattern of this.#patterns) this.#findNamed(pattern, notes);
    if (rootManifest.tables.has("package")) for (const folder of pathDependencies(rootManifest, this.#dependencies)) this.#visit(folder);
  }

  /** The members `members` entry `pattern` names. */
  #findNamed(pattern: string, notes: LinterNote[]): void {
    const paths = memberPaths(this.root, pattern);
    if (paths === undefined) notes.push({ linter: "clippy", text: `Cargo.toml [workspace] members ${JSON.stringify(pattern)} is outside the repository; it is not read.` });
    else if ("because" in paths) this.unread.push(paths);
    else for (const folder of paths.folders) this.#visit(folder);
  }

  /** Whether `exclude` leaves out the package in `folder`, and no `members` entry names it (`is_excluded`). */
  #excluded(folder: string): boolean {
    const file = components(manifestFile(folder))!;
    return this.#exclude.some(written => startsWith(file, written)) && !this.#patterns.some(written => startsWith(file, written));
  }

  /** The package in `folder`, a member unless excluded, and the packages it depends on by path. */
  #visit(folder: string): void {
    if (this.#visited.has(folder) || this.#excluded(folder)) return;
    this.#visited.add(folder);
    const manifest = readMember(this.root, folder);
    if ("why" in manifest) {
      this.unread.push({ folder, because: `${manifestFile(folder)} is not read: ${manifest.why}` });
      return;
    }
    this.members.push(manifest);
    for (const dependency of pathDependencies(manifest, this.#dependencies)) this.#visit(dependency);
  }
}

/** The packages below the root that are no member: Clippy lints each only when run in its folder. */
function nonMembers(root: string, known: ReadonlySet<string>): UnreadFolder[] {
  const folders = repositoryFiles(root).flatMap(file => (file.endsWith("/Cargo.toml") ? [file.slice(0, -"/Cargo.toml".length)] : []));
  return folders.filter(folder => !known.has(folder)).map(folder => ({
    folder,
    because: `${manifestFile(folder)} is not read: it is no member of the root's workspace, and Clippy lints it only when run in its folder`,
  }));
}

/**
 * The workspace at the root whose manifest is `rootManifest`, noting each folder Rerail cannot tell. Throws
 * `LintConfigFormatError` on a `[workspace]` Cargo refuses.
 */
export function readCargoWorkspace(root: string, rootManifest: CargoManifest, notes: LinterNote[]): CargoWorkspace {
  const workspace = rootManifest.tables.get("workspace");
  const finder = workspace === undefined ? undefined : new MemberFinder(root, workspace);
  finder?.find(rootManifest, notes);
  const members = finder?.members ?? [];
  const unread = finder?.unread ?? [];
  const known = new Set([...members, ...unread].map(found => found.folder));
  const all = [...unread, ...nonMembers(root, known)];
  for (const folder of all) notes.push({ linter: "clippy", text: `${folder.because}; Clippy's lints there stay as the pack has them.` });
  return { root: rootManifest, members, unread: all };
}
