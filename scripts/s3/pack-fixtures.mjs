#!/usr/bin/env node
// Pack test data (CONTRIBUTING.md, "Pack test data"): the files under packs/<id>/upstream/<tool>/ that are a tool's
// test cases and its recorded answers on them (PACK_FIXTURE_PATTERNS). None of it is in git. Each tool-pack's data is
// one content-pinned archive (test-data-archive.mjs: the canonical tar of its files, zstd-compressed) at
// s3://nera-test-fixtures/rerail/archives/<pack>/<tool>/<sha256>.tar.zst (us-east-2, versioned, never overwritten),
// and the tool directory's tracked test-data.lock names it by the tar's sha256, so a commit resolves to exactly one
// archive. Of the data,
//
// - authored: what Rerail wrote for the tool to score, which nothing can regenerate. It is the files a tool's own
//   upstream/<tool>/UPSTREAM.md lists as Rerail's (pack check 6 reads the same list), and everything under
//   upstream/<tool>/authored/ (AUTHORED_CORPUS_PATTERN: a contributor's code corpus with its ground truth, where the
//   corpus is no public repository a pin can name).
// - derived: everything else, upstream's tests copied or derived and the tool's recorded answers. A contributor or a
//   maintainer regenerates it from the tool-pack's pins with `npm run fixtures:derive` (scripts/pack-data/derive.ts).
//
// .gitignore covers the patterns, so a pulled copy is untracked and invisible. New or changed data reaches S3 only
// through the tool-pack's tracked publish/ directory (PUBLISH_DIRECTORY), the publish request: `npm run fixtures:stage`
// copies what differs from the locked archive into it, the pull request commits it, and the publish workflow
// (.github/workflows/pack-data-publish.yml, PUBLISH_WORKFLOW) packs it from the commit's blobs (`pack --from`), uploads
// it after a code owner approves, confirms every object, commits the new locks and deletes publish/. Structure metric
// #3 blocks the merge while a publish/ is tracked (CONTRIBUTING.md, "Pack test data").
//
//   npm run fixtures:pull [-- --pack <id> [--tool <tool>]] [--dry-run]   fetch the locked archives, verify, extract;
//                                                                          a tool-pack's publish/ is laid over them
//   npm run fixtures:stage -- --pack <id> [--tool <tool>] [--all] [--dry-run]
//       copy the pack test data on disk that the locked archive lacks or holds differently into publish/ (--all: all
//       of it, for a `replace` publish)
//   npm run fixtures:push [-- --pack <id> [--tool <tool>]] [--dry-run]   maintainer with write credentials (optional):
//       archive the data on disk, upload the archive unless S3 holds it (tagged), rewrite the lock
//   node scripts/s3/pack-fixtures.mjs pack --from <commit> [--replace] --out <dir> --manifest <file>
//       the publish workflow's prep: archive each tool-pack's publish/ from the commit's blobs alone (never checking it
//       out or running its code), over the locked archive (overlay) or alone (--replace), check every file's licence,
//       and write the archives and their manifest; no AWS write
//   node scripts/s3/pack-fixtures.mjs upload --objects <file> --archives <dir> [--uploaded <file>] <tags>
//       the publish workflow's upload: verify each archive against the objects prep named, then put it create-only, tagged
//   node scripts/s3/pack-fixtures.mjs confirm --objects <file> [--uploaded <file>] [--report <file>]
//       head and download every object again and verify it against its lock
//   node scripts/s3/pack-fixtures.mjs samples [--pack <id> [--tool <tool>]] [--dry-run]
//       put the samples archive each samples.lock names into ARCHIVE_CACHE from S3, verified, and extract it into
//       .rerail/pack-data/samples/<pack>/<tool>/<sha256>/ (a pull does this too)
//   node scripts/s3/pack-fixtures.mjs samples push [--pack <id> [--tool <tool>]]
//       maintainer: put the samples archive each samples.lock names from ARCHIVE_CACHE into S3 unless it holds it, read
//       it back, and print the `git add` for the locks
//   node scripts/s3/pack-fixtures.mjs check [--pack <id> [--tool <tool>]]
//       fail, listing each with its remedy, unless S3 holds every archive the locks name (head-object only)
//
// Every put is tagged (objectTagging): rerail:commit, rerail:pr, rerail:run-id and rerail:uploader. A maintainer's
// command defaults them to HEAD, "none", "local" and the `gh` login; --commit, --pr, --run-id and --uploader set them.
//
// A tool-pack's real-code samples (scripts/samples/format.ts), which its samples.lock names in test-data.lock's format,
// are one canonical archive at s3://nera-test-fixtures/rerail/archives/samples/<pack>/<tool>/<sha256>.tar.zst, written
// once, never overwritten. Nothing derives or records them, so a pull fetches the archive into ARCHIVE_CACHE (and CI's
// cache carries it) and extracts it outside the tool directory as a verified tree (extractTree); the conformance
// runner reads the tree through samplesTree.
//
// The consumers (the conformance runner, pack validation, the generators, the Rust tests) report a
// missing file with missingPackFixture, so a checkout without the data says to pull instead of failing on ENOENT.
// packs/<id>/upstream/<tool>/ keeps the rest in git: gates.yaml, generators, UPSTREAM.md,
// samples.lock, test-data.lock, and, while a pull request publishes data, publish/.

import { spawnSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

import { licenceTerms } from "../check/check-structure/metrics/07-pack-licence-annotations.mjs";
import { archiveSizeBound, buildArchive, compressArchive, decompressArchive, lockText, parseLock, readArchive, sha256, TEST_DATA_LOCK } from "./test-data-archive.mjs";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
export const BUCKET = "nera-test-fixtures";
export const REGION = "us-east-2";
/** Every archive's key is this prefix plus `<pack>/<tool>/<sha256>.tar.zst`. */
export const ARCHIVE_PREFIX = "rerail/archives/";
/** Where a pull keeps the archives it fetched (and CI restores them from its cache), under the same relative keys. */
export const ARCHIVE_CACHE = ".rerail/pack-data/archives";

/**
 * A tool-pack's real-code samples (scripts/samples/format.ts): real files, each labelled with the lines its pack rules
 * should report or as clean for them, in one canonical archive (test-data-archive.mjs) that the tool directory's tracked
 * samples.lock names in test-data.lock's format. Nothing derives them, so they live outside the tool directory.
 */
export const SAMPLES_LOCK = "samples.lock";
/** Every samples archive's key is this prefix plus `<pack>/<tool>/<sha256>.tar.zst`, by its tar's sha256. */
export const SAMPLES_PREFIX = `${ARCHIVE_PREFIX}samples/`;
/** Where a samples archive is extracted: `<pack>/<tool>/<sha256>/` under it, the tree the conformance runner reads. */
const SAMPLES_TREES = ".rerail/pack-data/samples";
/** The samples archive's one file outside a sample (LICENSES_FILE in scripts/samples/format.ts). */
const SAMPLES_LICENSES = "LICENSES.md";
/**
 * Every other path is a sample's file, `<sample-id>/<path>`, with a sample id as SAMPLE_ID in scripts/samples/format.ts
 * has it (this script imports no TypeScript, so the pattern is repeated here).
 */
const SAMPLE_PATH = /^[a-z0-9][a-z0-9._-]*-[0-9a-f]{12}\/[^/]/u;

/**
 * Where a contributor's code corpus and its ground truth go when the corpus is no public repository a pin can name:
 * authored pack test data by its place, which its tool's UPSTREAM.md need not list file by file.
 */
export const AUTHORED_CORPUS_PATTERN = "packs/*/upstream/*/authored/**";

/** Pack test data, as repository-relative globs (`*` stays within a path segment, `**` crosses them). */
export const PACK_FIXTURE_PATTERNS = Object.freeze([
  "packs/*/upstream/*/fixtures/**",
  "packs/*/upstream/*/tests/**",
  "packs/*/upstream/*/*.expected.json",
  "packs/*/upstream/*/cases.json",
  "packs/*/upstream/*/*.cases.json",
  "packs/*/upstream/*/fixtures.upstream-check.json",
  "packs/*/upstream/*/lizard-*.tsv",
  "packs/*/upstream/*/provenance.json",
  "packs/*/upstream/*/LICENSES.md",
  AUTHORED_CORPUS_PATTERN,
]);

/** What every consumer tells a contributor whose checkout lacks the data. */
export const PULL_INSTRUCTION =
  'pack test data is not in git: run `npm run fixtures:pull` (each tool-pack\'s archive, which its test-data.lock pins, from S3) (CONTRIBUTING.md, "Pack test data")';

/**
 * A tool directory's publish request: `packs/<pack>/upstream/<tool>/publish/`, tracked, never ignored. `publish/<path>`
 * becomes the test data archive's entry `<path>` (a path PACK_FIXTURE_PATTERNS matches in the tool directory), and
 * `publish/samples/<path>` the samples archive's entry `<path>`; anything else there is refused. No pattern of
 * PACK_FIXTURE_PATTERNS matches a publish/ path, so pulled data and a publish request never mix.
 */
export const PUBLISH_DIRECTORY = "publish";
/** Under publish/, the samples archive's entries (scripts/samples/pack.ts writes them there). */
const SAMPLES_IN_PUBLISH = "samples";
/** A tracked publish/ file's repository path: its pack, tool, and path under publish/. */
const PUBLISH_PATH = /^packs\/([^/]+)\/upstream\/([^/]+)\/publish\/(.+)$/u;
/** The workflow that publishes a pull request's publish/ (CONTRIBUTING.md, "Pack test data"). */
export const PUBLISH_WORKFLOW = "https://github.com/neralabs/rerail/actions/workflows/pack-data-publish.yml";
/** What a contributor does to publish a tool-pack's new or changed data. */
const publishInstruction = ({ pack, tool }) =>
  `stage it with \`npm run fixtures:stage -- --pack ${pack} --tool ${tool}\`, commit packs/${pack}/upstream/${tool}/${PUBLISH_DIRECTORY}/ in the pull request, and run the publish workflow (${PUBLISH_WORKFLOW}) for it (CONTRIBUTING.md, "Pack test data")`;

/** `{ pack, tool, directory, path }` for a repository path under a tool directory's publish/, or undefined. */
export function publishPath(path) {
  const match = PUBLISH_PATH.exec(path);
  return match === null ? undefined : { pack: match[1], tool: match[2], directory: `packs/${match[1]}/upstream/${match[2]}`, path: match[3] };
}

const escapeRegExp = (text) => text.replace(/[.+^${}()|[\]\\?]/gu, "\\$&");

function globToRegExp(glob) {
  const source = glob
    .split("**")
    .map((part) => part.split("*").map(escapeRegExp).join("[^/]*"))
    .join(".*");
  return new RegExp(`^${source}$`, "u");
}

const MATCHERS = PACK_FIXTURE_PATTERNS.map(globToRegExp);
const AUTHORED_CORPUS = globToRegExp(AUTHORED_CORPUS_PATTERN);
/** A row of an UPSTREAM.md file table: its first cell names a pack-relative path (conformance/pack-validation.ts too). */
const UPSTREAM_ROW = /^\|\s*`([^`]+)`/u;

/** A repository-relative path (either separator) to its POSIX form; an absolute path is taken relative to `root`. */
function repositoryPath(path, root = ROOT) {
  const relativePath = isAbsolute(path) ? relative(root, path) : path;
  return relativePath.split(sep).join("/");
}

/** Whether a file's repository-relative path (or its absolute path inside the checkout) is pack test data. */
export function isPackFixture(path) {
  const posix = repositoryPath(path);
  return MATCHERS.some((matcher) => matcher.test(posix));
}

/**
 * The authored files a tool's own UPSTREAM.md lists, as repository paths: `record` is its repository path
 * (packs/<id>/upstream/<tool>/UPSTREAM.md) and `text` its content. Only rows under the tool's own directory count, as
 * pack check 6 reads them.
 */
export function listedAuthored(record, text) {
  const [, pack, , tool] = repositoryPath(record).split("/");
  return text
    .split("\n")
    .map((line) => UPSTREAM_ROW.exec(line)?.[1])
    .filter((file) => file !== undefined && file.startsWith(`upstream/${tool}/`))
    .map((file) => `packs/${pack}/${file}`);
}

/** The repository paths of every tool's UPSTREAM.md in the checkout at `root` (packs/<id>/upstream/<tool>/UPSTREAM.md), sorted. */
export function toolRecords(root = ROOT) {
  const records = [];
  const packs = join(root, "packs");
  if (!existsSync(packs)) return records;
  for (const pack of readdirSync(packs, { withFileTypes: true }).filter((entry) => entry.isDirectory())) {
    const upstream = join(packs, pack.name, "upstream");
    if (!existsSync(upstream)) continue;
    for (const tool of readdirSync(upstream, { withFileTypes: true }).filter((entry) => entry.isDirectory())) {
      if (existsSync(join(upstream, tool.name, "UPSTREAM.md"))) records.push(`packs/${pack.name}/upstream/${tool.name}/UPSTREAM.md`);
    }
  }
  return records.sort();
}

/** Every file the tools' UPSTREAM.md list as Rerail's in the checkout at `root`, as repository paths, whether present or not. */
export function authoredListing(root = ROOT) {
  return new Set(toolRecords(root).flatMap((record) => listedAuthored(record, readFileSync(join(root, record), "utf8"))));
}

/**
 * What kind of pack test data a path is: "authored" (its tool's UPSTREAM.md lists it as Rerail's, or it is under
 * upstream/<tool>/authored/), "derived" (any other), or undefined when it is no pack test data. `authored` is
 * authoredListing's set.
 */
export function packFixtureKind(path, authored = authoredListing()) {
  const posix = repositoryPath(path);
  if (!isPackFixture(posix)) return undefined;
  return AUTHORED_CORPUS.test(posix) || authored.has(posix) ? "authored" : "derived";
}

/** Whether `path` is pack test data or a directory holding only pack test data (`…/fixtures`). */
function coversPackFixtures(path) {
  return isPackFixture(path) || isPackFixture(`${repositoryPath(path)}/-`);
}

/** The message for pack test data that is not in the checkout: a file (or directory) path, absolute or relative. */
export function missingPackFixture(path) {
  return `${repositoryPath(path)} is missing: ${PULL_INSTRUCTION}`;
}

/**
 * missingPackFixture's message when `path` is pack test data and its tool directory (packs/<id>/upstream/<tool>/) holds
 * none at all, i.e. the checkout was never pulled; otherwise undefined, and a missing file is the consumer's own finding.
 */
export function packFixtureAbsence(path) {
  if (!coversPackFixtures(path)) return undefined;
  const posix = repositoryPath(path);
  const toolDirectory = join(ROOT, ...posix.split("/").slice(0, 4));
  const holds = (name) => {
    const entry = join(toolDirectory, name);
    return coversPackFixtures(entry) && (!statSync(entry).isDirectory() || readdirSync(entry).length > 0);
  };
  const held = existsSync(toolDirectory) && readdirSync(toolDirectory).some(holds);
  return held ? undefined : missingPackFixture(path);
}

/** `path` when it exists; otherwise throws, saying to pull when the checkout has none of its tool's pack test data. */
export function requirePackFixture(path) {
  if (!existsSync(path)) throw new Error(packFixtureAbsence(path) ?? `${repositoryPath(path)} does not exist`);
  return path;
}

/** Every file under `path` (itself when it is a file), as absolute paths. */
function filesAt(path) {
  if (!existsSync(path)) return [];
  if (!statSync(path).isDirectory()) return [path];
  return readdirSync(path, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => join(entry.parentPath, entry.name));
}

const shellQuoted = (path) => (/^[\w./@+-]+$/u.test(path) ? path : `'${path.replaceAll("'", "'\\''")}'`);

/**
 * What a generator says after writing pack test data at `paths` (files or directories, in the checkout at
 * `root`): git ignores it, so new or changed data reaches S3 only staged into its tool-pack's publish/ and published.
 */
export function packFixturesWritten(paths, root = ROOT) {
  const written = [...new Set(paths.flatMap((path) => filesAt(isAbsolute(path) ? path : join(root, path))).map((path) => repositoryPath(path, root)))]
    .filter(isPackFixture)
    .sort();
  const byToolPack = new Map();
  for (const path of written) {
    const [, pack, , tool] = path.split("/");
    const directory = `packs/${pack}/upstream/${tool}`;
    byToolPack.set(directory, { pack, tool, count: (byToolPack.get(directory)?.count ?? 0) + 1 });
  }
  return [...byToolPack]
    .map(([directory, { pack, tool, count }]) => `Wrote ${count} pack test data file(s) under ${directory}/, which git ignores (the tool-pack's data is an archive in S3). To publish new or changed data, ${publishInstruction({ pack, tool })}.`)
    .join("\n");
}

