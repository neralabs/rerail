/**
 * `gitCrlfPaths` for callers that ask on every pass: each session's walk asks it for the changes it found, and a Git
 * process per question blocks the daemon's thread while it starts. The answer rests on two things Git reads.
 *
 * - The settings `core.autocrlf`, `core.eol`, `core.attributesFile` and `attr.tree`: asked of Git once per state of the
 *   configuration files (`checkoutConfiguration`, which stamps every file Git reads them from), in one `git config`.
 * - Each path's `text`, `crlf` and `eol` attributes: Git reads them from the `.gitattributes` of every folder from the
 *   checkout's top down to the path (from the index where the folder's file is not in the working tree), the common
 *   Git folder's `info/attributes`, the global file (`core.attributesFile`, else Git's default) and the system file
 *   (`git var GIT_ATTR_SYSTEM`). A path's answer from `git check-attr` is kept while each of those files holds its
 *   stamp, and the index too where a folder's file is tracked and absent; where none of those files exists, every
 *   attribute is unspecified and no Git is asked.
 *
 * Git is asked on every call, as `gitCrlfPaths` does, where those inputs cannot be named: a variable that moves where Git
 * looks (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE`, `GIT_ATTR_SOURCE`), `attr.tree` set, Git before
 * 2.42, a configuration or attributes file that cannot be stamped, and an input written less than 2 s ago (a write in
 * the same clock tick could leave its stamp unchanged).
 */
import { stat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileStamp } from "./file-stamp.js";
import {
  crlfRule,
  type CrlfRule,
  gitCrlfPaths,
  gitHome,
  type GitReadOptions,
  type GitResult,
  type LineEndingAttributes,
  lineEndingAttributes,
  runGitAsync,
} from "./git.js";
import { checkoutConfiguration, type CheckoutConfiguration, namedFiles } from "./git-head-reader.js";

export type GitCrlfReader = (cwd: string, paths: readonly string[], options?: GitReadOptions & { platform?: NodeJS.Platform }) => Promise<Map<string, CrlfRule>>;

/** The settings of a checkout's line endings and attribute files, as Git named them under one configuration state. */
interface Settings {
  readonly autocrlf: string | undefined;
  readonly coreEol: string | undefined;
  /** `core.attributesFile`, else Git's default global attributes file; absolute. */
  readonly globalAttributes: string;
  /** `attr.tree` is set: attributes come from a tree, which no file stamp follows. */
  readonly byTree: boolean;
}

/** Everything a path's attributes depend on beside its folders' `.gitattributes`, as one key. */
interface Sources {
  readonly key: string;
  readonly settled: boolean;
  /** Whether any of the files other than the folders' exists. */
  readonly any: boolean;
  /** The folders (relative, `/`-separated, `""` for the top) whose `.gitattributes` the index tracks. */
  readonly tracked: ReadonlySet<string>;
  readonly index: string;
}

interface Stamped {
  readonly stamp: string;
  readonly settled: boolean;
}

const SETTLE_MS = 2_000;
/** Paths whose attributes a reader keeps per checkout: past it, it starts again from none. */
const KEPT_PATHS = 20_000;
const MOVING_VARIABLES = ["GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE", "GIT_ATTR_SOURCE"];
const SETTINGS = String.raw`^(core\.(autocrlf|eol|attributesfile)|attr\.tree)$`;
const UNSPECIFIED: LineEndingAttributes = { text: "unspecified", crlf: "unspecified", eol: "unspecified" };

async function gitRun(cwd: string, args: readonly string[], signal?: AbortSignal): Promise<GitResult | undefined> {
  try {
    return await runGitAsync(cwd, args, { signal });
  } catch {
    return undefined;
  }
}

/**
 * `path`'s stamp, or `absent`. It is the stamp of the file a link names (`stat`): Git reads the global, system and
 * `info/attributes` files through a link, and a `.gitattributes` that is a link not at all, so following one only asks
 * Git again when its target changes, and a link to nothing reads absent, as Git finds no attributes there.
 */
async function stampOf(path: string): Promise<Stamped | undefined> {
  try {
    const info = await stat(path);
    return { stamp: fileStamp(info), settled: Date.now() - Math.floor(info.ctimeMs) >= SETTLE_MS };
  } catch (error) {
    const code = error instanceof Error && "code" in error ? error.code : undefined;
    return code === "ENOENT" || code === "ENOTDIR" ? { stamp: "absent", settled: true } : undefined;
  }
}

/** The settings Git names for `cwd` (`git config --get-regexp`, exit 1 when none is set); undefined when it cannot. */
async function askSettings(cwd: string, signal: AbortSignal | undefined): Promise<Settings | undefined> {
  const printed = await gitRun(cwd, ["config", "-z", "--get-regexp", SETTINGS], signal);
  if (printed === undefined || (printed.status !== 0 && !(printed.status === 1 && printed.stdout === ""))) return undefined;
  const values = new Map(printed.stdout.split("\0").filter(entry => entry !== "").map(entry => {
    const [name = "", ...value] = entry.split("\n");
    return [name, value.join("\n")] as const;
  }));
  return {
    autocrlf: values.get("core.autocrlf")?.toLowerCase(),
    coreEol: values.get("core.eol")?.toLowerCase(),
    globalAttributes: globalAttributesFile(cwd, values.get("core.attributesfile")),
    byTree: values.has("attr.tree"),
  };
}

