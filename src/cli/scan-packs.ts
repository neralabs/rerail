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
import { readGitBlobs, runGit } from "../engine/git.js";
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
  /** The commit the file was read from, or undefined for the working tree. */
  readonly commit?: string;
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

function gitOutput(root: string, args: readonly string[]): string | null {
  const result = runGit(root, args, { maxBuffer: 256 * 1024 * 1024 });
  if (result.error !== undefined) throw result.error;
  return result.status === 0 ? result.stdout : null;
}

/** `<mode> <type> <object>\t<path>` entries of `git ls-tree -z` output. */
function treeEntries(output: string): { mode: string; type: string; object: string; path: string }[] {
  return output.split("\0").flatMap(entry => {
    const match = /^(\d+) (\w+) ([0-9a-f]+)\t(.+)$/su.exec(entry);
    return match === null ? [] : [{ mode: match[1]!, type: match[2]!, object: match[3]!, path: match[4]! }];
  });
}

/** Whether a tree entry is a regular file: a blob that is not a symbolic link (`120000`) or a submodule. */
const isRegularBlob = (entry: { mode: string; type: string }): boolean => entry.type === "blob" && entry.mode !== "120000";

/**
 * The packs the scan's tree declares: the working tree's `.rerail/packs.json`, or the file at `rulesRef`. Undefined
 * when that tree has none. Throws when the file cannot be read or parsed, or `rulesRef` is not a commit.
 */
export async function declaredPacksOf(root: string, rulesRef: string | undefined): Promise<DeclaredPacks | undefined> {
  if (rulesRef === undefined) {
    const packs = readPacksFile(root);
    return packs === undefined ? undefined : { packs, label: PACKS_FILE };
  }
  const commit = gitOutput(root, ["rev-parse", "--verify", "--quiet", `${rulesRef}^{commit}`])?.trim();
  if (commit === undefined) throw new Error(`--rules-ref ${rulesRef} is not a commit in ${root}`);
  const label = `${rulesRef}:${PACKS_FILE}`;
  // Each part of the path is looked up in its own tree, so a symbolic link `.rerail` lists nothing, as it would be
  // left unread in a checkout.
  const entry = treeEntries(gitOutput(root, ["ls-tree", "-z", commit, "--", PACKS_FILE]) ?? "").find(found => found.path === PACKS_FILE);
  if (entry === undefined) return undefined;
  if (!isRegularBlob(entry)) throw new PacksFileError(`${label} is not a regular file`);
  const blob = (await readGitBlobs(root, [entry.object], { maxBlobBytes: MAX_PACKS_FILE_BYTES })).get(entry.object);
  if (blob === undefined) throw new PacksFileError(`${label} is larger than 1 MiB`);
  return { packs: parsePacksFile(blob.toString("utf8"), label), label, commit };
}

/**
 * The repository pack `pack` as it is at `commit`, written under `into`: its `pack.json`, `package.json` and `rules/`,
 * which is all a pack is read from. A folder keeps its name, which a pack without a `package.json` must have. A
 * symbolic link is not written, so a pack that needs one fails to load, as it would from a checkout that refuses it,
 * and nor is a file whose path has a part that is not a plain name (`..`, or a `\` or `:` Windows reads as a path), so
 * nothing is written outside `into`.
 */
async function repositoryPackAtCommit(root: string, commit: string, label: string, pack: RepositoryPack, into: string): Promise<string> {
  if (gitOutput(root, ["cat-file", "-t", `${commit}:${pack.path}`])?.trim() !== "tree") {
    throw new PacksFileError(`${label} lists ${pack.path}, which is not a folder at that commit`);
  }
  const directory = join(into, basename(pack.path));
  const prefix = `${pack.path}/`;
  const files = treeEntries(gitOutput(root, ["ls-tree", "-r", "-z", commit, "--", prefix]) ?? "")
    .map(entry => ({ ...entry, inside: entry.path.slice(prefix.length) }))
    .filter(entry => entry.path.startsWith(prefix) && isRegularBlob(entry))
    .filter(entry => entry.inside.split("/").every(part => part !== "" && part !== "." && part !== ".." && !/[\\:]/u.test(part)))
    .filter(entry => entry.inside === "pack.json" || entry.inside === "package.json" || entry.inside.startsWith("rules/"));
  const blobs = await readGitBlobs(root, files.map(entry => entry.object), { maxBlobBytes: MAX_REF_PACK_FILE_BYTES });
  mkdirSync(directory, { recursive: true });
  for (const file of files) {
    const blob = blobs.get(file.object);
    if (blob === undefined) continue;
    const target = join(directory, ...file.inside.split("/"));
    if (relative(directory, target).startsWith("..") || isAbsolute(relative(directory, target))) continue;
    mkdirSync(join(target, ".."), { recursive: true });
    writeFileSync(target, blob);
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
    await storeOf(home).install(specs, { preferOffline: true });
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
 * Checks that every folder `pack` names is a folder at `commit`: each part of its path a tree, so a symbolic link on
 * the way (a blob) or a missing folder is refused, as in a working tree.
 */
function checkPackFoldersAtCommit(root: string, commit: string, label: string, pack: DeclaredPack): void {
  for (const folder of pack.folders ?? []) {
    const parts = folder.split("/");
    const inTree = parts.every((_, index) => gitOutput(root, ["cat-file", "-t", `${commit}:${parts.slice(0, index + 1).join("/")}`])?.trim() === "tree");
    if (!inTree) throw new PacksFileError(`${label} lists the folder ${folder} for ${declaredPackLabel(pack)}, which is not a folder at that commit`);
  }
}

/** Checks every folder the declared packs name: in the working tree, or at `declared.commit`. */
function checkDeclaredFolders(root: string, declared: DeclaredPacks): void {
  for (const pack of declared.packs) {
    if (declared.commit === undefined) checkPackFolders(root, pack);
    else checkPackFoldersAtCommit(root, declared.commit, declared.label, pack);
  }
}

/**
 * The folder each declared pack is read from: a published pack installed under `scratch`; a repository pack in the
 * working tree, or written under `scratch` from `declared.commit`.
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
    const directory = declared.commit === undefined
      ? repositoryPackDirectory(root, pack)
      : await repositoryPackAtCommit(root, declared.commit, declared.label, pack, join(scratch, "repository-packs", String(index)));
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
 * working tree, or from `declared.commit`; its published packs installed under `scratch`. Every listed pack must
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
