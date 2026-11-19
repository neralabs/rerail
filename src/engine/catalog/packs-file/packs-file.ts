/**
 * Internal testing: `.rerail/packs.json`, the rule packs a repository enables, declared in the repository so every
 * machine that scans a commit runs the same packs (docs/repository-packs.md). A published pack is named by its id or
 * package name at one exact version; a pack kept in the repository by its folder, relative to the repository's root.
 * Either may name the folders it judges (`folders`); without them it judges the whole repository. Reading the file
 * only says which packs and where: fetching them and approving their content are the caller's and the approval
 * policy's.
 */
import { lstatSync, readdirSync, readFileSync, type Stats } from "node:fs";
import { join } from "node:path";
import semver from "semver";
import { z } from "zod";
import { parseJsonWithoutDuplicateKeys } from "../../json-duplicate-keys.js";
import type { RuleCatalog } from "../repository-model.js";
import { readRuleCatalog } from "../rule-catalog.js";
import { packIdOfPackage, rulePackPackageOf, type RulePackLocalSource } from "../rule-packs.js";

/** Where a repository declares its packs, relative to its top level. */
export const PACKS_FILE = ".rerail/packs.json";
/** The `schema` the file must carry: the shape repositories already commit, so `folders` stays optional in it. */
export const PACKS_FILE_SCHEMA = "rerail.packs/1";
/** A packs file larger than this is refused unread, as a rule document that large is. */
export const MAX_PACKS_FILE_BYTES = 1024 * 1024;

/**
 * Where a pack judges: `folders`, each relative to the repository's top level with `/` separators, as rule attachments
 * name folders; undefined for the whole repository.
 */
interface PackFolders {
  readonly folders?: readonly string[];
}

/** A pack published to npm, at one exact version. */
export interface PublishedPack extends PackFolders {
  readonly kind: "published";
  readonly packageName: string;
  readonly id: string;
  readonly version: string;
}

/** A pack kept in the repository: `path` is its folder, relative to the repository's top level, with `/` separators. */
export interface RepositoryPack extends PackFolders {
  readonly kind: "repository";
  readonly path: string;
}

export type DeclaredPack = PublishedPack | RepositoryPack;

/** The folders a declared pack is attached at: each of its `folders`, or the whole repository (`""`). */
export function packAttachmentFolders(pack: DeclaredPack): readonly string[] {
  return pack.folders ?? [""];
}

/** A packs file that cannot be read or does not say what it must. Its message names the file and what is wrong. */
export class PacksFileError extends Error {
  override readonly name = "PacksFileError";
}

/** `<id|package>@<version>` or the folder: how messages name a declared pack. */
export function declaredPackLabel(pack: DeclaredPack): string {
  return pack.kind === "published" ? `${pack.packageName}@${pack.version}` : pack.path;
}

/** The file's shape; each entry is told apart by its keys (`declaredPack`), so its error says which pack it is. */
const packsFileSchema = z.object({
  schema: z.literal(PACKS_FILE_SCHEMA),
  packs: z.array(z.record(z.string(), z.unknown())),
}).strict();

/** Why `path` is not a repository-relative folder inside the repository, or undefined when it is one. */
function pathProblem(path: string): string | undefined {
  if (path === "") return "is empty";
  if (path.includes("\\")) return "uses \\; separate folders with /";
  if (path.startsWith("/") || /^[A-Za-z]:/u.test(path)) return "is absolute; give it relative to the repository's root";
  if (path.split("/").some(segment => segment === "" || segment === "." || segment === "..")) return "must name a folder inside the repository, without empty, . or .. parts";
  return undefined;
}

/** An entry's `folders`: a non-empty list of distinct folders inside the repository, or undefined when it has none. */
function declaredFolders(value: unknown, at: string): readonly string[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value) || value.length === 0) throw new PacksFileError(`${at}.folders must be a list of at least one folder; leave it out for the whole repository`);
  return value.map((folder: unknown, index) => {
    if (typeof folder !== "string") throw new PacksFileError(`${at}.folders[${index}] is not a string`);
    const problem = pathProblem(folder);
    if (problem !== undefined) throw new PacksFileError(`${at}.folders[${index}] ${JSON.stringify(folder)} ${problem}`);
    if (value.indexOf(folder) !== index) throw new PacksFileError(`${at}.folders lists ${folder} twice`);
    return folder;
  });
}

