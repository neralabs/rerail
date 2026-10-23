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
// - derived: everything else, upstream's tests copied or derived and the tool's recorded answers. A maintainer
//   regenerates it from the tool-pack's pins with `npm run fixtures:derive` (scripts/pack-data/derive.ts) after a pin
//   changes; pull request CI derives only to check that a changed test-data.lock reproduces.
//
// .gitignore covers the patterns, so a pulled copy is untracked and invisible. A contributor checks new or changed data
// into a pull request with `git add -f`; before merge a maintainer archives the tool-pack's data with `fixtures:push`,
// commits the new lock and untracks the data (structure metric #3 fails while any pack test data is tracked, or a tool
// directory has no well-formed lock).
//
//   npm run fixtures:pull [-- --pack <id> [--tool <tool>]] [--dry-run]   fetch the locked archives, verify, extract;
//                                                                          tracked copies (a pull request's) win
//   npm run fixtures:push [-- --pack <id> [--tool <tool>]] [--dry-run]   maintainer: archive the data on disk, upload
//                                                                          the archive unless S3 holds it, rewrite the lock
//   npm run fixtures:push -- --from <ref> [--pack <id> [--tool <tool>]] [--replace] [--into <dir>]
//       maintainer, in a trusted checkout of main: archive a pull request's tracked pack test data from its commit's
//       blobs alone (never checking it out or running its code) over the archive its lock names, less the files its
//       UPSTREAM.md stopped listing as Rerail's (--replace: the tracked data alone, no archive under it), upload, and
//       print the git commands that commit the new locks and untrack the data in a checkout of the pull request (or
//       write the locks into --into <dir>)
//   node scripts/s3/pack-fixtures.mjs recordings [--pack <id> [--tool <tool>]] [--dry-run]
//       put the recordings each recording.lock names into RECORDING_CACHE from S3, verified, and extract its corpus
//       answers archive into RECORDING_CACHE/corpus/<sha256>/
//   node scripts/s3/pack-fixtures.mjs recordings push [--pack <id> [--tool <tool>]]
//       maintainer: put the recordings each recording.lock names from RECORDING_CACHE into S3 unless it holds them, read
//       them back, and print the `git add` for the locks
//   node scripts/s3/pack-fixtures.mjs check [--pack <id> [--tool <tool>]]
//       fail, listing each with its remedy, unless S3 holds every archive and recording the locks name (head-object
//       only)
//
// A tool-pack's two port-readiness recordings, which its recording.lock names, live in the same bucket and are written
// once, never overwritten, so a gate leg that misses its cache can fetch them instead of recording: recording.json at
// s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.json, by the sha256 of its bytes, and upstream's
// answers on the corpus, one file per answer (conformance/port-readiness/recording/corpus-answers.ts), as one canonical
// archive like a test data archive at s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/corpus/<sha256>.tar.zst,
// by its tar's sha256, file count and bytes. A pull keeps the archive at RECORDING_CACHE/<sha256>.tar.zst and the gates
// read the tree it extracts. `upload` (and so `push`) puts the selected tool-packs' recordings there from
// RECORDING_CACHE too.
//
// The consumers (the conformance runner, pack validation, the recorders and generators, the Rust tests) report a
// missing file with missingPackFixture, so a checkout without the data says to pull instead of failing on ENOENT.
// packs/<id>/upstream/<tool>/ keeps the rest in git: gates.yaml, recorders, UPSTREAM.md, goldens.tsv, recording.lock,
// test-data.lock.

import { spawnSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

import { archiveSizeBound, buildArchive, compressArchive, decompressArchive, lockText, parseLock, readArchive, sha256, TEST_DATA_LOCK } from "./test-data-archive.mjs";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
export const BUCKET = "nera-test-fixtures";
export const REGION = "us-east-2";
/** Every archive's key is this prefix plus `<pack>/<tool>/<sha256>.tar.zst`. */
export const ARCHIVE_PREFIX = "rerail/archives/";
/** Where a pull keeps the archives it fetched (and CI restores them from its cache), under the same relative keys. */
export const ARCHIVE_CACHE = ".rerail/pack-data/archives";
/**
 * Every recording's key is this prefix plus `<pack>/<tool>/<sha256>.json` (recording.json, by the sha256 of its bytes)
 * or `<pack>/<tool>/corpus/<sha256>.tar.zst` (the corpus answers archive, by its tar's sha256).
 */
export const RECORDING_PREFIX = "rerail/recordings/";
/**
 * Where the recorder writes recordings and the gates read them: recording.json as `<sha256>.json`, the corpus answers
 * archive as `<sha256>.tar.zst` and the tree extracted from it as `corpus/<sha256>/` (recordingCachePath in
 * conformance/port-readiness/recording/store.ts, corpusArchiveCachePath and corpusTreeDirectory in corpus-answers.ts);
 * CI restores the two files from its cache. The recorder's `--cache <dir>` reads `<dir>/recordings` instead, and
 * `recordings` takes it as `cache`.
 */
export const RECORDING_CACHE = ".rerail/port-readiness/recordings";
/** A tool-pack's manifest of its two recordings (recordingLockSchema in conformance/port-readiness/recording/store.ts). */
export const RECORDING_LOCK = "recording.lock";
/** The recording.lock format this script reads; a lock in another format is recorded again, never read. */
export const RECORDING_LOCK_FORMAT = "rerail.port-readiness-recording-lock/v3";
/** The lock's entry for the recording of upstream's own tests, named by the sha256 and size of its bytes. */
export const RECORDING_FILE = "recording.json";
/**
 * The lock's entry for upstream's answers on the corpus: the canonical tar (test-data-archive.mjs) of one file per
 * answer, named like a test data archive by the tar's sha256, file count and bytes, and stored zstd-compressed.
 */
export const CORPUS_ARCHIVE = "corpus.tar.zst";
/** Every path in a corpus answers archive is under one of these (conformance/port-readiness/recording/corpus-answers.ts). */
const CORPUS_TREE_ROOTS = Object.freeze(["config/", "answers/", "verdicts/"]);

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
  AUTHORED_CORPUS_PATTERN,
]);

/** What every consumer tells a contributor whose checkout lacks the data. */
export const PULL_INSTRUCTION =
  'pack test data is not in git: run `npm run fixtures:pull` (each tool-pack\'s archive, which its test-data.lock pins, from S3) (CONTRIBUTING.md, "Pack test data")';

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
 * What a recorder or generator says after writing pack test data at `paths` (files or directories, in the checkout at
 * `root`): the authored files among them go in a pull request with `git add -f`, and the derived ones never do (a
 * maintainer archives them).
 */
export function packFixturesWritten(paths, root = ROOT) {
  const authored = authoredListing(root);
  const written = [...new Set(paths.flatMap((path) => filesAt(isAbsolute(path) ? path : join(root, path))).map((path) => repositoryPath(path, root)))].sort();
  const own = written.filter((path) => packFixtureKind(path, authored) === "authored");
  const lines = [];
  if (written.length > own.length) {
    lines.push(`Wrote ${written.length - own.length} derived pack test data file(s), which git ignores: a maintainer regenerates them from the pins (\`npm run fixtures:derive\`) and archives them (\`npm run fixtures:push\`), so never commit them (CONTRIBUTING.md, "Pack test data").`);
  }
  if (own.length > 0) {
    lines.push(
      `Wrote ${own.length} authored pack test data file(s) (their UPSTREAM.md lists them as Rerail's), which git ignores (they live in the tool-pack's archive in S3). Put new or changed ones in your pull request with`,
      `  git add -f -- ${own.map(shellQuoted).join(" ")}`,
      `and a maintainer archives them before merge (CONTRIBUTING.md, "Pack test data").`,
    );
  }
  return lines.join("\n");
}

