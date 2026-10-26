/**
 * A tool-pack's recording as one keyed archive: every answer upstream gave the gates recorder, as a tree of one file
 * per answer, named by everything the answer depends on, packed as one canonical tar (scripts/s3/test-data-archive.mjs)
 * compressed with zstd, `recording.tar.zst` (`rerail.port-readiness-recording-archive/v1`), which `recording.lock` names
 * by its tar's sha256, file count and byte total (store.ts).
 *
 *   config/<config-id>.json                         the parts the config id hashes (canonical JSON)
 *   tool/<config-id>/<field>.json                   what upstream says about the whole tool (tree.ts)
 *   tool/<config-id>/name-sets/<set>.json           names and regexes several rules share (tree.ts)
 *   rules/<config-id>/<rule>/rule.json              a rule's facts, names and case index (tree.ts)
 *   rules/<config-id>/<rule>/cases/<key>.json       upstream's answer on one of its cases (tree.ts)
 *   texts/<sha256>.json                             one upstream test text (tree.ts)
 *   verdicts/<parsers-id>/<sha256><type>[.script].json    independent parsers' verdicts on one case source or probe (verdicts.ts)
 *
 * `<config-id>` is the sha256 of `config/<config-id>.json` (answerConfig): upstream's pins (repository, commit, version,
 * runtime), the recorder's runtime pin digests as recording.lock records them, every gates.yaml setting the adapter
 * reads (the upstream block, the whole ESLint block, rerail.in_scope), the "Not copied" prefixes of the pack's
 * UPSTREAM.md files, the digest of the recorder's adapter code for the tool-pack's adapter (ADAPTER_SOURCES), which
 * decides what the recorder asks upstream and how it reads the answer, and RECORDING_FORMAT, the version of the tree
 * the recorder writes from upstream's answers. Moving any of them is a new config id, under which every answer is
 * asked again; a record keeps one config id's files. An answer exists exactly while its file does, so the recorder asks
 * upstream only about the names a tree lacks, and the gates check by existence.
 *
 * Every file is canonicalText, so an answer has one form and the archive is a function of the answers alone.
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

import { extractRecordingTree, RECORDING_ARCHIVE, RECORDING_ARCHIVE_FORMAT, RECORDING_TREE_MARKER, RECORDING_TREE_ROOTS, recordingTreeReady, ROOT } from "../../../scripts/s3/pack-fixtures.mjs";
import { archiveSizeBound, buildArchive, compressArchive, decompressArchive, readArchive, sha256, type ArchiveEntry } from "../../../scripts/s3/test-data-archive.mjs";
import { trackedDigest } from "../cache.js";
import type { GatesManifest } from "../manifest.js";
import { canonicalText, type RecordedParser } from "./schema.js";

export { RECORDING_ARCHIVE, RECORDING_ARCHIVE_FORMAT };

/** The config object's own format: a change to what the config id hashes is a new format, and a new id. */
const CONFIG_FORMAT = 4;
const TOP_LEVEL = new Set(RECORDING_TREE_ROOTS.map((root) => root.replace(/\/$/u, "")));
const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** What recording.lock pins the archive by: its tar's sha256, how many files it holds and their total bytes. */
export interface ArchivePin {
  sha256: string;
  files: number;
  bytes: number;
}

/** A tool-pack's runtime pin digests, as recording.lock records them. */
export type RuntimeDigests = Record<string, Record<string, string>>;

const RECORDERS = "conformance/recorders/port-readiness";
const SPAN_UNITS = "conformance/port-readiness/span-units";
/** The recorder's language toolkits, each with the worker it runs and the toolkits it builds on. */
const PYTHON = [`${RECORDERS}/languages/python.ts`, `${RECORDERS}/languages/python_toolkit.py`];
const PHP = [`${RECORDERS}/languages/php.ts`, `${RECORDERS}/languages/php_toolkit.php`];
const BLADE = [`${RECORDERS}/languages/blade.ts`, `${RECORDERS}/languages/blade_toolkit.php`, ...PHP];
const ECMASCRIPT = [`${RECORDERS}/languages/ecmascript.ts`, `${RECORDERS}/languages/vue.ts`];
const RUST = [`${RECORDERS}/languages/rust.ts`];
/** An adapter's sources: the runtimes every adapter installs upstream with, its own files under adapters/, and `shared`. */
const sources = (own: readonly string[], ...shared: string[]): readonly string[] => [`${RECORDERS}/runtimes.ts`, ...own.map((file) => `${RECORDERS}/adapters/${file}`), ...shared];