function declaredPack(entry: Readonly<Record<string, unknown>>, at: string): DeclaredPack {
  const { folders: listed, ...rest } = entry;
  const folders = declaredFolders(listed, at);
  const where = folders === undefined ? {} : { folders };
  const keys = Object.keys(rest).sort().join(",");
  if (keys === "path") {
    if (typeof rest.path !== "string") throw new PacksFileError(`${at}.path is not a string`);
    const problem = pathProblem(rest.path);
    if (problem !== undefined) throw new PacksFileError(`${at}.path ${JSON.stringify(rest.path)} ${problem}`);
    return { kind: "repository", path: rest.path, ...where };
  }
  if (keys === "id,version") {
    if (typeof rest.id !== "string") throw new PacksFileError(`${at}.id is not a string`);
    const packageName = rulePackPackageOf(rest.id);
    if (packageName === undefined) throw new PacksFileError(`${at}.id ${JSON.stringify(rest.id)} is neither a pack id (complexity) nor a rule pack's package name (@acme/rerail-pack-sql)`);
    // One exact version: npm resolves it to the same content forever, unlike a range or a dist-tag.
    if (typeof rest.version !== "string" || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u.test(rest.version) || semver.valid(rest.version) !== rest.version) {
      throw new PacksFileError(`${at}.version must be one exact version, such as 1.4.0, not ${JSON.stringify(rest.version)}: a range or a tag runs other content as new versions publish`);
    }
    return { kind: "published", packageName, id: packIdOfPackage(packageName)!, version: rest.version, ...where };
  }
  throw new PacksFileError(`${at} must have "id" and "version" for a published pack, or "path" for a pack in the repository, and optionally "folders", not ${keys === "" ? "no keys" : keys.split(",").map(key => JSON.stringify(key)).join(", ")}`);
}

/**
 * The packs `text` declares, in the file's order. Throws `PacksFileError`, naming `label` and what is wrong, for text
 * that is not JSON or repeats a key, a `schema` other than `rerail.packs/1`, a key the format does not define, an
 * entry that is neither a published pack at an exact version nor a folder inside the repository, `folders` that are
 * not distinct folders inside the repository, or a pack listed twice.
 */
export function parsePacksFile(text: string, label: string = PACKS_FILE): DeclaredPack[] {
  let value: unknown;
  try {
    value = parseJsonWithoutDuplicateKeys(text, label);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new PacksFileError(message.startsWith(`${label}:`) ? message : `${label} is not valid JSON: ${message}`);
  }
  const parsed = packsFileSchema.safeParse(value);
  if (!parsed.success) {
    const issue = parsed.error.issues[0];
    const where = issue === undefined || issue.path.length === 0 ? "" : `${issue.path.map(String).join(".")}: `;
    throw new PacksFileError(`${label} is not a ${PACKS_FILE_SCHEMA} document: ${where}${issue?.message ?? "invalid"}`);
  }
  try {
    const packs = parsed.data.packs.map((entry, index) => declaredPack(entry, `packs[${index}]`));
    const seen = new Set<string>();
    for (const pack of packs) {
      const key = pack.kind === "published" ? pack.packageName : `path:${pack.path}`;
      if (seen.has(key)) throw new PacksFileError(`lists ${pack.kind === "published" ? pack.packageName : pack.path} twice`);
      seen.add(key);
    }
    return packs;
  } catch (error) {
    if (error instanceof PacksFileError) throw new PacksFileError(`${label} ${error.message}`);
    throw error;
  }
}

/** The lstat of `path`, or undefined when nothing is there. */
function lstatOrMissing(path: string): Stats | undefined {
  try {
    return lstatSync(path);
  } catch (error) {
    if (error instanceof Error && "code" in error && (error.code === "ENOENT" || error.code === "ENOTDIR")) return undefined;
    throw error;
  }
}

/**
 * The packs the working tree at `root` declares, or undefined when it has no `.rerail/packs.json`. A symbolic link is
 * never followed, for `.rerail` or the file: the packs a checkout declares are a file inside it. Throws
 * `PacksFileError` for a file that is not a regular file, is larger than 1 MiB or does not parse (`parsePacksFile`).
 */