/** packFixturesWritten for a recording at `path`: a fixtures recording names the case sources beside it too. */
export function recordingWritten(path, kind) {
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

/** The tracked pack test data under `paths` (repository paths) of the checkout at `root`, sorted. */
export function trackedPackFixtures(root = ROOT, paths = ["packs"]) {
  return nulList(runGit(root, ["ls-files", "-z", "--", ...paths])).filter(isPackFixture).sort(byBytes);
}

/** Tracked pack test data under `paths` whose working-tree copy differs from the index (edited or deleted), sorted. */
function editedPackFixtures(root, paths) {
  // A refreshed index compares content, not timestamps; it exits 1 when files differ, which ls-files then lists.
  spawnSync("git", ["update-index", "-q", "--refresh"], { cwd: root });
  return nulList(runGit(root, ["ls-files", "-z", "-m", "--", ...paths])).filter(isPackFixture).sort(byBytes);
}

/**
 * The working-tree state a pull keeps under `paths`: every tracked file's committed (index) content, except tracked
 * files edited or deleted in the working tree, which keep their edit. `snapshot` runs before the extraction and
 * `restore` after it.
 */
export function trackedSnapshot(root = ROOT, paths = ["packs"]) {
  const tracked = trackedPackFixtures(root, paths);
  const edited = new Map();
  for (const path of editedPackFixtures(root, paths)) {
    const absolute = join(root, path);
    edited.set(path, existsSync(absolute) ? readFileSync(absolute) : null);
  }
  return {
    tracked,
    edited,
    restore() {
      const unedited = tracked.filter((path) => !edited.has(path));
      if (unedited.length > 0) runGit(root, ["checkout", "--pathspec-from-file=-", "--pathspec-file-nul", "--"], `${unedited.join("\0")}\0`);
      for (const [path, bytes] of edited) {
        const absolute = join(root, path);
        if (bytes === null) rmSync(absolute, { force: true });
        else {
          mkdirSync(dirname(absolute), { recursive: true });
          writeFileSync(absolute, bytes);
        }
      }
      return { restored: unedited.length, kept: edited.size };
    },
  };
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
    throw new Error(`${label} is missing: a maintainer archives the tool-pack's test data with \`npm run fixtures:push -- --pack ${toolPack.pack} --tool ${toolPack.tool}\`, which writes it (CONTRIBUTING.md, "Pack test data")`);
  }
  return { label, ...parseLock(readFileSync(join(root, label), "utf8"), label) };
}

/** A tool-pack's recording's S3 key, for the recording's own sha256 `digest`. */
export function recordingKey({ pack, tool }, digest) {
  return `${RECORDING_PREFIX}${pack}/${tool}/${digest}.json`;
}

/** The paths a recording cache directory (RECORDING_CACHE, or the recorder's `<--cache>/recordings`) keeps recordings at. */
function cachePaths(cache) {
  return {
    recording: (digest) => join(cache, `${digest}.json`),
    corpus: (digest) => join(cache, `${digest}.tar.zst`),
    tree: (digest) => join(cache, "corpus", digest),
  };
}

/** Where the checkout at `root` keeps the recording whose sha256 is `digest` (RECORDING_CACHE/<sha256>.json). */
export function recordingCacheFile(root, digest) {
  return cachePaths(join(root, RECORDING_CACHE)).recording(digest);
}

/** A tool-pack's corpus answers archive's S3 key, for its tar's sha256 `digest`. */
export function corpusKey({ pack, tool }, digest) {
  return `${RECORDING_PREFIX}${pack}/${tool}/corpus/${digest}.tar.zst`;
}

/** Where the checkout at `root` keeps the corpus answers archive whose tar's sha256 is `digest` (RECORDING_CACHE/<sha256>.tar.zst). */
export function corpusCacheFile(root, digest) {
  return cachePaths(join(root, RECORDING_CACHE)).corpus(digest);
}

/** Where the checkout at `root` keeps that archive extracted, the tree the gates and the recorder read (RECORDING_CACHE/corpus/<sha256>/). */
export function corpusTreeDirectory(root, digest) {
  return cachePaths(join(root, RECORDING_CACHE)).tree(digest);
}

const SHA256_HEX = /^[0-9a-f]{64}$/u;
const isSha256 = (value) => typeof value === "string" && SHA256_HEX.test(value);
const isCount = (value) => Number.isSafeInteger(value) && value >= 0;

/**
 * What a recording.lock's `text` names: `recording`, as `{ name, sha256, size }`, and `corpus`, the archive as
 * `{ name, label, sha256, files, bytes }` (a lock for verifiedCorpus); throws, naming `label` (the lock's repository
 * path), on a lock of another format or one that names anything else. The lock's full schema is recordingLockSchema in
 * conformance/port-readiness/recording/store.ts; this reads only what fetching and uploading the recordings need.
 */
export function lockedRecordings(text, label) {
  let value;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new Error(`${label} is not JSON (${error instanceof Error ? error.message : String(error)})`);
  }
  const format = value !== null && typeof value === "object" ? value.format : undefined;
  if (format !== RECORDING_LOCK_FORMAT) {
    throw new Error(`${label} is format ${JSON.stringify(format)}, not ${RECORDING_LOCK_FORMAT}: record the tool-pack again (npm run port-readiness:record), which writes it`);
  }
  const files = value.files;
  const names = files !== null && typeof files === "object" && !Array.isArray(files) ? Object.keys(files).sort(byBytes) : [];
  if (names.length !== 2 || names[0] !== CORPUS_ARCHIVE || names[1] !== RECORDING_FILE) {
    throw new Error(`${label} is malformed: its "files" must name exactly ${CORPUS_ARCHIVE} and ${RECORDING_FILE}, not ${names.join(", ") || "nothing"}`);
  }
  const recording = files[RECORDING_FILE];
  if (!isSha256(recording?.sha256) || !Number.isSafeInteger(recording.size) || recording.size <= 0) {
    throw new Error(`${label} is malformed: files.${RECORDING_FILE} needs a sha256 of 64 lowercase hex digits and a positive size`);
  }
  const corpus = files[CORPUS_ARCHIVE];
  if (!isSha256(corpus?.sha256) || !isCount(corpus.files) || !isCount(corpus.bytes)) {
    throw new Error(`${label} is malformed: files.${CORPUS_ARCHIVE} needs a sha256 of 64 lowercase hex digits, a file count and a byte total`);
  }
  return {
    recording: { name: RECORDING_FILE, sha256: recording.sha256, size: recording.size },
    corpus: { name: CORPUS_ARCHIVE, label, sha256: corpus.sha256, files: corpus.files, bytes: corpus.bytes },
  };
}

/** A tool-pack's recording.lock in the checkout at `root`, as `{ label, recording, corpus }`; undefined when it has none. */
export function readRecordingLock(root, toolPack) {
  const label = `${toolPack.directory}/${RECORDING_LOCK}`;
  if (!existsSync(join(root, label))) return undefined;
  return { label, ...lockedRecordings(readFileSync(join(root, label), "utf8"), label) };
}

