/**
 * The packages Clippy lints in a repository, as Cargo 1.99 finds them (cargo 5f94df47 `src/workspace/workspace.rs`:
 * `find_members`, `find_path_deps`, `members_paths`, `is_excluded`), read as data: the root manifest; with a
 * `[workspace]`, each folder its `members` names, a glob expanded as the `glob` crate expands it, less what `exclude`
 * leaves out (unless a member names it), and each package a member depends on by `path` in the repository, followed
 * from member to member. Without a `[workspace]`, the root package is the only one.
 *
 * A member Rerail cannot read, or that Cargo refuses, a glob it does not expand, and a package below the root that is
 * no member (Clippy lints it only when run in its folder) are kept as folders Rerail cannot tell, with why.
 *
 * Also how Cargo builds each package for `cargo clippy`: the crate roots of the targets it checks by default (the
 * library, the binaries and the build script; not examples, tests or benches), and the rustflags its config files in
 * the repository pass after the `[lints]` flags.
 */
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { LintConfigFormatError, readFormat, readTomlTrees, type TomlValue } from "./formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile, type LinterNote } from "./linter-config.js";
import { repositoryFiles } from "./repository-files.js";
import { repositoryPath } from "./repository-paths.js";
import type { RustLintLevel } from "./rust-lint-attributes.js";

export type CargoTable = ReadonlyMap<string, TomlValue>;

