/**
 * Internal testing: the packs a headless `rerail scan` runs when the repository declares them in `.rerail/packs.json`
 * (docs/repository-packs.md). The file, and every pack it keeps in the repository, come from the tree the scan's rules
 * come from: the working tree, or the `--rules-ref` commit, and so do the option values it sets for a pack's rules.
 * A published pack is installed at its exact version into the scan's own folder, never the Rerail home's, so what the
 * home has installed changes nothing. A listed pack that does not run, or an option its rules refuse, fails the scan.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { basename, isAbsolute, join, relative } from "node:path";
import { declaredPackRuleConfigs, type DeclaredRuleConfig } from "../engine/catalog/packs-file/pack-config.js";
import type { RuleCatalog, RuleConfig } from "../engine/catalog/repository-model.js";
import { activeCoreVersion, installedPacksPrefix } from "../engine/catalog/rule-catalog.js";
import {
  checkPackFolders,
  declaredPackCatalog,
  declaredPackLabel,
  MAX_PACKS_FILE_BYTES,
  packAttachmentFolders,
  PACKS_FILE,
  PacksFileError,
  parsePacksFile,
  readPacksFile,
  repositoryPackDirectory,
  type DeclaredPack,
  type DeclaredPackDirectory,
  type PublishedPack,
  type RepositoryPack,
} from "../engine/catalog/packs-file/packs-file.js";
import type { NativeGitRevisionSource } from "../engine/native/native.js";
import { fetchProblemReason, npmPackStore, type PackStore } from "../setup/rule-pack-store.js";

/** One pack a scan ran, as its report names it. */
export interface ScanPack {
  readonly id: string;
  readonly packageName: string | null;
  readonly version: string | null;
  /**
   * `npm`: a published pack `.rerail/packs.json` lists, fetched for the scan. `repository`: a pack it keeps in the
   * repository, at `path`. `home`: installed in the Rerail home, from its configuration or `--pack`.
   */
  readonly from: "npm" | "repository" | "home";
  readonly path?: string;
  /** The folders `.rerail/packs.json` gives it; absent when it judges the whole repository. */
  readonly folders?: readonly string[];
  /** The option values `.rerail/packs.json` sets for its rules; absent when they run at their defaults. */
  readonly config?: RuleConfig;
}

/** Where a scan attaches one of its packs: the whole repository (`""`) or one folder. */
export interface ScanPackAttachment {
  readonly packId: string;
  readonly folder: string;
}

/** The packs a tree declares, and how messages name the file they came from. */
export interface DeclaredPacks {
  readonly packs: readonly DeclaredPack[];
  /** `.rerail/packs.json`, or `<ref>:.rerail/packs.json`. */
  readonly label: string;
  /** The tree at `--rules-ref` the file was read from, or undefined for the working tree. */
  readonly at?: RulesRefTree;
}

/**
 * The catalog a scan runs on when the repository declares its packs, those packs as the report names them, where
 * each is attached, and the option values the file sets for their rules.
 */
export interface DeclaredScanPacks {
  readonly catalog: RuleCatalog;
  readonly packs: readonly ScanPack[];
  readonly attachments: readonly ScanPackAttachment[];
  readonly ruleConfigs: readonly DeclaredRuleConfig[];
}

/** A file of a pack at a ref larger than this is left out, and the pack then fails to load with why. */
const MAX_REF_PACK_FILE_BYTES = 16 * 1024 * 1024;

/** The commit `--rules-ref` names, and its tree as the scan reads it: one resolution, which every read at the ref takes. */
export interface RulesRefTree {
  readonly ref: string;
  readonly commit: string;
  readonly source: NativeGitRevisionSource;
}

/** Git's modes of a tree entry: a folder, and the two of a regular file (not a symbolic link or a submodule). */
const TREE_MODE = 0o040000;
const isRegularMode = (mode: number): boolean => mode === 0o100644 || mode === 0o100755;

/** Whether `path` is a folder in `source`'s tree: each part of it a tree, so a symbolic link on the way is none. */
const isFolderAt = (source: NativeGitRevisionSource, path: string): boolean => source.pathStatus(`${path}/`).status === "selected";

/**
 * The packs the scan's tree declares: the working tree's `.rerail/packs.json`, or the file in the tree `at` names
 * (`--rules-ref`, resolved once by the scan). Undefined when that tree has none. Throws when the file cannot be read
 * or parsed.
 */