/** packFixturesWritten for expected findings at `path`: fixtures expected findings name the case sources beside them too. */
export function expectedFindingsWritten(path, kind) {
  const cases = kind === "fixtures" ? ["fixtures", "tests"].map((name) => join(dirname(path), name)) : [];
  return packFixturesWritten([path, ...cases.filter((directory) => existsSync(directory))]);
}

/** Runs `aws` with `args`; ENOENT when it is not installed. */
function runAws(args, options = {}) {
  return spawnSync("aws", args, { encoding: "utf8", maxBuffer: 1 << 28, ...options });
}

function runGit(root, args, input) {
  const run = spawnSync("git", ["--literal-pathspecs", ...args], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28, ...(input === undefined ? {} : { input }) });
  if (run.error !== undefined) throw run.error;
  if (run.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${run.stderr.trim()}`);
  return run.stdout;
}

const nulList = (text) => text.split("\0").filter((path) => path !== "");
const lastLine = (text) => (text ?? "").trim().split("\n").at(-1) ?? "";
const byBytes = (a, b) => Buffer.compare(Buffer.from(a, "utf8"), Buffer.from(b, "utf8"));

/** Fails with an instruction unless the aws CLI is installed and signed in. */
export function requireAws(aws = runAws) {
  const run = aws(["sts", "get-caller-identity", "--output", "json"]);
  if (run.error?.code === "ENOENT") {
    throw new Error(`the AWS CLI (aws) is not installed. Pack test data lives in s3://${BUCKET}/${ARCHIVE_PREFIX}: install AWS CLI v2 (https://aws.amazon.com/cli/), then sign in with credentials that can read it (CONTRIBUTING.md, "Pack test data").`);
  }
  if (run.error !== undefined) throw run.error;
  if (run.status !== 0) {
    throw new Error(`aws is not signed in (aws sts get-caller-identity: ${lastLine(run.stderr) || `exit ${run.status}`}). Sign in (aws login, aws sso login, or AWS_PROFILE) with credentials that can read s3://${BUCKET}/${ARCHIVE_PREFIX}; ask a maintainer for access (CONTRIBUTING.md, "Pack test data").`);
  }
}

/** The tool-packs of the checkout at `root`: every directory packs/<pack>/upstream/<tool>/, sorted, as `{ pack, tool, directory }`. */
export function toolPacks(root = ROOT) {
  const found = [];
  const packs = join(root, "packs");
  if (!existsSync(packs)) return found;
  for (const pack of readdirSync(packs, { withFileTypes: true }).filter((entry) => entry.isDirectory())) {
    const upstream = join(packs, pack.name, "upstream");
    if (!existsSync(upstream)) continue;
    for (const tool of readdirSync(upstream, { withFileTypes: true }).filter((entry) => entry.isDirectory())) {
      found.push({ pack: pack.name, tool: tool.name, directory: `packs/${pack.name}/upstream/${tool.name}` });
    }
  }
  return found.sort((a, b) => byBytes(a.directory, b.directory));
}

/** toolPacks narrowed to `pack` (and `tool`); throws when nothing matches. */
export function selectToolPacks(root = ROOT, pack = undefined, tool = undefined) {
  if (tool !== undefined && pack === undefined) throw new Error("--tool needs --pack");
  const chosen = toolPacks(root).filter((entry) => (pack === undefined || entry.pack === pack) && (tool === undefined || entry.tool === tool));
  if (chosen.length === 0) throw new Error(`no tool-pack packs/${pack ?? "*"}/upstream/${tool ?? "*"}/ in the checkout`);
  return chosen;
}

/**
 * Throws unless each directory from the checkout at `root` down to the repository path `directory` (each segment's
 * prefix, `directory` itself included) is a directory or absent: a write of pack test data through a symlink there would
 * land outside the tool directory. The message names the first one that is not.
 */
function refuseSymlinkedDirectories(root, directory) {
  const segments = directory.split("/");
  for (let length = 1; length <= segments.length; length += 1) {
    const path = segments.slice(0, length).join("/");
    const stat = lstatSync(join(root, path), { throwIfNoEntry: false });
    if (stat === undefined) return;
    if (stat.isSymbolicLink()) throw new Error(`${path} is a symlink: pack test data is never read or written through one, so nothing was written; make it a directory`);
    if (!stat.isDirectory()) throw new Error(`${path} is not a directory, so pack test data cannot be written under it; nothing was written`);
  }
}

/**
 * The tool directory's pack test data on disk, as paths relative to it, sorted by their UTF-8 bytes. The walk follows no
 * symlink. It throws, naming it, on a symlink that is pack test data or a directory that may hold it (`fixtures/`,
 * `authored/…`), and on a symlink from the checkout down to the tool directory, rather than leave its data out or read
 * through it.
 */
export function toolPackFiles(root, directory) {
  refuseSymlinkedDirectories(root, directory);
  if (!existsSync(join(root, directory))) return [];
  const found = [];
  const pending = [""];
  while (pending.length > 0) {
    const at = pending.pop();
    for (const entry of readdirSync(join(root, directory, at), { withFileTypes: true })) {
      const path = at === "" ? entry.name : `${at}/${entry.name}`;
      if (entry.isSymbolicLink()) {
        if (coversPackFixtures(`${directory}/${path}`)) throw new Error(`${directory}/${path} is a symlink: pack test data is never read or written through one; replace it with the files it points to`);
      } else if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile() && isPackFixture(`${directory}/${path}`)) found.push(path);
    }
  }
  return found.sort(byBytes);
}

/**
 * The files of the tool directory's publish/ on disk (a publish request), as paths relative to publish/, sorted by their
 * UTF-8 bytes; none when it has no publish/. Nothing in it is read through a symlink: a symlink or anything but a
 * regular file or directory there throws, naming it.
 */
export function publishFiles(root, directory) {
  const publish = `${directory}/${PUBLISH_DIRECTORY}`;
  refuseSymlinkedDirectories(root, publish);
  if (!existsSync(join(root, publish))) return [];
  const found = [];
  const pending = [""];
  while (pending.length > 0) {
    const at = pending.pop();
    for (const entry of readdirSync(join(root, publish, at), { withFileTypes: true })) {
      const path = at === "" ? entry.name : `${at}/${entry.name}`;
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile()) found.push(path);
      else throw new Error(`${publish}/${path} is not a regular file: a publish request holds files alone; replace it with the file it stands for`);
    }
  }
  return found.sort(byBytes);
}

/** Whether `path` (relative to publish/) is a samples entry, `samples/<path>`. */
const isPublishedSample = (path) => path.startsWith(`${SAMPLES_IN_PUBLISH}/`);

/** A tool-pack's archive's S3 key, for the tar sha256 `digest`. */
export function archiveKey({ pack, tool }, digest) {
  return `${ARCHIVE_PREFIX}${pack}/${tool}/${digest}.tar.zst`;
}

/** Where a pull keeps that archive in the checkout at `root` (ARCHIVE_CACHE plus the key's own `<pack>/<tool>/<sha256>.tar.zst`). */
export function archiveCachePath(root, toolPack, digest) {
  return join(root, ARCHIVE_CACHE, archiveKey(toolPack, digest).slice(ARCHIVE_PREFIX.length));
}

/** A tool-pack's lock: its repository path (`label`) and fields; throws when it is missing or malformed. */
export function readLock(root, toolPack) {
  const label = `${toolPack.directory}/${TEST_DATA_LOCK}`;
  if (!existsSync(join(root, label))) {
    throw new Error(`${label} is missing: to publish the tool-pack's test data, ${publishInstruction(toolPack)}`);
  }
  return { label, ...parseLock(readFileSync(join(root, label), "utf8"), label) };
}

const SHA256_HEX = /^[0-9a-f]{64}$/u;
const isSha256 = (value) => typeof value === "string" && SHA256_HEX.test(value);
const isCount = (value) => Number.isSafeInteger(value) && value >= 0;

/**
 * The entries of the compressed archive at `key` once it is the one `lock` (`{ label, sha256, files, bytes }`) names;
 * throws, naming the lock and key, otherwise. Beyond the sha256, file count and bytes, every entry's path must satisfy
 * `belongs` (`kind` says what such a path is), and the tar must be the canonical one of its own entries (no duplicate,
 * unsorted or extra record); decompression stops at the size the lock allows.
 */