/**
 * The recorder's code an adapter's answers depend on, as `git ls-files` pathspecs: the adapter and the drivers it runs
 * upstream through, the language toolkits it reads parsers' verdicts and literals with, the runtimes it installs
 * upstream with (runtimes.ts), and the span units that turn upstream's positions into spans. The gate code they import
 * besides (process.ts, types.ts, recorded-adapter.ts's licence reading) shapes no answer, but for recorded-adapter.ts's
 * decodeSource, which Bandit decodes its examples with, as the gates decode their committed copies: it is Bandit's. Its
 * digest is part of the config id and the parsers id, so a fix to how an answer is asked for or read is asked again
 * rather than mixed with answers the old code gave; recording-store.test.ts checks each adapter's imports and drivers
 * are covered.
 */
export const ADAPTER_SOURCES: Record<GatesManifest["adapter"], readonly string[]> = {
  bandit: sources(["bandit.ts", "bandit_driver.py"], ...PYTHON, `${SPAN_UNITS}/bandit.ts`, "conformance/port-readiness/recorded-adapter.ts"),
  ruff: sources(["ruff.ts", "ruff-tests.ts"], ...PYTHON, ...RUST, `${SPAN_UNITS}/ruff.ts`),
  eslint: sources(["eslint"], ...ECMASCRIPT, `${SPAN_UNITS}/eslint.ts`),
  phpcs: sources(["phpcs.ts", "phpcs_driver.php"], ...PHP, `${SPAN_UNITS}/phpcs.ts`),
  tlint: sources(["tlint.ts", "tlint_driver.php"], ...BLADE, `${SPAN_UNITS}/tlint.ts`),
  "blade-parser": sources(["blade-parser.ts", "blade_parser_driver.php", "blade_parser_tests.php", "blade_capture.php"], ...BLADE, `${SPAN_UNITS}/blade-parser.ts`),
  phpstan: sources(["phpstan.ts", "phpstan_driver.php"], ...PHP, `${SPAN_UNITS}/phpstan.ts`),
  psalm: sources(["psalm.ts", "psalm_driver.php"], ...PHP, `${SPAN_UNITS}/psalm.ts`, `${SPAN_UNITS}/bandit.ts`),
  bladestan: sources(["bladestan.ts", "bladestan_driver.php"], ...BLADE, `${SPAN_UNITS}/bladestan.ts`),
  sqlfluff: sources(["sqlfluff.ts", "sqlfluff_driver.py"], ...PYTHON, `${SPAN_UNITS}/sqlfluff.ts`),
  // Clippy reads its rustc target from the Ruff adapter.
  clippy: sources(["clippy.ts", "ruff.ts", "ruff-tests.ts"], ...PYTHON, ...RUST, `${SPAN_UNITS}/clippy.ts`, `${SPAN_UNITS}/ruff.ts`),
};

/**
 * The version of the tree the recorder writes for the same upstream output: the files it writes from upstream's
 * answers (record.ts's case layouts, not-copied skip, rule records and spans; tree.ts's case keys and rule and tool
 * files; schema.ts's spans and canonical text; verdicts.ts's verdict files). Bump it whenever the
 * recorder writes a different tree for the same upstream output (its keys, file layout, encodings, span handling, or
 * what is recorded), so every tool-pack records again rather than mixing files the old recorder wrote under names a
 * new one would write otherwise; never for a refactor or a comment. It is part of the config id and the parsers id,
 * and test/conformance/recorders/recording-format.test.ts pins the tree the recorder writes for a fixture tool-pack,
 * so a change to it fails until this is bumped and the pin updated.
 */
export const RECORDING_FORMAT = 2;

const sourcesDigests = new Map<GatesManifest["adapter"], string>();

/** trackedDigest of an adapter's ADAPTER_SOURCES in this package, once per process: the same in the gates and the recorder. */
export function adapterSourcesDigest(adapter: GatesManifest["adapter"]): string {
  let digest = sourcesDigests.get(adapter);
  if (digest === undefined) {
    digest = trackedDigest(ROOT, ADAPTER_SOURCES[adapter]);
    sourcesDigests.set(adapter, digest);
  }
  return digest;
}