/** A manifest, read: the tables Rerail reads from it. */
export interface CargoManifest {
  /** Its folder, relative to the repository root; `""` for the root. */
  readonly folder: string;
  /** `package`, `lints`, `lib`, `workspace`, `target` and the dependency tables, each the file sets. */
  readonly tables: ReadonlyMap<string, CargoTable>;
  /** Its `[[bin]]` tables, in order. */
  readonly bins: readonly CargoTable[];
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
const MANIFEST_TABLES = ["package", "lints", "lib", "workspace", "target", ...DEPENDENCY_TABLES];

export const manifestFile = (folder: string): string => (folder === "" ? "Cargo.toml" : `${folder}/Cargo.toml`);

const isTable = (value: unknown): value is Map<string, TomlValue> => value instanceof Map;

/** The manifest in `folder`, or why it is not read; undefined when there is none. */
export function readManifest(root: string, folder: string): CargoManifest | { readonly why: string } | undefined {
  const read = readRootFile(root, manifestFile(folder));
  if (!read.present) return undefined;
  if (read.text === undefined) return { why: NOT_A_FILE.replace(/\.$/u, "") };
  const text = read.text;
  // Cargo parses manifests as TOML 1.1.
  const parsed = readFormat(() => readTomlTrees(text, MANIFEST_TABLES, "1.1", ["bin"]));
  return parsed.ok ? { folder, tables: parsed.value.trees, bins: parsed.value.arrays.get("bin") ?? [] } : { why: parsed.why };
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

/** A number's, boolean's or date's text as TOML writes it; undefined for a string, array or table. */
export const scalarText = (value: TomlValue | undefined): string | undefined => (typeof value === "object" && "scalar" in value ? value.scalar : undefined);

/** A crate root `cargo clippy` checks, relative to the repository root, and whether it may not check it. */
export interface CrateRootFile {
  readonly file: string;
  /** Whether Cargo checks it turns on what Rerail does not match: a `[[bin]]` that may take its name, `required-features`. */
  readonly maybe: boolean;
}

/** Target path `written`, set at `where` in the manifest in `folder`, as a repository path; refuses one Rerail cannot read. */
function targetPath(folder: string, where: string, written: TomlValue | undefined): string {
  if (typeof written !== "string") throw new LintConfigFormatError(`${where} is not a path`);
  const path = repositoryPath(folder, written);
  if (path === undefined) throw new LintConfigFormatError(`${where} ${JSON.stringify(written)} is outside the repository`);
  return path;
}

/** The library's crate root: `[lib] path`, else `src/lib.rs`; none with `autolib = false` and no `[lib]`. */
function libraryRoot(manifest: CargoManifest, info: CargoTable): CrateRootFile[] {
  const lib = manifest.tables.get("lib") ?? new Map<string, TomlValue>();
  if (lib.has("path")) return [{ file: targetPath(manifest.folder, "[lib] path", lib.get("path")), maybe: false }];
  if (!manifest.tables.has("lib") && scalarText(info.get("autolib")) === "false") return [];
  return [{ file: targetPath(manifest.folder, "[lib] path", "src/lib.rs"), maybe: false }];
}

/** The build script's crate root: `package.build`'s path, else `build.rs`; none with `build = false`. */
function buildScriptRoot(manifest: CargoManifest, info: CargoTable): CrateRootFile[] {
  const build = info.get("build") ?? "build.rs";
  const flag = scalarText(build);
  if (flag === "false") return [];
  return [{ file: targetPath(manifest.folder, "package.build", flag === "true" ? "build.rs" : build), maybe: false }];
}

/** The binaries Cargo infers in the package in `folder`: `src/main.rs`, `src/bin/*.rs` and `src/bin/*\/main.rs`. */
function inferredBinaries(root: string, folder: string): string[] {
  const found = matching(root, repositoryPath(folder, "src/bin")!, /^/u).map(path => (path.endsWith(".rs") ? path : `${path}/main.rs`));
  return [repositoryPath(folder, "src/main.rs")!, ...found];
}

/**
 * The binaries' crate roots: each `[[bin]] path`, and Cargo's inferred binaries. With a `[[bin]]`, or `autobins =
 * false`, an inferred one may or may not be checked: a `[[bin]]` may take its name, or name it.
 */
function binaryRoots(root: string, manifest: CargoManifest, info: CargoTable): CrateRootFile[] {
  const autobins = scalarText(info.get("autobins")) !== "false";
  const named = manifest.bins.some(bin => !bin.has("path"));
  const maybe = manifest.bins.length > 0 || !autobins;
  const inferred = autobins || named ? inferredBinaries(root, manifest.folder).map(file => ({ file, maybe })) : [];
  const declared = manifest.bins.flatMap(bin => (bin.has("path") ? [{ file: targetPath(manifest.folder, "[[bin]] path", bin.get("path")), maybe: bin.has("required-features") }] : []));
  return [...inferred, ...declared];
}

/**
 * The crate roots `cargo clippy` checks by default in the package `manifest` describes, one with a `[package]` (cargo
 * `src/ops/cargo_compile/unit_generator.rs` `filter_default_targets`, `src/util/toml/targets.rs`): its library, its
 * binaries and its build script, each once, a `[[bin]]`'s over an inferred one. Throws `LintConfigFormatError` on a
 * target path that is no string or lies outside the repository.
 */
export function crateRoots(root: string, manifest: CargoManifest): CrateRootFile[] {
  const info = manifest.tables.get("package")!;
  const roots = [...libraryRoot(manifest, info), ...binaryRoots(root, manifest, info), ...buildScriptRoot(manifest, info)];
  return [...new Map(roots.map(found => [found.file, found])).values()];
}

/** A lint level a rustflag sets (`-W clippy::needless_else`), and where, for the person. */
export interface RustflagLint {
  /** As rustc reads it: `clippy::needless_else` for `clippy::needless-else`. */
  readonly name: string;
  readonly level: RustLintLevel;
  readonly because: string;
}

/** A Cargo config file's rustflags: the lint levels they set, in order, and whether a `[target]`'s may replace `[build]`'s. */
export interface CargoConfig {
  readonly file: string;
  /** The folder holding its `.cargo/`: Cargo reads it when run there or below; `""` for the root. */
  readonly folder: string;
  readonly lints: readonly RustflagLint[];
  /** A `[target.<platform>]` sets rustflags: on its platforms they replace `[build]`'s. */
  readonly targeted: boolean;
}

/** A Cargo config file Rerail cannot read, and why. */
export interface UnreadCargoConfig {
  readonly file: string;
  readonly folder: string;
  readonly why: string;
}

/** The rustc options Rerail reads in rustflags, each with the lint level it sets; `-C`, `--cfg`, `-L` and `-l` set none. */
const RUSTFLAG_OPTIONS: Readonly<Record<string, RustLintLevel | undefined>> = {
  "-A": "allow", "--allow": "allow", "-W": "warn", "--warn": "warn", "-D": "deny", "--deny": "deny", "-F": "forbid", "--forbid": "forbid",
  "-C": undefined, "--codegen": undefined, "--cfg": undefined, "--check-cfg": undefined, "-L": undefined, "-l": undefined,
};
/**
 * A rustflag as rustc reads it: a short option and its value written on (`-Wwarnings`), or a long one and `=value`;
 * no option (`""`) for any other flag.
 */
const RUSTFLAG = /^(-[A-Za-z](?!=)|--[a-z-]+(?==|$)|)=?(.*)$/su;

/**
 * The rustflag at `index` of `flags`, set at `where`: its option, its value (written on, or the next flag) and the
 * index after them. Refuses a flag Rerail does not read, which may set a lint level, and an option missing its value.
 */
function rustflagAt(flags: readonly string[], index: number, where: string): { option: string; value: string; next: number } {
  const match = RUSTFLAG.exec(flags[index]!)!;
  const [option, attached] = [match[1]!, match[2]!];
  if (!Object.hasOwn(RUSTFLAG_OPTIONS, option)) throw new LintConfigFormatError(`${where} has ${JSON.stringify(flags[index])}, which Rerail does not read and which may set lint levels`);
  const value = attached === "" ? flags[index + 1] : attached;
  if (value === undefined) throw new LintConfigFormatError(`${where} ends in ${option} without its value`);
  return { option, value, next: index + (attached === "" ? 2 : 1) };
}

/** The lint levels rustflags `flags`, set at `where`, set in order, as rustc reads them. */
function rustflagLints(flags: readonly string[], where: string): RustflagLint[] {
  const lints: RustflagLint[] = [];
  for (let index = 0; index < flags.length;) {
    const { option, value, next } = rustflagAt(flags, index, where);
    const level = RUSTFLAG_OPTIONS[option];
    if (level !== undefined) lints.push({ name: value.replaceAll("-", "_"), level, because: `${where} ${option} ${value}` });
    index = next;
  }
  return lints;
}

/** `table`'s `rustflags` as Cargo reads them: a list of strings, or one string split at whitespace; none when unset. */
function rustflagList(table: TomlValue | CargoTable | undefined, where: string): string[] {
  const flags = table instanceof Map ? table.get("rustflags") ?? [] : [];
  if (typeof flags === "string") return flags.split(/\s+/u).filter(flag => flag !== "");
  if (!Array.isArray(flags) || !flags.every(flag => typeof flag === "string")) throw new LintConfigFormatError(`${where} is not a string or a list of strings`);
  return flags as string[];
}

/** Cargo config file `file` in `folder`, whose text is `text`, its rustflags read. Throws `LintConfigFormatError` on one Rerail cannot read. */
function readCargoConfig(file: string, folder: string, text: string | undefined): CargoConfig {
  if (text === undefined) throw new LintConfigFormatError(NOT_A_FILE.replace(/\.$/u, ""));
  const { trees, paths } = readTomlTrees(text, ["build", "target"], "1.1");
  if (paths.includes("include")) throw new LintConfigFormatError("it includes other config files, which Rerail does not follow");
  const targets = [...(trees.get("target") ?? [])].map(([platform, table]) => ({ where: `${file} [target.${platform}] rustflags`, table }));
  const lists = [{ where: `${file} [build] rustflags`, table: trees.get("build") }, ...targets].map(({ where, table }) => ({ where, flags: rustflagList(table, where) }));
  return { file, folder, lints: lists.flatMap(({ where, flags }) => rustflagLints(flags, where)), targeted: lists.slice(1).some(({ flags }) => flags.length > 0) };
}

const CONFIG_FILE = /(?:^|\/)\.cargo\/config(?:\.toml)?$/u;

/**
 * The Cargo config files in the repository, one per folder (`.cargo/config`, which Cargo prefers, else
 * `.cargo/config.toml`), each read, or kept with why Rerail cannot read it.
 */
export function readCargoConfigs(root: string): { configs: CargoConfig[]; unread: UnreadCargoConfig[] } {
  const candidates = [...new Set([".cargo/config", ".cargo/config.toml", ...repositoryFiles(root).filter(path => CONFIG_FILE.test(path))])].sort();
  const chosen = new Map<string, { file: string; text: string | undefined }>();
  for (const file of candidates) {
    const read = readRootFile(root, file);
    const folder = file.replace(CONFIG_FILE, "");
    if (read.present && !chosen.has(folder)) chosen.set(folder, { file, text: read.text });
  }
  const configs: CargoConfig[] = [];
  const unread: UnreadCargoConfig[] = [];
  for (const [folder, { file, text }] of chosen) {
    const read = readFormat(() => readCargoConfig(file, folder, text));
    if (read.ok) configs.push(read.value);
    else unread.push({ file, folder, why: read.why });
  }
  return { configs, unread };
}