function verifiedArchive(compressed, lock, key, belongs, kind) {
  const object = `s3://${BUCKET}/${key}`;
  let tar;
  try {
    tar = decompressArchive(compressed, archiveSizeBound(lock));
  } catch (error) {
    throw new Error(`${lock.label}: ${object} is not a zstd archive of the size the lock allows (${error instanceof Error ? error.message : String(error)}): the download or the object is corrupt; nothing was extracted`);
  }
  const digest = sha256(tar);
  if (digest !== lock.sha256) {
    throw new Error(`${lock.label}: ${object} holds a tar whose sha256 is ${digest}, not the locked ${lock.sha256}: the download or the object is corrupt, or the lock was edited by hand; nothing was extracted`);
  }
  let entries;
  try {
    entries = readArchive(tar);
  } catch (error) {
    throw new Error(`${lock.label}: ${object} is not an archive of ${kind} (${error instanceof Error ? error.message : String(error)}); nothing was extracted`);
  }
  const bytes = entries.reduce((total, entry) => total + entry.bytes.length, 0);
  if (entries.length !== lock.files || bytes !== lock.bytes) {
    throw new Error(`${lock.label}: ${object} holds ${entries.length} file(s) of ${bytes} byte(s), but the lock says ${lock.files} of ${lock.bytes}: the lock was edited by hand; nothing was extracted`);
  }
  const foreign = entries.filter((entry) => !belongs(entry.path)).map((entry) => entry.path);
  if (foreign.length > 0) {
    throw new Error(`${lock.label}: ${object} holds ${foreign.length} file(s) that are not ${kind}, e.g. ${foreign.slice(0, 3).join(", ")}; nothing was extracted`);
  }
  let canonical;
  try {
    canonical = buildArchive(entries).sha256;
  } catch (error) {
    canonical = error instanceof Error ? error.message : String(error);
  }
  if (canonical !== lock.sha256) {
    throw new Error(`${lock.label}: ${object} is not the canonical archive of its own files (${canonical}); nothing was extracted`);
  }
  return entries;
}

/**
 * verifiedArchive for the test data archive `lock` names for the tool directory `directory`: every entry must be pack
 * test data of that directory, so an archive can never put a generator, gates.yaml or a lock in place.
 */
export function verifiedEntries(compressed, lock, key, directory) {
  return verifiedArchive(compressed, lock, key, (path) => isPackFixture(`${directory}/${path}`), `pack test data of ${directory}`);
}

/**
 * The file an extracted tree holds besides the archive's: the tar's sha256 and a newline, written into the partial
 * directory last, so a tree that has it was extracted whole from a verified copy of that archive.
 */
export const TREE_MARKER = ".tar-sha256";

/**
 * Whether `directory` is the extracted tree of the archive `lock` (`{ sha256, files, bytes }`) names: its marker names
 * that tar, and the canonical tar of the files it holds besides the marker (buildArchive) is that tar, so no file was
 * added, deleted, renamed, cut short or edited since. Anything but regular files and directories is no such tree.
 */
export function treeReady(directory, lock) {
  const marker = join(directory, TREE_MARKER);
  if (!existsSync(marker) || readFileSync(marker, "utf8") !== `${lock.sha256}\n`) return false;
  const entries = [];
  let bytes = 0;
  for (const entry of readdirSync(directory, { recursive: true, withFileTypes: true })) {
    if (entry.isDirectory() || (entry.parentPath === directory && entry.name === TREE_MARKER)) continue;
    if (!entry.isFile()) return false;
    const absolute = join(entry.parentPath, entry.name);
    // The sizes first: a tree with other totals is no copy, and is not read.
    bytes += statSync(absolute).size;
    entries.push({ path: relative(directory, absolute).split(sep).join("/"), absolute });
  }
  if (entries.length !== lock.files || bytes !== lock.bytes) return false;
  try {
    return buildArchive(entries.map(({ path, absolute }) => ({ path, bytes: readFileSync(absolute) }))).sha256 === lock.sha256;
  } catch {
    // A path no archive can hold (buildArchive refuses it) is no extraction of one.
    return false;
  }
}

/**
 * Writes verified `entries` of the archive `lock` names, and the marker, into this process's own partial directory,
 * then renames it into place as `directory`, so a tree is never half there. Another process that extracted the same
 * archive first wins: its tree is this one. A tree there that is not that archive's is replaced.
 */
export function extractTree(directory, entries, lock) {
  const partial = `${directory}.${process.pid}.partial`;
  rmSync(partial, { recursive: true, force: true });
  try {
    mkdirSync(partial, { recursive: true });
    for (const entry of entries) {
      const absolute = join(partial, entry.path);
      mkdirSync(dirname(absolute), { recursive: true });
      writeFileSync(absolute, entry.bytes);
    }
    writeFileSync(join(partial, TREE_MARKER), `${lock.sha256}\n`);
    for (let attempt = 1; ; attempt += 1) {
      try {
        renameSync(partial, directory);
        return;
      } catch (error) {
        if (treeReady(directory, lock)) return;
        if (attempt > 1) throw error;
        rmSync(directory, { recursive: true, force: true });
      }
    }
  } finally {
    rmSync(partial, { recursive: true, force: true });
  }
}

const NOT_FOUND = /\(404\)|Not Found|NoSuchKey|does not exist/u;

/** Downloads `key` to `destination`; false when S3 does not hold it. */
function download(aws, key, destination) {
  const run = aws(["s3", "cp", `s3://${BUCKET}/${key}`, destination, "--region", REGION, "--only-show-errors"]);
  if (run.error !== undefined) throw run.error;
  if (run.status === 0) return true;
  if (NOT_FOUND.test(run.stderr ?? "")) return false;
  throw new Error(`aws s3 cp s3://${BUCKET}/${key} failed: ${lastLine(run.stderr) || `exit ${run.status}`}`);
}

/** S3's head of `key` (head-object's JSON: `ContentLength` and the rest), or undefined when S3 does not hold it. */
function objectHead(aws, key) {
  const run = aws(["s3api", "head-object", "--bucket", BUCKET, "--key", key, "--region", REGION, "--output", "json"]);
  if (run.error !== undefined) throw run.error;
  if (run.status === 0) {
    try {
      return JSON.parse(run.stdout || "{}");
    } catch {
      throw new Error(`aws s3api head-object ${key} printed no JSON: ${lastLine(run.stdout)}`);
    }
  }
  if (NOT_FOUND.test(run.stderr ?? "")) return undefined;
  throw new Error(`aws s3api head-object ${key} failed: ${lastLine(run.stderr) || `exit ${run.status}`}`);
}

/** Whether S3 holds `key`. */
function objectExists(aws, key) {
  return objectHead(aws, key) !== undefined;
}

/** An S3 tag value: the characters S3 allows in one, at most 256 of them. */
const TAG_VALUE = /^[\p{L}\p{N} _.:/=+\-@]{1,256}$/u;

/**
 * The `--tagging` of every put: `rerail:commit` (the commit the data was packed from), `rerail:pr` (its pull request, or
 * "none"), `rerail:run-id` (the publish workflow's run, or "local") and `rerail:uploader` (the GitHub login of the code
 * owner who approved it, or of the maintainer who pushed it), URL-encoded. Throws on a missing or invalid value.
 */
export function objectTagging({ commit, pr, runId, uploader } = {}) {
  const tags = [["rerail:commit", commit], ["rerail:pr", pr], ["rerail:run-id", runId], ["rerail:uploader", uploader]];
  const invalid = tags.filter(([, value]) => typeof value !== "string" || !TAG_VALUE.test(value)).map(([name, value]) => `${name} ${JSON.stringify(value)}`);
  if (invalid.length > 0) throw new Error(`every upload is tagged, and these tags are missing or not S3 tag values: ${invalid.join(", ")} (--commit, --pr, --run-id, --uploader)`);
  return tags.map(([name, value]) => `${encodeURIComponent(name)}=${encodeURIComponent(value)}`).join("&");
}

/** Uploads `file` to `key`, tagged `tagging`, unless S3 holds the key already (a conditional write: an object is never overwritten). */
function putObject(aws, key, file, tagging) {
  const run = aws(["s3api", "put-object", "--bucket", BUCKET, "--key", key, "--body", file, "--if-none-match", "*", "--tagging", tagging, "--region", REGION, "--output", "json"]);
  if (run.error !== undefined) throw run.error;
  if (run.status === 0) return true;
  if (/PreconditionFailed|\(412\)/u.test(run.stderr ?? "")) return false;
  throw new Error(`aws s3api put-object ${key} failed: ${lastLine(run.stderr) || `exit ${run.status}`}`);
}

/**
 * The entries of the compressed archive at `cached` that `lock` names, as `verify` returns them: `entries` is undefined
 * when there is no such file, and `discarded` names a cached copy whose tar is not the locked one, which is deleted
 * (left in place on a dry run, which deletes nothing). A cached copy whose tar is the locked one decides: a lock whose
 * file or byte count disagrees with it was edited, and fetching the same bytes again (with credentials a cache hit in
 * CI does not have) changes nothing, so `verify` throws.
 */
function cachedEntries(root, cached, lock, verify, dryRun) {
  if (!existsSync(cached)) return { entries: undefined, discarded: undefined };
  const compressed = readFileSync(cached);
  let digest;
  try {
    digest = sha256(decompressArchive(compressed, archiveSizeBound(lock)));
  } catch {
    digest = undefined;
  }
  if (digest === lock.sha256) return { entries: verify(compressed), discarded: undefined };
  if (!dryRun) rmSync(cached, { force: true });
  return { entries: undefined, discarded: relative(root, cached) };
}

/** cachedEntries for the test data archive `lock` names, from ARCHIVE_CACHE. */
function cachedArchive(root, toolPack, lock, dryRun) {
  return cachedEntries(root, archiveCachePath(root, toolPack, lock.sha256), lock, (compressed) => verifiedEntries(compressed, lock, archiveKey(toolPack, lock.sha256), toolPack.directory), dryRun);
}

/** Downloads the archive `lock` names into ARCHIVE_CACHE through a partial file that must verify first; its entries. */
function downloadArchive(root, aws, toolPack, lock) {
  const key = archiveKey(toolPack, lock.sha256);
  const cached = archiveCachePath(root, toolPack, lock.sha256);
  mkdirSync(dirname(cached), { recursive: true });
  const partial = `${cached}.partial`;
  rmSync(partial, { force: true });
  try {
    if (!download(aws, key, partial)) {
      throw new Error(`${lock.label} names s3://${BUCKET}/${key}, which S3 does not hold: publish the data the lock was written for through the publish workflow (${PUBLISH_WORKFLOW}), or a maintainer with write credentials uploads it with \`npm run fixtures:push -- --pack ${toolPack.pack} --tool ${toolPack.tool}\` (CONTRIBUTING.md, "Pack test data")`);
    }
    const entries = verifiedEntries(readFileSync(partial), lock, key, toolPack.directory);
    renameSync(partial, cached);
    return entries;
  } finally {
    rmSync(partial, { force: true });
  }
}

/** Writes a buildArchive result, compressed, into ARCHIVE_CACHE under its sha256, through a partial file. */
function cacheArchive(root, toolPack, archive) {
  const cached = archiveCachePath(root, toolPack, archive.sha256);
  mkdirSync(dirname(cached), { recursive: true });
  writeFileSync(`${cached}.partial`, compressArchive(archive.tar));
  renameSync(`${cached}.partial`, cached);
}

/**
 * The once-only sign-in of a command that may fetch: requireAws on the first call. CI restores archives from its cache
 * with no credentials, so when a cached copy that is not the locked one was deleted (`discarded`, filled as the command
 * goes), a failed sign-in says the cache entry is corrupt or poisoned, which the next run would restore again. `what`
 * names the kind of archive.
 */
function signer(aws, discarded, what) {
  let signedIn = false;
  return () => {
    if (signedIn) return;
    try {
      requireAws(aws);
    } catch (error) {
      if (discarded.length === 0) throw error;
      throw new Error(`${discarded.join(", ")} was not the ${what} its lock names (a corrupt or poisoned cache entry) and was deleted, and S3 cannot be read here: ${error instanceof Error ? error.message : String(error)}. In CI, delete the Actions cache entry that restored it (gh cache delete <key>) and run the job again.`);
    }
    signedIn = true;
  };
}

/** A tool-pack's samples archive's S3 key, for its tar's sha256 `digest`. */
export function samplesKey({ pack, tool }, digest) {
  return `${SAMPLES_PREFIX}${pack}/${tool}/${digest}.tar.zst`;
}

/** Where the checkout at `root` keeps that archive (ARCHIVE_CACHE plus the key's own `samples/<pack>/<tool>/<sha256>.tar.zst`). */
export function samplesArchiveCachePath(root, toolPack, digest) {
  return join(root, ARCHIVE_CACHE, samplesKey(toolPack, digest).slice(ARCHIVE_PREFIX.length));
}

/** Where the checkout at `root` keeps that archive extracted, the tree the conformance runner reads (`<pack>/<tool>/<sha256>/` under SAMPLES_TREES). */
export function samplesTreeDirectory(root, { pack, tool }, digest) {
  return join(root, SAMPLES_TREES, pack, tool, digest);
}