export function readPacksFile(root: string): DeclaredPack[] | undefined {
  const folder = lstatOrMissing(join(root, ".rerail"));
  if (folder === undefined || !folder.isDirectory()) return undefined;
  const path = join(root, PACKS_FILE);
  const file = lstatOrMissing(path);
  if (file === undefined) return undefined;
  if (!file.isFile()) throw new PacksFileError(`${PACKS_FILE} is not a regular file`);
  if (file.size > MAX_PACKS_FILE_BYTES) throw new PacksFileError(`${PACKS_FILE} is larger than 1 MiB`);
  return parsePacksFile(readFileSync(path, "utf8"));
}

/** The first symbolic link at or under `inside`, a path relative to `directory`; undefined when there is none. */
function linkAt(directory: string, inside: string): string | undefined {
  const info = lstatOrMissing(join(directory, ...inside.split("/")));
  if (info === undefined) return undefined;
  if (info.isSymbolicLink()) return inside;
  if (!info.isDirectory()) return undefined;
  for (const name of readdirSync(join(directory, ...inside.split("/"))).sort()) {
    const link = linkAt(directory, `${inside}/${name}`);
    if (link !== undefined) return link;
  }
  return undefined;
}

/**
 * `relative` (a folder inside the repository, `/`-separated) in the working tree at `root`. Throws `PacksFileError`,
 * naming it as `what`, when it is missing, a part of its path is a symbolic link, or it is not a folder.
 */
function checkoutFolder(root: string, relative: string, what: string): string {
  let at = root;
  for (const segment of relative.split("/")) {
    at = join(at, segment);
    const info = lstatOrMissing(at);
    if (info === undefined) throw new PacksFileError(`${PACKS_FILE} lists ${what}, which is not in the repository`);
    if (info.isSymbolicLink()) throw new PacksFileError(`${PACKS_FILE} lists ${what}, whose path goes through a symbolic link`);
  }
  if (!lstatSync(at).isDirectory()) throw new PacksFileError(`${PACKS_FILE} lists ${what}, which is not a folder`);
  return at;
}

/**
 * Checks that every folder `pack` names (`folders`) is a folder in the working tree at `root`, reached through no
 * symbolic link, as a pack's `path` must be: a typo must not pass as a pack that judges nothing.
 */
export function checkPackFolders(root: string, pack: DeclaredPack): void {
  for (const folder of pack.folders ?? []) checkoutFolder(root, folder, `the folder ${folder} for ${declaredPackLabel(pack)}`);
}

/**
 * The folder of the repository pack `pack` in the working tree at `root`. Throws `PacksFileError` when a part of its
 * path, or what a pack is read from inside it (`pack.json`, `package.json`, `rules/`), is a symbolic link, so a pack is
 * always files inside the checkout, as a scan of the same commit with `--rules-ref` reads it; or when it is not a folder.
 */
export function repositoryPackDirectory(root: string, pack: RepositoryPack): string {
  const at = checkoutFolder(root, pack.path, pack.path);
  const link = ["pack.json", "package.json", "rules"].map(inside => linkAt(at, inside)).find(found => found !== undefined);
  if (link !== undefined) throw new PacksFileError(`${PACKS_FILE} lists ${pack.path}, which holds a symbolic link at ${link}`);
  return at;
}

/** One declared pack and the folder it was read into. */
export interface DeclaredPackDirectory {
  readonly pack: DeclaredPack;
  readonly directory: string;
  /** For a repository pack: where it is in the checkout, which the catalog shows as the folder it came from. */
  readonly checkoutPath?: string;
}

/**
 * The catalog of the declared packs in `directories`, read and checked as installed packs are (`readRuleCatalog`). A
 * repository pack counts as installed from a folder, whatever its `package.json` names, so it never passes as
 * Rerail's own and runs in Log until a person approves its content. A pack that fails a check is in `skipped`, with
 * its folder as `path`.
 */
export function declaredPackCatalog(directories: readonly DeclaredPackDirectory[], options: { coreVersion?: string; compile?: boolean } = {}): RuleCatalog {
  const localSources = new Map<string, RulePackLocalSource>(directories.flatMap(({ pack, directory, checkoutPath }) =>
    pack.kind === "repository" ? [[directory, { kind: "folder", path: checkoutPath ?? directory }] as const] : []));
  return readRuleCatalog(directories.map(({ directory }) => directory), { ...options, localSources });
}