export async function declaredPacksOf(root: string, at: RulesRefTree | undefined): Promise<DeclaredPacks | undefined> {
  if (at === undefined) {
    const packs = readPacksFile(root);
    return packs === undefined ? undefined : { packs, label: PACKS_FILE };
  }
  const label = `${at.ref}:${PACKS_FILE}`;
  // Each part of the path is looked up in its own tree, so a symbolic link `.rerail` lists nothing, as it would be
  // left unread in a checkout.
  const [folder, name] = [PACKS_FILE.slice(0, PACKS_FILE.lastIndexOf("/")), PACKS_FILE.slice(PACKS_FILE.lastIndexOf("/") + 1)];
  if (!isFolderAt(at.source, folder)) return undefined;
  const entry = (at.source.readDirectory(folder) ?? []).find(found => found.name === name);
  if (entry === undefined) return undefined;
  if (!isRegularMode(entry.mode)) throw new PacksFileError(`${label} is not a regular file`);
  const read = at.source.readSourceChecked(PACKS_FILE, MAX_PACKS_FILE_BYTES);
  if (read.status === "too-large") throw new PacksFileError(`${label} is larger than 1 MiB`);
  if (read.status === "absent") return undefined;
  return { packs: parsePacksFile(read.bytes.toString("utf8"), label), label, at };
}

/**
 * The regular files below `folder` in `source`'s tree, by their paths inside it, each part a plain name: a symbolic
 * link or a submodule is none, and a name that is not UTF-8 or not a plain name (`..`, or a `\` or `:` Windows reads as
 * a path) is left out, so nothing is written outside the folder it is written to.
 */
function regularFilesBelow(source: NativeGitRevisionSource, folder: string, inside = ""): string[] {
  return (source.readDirectory(inside === "" ? folder : `${folder}/${inside}`) ?? []).flatMap(({ name, mode }) => {
    if (name == null || name === "" || name === "." || name === ".." || /[\\:/]/u.test(name)) return [];
    const path = inside === "" ? name : `${inside}/${name}`;
    if (mode === TREE_MODE) return regularFilesBelow(source, folder, path);
    return isRegularMode(mode) ? [path] : [];
  });
}

/**
 * The repository pack `pack` as it is in the tree `at` names, written under `into`: its `pack.json`, `package.json`
 * and `rules/`, which is all a pack is read from. A folder keeps its name, which a pack without a `package.json` must
 * have. A symbolic link is not written, so a pack that needs one fails to load, as it would from a checkout that
 * refuses it, and nor is a file whose path has a part that is not a plain name, so nothing is written outside `into`.
 */
function repositoryPackAtCommit(at: RulesRefTree, label: string, pack: RepositoryPack, into: string): string {
  if (!isFolderAt(at.source, pack.path)) {
    throw new PacksFileError(`${label} lists ${pack.path}, which is not a folder at that commit`);
  }
  const directory = join(into, basename(pack.path));
  const files = regularFilesBelow(at.source, pack.path)
    .filter(inside => inside === "pack.json" || inside === "package.json" || inside.startsWith("rules/"));
  mkdirSync(directory, { recursive: true });
  for (const inside of files) {
    const read = at.source.readSourceChecked(`${pack.path}/${inside}`, MAX_REF_PACK_FILE_BYTES);
    if (read.status !== "ok") continue;
    const target = join(directory, ...inside.split("/"));
    if (relative(directory, target).startsWith("..") || isAbsolute(relative(directory, target))) continue;
    mkdirSync(join(target, ".."), { recursive: true });
    writeFileSync(target, read.bytes);
  }
  return directory;
}

/**
 * Installs `published` at their exact versions into a Rerail home of the scan's own under `scratch`, with npm's cache
 * (`npmPackStore`: no install script runs). Throws, naming each pack and npm's reason, when npm cannot give them.
 */
async function installPublished(published: readonly PublishedPack[], scratch: string, label: string, storeOf: (home: string) => PackStore): Promise<Map<PublishedPack, string>> {
  if (published.length === 0) return new Map();
  const home = join(scratch, "declared-packs");
  const specs = published.map(declaredPackLabel);
  try {
    await storeOf(home).install(specs);
  } catch (error) {
    throw new Error(`${label} lists ${specs.join(", ")}, which npm could not install: ${fetchProblemReason(specs.join(", "), error)}`);
  }
  const modules = join(installedPacksPrefix(home), "node_modules");
  return new Map(published.map(pack => [pack, join(modules, ...pack.packageName.split("/"))]));
}

/** The pack of `catalog` a declared pack loaded as; undefined when it did not load. */
function loadedPack(catalog: RuleCatalog, entry: DeclaredPackDirectory): ScanPack | undefined {
  const { pack, checkoutPath } = entry;
  const found = [...catalog.packs.values()].find(candidate => pack.kind === "published"
    ? candidate.packageName === pack.packageName && candidate.localSource === undefined
    : candidate.localSource?.path === checkoutPath);
  if (found === undefined) return undefined;
  return {
    id: found.id,
    packageName: found.packageName,
    version: found.version,
    ...(pack.kind === "published" ? { from: "npm" as const } : { from: "repository" as const, path: pack.path }),
    ...(pack.folders === undefined ? {} : { folders: pack.folders }),
    ...(pack.config === undefined ? {} : { config: pack.config }),
  };
}