/** `{ pack, tool, directory }` for a caller's pack and tool; throws on one that is no single path segment, which would name a lock elsewhere. */
function namedToolPack(pack, tool) {
  for (const [option, value] of [["pack", pack], ["tool", tool]]) {
    if (typeof value !== "string" || value === "" || value === "." || value === ".." || /[/\\]/u.test(value)) throw new Error(`${option} ${JSON.stringify(value)} is not a ${option} id`);
  }
  return { pack, tool, directory: `packs/${pack}/upstream/${tool}` };
}

/** A tool-pack's samples.lock in the checkout at `root`, as `{ label, sha256, files, bytes }`; undefined when it has none, and throws when it is malformed. */
export function readSamplesLock(root, { pack, tool }) {
  const label = `${namedToolPack(pack, tool).directory}/${SAMPLES_LOCK}`;
  if (!existsSync(join(root, label))) return undefined;
  return { label, ...parseLock(readFileSync(join(root, label), "utf8"), label) };
}

/**
 * verifiedArchive for the samples archive `lock` names: every entry must be the set's LICENSES.md or a sample's file
 * (`<sample-id>/<path>`), so an archive can put nothing beside the samples, not even the tree's marker.
 */
export function verifiedSamples(compressed, lock, key) {
  return verifiedArchive(compressed, lock, key, (path) => path === SAMPLES_LICENSES || SAMPLE_PATH.test(path), `real-code samples (${SAMPLES_LICENSES} or <sample-id>/<path>)`);
}

/**
 * How a sample set is made or edited: a maintainer's script loads the samples, changes them (labels, rule keys, files)
 * and stages the set with writeSampleSet (scripts/samples/pack.ts) into the tool-pack's publish/samples/, which the pull
 * request commits and the publish workflow archives, uploads and locks.
 */
const REPACK_SAMPLES = `stage the set with writeSampleSet (scripts/samples/pack.ts) into the tool-pack's ${PUBLISH_DIRECTORY}/${SAMPLES_IN_PUBLISH}/ and run the publish workflow (${PUBLISH_WORKFLOW}) for the pull request`;

/**
 * The verified entries of the samples archive `lock` names, as `{ entries, source }`: a cached copy in ARCHIVE_CACHE
 * whose tar is the locked one decides ("cache"); one that is not is deleted and named in `discarded` (left in place on
 * a dry run); otherwise it is fetched from S3 ("S3") through this process's partial file, which must verify before it
 * is kept. A dry run that would fetch logs so and returns no entries. An archive S3 does not hold fails: nothing in CI
 * makes samples.
 */
function samplesArchive(root, toolPack, lock, { dryRun, aws, signIn, discarded, log }) {
  const key = samplesKey(toolPack, lock.sha256);
  const cached = samplesArchiveCachePath(root, toolPack, lock.sha256);
  const fromCache = cachedEntries(root, cached, lock, (compressed) => verifiedSamples(compressed, lock, key), dryRun);
  if (fromCache.discarded !== undefined) discarded.push(fromCache.discarded);
  if (fromCache.entries !== undefined) return { entries: fromCache.entries, source: "cache" };
  if (dryRun) {
    log(`${fromCache.discarded === undefined ? "" : `would delete ${fromCache.discarded}, which is not the archive ${lock.label} names, and `}would fetch s3://${BUCKET}/${key} (${lock.label})`);
    return { entries: undefined, source: "S3" };
  }
  signIn();
  mkdirSync(dirname(cached), { recursive: true });
  const partial = `${cached}.${process.pid}.partial`;
  rmSync(partial, { force: true });
  try {
    if (!download(aws, key, partial)) {
      throw new Error(`${lock.label} names s3://${BUCKET}/${key}, which S3 does not hold: ${REPACK_SAMPLES}`);
    }
    const entries = verifiedSamples(readFileSync(partial), lock, key);
    renameSync(partial, cached);
    return { entries, source: "S3" };
  } finally {
    rmSync(partial, { force: true });
  }
}

/** samplesArchive for each of `selected` (tool-packs) that has a samples.lock, as `{ toolPack, lock, entries, source }`. */
function fetchSamples(root, selected, options) {
  return selected.flatMap((toolPack) => {
    const lock = readSamplesLock(root, toolPack);
    return lock === undefined ? [] : [{ toolPack, lock, ...samplesArchive(root, toolPack, lock, options) }];
  });
}

/**
 * Extracts each of `fetched` (fetchSamples's, verified) into its tree (samplesTreeDirectory) with extractTree, unless
 * the tree there is already that archive's (treeReady); the trees it wrote, as repository
 * paths. A dry run writes nothing.
 */
function extractSamples(root, fetched, { dryRun, log }) {
  const extracted = [];
  for (const { toolPack, lock, entries, source } of fetched) {
    const tree = samplesTreeDirectory(root, toolPack, lock.sha256);
    const shown = relative(root, tree).split(sep).join("/");
    if (treeReady(tree, lock)) {
      log(`${toolPack.directory}: samples archive ${lock.sha256.slice(0, 12)} (${source}) is extracted in ${shown}`);
      continue;
    }
    if (dryRun) {
      log(`would extract samples archive ${lock.sha256.slice(0, 12)} (${lock.label}) into ${shown}`);
      continue;
    }
    extractTree(tree, entries, lock);
    extracted.push(shown);
    log(`${toolPack.directory}: samples archive ${lock.sha256.slice(0, 12)} (${source}) extracted into ${shown}`);
  }
  return extracted.sort(byBytes);
}

/** A tool-pack's staged samples directory: `publish/samples/` in its tool directory (scripts/samples/pack.ts writes it). */
export const PUBLISH_SAMPLES = "publish/samples";
/** The licensing file of the staged copy only; it is never part of a samples archive. */
const PUBLISH_SAMPLES_REUSE = "REUSE.toml";

/** The files staged under the tool-pack's `publish/samples/`, as archive entries (REUSE.toml left out); [] when none. */
function stagedSamples(root, toolPack) {
  const directory = join(root, toolPack.directory, PUBLISH_SAMPLES);
  if (!existsSync(directory)) return [];
  const entries = [];
  for (const entry of readdirSync(directory, { recursive: true, withFileTypes: true })) {
    if (entry.isDirectory()) continue;
    const absolute = join(entry.parentPath, entry.name);
    const path = relative(directory, absolute).split(sep).join("/");
    if (!entry.isFile()) throw new Error(`${toolPack.directory}/${PUBLISH_SAMPLES}/${path} is not a regular file`);
    if (path !== PUBLISH_SAMPLES_REUSE) entries.push({ path, bytes: readFileSync(absolute) });
  }
  return entries;
}

/**
 * The samples tree of the tool-pack `pack`/`tool` the conformance runner reads, or undefined when it has neither a
 * samples.lock nor staged samples. The locked set is extracted into its tree (samplesTreeDirectory): a tree whose files
 * are still the locked archive's (treeReady) is used as it is; otherwise the archive is taken from
 * ARCHIVE_CACHE, or fetched from S3, verified, and only then extracted. Samples staged under `publish/samples/` are laid
 * over the locked set, file by file, in a merged tree `publish-<sha256 of its canonical tar>/` beside it.
 */
export function samplesTree({ root = ROOT, pack, tool, aws = runAws } = {}) {
  const toolPack = namedToolPack(pack, tool);
  const lock = readSamplesLock(root, toolPack);
  const staged = stagedSamples(root, toolPack);
  if (lock === undefined && staged.length === 0) return undefined;
  const tree = lock === undefined ? undefined : lockedSamplesTree(root, toolPack, lock, aws);
  if (staged.length === 0) return tree;
  const merged = new Map((tree === undefined ? [] : extractedEntries(tree)).map((entry) => [entry.path, entry]));
  for (const entry of staged) merged.set(entry.path, entry);
  const entries = [...merged.values()];
  const archive = buildArchive(entries);
  const view = { sha256: archive.sha256, files: archive.files, bytes: archive.bytes };
  const mergedTree = join(root, SAMPLES_TREES, pack, tool, `publish-${archive.sha256}`);
  if (!treeReady(mergedTree, view)) extractTree(mergedTree, entries, view);
  return mergedTree;
}

/** The locked samples set's extracted tree, extracted from a verified archive unless it is already there whole. */
function lockedSamplesTree(root, toolPack, lock, aws) {
  const tree = samplesTreeDirectory(root, toolPack, lock.sha256);
  if (treeReady(tree, lock)) return tree;
  const discarded = [];
  const { entries } = samplesArchive(root, toolPack, lock, { dryRun: false, aws, signIn: signer(aws, discarded, "samples archive"), discarded, log: () => {} });
  extractTree(tree, entries, lock);
  return tree;
}

/** An extracted tree's files besides its marker, as archive entries. */
function extractedEntries(tree) {
  return readdirSync(tree, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && !(entry.parentPath === tree && entry.name === TREE_MARKER))
    .map((entry) => {
      const absolute = join(entry.parentPath, entry.name);
      return { path: relative(tree, absolute).split(sep).join("/"), bytes: readFileSync(absolute) };
    });
}

/**
 * Puts the samples archive each selected tool-pack's samples.lock names into ARCHIVE_CACHE (a verified cached copy is
 * kept, anything else is fetched from S3 and verified), every one before any is extracted, then extracts each into its
 * tree (extractSamples). Tool-packs with no samples.lock are skipped. A dry run deletes, fetches and writes nothing.
 */
export function samples({ root = ROOT, pack: packId, tool, dryRun = false, aws = runAws, log = console.log } = {}) {
  const discarded = [];
  const fetched = fetchSamples(root, selectToolPacks(root, packId, tool), { dryRun, aws, signIn: signer(aws, discarded, "samples archive"), discarded, log });
  const extracted = extractSamples(root, fetched.filter(({ entries }) => entries !== undefined), { dryRun, log });
  const keys = (source) => fetched.filter((entry) => entry.source === source).map(({ toolPack, lock }) => samplesKey(toolPack, lock.sha256)).sort(byBytes);
  const result = { kept: keys("cache"), downloaded: dryRun ? [] : keys("S3"), extracted };
  log(`${dryRun ? "dry run: " : ""}${fetched.length} samples archive(s): ${result.kept.length} kept in ${ARCHIVE_CACHE}, ${result.downloaded.length} fetched from S3; ${extracted.length} tree(s) extracted`);
  return result;
}

/**
 * The test data entries of the tool directory's publish/ on disk (publishFiles less `samples/`), as `{ path, bytes }` by
 * their archive path, and the publish/ files that are neither test data nor samples (the publish workflow refuses
 * them), as paths relative to publish/.
 */
function publishedTestData(root, toolPack) {
  const files = publishFiles(root, toolPack.directory).filter((path) => !isPublishedSample(path));
  const foreign = files.filter((path) => !isPackFixture(`${toolPack.directory}/${path}`));
  const entries = files
    .filter((path) => isPackFixture(`${toolPack.directory}/${path}`))
    .map((path) => ({ path, bytes: readFileSync(join(root, toolPack.directory, PUBLISH_DIRECTORY, path)) }));
  return { entries, foreign };
}

/**
 * Fetches the archive each selected tool-pack's test-data.lock names (from ARCHIVE_CACHE when a verified copy is there,
 * otherwise from S3 into it), verifies it against the lock, and extracts it into the tool directory, then lays the
 * tool-pack's publish/ test data (a pull request's publish request) over it, so the checkout holds what publishing it
 * in overlay mode would. A tool-pack with no lock yet takes its publish/ alone. Every archive is verified before
 * anything is extracted, and every tool directory and directory an entry goes in, from the checkout down, must be no
 * symlink (nor may any pack test data in the tool directory, or anything in publish/, be one). Derived files on disk
 * that neither the archive nor publish/ holds are deleted (a stale derivation); authored ones are kept and reported. The
 * samples archive each selected tool-pack's samples.lock names is put into ARCHIVE_CACHE and verified with the test
 * data archives, before anything is extracted, then extracted into its tree outside the tool directory (samples).
 */