/** Undefined when `bytes` are the locked `recording`; otherwise what they hold instead. */
function recordingProblem(bytes, recording) {
  const digest = sha256(bytes);
  if (digest === recording.sha256 && bytes.length === recording.size) return undefined;
  return `holds ${bytes.length} byte(s) whose sha256 is ${digest}, not the locked ${recording.size} byte(s) whose sha256 is ${recording.sha256}`;
}

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
 * verifiedArchive for the corpus answers archive a recording.lock names (`lock`, lockedRecordings' `corpus`): every
 * entry must be under the tree's config/, answers/ or verdicts/. readArchive already refuses two paths that differ only
 * by case, which a case-insensitive file system (macOS's) would extract as one file (checkTreePaths in corpus-answers.ts
 * checks the same).
 */
export function verifiedCorpus(compressed, lock, key) {
  return verifiedArchive(compressed, lock, key, (path) => CORPUS_TREE_ROOTS.some((root) => path.startsWith(root)), `corpus answers (under ${CORPUS_TREE_ROOTS.join(", ")})`);
}

/**
 * The file an extracted corpus answers tree holds besides the archive's: the tar's sha256 and a newline, written into
 * the partial directory last, so a tree that has it was extracted whole from a verified copy of that archive.
 * corpus-answers.ts reads and writes trees through corpusTreeReady and extractCorpusTree too.
 */
export const CORPUS_TREE_MARKER = ".tar-sha256";

/**
 * Whether `directory` is the extracted tree of the archive `lock` (`{ sha256, files, bytes }`) names: its marker names
 * that tar, and it still holds as many regular files and bytes besides the marker (none deleted or cut short since).
 */
export function corpusTreeReady(directory, lock) {
  const marker = join(directory, CORPUS_TREE_MARKER);
  if (!existsSync(marker) || readFileSync(marker, "utf8") !== `${lock.sha256}\n`) return false;
  let files = 0;
  let bytes = 0;
  for (const entry of readdirSync(directory, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile() || (entry.parentPath === directory && entry.name === CORPUS_TREE_MARKER)) continue;
    files += 1;
    bytes += statSync(join(entry.parentPath, entry.name)).size;
  }
  return files === lock.files && bytes === lock.bytes;
}

/**
 * Writes verified `entries` of the archive `lock` names, and the marker, into this process's own partial directory,
 * then renames it into place as `directory`, so a tree is never half there. Another process that extracted the same
 * archive first wins: its tree is this one. A tree there that is not that archive's is replaced.
 */
export function extractCorpusTree(directory, entries, lock) {
  const partial = `${directory}.${process.pid}.partial`;
  rmSync(partial, { recursive: true, force: true });
  try {
    mkdirSync(partial, { recursive: true });
    for (const entry of entries) {
      const absolute = join(partial, entry.path);
      mkdirSync(dirname(absolute), { recursive: true });
      writeFileSync(absolute, entry.bytes);
    }
    writeFileSync(join(partial, CORPUS_TREE_MARKER), `${lock.sha256}\n`);
    for (let attempt = 1; ; attempt += 1) {
      try {
        renameSync(partial, directory);
        return;
      } catch (error) {
        if (corpusTreeReady(directory, lock)) return;
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

/** Whether S3 holds `key`. */
function objectExists(aws, key) {
  const run = aws(["s3api", "head-object", "--bucket", BUCKET, "--key", key, "--region", REGION]);
  if (run.error !== undefined) throw run.error;
  if (run.status === 0) return true;
  if (NOT_FOUND.test(run.stderr ?? "")) return false;
  throw new Error(`aws s3api head-object ${key} failed: ${lastLine(run.stderr) || `exit ${run.status}`}`);
}

/** Uploads `file` to `key` unless S3 holds the key already (a conditional write: an object is never overwritten). */
function putObject(aws, key, file) {
  const run = aws(["s3api", "put-object", "--bucket", BUCKET, "--key", key, "--body", file, "--if-none-match", "*", "--region", REGION, "--output", "json"]);
  if (run.error !== undefined) throw run.error;
  if (run.status === 0) return true;
  if (/PreconditionFailed|\(412\)/u.test(run.stderr ?? "")) return false;
  throw new Error(`aws s3api put-object ${key} failed: ${lastLine(run.stderr) || `exit ${run.status}`}`);
}

/** The narrowest paths whose pack test data `git rm -r --cached` removes for `tracked`: each file's shortest covering ancestor. */
function untrackPaths(tracked) {
  const covers = new Set();
  for (const path of tracked) {
    const segments = path.split("/");
    for (let length = 5; length <= segments.length; length += 1) {
      const candidate = segments.slice(0, length).join("/");
      if (length === segments.length || isPackFixture(`${candidate}/-`)) {
        covers.add(candidate);
        break;
      }
    }
  }
  return [...covers].sort(byBytes);
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
      throw new Error(`${lock.label} names s3://${BUCKET}/${key}, which S3 does not hold: a maintainer uploads it with \`npm run fixtures:push -- --pack ${toolPack.pack} --tool ${toolPack.tool}\` from the data the lock was written for (CONTRIBUTING.md, "Pack test data")`);
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
 * Fetches the archive each selected tool-pack's test-data.lock names (from ARCHIVE_CACHE when a verified copy is there,
 * otherwise from S3 into it), verifies it against the lock, and extracts it into the tool directory. Every archive is
 * verified before anything is extracted, and every tool directory and directory an entry goes in, from the checkout
 * down, must be no symlink (nor may any pack test data in the tool directory be one). Untracked derived files the
 * archive lacks are deleted (a stale derivation); untracked authored ones are kept and reported. Then each tracked file
 * gets its own content back (its index copy, or the working-tree edit of one being edited), so a pull request's files
 * win over the archive's.
 */
export function pull({ root = ROOT, pack, tool, dryRun = false, aws = runAws, log = console.log } = {}) {
  const selected = selectToolPacks(root, pack, tool);
  const locked = selected.map((toolPack) => ({ toolPack, lock: readLock(root, toolPack) }));
  let signedIn = false;
  const discarded = [];
  const fetched = [];
  for (const { toolPack, lock } of locked) {
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
      if (!signedIn) {
        try {
          requireAws(aws);
        } catch (error) {
          // CI restores the archives from its cache with no credentials: a cached archive that is not the locked one is
          // a poisoned or corrupt cache entry, which the next run would restore again.
          if (discarded.length === 0) throw error;
          throw new Error(`${discarded.join(", ")} was not the archive its lock names (a corrupt or poisoned cache entry) and was deleted, and S3 cannot be read here: ${error instanceof Error ? error.message : String(error)}. In CI, delete the Actions cache entry that restored it (gh cache delete <key>) and run the job again.`);
        }
      }
      signedIn = true;
      entries = downloadArchive(root, aws, toolPack, lock);
    }
    fetched.push({ toolPack, lock, entries, source });
  }
  if (dryRun) {
    log(`dry run: ${fetched.length} of ${locked.length} archive(s) are in ${ARCHIVE_CACHE}; nothing was extracted`);
    return { toolPacks: [], restored: 0, kept: 0 };
  }

  // No write may go through a symlink: every check runs before the first write.
  const onDisk = new Map();
  const directories = new Set();
  for (const { toolPack, entries } of fetched) {
    onDisk.set(toolPack.directory, toolPackFiles(root, toolPack.directory));
    for (const entry of entries) {
      const path = `${toolPack.directory}/${entry.path}`;
      directories.add(path.slice(0, path.lastIndexOf("/")));
    }
  }
  for (const directory of [...directories].sort(byBytes)) refuseSymlinkedDirectories(root, directory);

  const snapshot = trackedSnapshot(root, selected.map((toolPack) => toolPack.directory));
  const tracked = new Set(snapshot.tracked);
  const authored = authoredListing(root);
  for (const { toolPack, lock, entries, source } of fetched) {
    const archived = new Set(entries.map((entry) => entry.path));
    const stale = onDisk.get(toolPack.directory).filter((path) => !archived.has(path) && !tracked.has(`${toolPack.directory}/${path}`));
    const kept = stale.filter((path) => packFixtureKind(`${toolPack.directory}/${path}`, authored) === "authored");
    for (const path of stale) if (!kept.includes(path)) rmSync(join(root, toolPack.directory, path));
    for (const entry of entries) {
      const absolute = join(root, toolPack.directory, entry.path);
      mkdirSync(dirname(absolute), { recursive: true });
      writeFileSync(absolute, entry.bytes);
    }
    const notes = [
      ...(stale.length > kept.length ? [`deleted ${stale.length - kept.length} stale derived file(s) the archive lacks`] : []),
      ...(kept.length > 0 ? [`kept ${kept.length} untracked authored file(s) the archive lacks (check them into your pull request with git add -f, or delete them): ${kept.slice(0, 5).join(", ")}`] : []),
    ];
    log(`${toolPack.directory}: ${entries.length} file(s) from archive ${lock.sha256.slice(0, 12)} (${source})${notes.length > 0 ? `; ${notes.join("; ")}` : ""}`);
  }
  const result = snapshot.restore();
  if (snapshot.tracked.length > 0) log(`${result.restored} tracked file(s) keep their committed content and ${result.kept} their working-tree edit`);
  return { toolPacks: fetched.map(({ toolPack, source }) => ({ ...toolPack, source })), ...result };
}

/**
 * Puts the recordings each selected tool-pack's recording.lock names into the recording cache (`cache`, by default
 * RECORDING_CACHE), verified, and extracts the corpus answers archive into `<cache>/corpus/<sha256>/`. recording.json
 * is verified against its sha256 and size; the archive as a pull verifies a test data archive (verifiedCorpus), and
 * only then extracted (extractCorpusTree). A valid local copy is kept, and a tree whose marker names the archive needs
 * no archive; anything else is fetched from S3 through a partial file that must verify first (an invalid local copy is
 * deleted, except on a dry run, which deletes and writes nothing). A recording S3 does not hold is a note, not a
 * failure: CI records the tool-pack on a cache miss. aws is needed only when something must be fetched.
 */
export function recordings({ root = ROOT, pack: packId, tool, cache = join(root, RECORDING_CACHE), dryRun = false, aws = runAws, log = console.log } = {}) {
  const paths = cachePaths(cache);
  const shown = (path) => relative(root, path).split(sep).join("/");
  const kept = [];
  const downloaded = [];
  const missing = [];
  const extracted = [];
  const discarded = [];
  let signedIn = false;
  const signIn = () => {
    if (signedIn) return;
    try {
      requireAws(aws);
    } catch (error) {
      // As in pull: CI restores the recordings from its cache with no credentials, so a cached copy that is not the
      // locked one is a poisoned or corrupt cache entry, which the next run would restore again.
      if (discarded.length === 0) throw error;
      throw new Error(`${discarded.join(", ")} was not the recording its lock names (a corrupt or poisoned cache entry) and was deleted, and S3 cannot be read here: ${error instanceof Error ? error.message : String(error)}. In CI, delete the Actions cache entry that restored it (gh cache delete <key>) and run the job again.`);
    }
    signedIn = true;
  };
  /** Downloads `key` into `file` through this process's partial file `check` must accept first; false, logging `absent`, when S3 does not hold it. */
  const fetch = (key, file, check, absent) => {
    signIn();
    mkdirSync(dirname(file), { recursive: true });
    const partial = `${file}.${process.pid}.partial`;
    rmSync(partial, { force: true });
    try {
      if (!download(aws, key, partial)) {
        missing.push(key);
        log(absent);
        return false;
      }
      check(readFileSync(partial));
      renameSync(partial, file);
      downloaded.push(key);
      return true;
    } finally {
      rmSync(partial, { force: true });
    }
  };
  for (const toolPack of selectToolPacks(root, packId, tool)) {
    const locked = readRecordingLock(root, toolPack);
    if (locked === undefined) continue;
    const record = `npm run port-readiness:record -- --pack ${toolPack.pack} --tool ${toolPack.tool}`;
    const { recording, corpus } = locked;

    const key = recordingKey(toolPack, recording.sha256);
    const file = paths.recording(recording.sha256);
    const fileProblem = existsSync(file) ? recordingProblem(readFileSync(file), recording) : "is not there";
    if (fileProblem === undefined) {
      kept.push(key);
    } else if (dryRun) {
      log(`${existsSync(file) ? `would delete ${shown(file)}, which ${fileProblem}, and ` : ""}would fetch s3://${BUCKET}/${key} (${locked.label}: ${recording.name})`);
    } else {
      if (existsSync(file)) {
        rmSync(file);
        discarded.push(shown(file));
      }
      const check = (bytes) => {
        const problem = recordingProblem(bytes, recording);
        if (problem !== undefined) throw new Error(`${locked.label}: s3://${BUCKET}/${key} (${recording.name}) ${problem}; nothing was written`);
      };
      fetch(key, file, check, `${locked.label}: ${recording.name} ${recording.sha256} is in neither ${shown(cache)}/ nor s3://${BUCKET}/${key}; CI records the tool-pack on a cache miss, or record it here with \`${record} --full\``);
    }

    const archive = corpusKey(toolPack, corpus.sha256);
    const tree = paths.tree(corpus.sha256);
    if (corpusTreeReady(tree, corpus)) {
      kept.push(archive);
      continue;
    }
    const cached = paths.corpus(corpus.sha256);
    const fromCache = cachedEntries(root, cached, corpus, (compressed) => verifiedCorpus(compressed, corpus, archive), dryRun);
    if (fromCache.discarded !== undefined) discarded.push(fromCache.discarded);
    let entries = fromCache.entries;
    if (entries !== undefined) {
      kept.push(archive);
    } else if (dryRun) {
      log(`${fromCache.discarded === undefined ? "" : `would delete ${fromCache.discarded}, which is not the archive ${locked.label} names, and `}would fetch s3://${BUCKET}/${archive} (${locked.label}: ${corpus.name}) and extract it into ${shown(tree)}`);
      continue;
    } else {
      const absent = `${locked.label}: ${corpus.name} ${corpus.sha256} is in neither ${shown(cache)}/ nor s3://${BUCKET}/${archive}; CI records the tool-pack on a cache miss, or record its corpus here from no answers with \`${record} --corpus --full\``;
      if (!fetch(archive, cached, (bytes) => (entries = verifiedCorpus(bytes, corpus, archive)), absent)) continue;
    }
    if (dryRun) {
      log(`would extract ${shown(cached)} into ${shown(tree)}`);
      continue;
    }
    extractCorpusTree(tree, entries, corpus);
    extracted.push(shown(tree));
  }
  log(`${dryRun ? "dry run: " : ""}${shown(cache)}: ${kept.length} recording(s) kept, ${downloaded.length} fetched from S3, ${missing.length} in neither; ${extracted.length} corpus answers tree(s) extracted`);
  return { kept: kept.sort(byBytes), downloaded: downloaded.sort(byBytes), missing: missing.sort(byBytes), extracted: extracted.sort(byBytes) };
}

/**
 * Archives each selected tool-pack's test data as it is on disk into ARCHIVE_CACHE and rewrites its test-data.lock; no
 * network. It refuses on tracked pack test data edited in the working tree, whose archive would not be what the pull
 * request committed, and on a symlink among the data or on the way to its tool directory (toolPackFiles), which it
 * would otherwise leave out or read through. The bump workflow (port-readiness-bump.yml) packs this way, without
 * credentials; `upload` then puts the archives in S3.
 */
export function pack({ root = ROOT, pack: packId, tool, dryRun = false, log = console.log } = {}) {
  const selected = selectToolPacks(root, packId, tool);
  const edited = editedPackFixtures(root, selected.map((toolPack) => toolPack.directory));
  if (edited.length > 0) {
    throw new Error(`tracked pack test data differs from the index in the working tree; commit or restore it first, so the archive holds what the pull request committed:\n${edited.map((path) => `  ${path}`).join("\n")}`);
  }
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
 * What uploading the two recordings a recording.lock (`locked`, readRecordingLock's `{ label, recording, corpus }`)
 * names asks of the checkout at `root`: one plan each for uploadObjects, with its local copy in RECORDING_CACHE when
 * there is one. A local copy that is not what the lock names is refused, so nothing else is ever put under its key.
 */
function recordingPlans(root, toolPack, locked) {
  const { recording, corpus } = locked;
  const record = `npm run port-readiness:record -- --pack ${toolPack.pack} --tool ${toolPack.tool}`;
  const fromCi = `download the pull request's CI recordings with \`gh run download <run-id> -n recordings-${toolPack.pack}--${toolPack.tool} -D ${RECORDING_CACHE}\``;

  const key = recordingKey(toolPack, recording.sha256);
  const path = recordingCacheFile(root, recording.sha256);
  const local = existsSync(path);
  const localProblem = local ? recordingProblem(readFileSync(path), recording) : undefined;
  if (localProblem !== undefined) {
    throw new Error(`${locked.label}: ${relative(root, path)} (${recording.name}, for s3://${BUCKET}/${key}) ${localProblem}, so it is not the recording the lock names; delete it and record or download it again. Nothing was uploaded`);
  }
  const check = (bytes) => {
    const problem = recordingProblem(bytes, recording);
    if (problem !== undefined) throw new Error(`${locked.label}: s3://${BUCKET}/${key} (${recording.name}) ${problem}`);
  };
  const absent = `${locked.label} names ${recording.name} ${recording.sha256}, which is in neither ${RECORDING_CACHE}/${recording.sha256}.json nor s3://${BUCKET}/${key}: record it with \`${record}\`, or ${fromCi}`;

  const archive = corpusKey(toolPack, corpus.sha256);
  const archivePath = corpusCacheFile(root, corpus.sha256);
  const archiveLocal = existsSync(archivePath);
  const archiveCheck = (bytes) => {
    verifiedCorpus(bytes, corpus, archive);
  };
  if (archiveLocal) {
    try {
      archiveCheck(readFileSync(archivePath));
    } catch (error) {
      throw new Error(`${relative(root, archivePath)} is not the ${corpus.name} ${locked.label} names (${error instanceof Error ? error.message : String(error)}); delete it and record the corpus again with \`${record} --corpus\`, or download it again. Nothing was uploaded`);
    }
  }
  const archiveAbsent = `${locked.label} names ${corpus.name} ${corpus.sha256}, which is in neither ${RECORDING_CACHE}/${corpus.sha256}.tar.zst nor s3://${BUCKET}/${archive}: record it with \`${record} --corpus\`, or ${fromCi}`;

  return [
    { directory: toolPack.directory, label: locked.label, what: recording.name, key, path, local, check, absent },
    { directory: toolPack.directory, label: locked.label, what: corpus.name, key: archive, path: archivePath, local: archiveLocal, check: archiveCheck, absent: archiveAbsent },
  ];
}

/**
 * Puts each plan's local copy into S3 unless S3 holds its key already (a conditional write: an object is never
 * overwritten), then reads back what S3 holds and checks it as the plan says. A plan with no local copy needs S3 to hold
 * its key, and every plan is checked for that before anything is put. Returns the keys put, sorted.
 */
function uploadObjects(aws, plans, log) {
  const sorted = [...plans].sort((a, b) => byBytes(a.key, b.key));
  const absent = sorted.filter((plan) => !plan.local && !objectExists(aws, plan.key));
  if (absent.length > 0) throw new Error(`${absent.map((plan) => plan.absent).join("\n")}\nNothing was uploaded.`);
  const uploaded = [];
  for (const plan of sorted) {
    const put = plan.local && !objectExists(aws, plan.key) && putObject(aws, plan.key, plan.path);
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
 * Puts each selected tool-pack's locked archive from ARCHIVE_CACHE, and the recordings its recording.lock names from
 * RECORDING_CACHE, into S3 unless S3 holds their keys already, then reads each back and verifies it against its lock.
 * An archive or recording with no local copy is fine when S3 holds it (and verifies); one in neither place fails the
 * upload, before anything is put.
 */
export function upload({ root = ROOT, pack: packId, tool, aws = runAws, log = console.log } = {}) {
  const selected = selectToolPacks(root, packId, tool);
  const plans = [
    ...archivePlans(root, selected.map((toolPack) => ({ toolPack, lock: readLock(root, toolPack) }))),
    ...lockedRecordingPlans(root, selected).flatMap(({ plans: recorded }) => recorded),
  ];
  requireAws(aws);
  return { uploaded: uploadObjects(aws, plans, log) };
}

/** recordingPlans for each of `selected` (tool-packs) that has a recording.lock, as `{ label, plans }`. */
function lockedRecordingPlans(root, selected) {
  return selected.flatMap((toolPack) => {
    const locked = readRecordingLock(root, toolPack);
    return locked === undefined ? [] : [{ label: locked.label, plans: recordingPlans(root, toolPack, locked) }];
  });
}

/**
 * Maintainer: `upload`'s recordings alone. Puts the recording.json and the corpus answers archive each selected
 * tool-pack's recording.lock names from RECORDING_CACHE into S3 unless S3 holds them already (the same conditional put,
 * so an object is never overwritten), reads each back and verifies it against the lock, then prints the `git add` that
 * commits the locks. Run again, it puts nothing ("S3 already holds"). One in neither RECORDING_CACHE nor S3 fails it,
 * before anything is put.
 */
export function recordingsPush({ root = ROOT, pack: packId, tool, dryRun = false, aws = runAws, log = console.log } = {}) {
  if (dryRun) throw new Error("recordings push has no --dry-run");
  const locked = lockedRecordingPlans(root, selectToolPacks(root, packId, tool));
  if (locked.length === 0) throw new Error(`no selected tool-pack has a ${RECORDING_LOCK}: record one with \`npm run port-readiness:record -- --pack <id> --tool <tool>\``);
  requireAws(aws);
  const uploaded = uploadObjects(aws, locked.flatMap(({ plans }) => plans), log);
  const locks = locked.map(({ label }) => label);
  log(`S3 holds the recordings the lock(s) name; commit them:\n  git add -- ${locks.map(shellQuoted).join(" ")}`);
  return { uploaded, locks };
}

/** A tool directory's file whose change selects its tool-pack for push --from: a lock, or the UPSTREAM.md listing its authored files. */
const SELECTING_FILE = /^packs\/[^/]+\/upstream\/[^/]+\/(?:test-data\.lock|recording\.lock|UPSTREAM\.md)$/u;
const TOOL_DIRECTORY = /^packs\/([^/]+)\/upstream\/([^/]+)\//u;

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

/** A blob's bytes by object id, exactly as committed: cat-file applies no filter, textconv or attribute. */
function gitBlob(root, oid) {
  const run = spawnSync("git", ["cat-file", "blob", oid], { cwd: root, maxBuffer: 1 << 30 });
  if (run.error !== undefined) throw run.error;
  if (run.status !== 0) throw new Error(`git cat-file blob ${oid} failed: ${run.stderr.toString("utf8").trim()}`);
  return run.stdout;
}

/**
 * The file at repository path `path` in the checkout `into`, as `{ target, bytes }` (bytes undefined when there is
 * none), refusing a missing or symlinked directory on the way and anything there but a regular file. It reads files
 * only: no git command runs in that checkout, whose configuration a pull request's code may have changed if it ever ran
 * there.
 */
function intoFile(into, path) {
  const segments = path.split("/");
  let directory = resolve(into);
  for (const segment of segments.slice(0, -1)) {
    directory = join(directory, segment);
    const stat = lstatSync(directory, { throwIfNoEntry: false });
    if (stat?.isDirectory() !== true) {
      const what = stat === undefined ? "does not exist" : stat.isSymbolicLink() ? "is a symlink" : "is not a directory";
      throw new Error(`--into ${into}: ${relative(resolve(into), directory)} ${what} there, so ${path} cannot be read or written through it`);
    }
  }
  const target = join(directory, segments.at(-1));
  const stat = lstatSync(target, { throwIfNoEntry: false });
  if (stat !== undefined && !stat.isFile()) throw new Error(`--into ${into}: ${path} is not a regular file there; nothing is read or written through it`);
  return { target, bytes: stat === undefined ? undefined : readFileSync(target) };
}

/**
 * push --from: archives a pull request's tracked pack test data from the commit `from` names, reading nothing of it but
 * the blobs of its pack test data, UPSTREAM.md, test-data.lock and recording.lock files (git plumbing in the trusted
 * checkout at `root`; the commit is never checked out and none of its code runs). Per selected tool-pack, the archive is
 * the one the commit's test-data.lock names, less the files the merge base's UPSTREAM.md listed as Rerail's and the
 * commit's no longer lists, overlaid with the commit's tracked pack test data; with `replace`, it is the commit's
 * tracked pack test data of the tool directory alone. Each archive's log names the files it drops from the one the lock
 * named. It is uploaded with the recordings the commit's recording.lock names. Then come the git commands that finish
 * the pull request in a checkout of it: write the new locks (or, with `into`, a checkout of that commit, they are
 * written there), `git add` them, and untrack the data.
 */
function pushFrom({ root, from, pack: packId, tool, into, replace, aws, log }) {
  if (tool !== undefined && packId === undefined) throw new Error("--tool needs --pack");
  if (from === "" || from.startsWith("-")) throw new Error(`--from ${JSON.stringify(from)} is not a git revision`);
  let commit;
  try {
    commit = runGit(root, ["rev-parse", "--verify", "--quiet", `${from}^{commit}`]).trim();
  } catch {
    throw new Error(`--from ${from} is no commit in ${root}; fetch the pull request first (git fetch origin pull/<n>/head:pr-<n>)`);
  }
  const at = `${from} (${commit.slice(0, 12)})`;
  const tree = treeEntries(root, commit);
  /** The bytes of the regular file at `path` in `entries`, the tree of the commit `where` describes. */
  const blobIn = (entries, where, path) => {
    const { mode, oid } = entries.get(path);
    if (mode !== "100644" && mode !== "100755") throw new Error(`${path} at ${where} is not a regular file (mode ${mode}); pack test data, records and locks are regular files`);
    return gitBlob(root, oid);
  };
  const blob = (path) => blobIn(tree, at, path);
  let changed = new Set();
  let mergeBase;
  let mergeBaseProblem;
  try {
    mergeBase = runGit(root, ["merge-base", "HEAD", commit]).trim();
    const paths = nulList(runGit(root, ["diff", "--name-only", "-z", "--no-renames", "--no-ext-diff", "--no-textconv", mergeBase, commit, "--", "packs"]));
    changed = new Set(paths.filter((path) => SELECTING_FILE.test(path)));
  } catch (error) {
    mergeBase = undefined;
    mergeBaseProblem = error instanceof Error ? error.message : String(error);
    if (packId === undefined) {
      throw new Error(`which tool-packs ${at} changes is unknown without its merge base with HEAD (${mergeBaseProblem}); name them with --pack and --tool`);
    }
  }
  let baseTree;
  /** The authored files the tool directory's UPSTREAM.md lists at the merge base and no longer lists at the commit. */
  const unlisted = (toolPack) => {
    if (mergeBase === undefined) {
      throw new Error(`which UPSTREAM.md rows ${at} removes is unknown without its merge base with HEAD (${mergeBaseProblem}), so its archive cannot drop their files; rerun with --replace to archive exactly its tracked pack test data`);
    }
    baseTree ??= treeEntries(root, mergeBase);
    const record = `${toolPack.directory}/UPSTREAM.md`;
    const listing = (entries, where) => (entries.has(record) ? listedAuthored(record, blobIn(entries, where, record).toString("utf8")) : []);
    const kept = new Set(listing(tree, at));
    return new Set(listing(baseTree, `the merge base ${mergeBase.slice(0, 12)}`).filter((path) => !kept.has(path) && path.startsWith(`${toolPack.directory}/`)));
  };
  const tracked = [...tree.keys()].filter(isPackFixture).sort(byBytes);
  const found = new Map();
  for (const path of tree.keys()) {
    const match = TOOL_DIRECTORY.exec(path);
    if (match === null) continue;
    if ([match[1], match[2]].some((segment) => segment === "." || segment === "..")) throw new Error(`${path} at ${at} has a "." or ".." segment`);
    const directory = `packs/${match[1]}/upstream/${match[2]}`;
    found.set(directory, { pack: match[1], tool: match[2], directory });
  }
  const owns = (toolPack) => (path) => path.startsWith(`${toolPack.directory}/`);
  const candidates = [...found.values()].sort((a, b) => byBytes(a.directory, b.directory));
  const selected = packId === undefined
    ? candidates.filter((toolPack) => tracked.some(owns(toolPack)) || [TEST_DATA_LOCK, RECORDING_LOCK, "UPSTREAM.md"].some((name) => changed.has(`${toolPack.directory}/${name}`)))
    : candidates.filter((toolPack) => toolPack.pack === packId && (tool === undefined || toolPack.tool === tool));
  if (selected.length === 0) {
    if (packId !== undefined) throw new Error(`no tool-pack packs/${packId}/upstream/${tool ?? "*"}/ at ${at}`);
    log(`${at} tracks no pack test data and changes no ${TEST_DATA_LOCK}, ${RECORDING_LOCK} or UPSTREAM.md against its merge base with HEAD: nothing to push`);
    return { changedLocks: [], uploaded: [], tracked: [], dropped: [], script: "" };
  }
  const intoRoot = into === undefined ? undefined : resolve(into);

  requireAws(aws);
  const builds = [];
  const dropped = [];
  for (const toolPack of selected) {
    const label = `${toolPack.directory}/${TEST_DATA_LOCK}`;
    const before = tree.has(label) ? blob(label).toString("utf8") : undefined;
    let baseEntries = [];
    let start = `no archive (${at} has no ${label})`;
    if (before !== undefined) {
      const lockLabel = `${label} at ${from}`;
      const lock = { label: lockLabel, ...parseLock(before, lockLabel) };
      if (changed.has(label)) log(`${label}: ${at} changes it against its merge base with HEAD; the archive starts from the one ${from}'s lock names, ${lock.sha256}`);
      // --replace needs the archive only to report what it drops, so one S3 does not hold is no failure there.
      const cached = cachedArchive(root, toolPack, lock).entries;
      if (cached !== undefined || !replace || objectExists(aws, archiveKey(toolPack, lock.sha256))) baseEntries = cached ?? downloadArchive(root, aws, toolPack, lock);
      else log(`${toolPack.directory}: S3 does not hold archive ${lock.sha256}, which ${lockLabel} names, so what --replace drops from it is not listed`);
      start = `archive ${lock.sha256.slice(0, 12)}`;
    }
    const own = tracked.filter(owns(toolPack)).map((path) => ({ path, bytes: blob(path) }));
    const files = new Map();
    if (!replace) {
      const gone = baseEntries.length > 0 ? unlisted(toolPack) : new Set();
      for (const entry of baseEntries) if (!gone.has(`${toolPack.directory}/${entry.path}`)) files.set(entry.path, entry.bytes);
    }
    for (const { path, bytes } of own) files.set(path.slice(toolPack.directory.length + 1), bytes);
    if (files.size === 0) {
      throw new Error(replace
        ? `--replace: ${toolPack.directory} tracks no pack test data at ${at}, so its archive would be empty; select the tool-packs to replace with --pack and --tool`
        : `${toolPack.directory} has neither a ${TEST_DATA_LOCK} nor tracked pack test data at ${at}: nothing to archive`);
    }
    const drops = baseEntries.map((entry) => entry.path).filter((path) => !files.has(path)).sort(byBytes);
    dropped.push(...drops.map((path) => `${toolPack.directory}/${path}`));
    const archive = buildArchive([...files].map(([path, bytes]) => ({ path, bytes })));
    cacheArchive(root, toolPack, archive);
    const lock = { label, sha256: archive.sha256, files: archive.files, bytes: archive.bytes };
    const recordingLabel = `${toolPack.directory}/${RECORDING_LOCK}`;
    const recordingsAt = tree.has(recordingLabel)
      ? recordingPlans(root, toolPack, { label: `${recordingLabel} at ${from}`, ...lockedRecordings(blob(recordingLabel).toString("utf8"), `${recordingLabel} at ${from}`) })
      : [];
    const text = lockText(archive);
    if (intoRoot !== undefined) {
      // A checkout of the commit holds its lock and its tracked pack test data as committed.
      const expected = [{ path: label, bytes: before === undefined ? undefined : Buffer.from(before, "utf8") }, ...own];
      const differ = expected.filter(({ path, bytes }) => {
        const current = intoFile(intoRoot, path).bytes;
        return current === undefined || bytes === undefined ? current !== bytes : !current.equals(bytes);
      });
      if (differ.length > 0) throw new Error(`--into ${into} is no checkout of ${at}: ${differ.slice(0, 3).map(({ path }) => path).join(", ")} differ(s) there; check the pull request out there first`);
    }
    builds.push({
      label,
      text,
      changed: text !== before,
      tracked: own.map(({ path }) => path),
      plans: [...archivePlans(root, [{ toolPack, lock }]), ...recordingsAt],
      target: intoRoot !== undefined && text !== before ? intoFile(intoRoot, label).target : undefined,
    });
    const made = replace ? `${own.length} tracked file(s) at ${from} alone (--replace)` : `${start} plus ${own.length} tracked file(s) at ${from}`;
    log(`${toolPack.directory}: ${made} make archive ${archive.sha256} (${archive.files} file(s), ${archive.bytes} byte(s))`);
    if (drops.length > 0) {
      const why = replace ? `${from} does not track them` : `the UPSTREAM.md at ${from} no longer lists them as Rerail's`;
      log(`${toolPack.directory}: archive ${archive.sha256.slice(0, 12)} drops ${drops.length} file(s) of ${start} (${why}):\n${drops.map((path) => `  ${path}`).join("\n")}`);
    }
  }
  const uploaded = uploadObjects(aws, builds.flatMap((build) => build.plans), log);

  const changedBuilds = builds.filter((build) => build.changed);
  for (const build of changedBuilds) if (build.target !== undefined) writeFileSync(build.target, build.text);
  const untracked = builds.flatMap((build) => build.tracked).sort(byBytes);
  const script = [
    ...(intoRoot === undefined ? changedBuilds.flatMap((build) => [`cat > ${shellQuoted(build.label)} <<'EOF'`, build.text.trimEnd(), "EOF"]) : []),
    ...(changedBuilds.length > 0 ? [`git add -- ${changedBuilds.map((build) => shellQuoted(build.label)).join(" ")}`] : []),
    ...(untracked.length > 0 ? [`git rm -r -q --cached -- ${untrackPaths(untracked).map(shellQuoted).join(" ")}`] : []),
  ].join("\n");
  if (script === "") log(`${at}: its locks already name these archives and it tracks no pack test data; nothing to commit`);
  else {
    const wrote = intoRoot !== undefined && changedBuilds.length > 0 ? `Wrote ${changedBuilds.map((build) => build.label).join(", ")} in ${into}. ` : "";
    const where = intoRoot === undefined ? `in a checkout of ${at}` : `in ${into}`;
    log(`${wrote}To commit the new lock(s) and untrack the pack test data the archives now hold (it stays on disk, ignored), run ${where}:\n${script}`);
  }
  return { changedLocks: changedBuilds.map((build) => build.label), uploaded, tracked: untracked, dropped: dropped.sort(byBytes), script };
}

/**
 * Maintainer: `pack`, then `upload`, for the selected tool-packs; then the commands that finish the change: `git add`
 * the changed locks, and `git rm --cached` the tracked pack test data the archives now hold. With `from` (a git
 * revision), pushFrom instead: a pull request's data from its commit, with nothing of it checked out or run, and with
 * `replace` too, no base archive under it.
 */
export function push({ root = ROOT, pack: packId, tool, dryRun = false, from, into, replace = false, aws = runAws, log = console.log } = {}) {
  if (from !== undefined) {
    if (dryRun) throw new Error("push --from has no --dry-run");
    return pushFrom({ root, from, pack: packId, tool, into, replace, aws, log });
  }
  if (into !== undefined) throw new Error("--into needs --from");
  if (replace) throw new Error("--replace needs --from");
  const { changedLocks } = pack({ root, pack: packId, tool, dryRun, log });
  if (dryRun) return { changedLocks, uploaded: [], tracked: [] };
  const { uploaded } = upload({ root, pack: packId, tool, aws, log });
  const tracked = trackedPackFixtures(root, selectToolPacks(root, packId, tool).map((toolPack) => toolPack.directory));
  if (changedLocks.length > 0) log(`Commit the new lock(s):\n  git add -- ${changedLocks.map(shellQuoted).join(" ")}`);
  if (tracked.length > 0) {
    log(`The archive(s) hold the ${tracked.length} tracked pack test data file(s); untrack them (they stay on disk, ignored) and commit that too:\n  git rm -r -q --cached -- ${untrackPaths(tracked).map(shellQuoted).join(" ")}`);
  }
  return { changedLocks, uploaded, tracked };
}

/** CI's cache entry for the selected tool-packs' archives: its key (from the locked sha256s alone) and its paths. */
export function archiveCache({ root = ROOT, pack: packId, tool } = {}) {
  const locked = selectToolPacks(root, packId, tool).map((toolPack) => ({ toolPack, lock: readLock(root, toolPack) }));
  const key = locked.length === 1
    ? `pack-test-data-${locked[0].toolPack.pack}-${locked[0].toolPack.tool}-${locked[0].lock.sha256}`
    : `pack-test-data-${sha256(Buffer.from(locked.map(({ toolPack, lock }) => `${toolPack.directory} ${lock.sha256}\n`).join("")))}`;
  return { key, paths: locked.map(({ toolPack, lock }) => relative(root, archiveCachePath(root, toolPack, lock.sha256)).split(sep).join("/")) };
}

/**
 * Whether S3 holds every archive the selected tool-packs' test-data.lock files name and every recording their
 * recording.lock files name, asked with head-object alone (nothing is downloaded). Throws, listing each missing key with
 * its lock and the remedy, sorted by key, when S3 lacks any.
 */
export function check({ root = ROOT, pack: packId, tool, aws = runAws, log = console.log } = {}) {
  const wanted = selectToolPacks(root, packId, tool).flatMap((toolPack) => {
    const push = `npm run fixtures:push -- --from <ref> --pack ${toolPack.pack} --tool ${toolPack.tool}`;
    const lock = readLock(root, toolPack);
    const archive = archiveKey(toolPack, lock.sha256);
    const recorded = readRecordingLock(root, toolPack);
    return [
      {
        key: archive,
        missing: `${lock.label} names s3://${BUCKET}/${archive}, which S3 does not hold: a maintainer archives the pull request's pack test data with \`${push}\``,
      },
      ...(recorded === undefined ? [] : [
        { name: recorded.recording.name, key: recordingKey(toolPack, recorded.recording.sha256) },
        { name: recorded.corpus.name, key: corpusKey(toolPack, recorded.corpus.sha256) },
      ]).map(({ name, key }) => ({
        key,
        missing: `${recorded.label} names ${name} at s3://${BUCKET}/${key}, which S3 does not hold: a maintainer downloads the pull request's CI recordings with \`gh run download <run> -n recordings-${toolPack.pack}--${toolPack.tool} -D ${RECORDING_CACHE}\`, then puts them in S3 with \`${push}\``,
      })),
    ];
  });
  requireAws(aws);
  const missing = wanted.filter(({ key }) => !objectExists(aws, key)).sort((a, b) => byBytes(a.key, b.key));
  if (missing.length > 0) {
    throw new Error(`S3 lacks ${missing.length} of the ${wanted.length} object(s) the locks name (CONTRIBUTING.md, "Pack test data"):\n${missing.map(({ missing: line }) => `  ${line}`).join("\n")}`);
  }
  const recordingCount = wanted.filter(({ key }) => key.startsWith(RECORDING_PREFIX)).length;
  log(`S3 holds every object the locks name: ${wanted.length - recordingCount} archive(s) and ${recordingCount} recording(s)`);
  return { checked: wanted.map(({ key }) => key).sort(byBytes) };
}

const USAGE = `usage: node scripts/s3/pack-fixtures.mjs <command> [--pack <id> [--tool <tool>]] [--dry-run]
  pull        fetch, verify and extract the archives the selected tool-packs' test-data.lock files name
  push        maintainer: pack, then upload
              push --from <ref> [--replace] [--into <dir>]: archive a pull request's tracked pack test data from its
              commit (never checked out or run) over the archive its lock names, less the files its UPSTREAM.md no
              longer lists (--replace: its tracked data alone), upload, and print the git commands that finish it in a
              checkout of it (--into: a checkout of that commit, where the locks are written)
  pack        archive the selected tool-packs' data on disk into ${ARCHIVE_CACHE} and rewrite their locks (no network)
  upload      put the locked archives from ${ARCHIVE_CACHE}, and the locked recordings from ${RECORDING_CACHE}, into S3
              unless it holds them, and read them back
  recordings  fetch the recordings the selected tool-packs' recording.lock files name into ${RECORDING_CACHE}, verified,
              and extract each corpus answers archive into ${RECORDING_CACHE}/corpus/<sha256>/
              recordings push: maintainer: put those recordings from ${RECORDING_CACHE} into S3 unless it holds them,
              read them back, and print the git add for the locks (no --dry-run)
  check       fail, listing each with its remedy, unless S3 holds every archive and recording the selected tool-packs'
              locks name (head-object only; nothing is downloaded)
  cache       CI: the Actions cache key and paths for the selected tool-packs' archives, as JSON`;

const COMMANDS = { pull, push, pack, upload, recordings, check, cache: (options) => console.log(JSON.stringify(archiveCache(options))) };
/** Commands with a subcommand, as `<command> <subcommand>`. */
const SUBCOMMANDS = { "recordings push": recordingsPush };

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  let parsed;
  try {
    parsed = parseArgs({
      allowPositionals: true,
      strict: true,
      options: {
        pack: { type: "string" },
        tool: { type: "string" },
        "dry-run": { type: "boolean", default: false },
        from: { type: "string" },
        into: { type: "string" },
        replace: { type: "boolean", default: false },
      },
    });
  } catch (error) {
    console.error(`${error instanceof Error ? error.message : String(error)}\n${USAGE}`);
    process.exitCode = 2;
  }
  const [command, ...extra] = parsed?.positionals ?? [];
  const name = [command, ...extra].join(" ");
  const run = Object.hasOwn(SUBCOMMANDS, name) ? SUBCOMMANDS[name] : extra.length === 0 && Object.hasOwn(COMMANDS, command ?? "") ? COMMANDS[command] : undefined;
  if (parsed !== undefined && run === undefined) {
    console.error(USAGE);
    process.exitCode = 2;
  } else if (parsed !== undefined && command !== "push" && (parsed.values.from !== undefined || parsed.values.into !== undefined || parsed.values.replace)) {
    console.error(`--from, --into and --replace belong to push\n${USAGE}`);
    process.exitCode = 2;
  } else if (parsed !== undefined) {
    try {
      const { pack: packId, tool, "dry-run": dryRun, from, into, replace } = parsed.values;
      run({ pack: packId, tool, dryRun, ...(command === "push" ? { from, into, replace } : {}) });
    } catch (error) {
      console.error(`fixtures:${name}: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 1;
    }
  }
}