/** `core.attributesFile` as Git finds it (`~/` in Git's home), else `git/attributes` in `XDG_CONFIG_HOME` or `~/.config`. */
function globalAttributesFile(cwd: string, configured: string | undefined): string {
  if (configured !== undefined && configured !== "") return configured.startsWith("~/") ? join(gitHome(), configured.slice(2)) : resolve(cwd, configured);
  const xdg = process.env.XDG_CONFIG_HOME;
  return join(xdg === undefined || xdg === "" ? join(gitHome(), ".config") : xdg, "git", "attributes");
}

/** The system attributes file `git var` names (Git 2.42 and later), none under `GIT_ATTR_NOSYSTEM`; undefined before. */
async function askSystemAttributes(cwd: string): Promise<string[] | undefined> {
  return namedFiles(await gitRun(cwd, ["var", "GIT_ATTR_SYSTEM"]));
}

/** The folders whose `.gitattributes` the index tracks, relative and `/`-separated; undefined when Git cannot list them. */
async function askTracked(cwd: string, signal: AbortSignal | undefined): Promise<Set<string> | undefined> {
  const printed = await gitRun(cwd, ["ls-files", "-z", "--cached", "--", ":(glob)**/.gitattributes"], signal);
  if (printed?.status !== 0) return undefined;
  return new Set(printed.stdout.split("\0").filter(path => path !== "").map(path => path.split("/").slice(0, -1).join("/")));
}

/** Each folder from the top down to `path`'s, relative and `/`-separated, `""` for the top. */
function foldersOf(path: string): string[] {
  const parts = path.split("/").slice(0, -1);
  return ["", ...parts.map((_, at) => parts.slice(0, at + 1).join("/"))];
}

/** A path's key: what it depends on beside the folders, each folder's file, and the index where a tracked one is absent. */
function pathKey(sources: Sources, path: string, folders: ReadonlyMap<string, Stamped | undefined>): PathKey | undefined {
  const parts: PathKey[] = [];
  for (const folder of foldersOf(path)) {
    const stamped = folders.get(folder);
    if (stamped === undefined) return undefined;
    parts.push(folderPart(sources, folder, stamped));
  }
  return {
    key: [sources.key, ...parts.map(part => part.key)].join("\0"),
    settled: sources.settled && parts.every(part => part.settled),
    any: sources.any || parts.some(part => part.any),
  };
}

/** One folder's part of a path's key: its `.gitattributes`' stamp, or the index's where that file is tracked and absent. */
function folderPart(sources: Sources, folder: string, stamped: Stamped): PathKey {
  const fromIndex = stamped.stamp === "absent" && sources.tracked.has(folder);
  return { key: fromIndex ? `index\0${sources.index}` : stamped.stamp, settled: stamped.settled, any: fromIndex || stamped.stamp !== "absent" };
}

interface PathKey {
  readonly key: string;
  readonly settled: boolean;
  /** Whether any file Git reads the path's attributes from exists. */
  readonly any: boolean;
}

interface KeptAttributes {
  readonly key: string;
  readonly attributes: LineEndingAttributes;
}

type Kept = Map<string, KeptAttributes>;

/** A path's attributes without Git: kept under the same key, or none where no file can set them; undefined otherwise. */
function withoutGit(held: KeptAttributes | undefined, key: PathKey | undefined): LineEndingAttributes | undefined {
  if (key === undefined) return undefined;
  if (held?.key === key.key) return held.attributes;
  return key.any ? undefined : UNSPECIFIED;
}

/** The settings and attribute files a call found named, and the stamps of those files and of the index. */
interface Named {
  readonly config: CheckoutConfiguration;
  readonly files: readonly string[];
}

/** {@link Sources} from the configuration's key, the attribute files' stamps, the tracked folders and the index's stamp. */
function sourcesOf(found: Named, stamps: readonly Stamped[], tracked: ReadonlySet<string>, index: Stamped): Sources {
  return {
    key: [found.config.key, ...found.files.map((path, at) => `${path}\0${stamps[at]!.stamp}`)].join("\0"),
    settled: found.config.settled && stamps.every(stamped => stamped.settled),
    any: stamps.some(stamped => stamped.stamp !== "absent"),
    tracked,
    index: index.stamp,
  };
}

/** Each of `paths`' folders' `.gitattributes` in `cwd`, stamped once per call. */
async function folderStamps(cwd: string, paths: readonly string[]): Promise<Map<string, Stamped | undefined>> {
  const folders = [...new Set(paths.flatMap(foldersOf))];
  const stamps = await Promise.all(folders.map(async folder => await stampOf(join(cwd, folder, ".gitattributes"))));
  return new Map(folders.map((folder, at) => [folder, stamps[at]]));
}