export function pull({ root = ROOT, pack, tool, dryRun = false, aws = runAws, log = console.log } = {}) {
  const selected = selectToolPacks(root, pack, tool);
  const locked = selected.map((toolPack) => {
    const published = publishedTestData(root, toolPack);
    const lock = published.entries.length > 0 && !existsSync(join(root, toolPack.directory, TEST_DATA_LOCK)) ? undefined : readLock(root, toolPack);
    return { toolPack, lock, published };
  });
  const discarded = [];
  const signIn = signer(aws, discarded, "archive");
  const fetched = [];
  for (const { toolPack, lock, published } of locked) {
    if (lock === undefined) {
      fetched.push({ toolPack, lock, entries: [], source: "publish/", published });
      continue;
    }
    const fromCache = cachedArchive(root, toolPack, lock, dryRun);
    if (fromCache.discarded !== undefined) discarded.push(fromCache.discarded);
    let entries = fromCache.entries;
    let source = "cache";
    if (entries === undefined) {
      source = "S3";
      if (dryRun) {
        log(`${fromCache.discarded === undefined ? "" : `would delete ${fromCache.discarded}, which is not the archive ${lock.label} names, and `}would fetch s3://${BUCKET}/${archiveKey(toolPack, lock.sha256)} (${lock.label})`);
        continue;
      }
      signIn();
      entries = downloadArchive(root, aws, toolPack, lock);
    }
    fetched.push({ toolPack, lock, entries, source, published });
  }
  const fetchedSamples = fetchSamples(root, selected, { dryRun, aws, signIn, discarded, log });
  if (dryRun) {
    log(`dry run: ${fetched.length} of ${locked.length} archive(s) and ${fetchedSamples.filter(({ entries }) => entries !== undefined).length} of ${fetchedSamples.length} samples archive(s) are in ${ARCHIVE_CACHE}; nothing was extracted`);
    return { toolPacks: [], samples: [] };
  }

  // No write may go through a symlink: every check runs before the first write.
  const onDisk = new Map();
  const directories = new Set();
  for (const { toolPack, entries, published } of fetched) {
    onDisk.set(toolPack.directory, toolPackFiles(root, toolPack.directory));
    for (const entry of [...entries, ...published.entries]) {
      const path = `${toolPack.directory}/${entry.path}`;
      directories.add(path.slice(0, path.lastIndexOf("/")));
    }
  }
  for (const directory of [...directories].sort(byBytes)) refuseSymlinkedDirectories(root, directory);

  const authored = authoredListing(root);
  for (const { toolPack, lock, entries, source, published } of fetched) {
    const extracted = new Map([...entries, ...published.entries].map((entry) => [entry.path, entry.bytes]));
    const stale = onDisk.get(toolPack.directory).filter((path) => !extracted.has(path));
    const kept = stale.filter((path) => packFixtureKind(`${toolPack.directory}/${path}`, authored) === "authored");
    for (const path of stale) if (!kept.includes(path)) rmSync(join(root, toolPack.directory, path));
    for (const [path, bytes] of extracted) {
      const absolute = join(root, toolPack.directory, path);
      mkdirSync(dirname(absolute), { recursive: true });
      writeFileSync(absolute, bytes);
    }
    const notes = [
      ...(published.entries.length > 0 ? [`${published.entries.length} file(s) from ${PUBLISH_DIRECTORY}/ laid over it`] : []),
      ...(published.foreign.length > 0 ? [`left out ${published.foreign.length} ${PUBLISH_DIRECTORY}/ file(s) that are no pack test data, which publishing refuses: ${published.foreign.slice(0, 5).join(", ")}`] : []),
      ...(stale.length > kept.length ? [`deleted ${stale.length - kept.length} stale derived file(s) the archive lacks`] : []),
      ...(kept.length > 0 ? [`kept ${kept.length} authored file(s) the archive lacks (stage them with npm run fixtures:stage to publish them, or delete them): ${kept.slice(0, 5).join(", ")}`] : []),
    ];
    const from = lock === undefined ? `no archive (no ${TEST_DATA_LOCK} yet)` : `archive ${lock.sha256.slice(0, 12)} (${source})`;
    log(`${toolPack.directory}: ${entries.length} file(s) from ${from}${notes.length > 0 ? `; ${notes.join("; ")}` : ""}`);
  }
  extractSamples(root, fetchedSamples, { dryRun, log });
  return {
    toolPacks: fetched.map(({ toolPack, source, published }) => ({ ...toolPack, source, published: published.entries.length })),
    samples: fetchedSamples.map(({ toolPack, source }) => ({ ...toolPack, source })),
  };
}

/**
 * Archives each selected tool-pack's test data as it is on disk into ARCHIVE_CACHE and rewrites its test-data.lock; no
 * network. It refuses a symlink among the data or on the way to its tool directory (toolPackFiles), which it would
 * otherwise leave out or read through. `push` packs this way before it uploads.
 */
export function pack({ root = ROOT, pack: packId, tool, dryRun = false, log = console.log } = {}) {
  const selected = selectToolPacks(root, packId, tool);
  // Every tool-pack's data is listed, refusing a symlink or nothing to archive, before any lock is written.
  const listed = selected.map((toolPack) => {
    const files = toolPackFiles(root, toolPack.directory);
    if (files.length === 0) throw new Error(`${toolPack.directory}: no pack test data on disk to archive; pull it (npm run fixtures:pull) or derive it (npm run fixtures:derive) first`);
    return { toolPack, files };
  });
  const changedLocks = [];
  for (const { toolPack, files } of listed) {
    const archive = buildArchive(files.map((path) => ({ path, bytes: readFileSync(join(root, toolPack.directory, path)) })));
    const label = `${toolPack.directory}/${TEST_DATA_LOCK}`;
    const lockPath = join(root, label);
    const before = existsSync(lockPath) ? readFileSync(lockPath, "utf8") : undefined;
    const text = lockText(archive);
    let previous;
    try {
      previous = before === undefined ? undefined : parseLock(before, label);
    } catch {
      previous = undefined;
    }
    const summary = `${archive.files} file(s), ${archive.bytes} byte(s)${previous === undefined ? "" : `; the lock had ${previous.files} file(s), ${previous.bytes} byte(s)`}`;
    if (dryRun) {
      log(`${toolPack.directory}: would ${text === before ? "keep" : "write"} ${label}, archive ${archive.sha256} (${summary})`);
      continue;
    }
    cacheArchive(root, toolPack, archive);
    if (text !== before) {
      writeFileSync(lockPath, text);
      changedLocks.push(label);
    }
    log(`${toolPack.directory}: archive ${archive.sha256} in ${ARCHIVE_CACHE}; ${label} ${text === before ? "unchanged" : "written"} (${summary})`);
  }
  return { changedLocks };
}

/**
 * What uploading the archives `locked` (each `{ toolPack, lock }`) asks of the checkout at `root`: one plan per archive
 * for uploadObjects, with its local copy in ARCHIVE_CACHE when there is one. A local copy must pass the pull's own check
 * against the lock first, so nothing but the bytes the lock names is ever uploaded under its key.
 */
function archivePlans(root, locked) {
  return locked.map(({ toolPack, lock }) => {
    const key = archiveKey(toolPack, lock.sha256);
    const path = archiveCachePath(root, toolPack, lock.sha256);
    const check = (bytes) => verifiedEntries(bytes, lock, key, toolPack.directory);
    const local = existsSync(path);
    if (local) check(readFileSync(path));
    const absent = `${lock.label} names s3://${BUCKET}/${key}, which neither S3 nor ${relative(root, path)} holds: pack the tool-pack first (npm run fixtures:push packs and uploads)`;
    return { directory: toolPack.directory, label: lock.label, what: "archive", key, path, local, check, absent };
  });
}

/**
 * Puts each plan's local copy into S3, tagged (objectTagging's `tags`), unless S3 holds its key already (a conditional
 * write: an object is never overwritten, and one S3 held keeps its first upload's tags), then reads back what S3 holds
 * and checks it as the plan says. A plan with no local copy needs S3 to hold its key, and every plan is checked for
 * that, and the tags for being valid, before anything is put. Returns the keys put, sorted.
 */
function uploadObjects(aws, plans, log, tags) {
  const tagging = objectTagging(tags);
  const sorted = [...plans].sort((a, b) => byBytes(a.key, b.key));
  const absent = sorted.filter((plan) => !plan.local && !objectExists(aws, plan.key));
  if (absent.length > 0) throw new Error(`${absent.map((plan) => plan.absent).join("\n")}\nNothing was uploaded.`);
  const uploaded = [];
  for (const plan of sorted) {
    const put = plan.local && !objectExists(aws, plan.key) && putObject(aws, plan.key, plan.path, tagging);
    const readback = `${plan.path}.readback`;
    mkdirSync(dirname(readback), { recursive: true });
    try {
      if (!download(aws, plan.key, readback)) throw new Error(`${plan.label}: s3://${BUCKET}/${plan.key} is not in S3 after its upload`);
      plan.check(readFileSync(readback));
    } finally {
      rmSync(readback, { force: true });
    }
    if (put) uploaded.push(plan.key);
    log(`${plan.directory}: ${put ? "uploaded" : "S3 already holds"} s3://${BUCKET}/${plan.key} (${plan.what}), verified`);
  }
  return uploaded;
}

/**
 * Puts each selected tool-pack's locked archive from ARCHIVE_CACHE into S3, tagged `tags`, unless S3 holds its key
 * already, then reads each back and verifies it
 * against its lock. An archive with no local copy is fine when S3 holds it (and verifies); one in neither place fails
 * the upload, before anything is put.
 */
export function upload({ root = ROOT, pack: packId, tool, tags, aws = runAws, log = console.log } = {}) {
  const selected = selectToolPacks(root, packId, tool);
  const plans = archivePlans(root, selected.map((toolPack) => ({ toolPack, lock: readLock(root, toolPack) })));
  objectTagging(tags);
  requireAws(aws);
  return { uploaded: uploadObjects(aws, plans, log, tags) };
}

/**
 * What uploading the samples archive a samples.lock (`lock`, readSamplesLock's) names asks of the checkout at `root`: one
 * plan for uploadObjects, with its local copy in ARCHIVE_CACHE when there is one. A local copy that is not what the
 * lock names is refused, so nothing else is ever put under its key.
 */
function samplesPlan(root, toolPack, lock) {
  const key = samplesKey(toolPack, lock.sha256);
  const path = samplesArchiveCachePath(root, toolPack, lock.sha256);
  const shown = relative(root, path).split(sep).join("/");
  const local = existsSync(path);
  const check = (bytes) => {
    verifiedSamples(bytes, lock, key);
  };
  if (local) {
    try {
      check(readFileSync(path));
    } catch (error) {
      throw new Error(`${shown} is not the samples archive ${lock.label} names (${error instanceof Error ? error.message : String(error)}); delete it and ${REPACK_SAMPLES}. Nothing was uploaded`);
    }
  }
  const absent = `${lock.label} names samples archive ${lock.sha256}, which is in neither ${shown} nor s3://${BUCKET}/${key}: ${REPACK_SAMPLES}`;
  return { directory: toolPack.directory, label: lock.label, what: "samples archive", key, path, local, check, absent };
}

/**
 * Maintainer: puts the samples archive each selected tool-pack's samples.lock names from ARCHIVE_CACHE into S3 unless S3
 * holds it already (a conditional put, so an object is never overwritten), reads each back and verifies it against the
 * lock, then prints the `git add` that commits the locks. Run again, it puts nothing ("S3 already holds"). A cached
 * archive that is not the locked one, or one in neither ARCHIVE_CACHE nor S3, fails it before anything is put.
 */
export function samplesPush({ root = ROOT, pack: packId, tool, dryRun = false, tags, aws = runAws, log = console.log } = {}) {
  if (dryRun) throw new Error("samples push has no --dry-run");
  const locked = selectToolPacks(root, packId, tool).flatMap((toolPack) => {
    const lock = readSamplesLock(root, toolPack);
    return lock === undefined ? [] : [samplesPlan(root, toolPack, lock)];
  });
  if (locked.length === 0) throw new Error(`no selected tool-pack has a ${SAMPLES_LOCK}: to make one, ${REPACK_SAMPLES}`);
  objectTagging(tags);
  requireAws(aws);
  const uploaded = uploadObjects(aws, locked, log, tags);
  const locks = locked.map(({ label }) => label);
  log(`S3 holds the samples archive(s) the lock(s) name; commit them:\n  git add -- ${locks.map(shellQuoted).join(" ")}`);
  return { uploaded, locks };
}

/** Every entry under packs/ of `commit`'s tree in the repository at `root`, as path → `{ mode, oid }`. */
function treeEntries(root, commit) {
  const entries = new Map();
  for (const line of nulList(runGit(root, ["ls-tree", "-r", "-z", "--full-tree", commit, "--", "packs"]))) {
    const tab = line.indexOf("\t");
    const [mode, , oid] = line.slice(0, tab).split(" ");
    entries.set(line.slice(tab + 1), { mode, oid });
  }
  return entries;
}

/**
 * The bytes of each blob `oids` names, by object id, exactly as committed: cat-file applies no filter, textconv or
 * attribute. One `git cat-file --batch` reads them all.
 */
function gitBlobs(root, oids) {
  const unique = [...new Set(oids)];
  const blobs = new Map();
  if (unique.length === 0) return blobs;
  const run = spawnSync("git", ["cat-file", "--batch"], { cwd: root, input: `${unique.join("\n")}\n`, maxBuffer: 1 << 30 });
  if (run.error !== undefined) throw run.error;
  if (run.status !== 0) throw new Error(`git cat-file --batch failed: ${run.stderr.toString("utf8").trim()}`);
  const output = run.stdout;
  let at = 0;
  for (const oid of unique) {
    const end = output.indexOf(0x0a, at);
    const [name, type, size] = output.subarray(at, end).toString("utf8").split(" ");
    if (name !== oid || type !== "blob") throw new Error(`git cat-file --batch: ${oid} is ${type ?? "missing"}, not a blob`);
    const length = Number(size);
    blobs.set(oid, output.subarray(end + 1, end + 1 + length));
    at = end + 1 + length + 1;
  }
  return blobs;
}