/**
 * Checks that every folder `pack` names is a folder at the tree `at` names: each part of its path a tree, so a symbolic link on
 * the way (a blob) or a missing folder is refused, as in a working tree.
 */
function checkPackFoldersAtCommit(at: RulesRefTree, label: string, pack: DeclaredPack): void {
  for (const folder of pack.folders ?? []) {
    if (!isFolderAt(at.source, folder)) throw new PacksFileError(`${label} lists the folder ${folder} for ${declaredPackLabel(pack)}, which is not a folder at that commit`);
  }
}

/** Checks every folder the declared packs name: in the working tree, or at `declared.at`. */
function checkDeclaredFolders(root: string, declared: DeclaredPacks): void {
  for (const pack of declared.packs) {
    if (declared.at === undefined) checkPackFolders(root, pack);
    else checkPackFoldersAtCommit(declared.at, declared.label, pack);
  }
}

/**
 * The folder each declared pack is read from: a published pack installed under `scratch`; a repository pack in the
 * working tree, or written under `scratch` from `declared.at`.
 */
async function declaredPackDirectories(
  root: string, declared: DeclaredPacks, scratch: string, storeOf: (home: string) => PackStore,
): Promise<DeclaredPackDirectory[]> {
  const published = await installPublished(declared.packs.filter((pack): pack is PublishedPack => pack.kind === "published"), scratch, declared.label, storeOf);
  const directories: DeclaredPackDirectory[] = [];
  for (const [index, pack] of declared.packs.entries()) {
    if (pack.kind === "published") {
      directories.push({ pack, directory: published.get(pack)! });
      continue;
    }
    const checkoutPath = join(root, ...pack.path.split("/"));
    const directory = declared.at === undefined
      ? repositoryPackDirectory(root, pack)
      : repositoryPackAtCommit(declared.at, declared.label, pack, join(scratch, "repository-packs", String(index)));
    directories.push({ pack, directory, checkoutPath });
  }
  return directories;
}

/**
 * Each declared pack as `catalog` loaded it, beside its declaration. Throws, naming `label` and each pack, when one did
 * not load or npm installed another version than the one listed.
 */
function loadedDeclaredPacks(catalog: RuleCatalog, directories: readonly DeclaredPackDirectory[], label: string): Array<{ pack: DeclaredPack; loaded: ScanPack }> {
  const problems: string[] = [];
  const loaded: Array<{ pack: DeclaredPack; loaded: ScanPack }> = [];
  for (const entry of directories) {
    const found = loadedPack(catalog, entry);
    const skipped = catalog.skipped.find(left => left.path === entry.directory);
    if (found === undefined) problems.push(`${declaredPackLabel(entry.pack)}: ${skipped?.reason ?? "it did not load"}`);
    else if (entry.pack.kind === "published" && found.version !== entry.pack.version) problems.push(`${declaredPackLabel(entry.pack)}: npm installed ${found.version ?? "no version"}`);
    else loaded.push({ pack: entry.pack, loaded: found });
  }
  if (problems.length > 0) throw new Error(`${label} lists packs that do not run: ${problems.join("; ")}`);
  return loaded;
}

/**
 * The catalog of the packs `declared` lists, for a scan of the checkout at `root`: its repository packs read from the
 * working tree, or from `declared.at`; its published packs installed under `scratch`. Every listed pack must
 * load, at the version listed: one that does not fails the scan, naming it and why. So does a `config` its rules do
 * not declare or allow (`declaredPackRuleConfigs`).
 */
export async function declaredScanPacks(
  root: string, declared: DeclaredPacks, scratch: string, storeOf: (home: string) => PackStore = npmPackStore,
): Promise<DeclaredScanPacks> {
  checkDeclaredFolders(root, declared);
  const directories = await declaredPackDirectories(root, declared, scratch, storeOf);
  const catalog = declaredPackCatalog(directories, { coreVersion: activeCoreVersion(), compile: true });
  const loaded = loadedDeclaredPacks(catalog, directories, declared.label);
  return {
    catalog,
    packs: loaded.map(({ loaded: pack }) => pack),
    attachments: loaded.flatMap(({ pack, loaded: { id } }) => packAttachmentFolders(pack).map(folder => ({ packId: id, folder }))),
    ruleConfigs: loaded.flatMap(({ pack, loaded: { id } }) => declaredPackRuleConfigs(catalog, pack, id, declared.label)),
  };
}