/** An UPSTREAM.md line naming an upstream path prefix nothing is copied from (conformance/pack-validation.ts reads it too). */
const NOT_COPIED = /^- Not copied: `([^`]+)`/gmu;

/**
 * The upstream path prefixes the pack's `licenses/<project>/UPSTREAM.md` list as "Not copied", sorted: material an
 * upstream derives from a copyleft project (Ruff's fixtures for its pylint, flake8-builtins, flake8-django,
 * flake8-executable and refurb ports). No case source under one is committed, and no recording holds such a file's
 * text; a case under one is recorded as skipped, so the prefixes are part of the config id.
 */
export function notCopiedPrefixes(root: string, pack: string): string[] {
  const licenses = join(root, "packs", pack, "licenses");
  if (!existsSync(licenses)) return [];
  return readdirSync(licenses, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && existsSync(join(licenses, entry.name, "UPSTREAM.md")))
    .flatMap((entry) => [...readFileSync(join(licenses, entry.name, "UPSTREAM.md"), "utf8").matchAll(NOT_COPIED)].map((match) => match[1]!))
    .sort(byText);
}

/**
 * The parts a config id hashes, `config/<id>.json`'s content: everything that shapes a recorded answer, fact or text
 * (the module comment lists them). Every gates.yaml setting an adapter reads is here: the upstream block, the whole
 * ESLint block (which tests are read and how: tests, test_fixtures, case_testers, fixture_parsers, parsers, test_cwd,
 * esm_tests, implementation, helpers and the rest) and rerail.in_scope (how the ESLint driver handles components);
 * the other sections (popularity, known_gaps) shape no answer. `root` is the checkout whose pack's
 * UPSTREAM.md files name the not-copied prefixes.
 */
export function answerConfig(root: string, manifest: GatesManifest, runtimes: RuntimeDigests): Record<string, unknown> {
  const { upstream, eslint } = manifest;
  return {
    format: CONFIG_FORMAT,
    recording: RECORDING_FORMAT,
    adapter: manifest.adapter,
    sources: adapterSourcesDigest(manifest.adapter),
    repository: upstream.repository,
    commit: upstream.commit,
    version: upstream.version,
    runtime: upstream.runtime,
    runtimes,
    gates: { upstream, eslint, inScope: manifest.rerail.in_scope },
    notCopied: notCopiedPrefixes(root, manifest.rerail.pack),
  };
}

/** The config id: the sha256 of the config's canonical text, which is `config/<id>.json`'s bytes. */
export function configId(config: Record<string, unknown>): string {
  return sha256(canonicalText(config));
}

export function configPath(id: string): string {
  return `config/${id}.json`;
}

/** The config ids a tree holds, by its config/ files. */
export function treeConfigs(tree: RecordingTree): string[] {
  return tree.list("config/").map((path) => /^config\/([0-9a-f]{64})\.json$/u.exec(path)?.[1]).filter((id): id is string => id !== undefined);
}

/** The parsers id: the sha256 of the canonical text of the recorded parser list, the adapter's sources digest and RECORDING_FORMAT. */
export function parsersId(adapter: GatesManifest["adapter"], parsers: readonly RecordedParser[]): string {
  return sha256(canonicalText({ parsers, sources: adapterSourcesDigest(adapter), recording: RECORDING_FORMAT }));
}

/** Read access to a tree, by its paths: an extracted directory, or files in memory. */
export interface RecordingTree {
  /** The file's text, or undefined when the tree has no file there. */
  text(path: string): string | undefined;
  /** Every file path under `prefix` (a directory, with its final `/`), sorted. */
  list(prefix: string): string[];
}

export function directoryTree(directory: string): RecordingTree {
  return {
    text(path) {
      const file = join(directory, path);
      return existsSync(file) ? readFileSync(file, "utf8") : undefined;
    },
    list(prefix) {
      return [...treeFiles(join(directory, prefix), false).keys()].map((path) => `${prefix}${path}`).sort(byText);
    },
  };
}

export function memoryTree(files: ReadonlyMap<string, Buffer>): RecordingTree {
  return {
    text: (path) => files.get(path)?.toString("utf8"),
    list: (prefix) => [...files.keys()].filter((path) => path.startsWith(prefix)).sort(byText),
  };
}

/**
 * Every file of an extracted tree, `/`-separated paths relative to it, unsorted; the extraction's marker is none of
 * them. With `read` false the values are empty: only the paths are listed.
 */
export function treeFiles(directory: string, read = true): Map<string, Buffer> {
  const files = new Map<string, Buffer>();
  const empty = Buffer.alloc(0);
  const walk = (relative: string): void => {
    for (const entry of readdirSync(join(directory, relative), { withFileTypes: true })) {
      const path = relative === "" ? entry.name : `${relative}/${entry.name}`;
      if (path === RECORDING_TREE_MARKER) continue;
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile()) files.set(path, read ? readFileSync(join(directory, path)) : empty);
      else throw new Error(`${join(directory, path)} is neither a file nor a directory`);
    }
  };
  if (existsSync(directory)) walk("");
  return files;
}

/**
 * Throws unless every path is under one of the tree's top-level directories, and no two differ only by letter case: a
 * tree is extracted on macOS's case-insensitive file system too, where such two would be one file.
 */
export function checkTreePaths(paths: Iterable<string>): void {
  const folded = new Map<string, string>();
  for (const path of paths) {
    const slash = path.indexOf("/");
    if (slash <= 0 || !TOP_LEVEL.has(path.slice(0, slash))) throw new Error(`a recording archive holds only ${[...TOP_LEVEL].map((root) => `${root}/`).join(", ")}, not ${path}`);
    const key = path.toLowerCase();
    const other = folded.get(key);
    if (other !== undefined) throw new Error(`two recording files differ only by letter case, which a case-insensitive file system cannot hold: ${other} and ${path}`);
    folded.set(key, path);
  }
}

/** A tree packed: the archive's pin, its compressed bytes, and the entries it holds. */
export interface PackedArchive extends ArchivePin {
  compressed: Buffer;
  entries: ArchiveEntry[];
}

/** The canonical archive of a tree (test-data-archive.mjs): the same files always make the same tar. */
export function packTree(files: ReadonlyMap<string, Buffer>): PackedArchive {
  checkTreePaths(files.keys());
  const entries = [...files].map(([path, bytes]) => ({ path, bytes }));
  const archive = buildArchive(entries);
  return { sha256: archive.sha256, files: archive.files, bytes: archive.bytes, compressed: compressArchive(archive.tar), entries };
}

/**
 * The entries of an archive's tar, once it is the one `pin` names: its sha256, its file count and bytes, the canonical
 * tar of its own entries, holding only a tree's paths. Anything else throws, and nothing is extracted.
 */
function verifiedTreeEntries(tar: Buffer, pin: ArchivePin, label: string): ArchiveEntry[] {
  const digest = sha256(tar);
  if (digest !== pin.sha256) throw new Error(`${label}: the archive's tar has sha256 ${digest}, not the locked ${pin.sha256}`);
  const entries = readArchive(tar);
  const bytes = entries.reduce((total, entry) => total + entry.bytes.length, 0);
  if (entries.length !== pin.files || bytes !== pin.bytes) throw new Error(`${label}: the archive holds ${entries.length} file(s) of ${bytes} byte(s), but the lock says ${pin.files} of ${pin.bytes}`);
  checkTreePaths(entries.map((entry) => entry.path));
  if (buildArchive(entries).sha256 !== pin.sha256) throw new Error(`${label}: the archive is not the canonical archive of its own files`);
  return entries;
}