/** The commit `from` (a git revision) names in the repository at `root`, as its full object id. */
function resolveCommit(root, from) {
  if (typeof from !== "string" || from === "" || from.startsWith("-")) throw new Error(`--from ${JSON.stringify(from)} is not a git revision`);
  try {
    return runGit(root, ["rev-parse", "--verify", "--quiet", `${from}^{commit}`]).trim();
  } catch {
    throw new Error(`--from ${from} is no commit in ${root}; fetch the pull request first (git fetch origin +refs/pull/<n>/head:refs/publish/head)`);
  }
}

/** A pack or tool id a publish request may name: one lowercase path segment. */
const TOOL_ID = /^[a-z0-9][a-z0-9._-]*$/u;
/** Git's modes for a regular file. */
const REGULAR_FILE_MODES = new Set(["100644", "100755"]);

/**
 * The publish requests in a commit's tree (`tree`, treeEntries's; `at` describes the commit): per tool directory with a
 * tracked publish/, `{ toolPack, testData, samples, paths }`, where `testData` and `samples` are `{ path, oid }` by
 * archive path and `paths` every publish/ path. Throws, listing each, on a publish/ path no archive takes: neither pack
 * test data of its tool directory nor under publish/samples/, no regular file, or in a pack or tool that is no id.
 */
function publishRequests(tree, at) {
  const requests = new Map();
  const refused = [];
  for (const [path, { mode, oid }] of tree) {
    const found = publishPath(path);
    if (found === undefined) continue;
    if (!TOOL_ID.test(found.pack) || !TOOL_ID.test(found.tool)) refused.push(`${path} (no pack or tool id)`);
    else if (found.path.split("/").some((segment) => segment === "" || segment === "." || segment === "..")) refused.push(`${path} (an empty, "." or ".." segment)`);
    else if (!REGULAR_FILE_MODES.has(mode)) refused.push(`${path} (mode ${mode}: not a regular file)`);
    else {
      const sample = isPublishedSample(found.path);
      if (!sample && !isPackFixture(`${found.directory}/${found.path}`)) {
        refused.push(`${path} (neither pack test data of ${found.directory} nor under ${PUBLISH_DIRECTORY}/${SAMPLES_IN_PUBLISH}/)`);
        continue;
      }
      if (!requests.has(found.directory)) requests.set(found.directory, { toolPack: namedToolPack(found.pack, found.tool), testData: [], samples: [], paths: [] });
      const request = requests.get(found.directory);
      request.paths.push(path);
      if (sample) request.samples.push({ path: found.path.slice(SAMPLES_IN_PUBLISH.length + 1), oid });
      else request.testData.push({ path: found.path, oid });
    }
  }
  if (refused.length > 0) {
    throw new Error(`${at}: ${PUBLISH_DIRECTORY}/ holds ${refused.length} path(s) no archive takes, so nothing was packed (CONTRIBUTING.md, "Pack test data"):\n${refused.sort(byBytes).map((path) => `  ${path}`).join("\n")}`);
  }
  return new Map([...requests].sort(([a], [b]) => byBytes(a, b)));
}

/** The licence of Rerail's own files, every pack test data file no licenses/<project>/UPSTREAM.md lists. */
const RERAIL_LICENCE = "Apache-2.0";
/** The licence an UPSTREAM.md records (metrics/07-pack-licence-annotations.mjs's UPSTREAM_LICENSE). */
const UPSTREAM_LICENSE = /\bLicense: ([A-Za-z0-9][A-Za-z0-9.+-]*)/u;
/** A pack's licenses/<project>/UPSTREAM.md, which lists the files copied from that upstream and its licence. */
const LICENSES_RECORD = /^packs\/([^/]+)\/licenses\/[^/]+\/UPSTREAM\.md$/u;
/** The repository's licence allowlist: the SPDX ids anything put in S3 may be under. */
const LICENCE_ALLOWLIST = "scripts/check/licence-allowlist.json";

/**
 * Whether data under the SPDX expression `expression` may go to S3: `A OR B` when either side may, `A AND B` (terms as
 * licenceTerms reads them) when every term is on `allowed`.
 */
export function licenceAllowed(expression, allowed) {
  return String(expression)
    .split(/\s+OR\s+/u)
    .some((alternative) => {
      const terms = licenceTerms(alternative);
      return terms.length > 0 && terms.every((term) => allowed.has(term));
    });
}

/** The allowlist this checkout's code (main's, in the publish workflow) reads. */
function licenceAllowlist() {
  return new Set(JSON.parse(readFileSync(join(ROOT, LICENCE_ALLOWLIST), "utf8")));
}

/**
 * The licence of each pack test data file a pack's licenses/<project>/UPSTREAM.md lists, by repository path, as the
 * commit's tree (`tree`, read through `blob`) records it; a listed file whose UPSTREAM.md records none gets a note,
 * which no allowlist takes.
 */
function upstreamLicences(tree, blob) {
  const licences = new Map();
  for (const path of tree.keys()) {
    const match = LICENSES_RECORD.exec(path);
    if (match === null) continue;
    const text = blob(path).toString("utf8");
    const licence = UPSTREAM_LICENSE.exec(text)?.[1] ?? `no licence (${path} records no "License:")`;
    for (const line of text.split("\n")) {
      const file = UPSTREAM_ROW.exec(line)?.[1];
      if (file !== undefined) licences.set(`packs/${match[1]}/${file}`, licence);
    }
  }
  return licences;
}

/**
 * Per sample id, the licence its labels file (`<sample-id>/<path>.expected.json`) records as `source.license`; the set's
 * LICENSES.md is Rerail's. A file of a sample with no readable labels gets a note, which no allowlist takes.
 */
function sampleLicences(entries) {
  const bySample = new Map();
  for (const { path, bytes } of entries) {
    if (!path.endsWith(".expected.json")) continue;
    let licence;
    try {
      licence = JSON.parse(bytes.toString("utf8"))?.source?.license;
    } catch {
      licence = undefined;
    }
    if (typeof licence === "string") bySample.set(path.slice(0, path.indexOf("/")), licence);
  }
  return (path) => (path === SAMPLES_LICENSES ? RERAIL_LICENCE : bySample.get(path.slice(0, path.indexOf("/"))) ?? "no licence (the sample has no labels file naming source.license)");
}

/** `{ added, changed, dropped }`: the archive paths `next` adds to, changes in and drops from `base` (both path → bytes), sorted. */
function archiveChanges(base, next) {
  const sorted = (paths) => paths.sort(byBytes);
  return {
    added: sorted([...next.keys()].filter((path) => !base.has(path))),
    changed: sorted([...next].filter(([path, bytes]) => base.has(path) && !base.get(path).equals(bytes)).map(([path]) => path)),
    dropped: sorted([...base.keys()].filter((path) => !next.has(path))),
  };
}

/** The format of `pack --from`'s manifest. */
export const PUBLISH_MANIFEST_FORMAT = "rerail.pack-data-publish/v1";

/**
 * The publish workflow's prep (`pack --from <commit>`): archives every tool-pack's publish/ in the commit `from` names,
 * reading nothing of it but blobs (git plumbing in the trusted checkout at `root`, main's; the commit is never checked
 * out and none of its code runs). Per tool-pack and kind present in its publish/:
 *
 * - test data (`publish/<path>`): overlay (default): the archive the commit's test-data.lock names, less the files the
 *   merge base's UPSTREAM.md listed as Rerail's and the commit's no longer lists, overlaid with publish/; `replace`:
 *   publish/ alone;
 * - samples (`publish/samples/<path>`): validated by scripts/samples/publish.ts's publishedSamples, then laid over the
 *   archive the commit's samples.lock names, or alone with `replace`.
 *
 * Base archives come from ARCHIVE_CACHE or S3 (a read). Every file going to S3 must be under a licence the allowlist
 * takes (licenceAllowed): test data by the pack's licenses/<project>/UPSTREAM.md that lists it, else Rerail's
 * Apache-2.0, and samples by their labels' `source.license`; a failure lists every file and licence before anything is
 * written. Then each archive is written compressed under `out` (its key less ARCHIVE_PREFIX), the manifest
 * (PUBLISH_MANIFEST_FORMAT) to `manifest` and the approver's summary (publishSummary) to `summary`, when given. A
 * commit with no publish/ fails: nothing to publish.
 */
export async function packFrom({ root = ROOT, from, replace = false, out, manifest, summary, aws = runAws, log = console.log } = {}) {
  if (out === undefined) throw new Error("pack --from needs --out <dir>, where the archives are written");
  const commit = resolveCommit(root, from);
  const at = `${from} (${commit.slice(0, 12)})`;
  const tree = treeEntries(root, commit);
  const requests = publishRequests(tree, at);
  if (requests.size === 0) throw new Error(`${at} has no ${PUBLISH_DIRECTORY}/ in any packs/<pack>/upstream/<tool>/: nothing to publish`);
  const blob = (path) => {
    const { mode, oid } = tree.get(path);
    if (!REGULAR_FILE_MODES.has(mode)) throw new Error(`${path} at ${at} is not a regular file (mode ${mode})`);
    return gitBlobs(root, [oid]).get(oid);
  };
  const lockAt = (label) => (tree.has(label) ? { label: `${label} at ${at}`, ...parseLock(blob(label).toString("utf8"), `${label} at ${at}`) } : undefined);
  const discarded = [];
  const signIn = signer(aws, discarded, "archive");
  const licensed = upstreamLicences(tree, blob);

  let mergeBase;
  /** The authored files the tool directory's UPSTREAM.md lists at the merge base with HEAD and no longer lists at the commit. */
  const unlisted = (toolPack) => {
    if (mergeBase === undefined) {
      let base;
      try {
        base = runGit(root, ["merge-base", "HEAD", commit]).trim();
      } catch (error) {
        throw new Error(`which UPSTREAM.md rows ${at} removes is unknown without its merge base with HEAD (${error instanceof Error ? error.message : String(error)}), so its archive cannot drop their files; publish with replace to archive exactly its publish/`);
      }
      mergeBase = treeEntries(root, base);
    }
    const record = `${toolPack.directory}/UPSTREAM.md`;
    const listing = (entries) => {
      const entry = entries.get(record);
      return entry === undefined || !REGULAR_FILE_MODES.has(entry.mode) ? [] : listedAuthored(record, gitBlobs(root, [entry.oid]).get(entry.oid).toString("utf8"));
    };
    const kept = new Set(listing(tree));
    return new Set(listing(mergeBase).filter((path) => !kept.has(path) && path.startsWith(`${toolPack.directory}/`)));
  };

  /** The verified entries of the base archive `lock` names (from ARCHIVE_CACHE, else S3); [] with `optional` when S3 lacks a test data archive. */
  const baseEntries = (toolPack, lock, kind, optional) => {
    if (kind === "samples") return samplesArchive(root, toolPack, lock, { dryRun: false, aws, signIn, discarded, log }).entries;
    const cached = cachedArchive(root, toolPack, lock, false);
    if (cached.discarded !== undefined) discarded.push(cached.discarded);
    if (cached.entries !== undefined) return cached.entries;
    signIn();
    if (optional && !objectExists(aws, archiveKey(toolPack, lock.sha256))) {
      log(`${toolPack.directory}: S3 does not hold archive ${lock.sha256}, which ${lock.label} names, so what replace drops from it is not listed`);
      return [];
    }
    return downloadArchive(root, aws, toolPack, lock);
  };

  const allowed = licenceAllowlist();
  const builds = [];
  const refused = [];
  for (const { toolPack, testData, samples } of requests.values()) {
    const kinds = [
      ...(testData.length > 0 ? [{ kind: "test-data", name: TEST_DATA_LOCK, files: testData }] : []),
      ...(samples.length > 0 ? [{ kind: "samples", name: SAMPLES_LOCK, files: samples }] : []),
    ];
    for (const { kind, name, files } of kinds) {
      const label = `${toolPack.directory}/${name}`;
      const lock = lockAt(label);
      const base = new Map(lock === undefined ? [] : baseEntries(toolPack, lock, kind, replace).map((entry) => [entry.path, entry.bytes]));
      const blobs = gitBlobs(root, files.map(({ oid }) => oid));
      let published = files.map(({ path, oid }) => ({ path, bytes: blobs.get(oid) }));
      if (kind === "samples") {
        const { publishedSamples } = await import("../samples/publish.ts");
        published = publishedSamples(published);
      }
      const next = new Map();
      if (!replace) {
        const gone = kind === "test-data" && base.size > 0 ? unlisted(toolPack) : new Set();
        for (const [path, bytes] of base) if (!gone.has(`${toolPack.directory}/${path}`)) next.set(path, bytes);
      }
      for (const { path, bytes } of published) next.set(path, bytes);
      const entries = [...next].map(([path, bytes]) => ({ path, bytes }));
      const archive = buildArchive(entries);
      const key = kind === "samples" ? samplesKey(toolPack, archive.sha256) : archiveKey(toolPack, archive.sha256);
      const compressed = compressArchive(archive.tar);
      // The archive passes the check upload and confirm make, before anything asks for approval.
      if (kind === "samples") verifiedSamples(compressed, { label, ...archive }, key);
      else verifiedEntries(compressed, { label, ...archive }, key, toolPack.directory);
      const licenceOf = kind === "samples" ? sampleLicences(entries) : (path) => licensed.get(`${toolPack.directory}/${path}`) ?? RERAIL_LICENCE;
      const licences = {};
      for (const { path } of entries) {
        const licence = licenceOf(path);
        licences[licence] = (licences[licence] ?? 0) + 1;
        if (!licenceAllowed(licence, allowed)) refused.push(`${toolPack.directory}/${kind === "samples" ? `${PUBLISH_DIRECTORY}/${SAMPLES_IN_PUBLISH}/` : ""}${path}: ${licence}`);
      }
      builds.push({ kind, toolPack, label, lock, key, archive, compressed, licences, changes: archiveChanges(base, next) });
    }
  }
  if (refused.length > 0) {
    throw new Error(`${refused.length} file(s) going to S3 are under a licence ${LICENCE_ALLOWLIST} does not take (an \`A OR B\` passes when either side is on it), so nothing was packed and no approval is asked:\n${refused.map((line) => `  ${line}`).join("\n")}`);
  }

  const objects = builds.map(({ kind, toolPack, label, lock, key, archive, compressed, licences, changes }) => {
    const file = join(out, key.slice(ARCHIVE_PREFIX.length));
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, compressed);
    return {
      kind,
      pack: toolPack.pack,
      tool: toolPack.tool,
      lock: label,
      key,
      sha256: archive.sha256,
      files: archive.files,
      bytes: archive.bytes,
      compressedBytes: compressed.length,
      lockText: lockText(archive),
      base: lock?.sha256 ?? null,
      licences: Object.fromEntries(Object.entries(licences).sort(([a], [b]) => byBytes(a, b))),
      ...changes,
    };
  });
  const result = {
    format: PUBLISH_MANIFEST_FORMAT,
    commit,
    replace,
    objects,
    remove: [...requests.values()].flatMap(({ paths }) => paths).sort(byBytes),
  };
  if (manifest !== undefined) {
    mkdirSync(dirname(resolve(manifest)), { recursive: true });
    writeFileSync(manifest, `${JSON.stringify(result, null, 2)}\n`);
  }
  const text = publishSummary(result);
  if (summary !== undefined) writeFileSync(summary, text);
  log(text);
  return result;
}