/** The reader of this module's comment; `readGitCrlfPaths` is the daemon's one. */
export function gitCrlfReader(): GitCrlfReader {
  const settings = new Map<string, { readonly key: string; readonly settings: Settings }>();
  const tracked = new Map<string, { readonly index: string; readonly folders: ReadonlySet<string> }>();
  const kept = new Map<string, Kept>();
  let system: Promise<string[] | undefined> | undefined;

  const settingsFor = async (cwd: string, config: CheckoutConfiguration, signal: AbortSignal | undefined): Promise<Settings | undefined> => {
    const known = settings.get(cwd);
    if (known?.key === config.key) return known.settings;
    const asked = await askSettings(cwd, signal);
    if (asked !== undefined && config.settled) settings.set(cwd, { key: config.key, settings: asked });
    return asked;
  };

  const trackedFor = async (cwd: string, index: Stamped, signal: AbortSignal | undefined): Promise<ReadonlySet<string> | undefined> => {
    const known = tracked.get(cwd);
    if (known?.index === index.stamp) return known.folders;
    const asked = await askTracked(cwd, signal);
    if (asked !== undefined && index.settled) tracked.set(cwd, { index: index.stamp, folders: asked });
    return asked;
  };

  /** The settings and the attribute files beside the folders', for a checkout whose configuration the files name. */
  const namedFor = async (cwd: string, signal: AbortSignal | undefined) => {
    const config = await checkoutConfiguration(cwd, signal);
    const named = config === undefined ? undefined : await settingsFor(cwd, config, signal);
    system ??= askSystemAttributes(cwd);
    const systemFile = await system;
    if (config === undefined || named === undefined || named.byTree || systemFile === undefined) return undefined;
    return { config, named, files: [join(config.layout.commonDir, "info", "attributes"), named.globalAttributes, ...systemFile] };
  };

  /** Everything beside the folders' files, named and stamped; undefined where Git must answer every call. */
  const sourcesFor = async (cwd: string, signal: AbortSignal | undefined): Promise<{ sources: Sources; settings: Settings } | undefined> => {
    const found = MOVING_VARIABLES.some(name => process.env[name]) ? undefined : await namedFor(cwd, signal);
    if (found === undefined) return undefined;
    const [index, ...stamps] = await Promise.all([join(found.config.layout.gitDir, "index"), ...found.files].map(stampOf));
    const folders = index === undefined ? undefined : await trackedFor(cwd, index, signal);
    if (folders === undefined || !stamps.every(stamped => stamped !== undefined)) return undefined;
    return { settings: found.named, sources: sourcesOf(found, stamps, folders, index!) };
  };

  /** What `cwd`'s reader keeps per path, started again from none past `KEPT_PATHS`. */
  const keptFor = (cwd: string): Kept => {
    const known: Kept = kept.get(cwd) ?? new Map();
    if (known.size > KEPT_PATHS) known.clear();
    kept.set(cwd, known);
    return known;
  };

  /** Each asked path's attributes: kept, none where no file can set them, else Git's (kept where its inputs settled). */
  const attributesFor = async (cwd: string, sources: Sources, asked: readonly string[], signal: AbortSignal | undefined) => {
    const folders = await folderStamps(cwd, asked);
    const known = keptFor(cwd);
    const keys = new Map(asked.map(path => [path, pathKey(sources, path, folders)]));
    const answers = new Map<string, LineEndingAttributes>();
    for (const [path, key] of keys) {
      const attributes = withoutGit(known.get(path), key);
      if (attributes !== undefined) answers.set(path, attributes);
    }
    const stale = asked.filter(path => !answers.has(path));
    const fromGit = stale.length === 0 ? new Map<string, LineEndingAttributes>() : await lineEndingAttributes(cwd, stale, signal);
    for (const [path, attributes] of fromGit) {
      const key = keys.get(path);
      answers.set(path, attributes);
      if (key?.settled === true) known.set(path, { key: key.key, attributes });
    }
    return answers;
  };

  return async (cwd, paths, options = {}) => {
    const { platform = process.platform, signal } = options;
    const asked = paths.filter(path => !path.includes("\0"));
    const read = asked.length === 0 ? undefined : await sourcesFor(cwd, signal);
    if (read === undefined) return await gitCrlfPaths(cwd, asked, { platform, signal });
    const attributes = await attributesFor(cwd, read.sources, asked, signal);
    const rules = new Map<string, CrlfRule>();
    for (const [path, held] of attributes) {
      const rule = crlfRule(held, read.settings, platform);
      if (rule !== undefined) rules.set(path, rule);
    }
    return rules;
  };
}

/** The daemon's one reader: every session's walk shares the settings and attributes Git named. */
export const readGitCrlfPaths: GitCrlfReader = gitCrlfReader();