/** The entries of a compressed archive, once it is the one `pin` names (verifiedTreeEntries); anything else throws. */
export function unpackArchive(compressed: Buffer, pin: ArchivePin, label: string): ArchiveEntry[] {
  return verifiedTreeEntries(decompressArchive(compressed, archiveSizeBound(pin)), pin, label);
}

/** Where the cache keeps a tool-pack's compressed archive, by its tar's sha256. */
export function archiveCachePath(cacheRoot: string, digest: string): string {
  return join(cacheRoot, "recordings", `${digest}.tar.zst`);
}

/** Where the cache keeps the archive's extracted tree. */
export function treeDirectory(cacheRoot: string, digest: string): string {
  return join(cacheRoot, "recordings", "tree", digest);
}

/**
 * Put a packed tree in the cache: the compressed archive under its sha256, and its extracted tree beside it, through
 * pack-fixtures.mjs's extractRecordingTree (the marker naming the tar, and another process's tree of it accepted).
 */
export function cacheArchive(cacheRoot: string, packed: PackedArchive): void {
  const path = archiveCachePath(cacheRoot, packed.sha256);
  mkdirSync(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  writeFileSync(temporary, packed.compressed);
  renameSync(temporary, path);
  const directory = treeDirectory(cacheRoot, packed.sha256);
  if (!recordingTreeReady(directory, packed)) extractRecordingTree(directory, packed.entries, packed);
}

/**
 * The extracted tree of the archive `pin` names: the cached tree when its marker names that archive and the canonical
 * tar of its files is that archive's (pack-fixtures.mjs's recordingTreeReady), else the cached archive verified and
 * extracted. Undefined when the cache has neither, or its archive's tar is not the locked one (corrupt, cut short or
 * another archive), which is deleted: fetch it (`pack-fixtures.mjs recordings`). A cached archive whose tar is the
 * locked one but whose files the lock does not describe throws, since fetching the same bytes again changes nothing.
 */
export function cachedTree(cacheRoot: string, pin: ArchivePin, label: string): string | undefined {
  const directory = treeDirectory(cacheRoot, pin.sha256);
  if (recordingTreeReady(directory, pin)) return directory;
  const archive = archiveCachePath(cacheRoot, pin.sha256);
  if (!existsSync(archive)) return undefined;
  let tar: Buffer | undefined;
  try {
    tar = decompressArchive(readFileSync(archive), archiveSizeBound(pin));
  } catch {
    tar = undefined;
  }
  if (tar === undefined || sha256(tar) !== pin.sha256) {
    rmSync(archive, { force: true });
    return undefined;
  }
  extractRecordingTree(directory, verifiedTreeEntries(tar, pin, label), pin);
  return directory;
}