/**
 * How many paths of each change list the summary shows. The approver reads it in place of CI deriving the data again,
 * and a replace publish's dropped files appear nowhere else; 50 per list keeps a bump's summary well under the step
 * summary's 1 MiB.
 */
const SUMMARY_PATHS = 50;

/** The approver's view of a manifest (packFrom's), as markdown: one row per archive, then what each changes. */
export function publishSummary(manifest) {
  const shown = (paths) => (paths.length === 0 ? "" : `: ${paths.slice(0, SUMMARY_PATHS).map((path) => `\`${path}\``).join(", ")}${paths.length > SUMMARY_PATHS ? ", …" : ""}`);
  return [
    `### Pack data to publish from ${manifest.commit} (${manifest.replace ? "replace" : "overlay"})`,
    "",
    "| Kind | Tool-pack | sha256 | Files | Bytes | Licences |",
    "|---|---|---|---|---|---|",
    ...manifest.objects.map((object) => `| ${object.kind} | \`${object.pack}/${object.tool}\` | \`${object.sha256}\` | ${object.files} | ${object.bytes} | ${Object.entries(object.licences).map(([licence, count]) => `${licence} ${count}`).join(", ")} |`),
    "",
    ...manifest.objects.flatMap((object) => [
      `- ${object.kind} \`${object.pack}/${object.tool}\` against ${object.base === null ? "no locked archive" : `the locked \`${object.base}\``}:`,
      `  - ${object.added.length} added${shown(object.added)}`,
      `  - ${object.changed.length} changed${shown(object.changed)}`,
      `  - ${object.dropped.length} dropped${shown(object.dropped)}`,
    ]),
    "",
    `${manifest.remove.length} ${PUBLISH_DIRECTORY}/ file(s) are deleted by the commit that writes the locks.`,
    "",
  ].join("\n");
}

/**
 * Copies the selected tool-packs' pack test data on disk that their locked archive lacks or holds differently into
 * their publish/ (with `all`, every file, for a `replace` publish), and deletes the test data files publish/ holds that
 * are no longer to be staged; publish/samples/ is left alone. The locked archive comes from ARCHIVE_CACHE or S3. A file
 * the archive holds and the disk lacks cannot be expressed as an overlay: it is reported, with the remedy. A dry run
 * writes and deletes nothing in publish/.
 */
export function stage({ root = ROOT, pack: packId, tool, all = false, dryRun = false, aws = runAws, log = console.log } = {}) {
  if (packId === undefined) throw new Error("stage needs --pack (and --tool)");
  const selected = selectToolPacks(root, packId, tool);
  const discarded = [];
  const signIn = signer(aws, discarded, "archive");
  const results = [];
  for (const toolPack of selected) {
    const onDisk = new Map(toolPackFiles(root, toolPack.directory).map((path) => [path, readFileSync(join(root, toolPack.directory, path))]));
    if (onDisk.size === 0) throw new Error(`${toolPack.directory}: no pack test data on disk to stage; pull it (npm run fixtures:pull) and make your change first`);
    let base = new Map();
    if (existsSync(join(root, toolPack.directory, TEST_DATA_LOCK))) {
      const lock = readLock(root, toolPack);
      const cached = cachedArchive(root, toolPack, lock, dryRun);
      if (cached.discarded !== undefined) discarded.push(cached.discarded);
      let entries = cached.entries;
      if (entries === undefined) {
        signIn();
        entries = downloadArchive(root, aws, toolPack, lock);
      }
      base = new Map(entries.map((entry) => [entry.path, entry.bytes]));
    }
    const staged = [...onDisk].filter(([path, bytes]) => all || !base.get(path)?.equals(bytes)).map(([path]) => path);
    const wanted = new Set(staged);
    const removed = publishFiles(root, toolPack.directory).filter((path) => !isPublishedSample(path) && !wanted.has(path));
    const missing = [...base.keys()].filter((path) => !onDisk.has(path)).sort(byBytes);
    const publish = `${toolPack.directory}/${PUBLISH_DIRECTORY}`;
    if (!dryRun) {
      for (const path of removed) rmSync(join(root, publish, path));
      for (const path of staged) {
        refuseSymlinkedDirectories(root, dirname(`${publish}/${path}`));
        mkdirSync(dirname(join(root, publish, path)), { recursive: true });
        writeFileSync(join(root, publish, path), onDisk.get(path));
      }
    }
    log(`${toolPack.directory}: ${dryRun ? "would stage" : "staged"} ${staged.length} file(s) in ${publish}/${all ? " (all of the data on disk)" : ""}${removed.length > 0 ? `; ${dryRun ? "would delete" : "deleted"} ${removed.length} staged file(s) no longer to publish` : ""}`);
    if (missing.length > 0) {
      log(`${toolPack.directory}: ${missing.length} file(s) of the locked archive are not on disk, e.g. ${missing.slice(0, 5).join(", ")}; ${all ? "a publish with replace drops them" : "an overlay cannot delete a file: to publish exactly the data on disk, stage with --all and publish with replace"}`);
    }
    results.push({ ...toolPack, staged, removed, missing });
  }
  const directories = results.filter(({ staged }) => staged.length > 0).map(({ directory }) => `${directory}/${PUBLISH_DIRECTORY}`);
  if (!dryRun && directories.length > 0) {
    log(`Commit it and run the publish workflow (${PUBLISH_WORKFLOW}) for your pull request${all ? " with replace" : ""} (CONTRIBUTING.md, "Pack test data"):\n  git add -- ${directories.map(shellQuoted).join(" ")}`);
  }
  return { toolPacks: results };
}

/**
 * The objects the publish workflow's prep named (its job output, packFrom's manifest objects), each checked and with
 * what verifies it: `{ kind, toolPack, key, lock, compressedBytes, check }`, the key recomputed from the kind, pack,
 * tool and sha256. Throws on anything else, a duplicate key included.
 */
export function publishedObjects(objects) {
  if (!Array.isArray(objects) || objects.length === 0) throw new Error("--objects names no object: it must be the JSON array of the objects the publish workflow's prep packed");
  const keys = new Set();
  return objects.map((object, index) => {
    const where = `object ${index}`;
    if (object === null || typeof object !== "object") throw new Error(`${where} is not an object`);
    const { kind, pack, tool, sha256: digest, files, bytes, compressedBytes } = object;
    if (kind !== "test-data" && kind !== "samples") throw new Error(`${where}: kind ${JSON.stringify(kind)} is neither "test-data" nor "samples"`);
    if (typeof pack !== "string" || typeof tool !== "string" || !TOOL_ID.test(pack) || !TOOL_ID.test(tool)) throw new Error(`${where}: pack ${JSON.stringify(pack)} and tool ${JSON.stringify(tool)} must be ids`);
    if (!isSha256(digest) || !isCount(files) || !isCount(bytes) || !isCount(compressedBytes)) throw new Error(`${where}: needs a sha256 of 64 lowercase hex digits and counts of files, bytes and compressedBytes`);
    const toolPack = namedToolPack(pack, tool);
    const key = kind === "samples" ? samplesKey(toolPack, digest) : archiveKey(toolPack, digest);
    if (object.key !== undefined && object.key !== key) throw new Error(`${where}: its key ${JSON.stringify(object.key)} is not ${key}, which its kind, pack, tool and sha256 make`);
    if (keys.has(key)) throw new Error(`${where}: ${key} is named twice`);
    keys.add(key);
    const lock = { label: `${toolPack.directory}/${kind === "samples" ? SAMPLES_LOCK : TEST_DATA_LOCK}`, sha256: digest, files, bytes };
    const check = (compressed) => (kind === "samples" ? verifiedSamples(compressed, lock, key) : verifiedEntries(compressed, lock, key, toolPack.directory));
    return { kind, toolPack, key, lock, compressedBytes, check };
  });
}

/**
 * The publish workflow's upload (`upload --objects`): verifies each archive under `archives` (packFrom's `out`) against
 * the object prep named (publishedObjects: its compressed size, then verifiedEntries / verifiedSamples), every one
 * before anything is put, then puts each create-only, tagged `tags`, and reads it back (uploadObjects). Returns the
 * keys put.
 */
export function uploadPublished({ objects, archives, tags, aws = runAws, log = console.log } = {}) {
  if (archives === undefined) throw new Error("upload --objects needs --archives <dir>, where prep's archives are");
  const named = publishedObjects(objects);
  const problems = [];
  const plans = named.flatMap(({ kind, toolPack, key, lock, compressedBytes, check }) => {
    const path = join(archives, key.slice(ARCHIVE_PREFIX.length));
    try {
      if (!existsSync(path)) throw new Error(`${path} is missing`);
      const compressed = readFileSync(path);
      if (compressed.length !== compressedBytes) throw new Error(`${path} is ${compressed.length} byte(s), not the ${compressedBytes} prep named`);
      check(compressed);
    } catch (error) {
      problems.push(`${key}: ${error instanceof Error ? error.message : String(error)}`);
      return [];
    }
    return [{ directory: toolPack.directory, label: lock.label, what: kind === "samples" ? "samples archive" : "archive", key, path, local: true, check, absent: "" }];
  });
  if (problems.length > 0) throw new Error(`${problems.length} archive(s) are not what prep named, so nothing was uploaded:\n${problems.map((line) => `  ${line}`).join("\n")}`);
  objectTagging(tags);
  requireAws(aws);
  return { uploaded: uploadObjects(aws, plans, log, tags) };
}

/**
 * The publish workflow's confirmation (`confirm --objects`), after every put: for each object prep named, S3's head
 * must find it, with the compressed size prep named when this run put it (`uploaded`; one S3 held before may be
 * compressed otherwise, and the read-back decides), and a fresh download must verify against its lock
 * (verifiedEntries / verifiedSamples). Every object is tried; returns `{ confirmed, failed }`, the keys that verified
 * and `{ key, reason }` for each that did not.
 */
export function confirm({ objects, uploaded = [], aws = runAws, log = console.log } = {}) {
  const named = publishedObjects(objects);
  const put = new Set(uploaded);
  requireAws(aws);
  const scratch = mkdtempSync(join(tmpdir(), "pack-data-confirm-"));
  const confirmed = [];
  const failed = [];
  try {
    for (const { key, compressedBytes, check } of named) {
      try {
        const head = objectHead(aws, key);
        if (head === undefined) throw new Error("S3 does not hold it");
        if (put.has(key) && head.ContentLength !== compressedBytes) throw new Error(`S3 holds ${head.ContentLength} byte(s), not the ${compressedBytes} this run put`);
        const file = join(scratch, "object.tar.zst");
        if (!download(aws, key, file)) throw new Error("S3 does not hold it on download");
        try {
          check(readFileSync(file));
        } finally {
          rmSync(file, { force: true });
        }
        confirmed.push(key);
        log(`confirmed s3://${BUCKET}/${key}`);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        failed.push({ key, reason });
        log(`not confirmed s3://${BUCKET}/${key}: ${reason}`);
      }
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  return { confirmed, failed };
}

/**
 * Maintainer with write credentials (optional; the publish workflow is the routine path): `pack`, then `upload`
 * tagged `tags`, for the selected tool-packs, then the `git add` for the changed locks.
 */
export function push({ root = ROOT, pack: packId, tool, dryRun = false, tags, aws = runAws, log = console.log } = {}) {
  if (!dryRun) objectTagging(tags);
  const { changedLocks } = pack({ root, pack: packId, tool, dryRun, log });
  if (dryRun) return { changedLocks, uploaded: [] };
  const { uploaded } = upload({ root, pack: packId, tool, tags, aws, log });
  if (changedLocks.length > 0) log(`Commit the new lock(s):\n  git add -- ${changedLocks.map(shellQuoted).join(" ")}`);
  return { changedLocks, uploaded };
}

/**
 * CI's cache entry for the selected tool-packs' archives and their samples archives: its key (from the locked sha256s
 * alone) and its paths.
 */
export function archiveCache({ root = ROOT, pack: packId, tool } = {}) {
  const selected = selectToolPacks(root, packId, tool);
  const locked = selected.map((toolPack) => ({ toolPack, lock: readLock(root, toolPack), samplesLock: readSamplesLock(root, toolPack) }));
  const [only] = locked;
  const key = locked.length === 1
    ? `pack-test-data-${only.toolPack.pack}-${only.toolPack.tool}-${only.lock.sha256}${only.samplesLock === undefined ? "" : `-samples-${only.samplesLock.sha256}`}`
    : `pack-test-data-${sha256(Buffer.from(locked.flatMap(({ toolPack, lock, samplesLock }) => [
      `${toolPack.directory} ${lock.sha256}\n`,
      ...(samplesLock === undefined ? [] : [`${samplesLock.label} ${samplesLock.sha256}\n`]),
    ]).join("")))}`;
  const paths = [
    ...locked.flatMap(({ toolPack, lock, samplesLock }) => [
      archiveCachePath(root, toolPack, lock.sha256),
      ...(samplesLock === undefined ? [] : [samplesArchiveCachePath(root, toolPack, samplesLock.sha256)]),
    ]),
  ];
  return { key, paths: paths.map((path) => relative(root, path).split(sep).join("/")) };
}

/**
 * Whether S3 holds every archive the selected tool-packs' test-data.lock files name and every samples archive their
 * samples.lock files name, asked with head-object alone
 * (nothing is downloaded). Throws, listing each missing key with its lock and the remedy, sorted by key, when S3 lacks
 * any.
 */
export function check({ root = ROOT, pack: packId, tool, aws = runAws, log = console.log } = {}) {
  const selected = selectToolPacks(root, packId, tool);
  const wanted = selected.flatMap((toolPack) => {
    const lock = readLock(root, toolPack);
    const archive = archiveKey(toolPack, lock.sha256);
    const samplesLock = readSamplesLock(root, toolPack);
    return [
      {
        key: archive,
        missing: `${lock.label} names s3://${BUCKET}/${archive}, which S3 does not hold: publish the pull request's data through the publish workflow (${PUBLISH_WORKFLOW}), which commits the lock naming what it uploaded`,
      },
      ...(samplesLock === undefined ? [] : [{
        key: samplesKey(toolPack, samplesLock.sha256),
        missing: `${samplesLock.label} names s3://${BUCKET}/${samplesKey(toolPack, samplesLock.sha256)}, which S3 does not hold: ${REPACK_SAMPLES}`,
      }]),
    ];
  });
  requireAws(aws);
  const missing = wanted.filter(({ key }) => !objectExists(aws, key)).sort((a, b) => byBytes(a.key, b.key));
  if (missing.length > 0) {
    throw new Error(`S3 lacks ${missing.length} of the ${wanted.length} object(s) the locks name (CONTRIBUTING.md, "Pack test data"):\n${missing.map(({ missing: line }) => `  ${line}`).join("\n")}`);
  }
  const samplesCount = wanted.filter(({ key }) => key.startsWith(SAMPLES_PREFIX)).length;
  log(`S3 holds every object the locks name: ${wanted.length - samplesCount} archive(s) and ${samplesCount} samples archive(s)`);
  return { checked: wanted.map(({ key }) => key).sort(byBytes) };
}

const USAGE = `usage: node scripts/s3/pack-fixtures.mjs <command> [--pack <id> [--tool <tool>]] [--dry-run]
  pull        fetch, verify and extract the archives the selected tool-packs' test-data.lock files name, and their
              samples (as samples does), into ${ARCHIVE_CACHE}, verified, and lay each tool-pack's ${PUBLISH_DIRECTORY}/ over them
  stage       --pack <id> [--tool <tool>] [--all]: copy the pack test data on disk that the locked archive lacks or
              holds differently into the tool-pack's ${PUBLISH_DIRECTORY}/ (--all: all of it, for a replace publish)
  push        maintainer with write credentials: pack, then upload (tagged)
  pack        archive the selected tool-packs' data on disk into ${ARCHIVE_CACHE} and rewrite their locks (no network)
              pack --from <commit> [--replace] --out <dir> [--manifest <file>] [--summary <file>]: the publish
              workflow's prep: archive every tool-pack's ${PUBLISH_DIRECTORY}/ from the commit's blobs (never checked out or
              run), over the locked archive (--replace: alone), check every file's licence, and write the archives
  upload      put the locked archives from ${ARCHIVE_CACHE} into S3 (tagged) unless it holds them, and read them back
              upload --objects <file> --archives <dir> [--uploaded <file>]: the publish workflow's upload: verify
              prep's archives against the objects it named, put them create-only (tagged), and write the keys put
  confirm     --objects <file> [--uploaded <file>] [--report <file>]: head and download every object prep named and
              verify it against its lock; the report is {"confirmed": <every one verified>, "failed": [...]}
  samples     fetch the samples archive the selected tool-packs' samples.lock files name into ${ARCHIVE_CACHE}/samples/,
              verified, and extract each into ${SAMPLES_TREES}/<pack>/<tool>/<sha256>/
              samples push: maintainer: put those archives from ${ARCHIVE_CACHE}/samples/ into S3 (tagged) unless it
              holds them, read them back, and print the git add for the locks (no --dry-run)
  check       fail, listing each with its remedy, unless S3 holds every archive the selected tool-packs' locks name
              (head-object only; nothing is downloaded)
  cache       CI: the Actions cache key and paths for the selected tool-packs' archives and samples archives, as JSON
Every upload is tagged: --commit (default HEAD), --pr (default none), --run-id (default local) and --uploader (default
the gh CLI's login).`;

/** The tags of a command that uploads: the options given, else HEAD, "none", "local" and the gh CLI's login. */
function commandTags(values) {
  const gh = () => {
    const run = spawnSync("gh", ["api", "user", "--jq", ".login"], { encoding: "utf8" });
    if (run.error === undefined && run.status === 0 && run.stdout.trim() !== "") return run.stdout.trim();
    throw new Error(`every upload is tagged with its uploader's GitHub login, and \`gh api user\` gave none (${lastLine(run.stderr) || run.error?.message || `exit ${run.status}`}): sign in with gh auth login, or pass --uploader <login>`);
  };
  return {
    commit: values.commit ?? runGit(ROOT, ["rev-parse", "HEAD"]).trim(),
    pr: values.pr ?? "none",
    runId: values["run-id"] ?? "local",
    uploader: values.uploader ?? gh(),
  };
}

const readJson = (file) => JSON.parse(readFileSync(file, "utf8"));
const writeJson = (file, value) => {
  if (file !== undefined) writeFileSync(file, `${JSON.stringify(value)}\n`);
};
const SELECT = ["pack", "tool", "dry-run"];
const TAGS = ["commit", "pr", "run-id", "uploader"];

/** Each command's options, and how it runs with the parsed `values`. */
const COMMANDS = {
  pull: { options: SELECT, run: (values) => pull(values) },
  stage: { options: [...SELECT, "all"], run: (values) => stage(values) },
  push: { options: [...SELECT, ...TAGS], run: (values) => push({ ...values, tags: values.dryRun ? undefined : commandTags(values) }) },
  pack: {
    options: [...SELECT, "from", "replace", "out", "manifest", "summary"],
    run: (values) => {
      if (values.from === undefined) {
        if (values.replace || [values.out, values.manifest, values.summary].some((value) => value !== undefined)) throw new Error("--replace, --out, --manifest and --summary need --from");
        return pack(values);
      }
      if (values.pack !== undefined || values.tool !== undefined || values.dryRun) throw new Error("pack --from packs every tool-pack's publish/ and has no --pack, --tool or --dry-run");
      return packFrom(values);
    },
  },
  upload: {
    options: ["pack", "tool", "objects", "archives", "uploaded", ...TAGS],
    run: (values) => {
      if (values.objects === undefined) {
        if (values.archives !== undefined || values.uploaded !== undefined) throw new Error("--archives and --uploaded need --objects");
        return upload({ ...values, tags: commandTags(values) });
      }
      if (values.pack !== undefined || values.tool !== undefined) throw new Error("upload --objects uploads the objects it names and has no --pack or --tool");
      const result = uploadPublished({ objects: readJson(values.objects), archives: values.archives, tags: commandTags(values) });
      writeJson(values.uploaded, result.uploaded);
      return result;
    },
  },
  confirm: {
    options: ["objects", "uploaded", "report"],
    run: (values) => {
      if (values.objects === undefined) throw new Error("confirm needs --objects <file>");
      const { confirmed, failed } = confirm({ objects: readJson(values.objects), uploaded: values.uploaded === undefined ? [] : readJson(values.uploaded) });
      writeJson(values.report, { confirmed: failed.length === 0, objects: confirmed, failed });
      if (failed.length > 0) throw new Error(`${failed.length} object(s) are not confirmed in S3:\n${failed.map(({ key, reason }) => `  ${key}: ${reason}`).join("\n")}`);
    },
  },
  samples: { options: SELECT, run: (values) => samples(values) },
  "samples push": { options: ["pack", "tool", ...TAGS], run: (values) => samplesPush({ ...values, tags: commandTags(values) }) },
  check: { options: ["pack", "tool"], run: (values) => check(values) },
  cache: { options: ["pack", "tool"], run: (values) => console.log(JSON.stringify(archiveCache(values))) },
};

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const string = { type: "string" };
  const flag = { type: "boolean" };
  let parsed;
  try {
    parsed = parseArgs({
      allowPositionals: true,
      strict: true,
      options: {
        pack: string, tool: string, "dry-run": flag, all: flag,
        from: string, replace: flag, out: string, manifest: string, summary: string,
        objects: string, archives: string, uploaded: string, report: string,
        commit: string, pr: string, "run-id": string, uploader: string,
      },
    });
  } catch (error) {
    console.error(`${error instanceof Error ? error.message : String(error)}\n${USAGE}`);
    process.exitCode = 2;
  }
  const name = (parsed?.positionals ?? []).join(" ");
  const command = Object.hasOwn(COMMANDS, name) ? COMMANDS[name] : undefined;
  const foreign = parsed === undefined || command === undefined ? [] : Object.keys(parsed.values).filter((option) => !command.options.includes(option));
  if (parsed !== undefined && command === undefined) {
    console.error(USAGE);
    process.exitCode = 2;
  } else if (foreign.length > 0) {
    console.error(`${name} takes no ${foreign.map((option) => `--${option}`).join(", ")}\n${USAGE}`);
    process.exitCode = 2;
  } else if (parsed !== undefined) {
    const { "dry-run": dryRun = false, all = false, replace = false, ...rest } = parsed.values;
    try {
      await command.run({ ...rest, dryRun, all, replace });
    } catch (error) {
      console.error(`fixtures:${name}: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 1;
    }
  }
}
