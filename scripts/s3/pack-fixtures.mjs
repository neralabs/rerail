#!/usr/bin/env node
// Pack test data (CONTRIBUTING.md, "Pack test data"): the files under packs/<id>/upstream/<tool>/ that are a tool's
// test cases and its recorded answers on them (PACK_FIXTURE_PATTERNS). None of it is in git, and it is of two kinds:
//
// - authored: what Rerail wrote for the tool to score, which nothing can regenerate. It is the files a tool's own
//   upstream/<tool>/UPSTREAM.md lists as Rerail's (pack check 6 reads the same list), and everything under
//   upstream/<tool>/authored/ (AUTHORED_CORPUS_PATTERN: a contributor's code corpus with its ground truth, where the
//   corpus is no public repository a pin can name). It lives in S3: each file's key is `rerail/` + its repository
//   path in the versioned bucket s3://nera-test-fixtures (us-east-2).
// - derived: everything else, upstream's tests copied or derived and the tool's recorded answers. `npm run
//   fixtures:derive` (scripts/pack-data/derive.ts) regenerates it from the tool-pack's pins, and CI caches it; it is
//   never committed and never uploaded.
//
// .gitignore covers the patterns, so a pulled or derived copy is untracked and invisible; a contributor checks new or
// changed authored data into a pull request with `git add -f`, and a maintainer moves it to S3 before merge (structure
// metric #3 fails while any authored data is tracked, and on any tracked derived data).
//
//   npm run fixtures:pull [-- --dry-run]   fetch every pack's authored data; tracked copies (a pull request's) win
//   npm run fixtures:push [-- --dry-run]   maintainer: upload the tracked authored copies, verify them, untrack them
//
// The consumers (the conformance runner, pack validation, the recorders and generators, the Rust tests) report a
// missing file with missingPackFixture, so a checkout without the data says to pull and derive instead of failing on
// ENOENT. packs/<id>/upstream/<tool>/ keeps the rest in git: gates.yaml, recorders, UPSTREAM.md, goldens.tsv,
// recording.lock.

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
export const BUCKET = "nera-test-fixtures";
export const REGION = "us-east-2";
/** Every key is this prefix plus the file's repository path. */
export const KEY_PREFIX = "rerail/";

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
  'pack test data is not in git: run `npm run fixtures:pull` (what Rerail wrote, from S3) and `npm run fixtures:derive` (the rest, from the pinned upstreams) (CONTRIBUTING.md, "Pack test data")';

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
 * `root`): the authored files among them go in a pull request with `git add -f`, and the derived ones never do.
 */
export function packFixturesWritten(paths, root = ROOT) {
  const authored = authoredListing(root);
  const written = [...new Set(paths.flatMap((path) => filesAt(isAbsolute(path) ? path : join(root, path))).map((path) => repositoryPath(path, root)))].sort();
  const own = written.filter((path) => packFixtureKind(path, authored) === "authored");
  const lines = [];
  if (written.length > own.length) {
    lines.push(`Wrote ${written.length - own.length} derived pack test data file(s), which git ignores: \`npm run fixtures:derive\` and CI regenerate them from the pins, so never commit them.`);
  }
  if (own.length > 0) {
    lines.push(
      `Wrote ${own.length} authored pack test data file(s) (their UPSTREAM.md lists them as Rerail's), which git ignores (they live in S3). Put new or changed ones in your pull request with`,
      `  git add -f -- ${own.map(shellQuoted).join(" ")}`,
      `and a maintainer moves them to S3 before merge (CONTRIBUTING.md, "Pack test data").`,
    );
  }
  return lines.join("\n");
}

/** packFixturesWritten for a recording at `path`: a fixtures recording names the case sources beside it too. */
export function recordingWritten(path, kind) {
  const cases = kind === "fixtures" ? ["fixtures", "tests"].map((name) => join(dirname(path), name)) : [];
  return packFixturesWritten([path, ...cases.filter((directory) => existsSync(directory))]);
}

/** How many files one `aws s3` command names with `--include` filters. */
const FILTER_CHUNK = 400;

/**
 * The `aws s3 sync` runs that fetch the authored pack test data of the checkout at `root`, each a repository-path
 * prefix and its filters (aws's `*` crosses `/`): one under packs/ for every upstream/<tool>/authored/ directory, and
 * per tool directory the files its UPSTREAM.md lists. Derived data in the bucket is never fetched.
 */
export function authoredSyncs(root = ROOT) {
  const syncs = [{ prefix: "packs/", filters: ["--exclude", "*", "--include", AUTHORED_CORPUS_PATTERN.slice("packs/".length).replaceAll("**", "*")] }];
  for (const [prefix, paths] of byToolDirectory([...authoredListing(root)].sort())) {
    for (let start = 0; start < paths.length; start += FILTER_CHUNK) {
      syncs.push({ prefix, filters: ["--exclude", "*", ...paths.slice(start, start + FILTER_CHUNK).flatMap((path) => ["--include", literalFilter(path.slice(prefix.length))])] });
    }
  }
  return syncs;
}

/** A path as an aws CLI filter matching only itself: fnmatch's `*`, `?` and `[` bracketed. */
export function literalFilter(path) {
  return path.replace(/[*?[]/gu, "[$&]");
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

/** Fails with an instruction unless the aws CLI is installed and signed in. */
export function requireAws(aws = runAws) {
  const run = aws(["sts", "get-caller-identity", "--output", "json"]);
  if (run.error?.code === "ENOENT") {
    throw new Error(`the AWS CLI (aws) is not installed. Pack test data lives in s3://${BUCKET}/${KEY_PREFIX}: install AWS CLI v2 (https://aws.amazon.com/cli/), then sign in with credentials that can read it (CONTRIBUTING.md, "Pack test data").`);
  }
  if (run.error !== undefined) throw run.error;
  if (run.status !== 0) {
    const reason = (run.stderr ?? "").trim().split("\n").at(-1) ?? "";
    throw new Error(`aws is not signed in (aws sts get-caller-identity: ${reason || `exit ${run.status}`}). Sign in (aws sso login, or AWS_PROFILE) with credentials that can read s3://${BUCKET}/${KEY_PREFIX}; ask a maintainer for access (CONTRIBUTING.md, "Pack test data").`);
  }
}

function awsChecked(aws, args) {
  const run = aws(args, { stdio: ["ignore", "inherit", "inherit"] });
  if (run.error !== undefined) throw run.error;
  if (run.status !== 0) throw new Error(`aws ${args.slice(0, 3).join(" ")} … failed with exit ${run.status}`);
}

/** The tracked pack test data of the checkout at `root`, sorted. */
export function trackedPackFixtures(root = ROOT) {
  return nulList(runGit(root, ["ls-files", "-z", "--", "packs"])).filter(isPackFixture).sort();
}

/** Tracked pack test data whose working-tree copy differs from the index (edited or deleted), sorted. */
function editedPackFixtures(root) {
  // A refreshed index compares content, not timestamps; it exits 1 when files differ, which ls-files then lists.
  spawnSync("git", ["update-index", "-q", "--refresh"], { cwd: root });
  return nulList(runGit(root, ["ls-files", "-z", "-m", "--", "packs"])).filter(isPackFixture).sort();
}

/**
 * The working-tree state a pull keeps: every tracked file's committed (index) content, except tracked files edited or
 * deleted in the working tree, which keep their edit. `snapshot` runs before the sync and `restore` after it.
 */
export function trackedSnapshot(root = ROOT) {
  const tracked = trackedPackFixtures(root);
  const edited = new Map();
  for (const path of editedPackFixtures(root)) {
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

/**
 * Fetches every pack's authored test data from S3 into packs/ (authoredSyncs), then puts back each tracked file's own
 * content (its index copy, or the working-tree edit of one being edited), so a pull request's files win over S3's.
 * The derived data is `npm run fixtures:derive`'s.
 */
export function pull({ root = ROOT, dryRun = false, aws = runAws, log = console.log } = {}) {
  requireAws(aws);
  const snapshot = trackedSnapshot(root);
  const source = `s3://${BUCKET}/${KEY_PREFIX}packs/`;
  for (const { prefix, filters } of authoredSyncs(root)) {
    // --exact-timestamps: a same-size file whose time is not S3's is fetched again (aws stamps fetched files with S3's).
    awsChecked(aws, ["s3", "sync", `s3://${BUCKET}/${KEY_PREFIX}${prefix}`, join(root, prefix), "--region", REGION, "--exact-timestamps", "--no-progress", ...(dryRun ? ["--dryrun"] : ["--only-show-errors"]), ...filters]);
  }
  if (dryRun) {
    log(`dry run: would restore ${snapshot.tracked.length - snapshot.edited.size} tracked file(s) to their committed content and keep ${snapshot.edited.size} edited one(s)`);
    return { restored: 0, kept: 0 };
  }
  const result = snapshot.restore();
  log(`pulled the authored pack test data from ${source} into packs/${snapshot.tracked.length === 0 ? "" : `; ${result.restored} tracked file(s) keep their committed content and ${result.kept} their working-tree edit`}; \`npm run fixtures:derive\` makes the rest`);
  return result;
}

/** Pack test data grouped by tool directory (`packs/<id>/upstream/<tool>/`), the prefix a sync or download runs under. */
function byToolDirectory(paths) {
  const groups = new Map();
  for (const path of paths) {
    const prefix = `${path.split("/").slice(0, 4).join("/")}/`;
    groups.set(prefix, [...(groups.get(prefix) ?? []), path]);
  }
  return groups;
}

/** The tracked pack test data of the checkout at `root` that is derived (packFixtureKind), sorted. */
export function trackedDerivedPackFixtures(root = ROOT) {
  const authored = authoredListing(root);
  return trackedPackFixtures(root).filter((path) => packFixtureKind(path, authored) === "derived");
}

/**
 * Maintainer only: uploads every tracked authored pack test data file to its key, downloads them back and compares
 * bytes, then removes them from the index (the working-tree files stay, ignored). Refuses on staged changes, so the
 * removal is the only thing staged, on tracked files edited in the working tree, whose upload would not be what was
 * committed, and on tracked derived data, which CI regenerates and S3 never holds.
 */
export function push({ root = ROOT, dryRun = false, aws = runAws, log = console.log } = {}) {
  if (spawnSync("git", ["diff", "--cached", "--quiet"], { cwd: root }).status !== 0) {
    throw new Error("the index has staged changes: commit or unstage them first, so the pushed files' removal is the only staged change");
  }
  const derived = trackedDerivedPackFixtures(root);
  if (derived.length > 0) {
    throw new Error(`tracked pack test data is derived: \`npm run fixtures:derive\` and CI regenerate it from the pins, so it is never committed or uploaded. Untrack it (git rm --cached) first:\n${derived.map((path) => `  ${path}`).join("\n")}`);
  }
  const tracked = trackedPackFixtures(root);
  if (tracked.length === 0) {
    log("no tracked pack test data: nothing to push");
    return { pushed: [] };
  }
  const edited = editedPackFixtures(root);
  if (edited.length > 0) {
    throw new Error(`tracked pack test data differs from the index in the working tree; commit or restore it first:\n${edited.map((path) => `  ${path}`).join("\n")}`);
  }
  requireAws(aws);
  if (dryRun) {
    for (const path of tracked) log(`would upload ${path} to s3://${BUCKET}/${KEY_PREFIX}${path}`);
    log(`dry run: would upload, verify and untrack ${tracked.length} file(s)`);
    return { pushed: tracked };
  }
  const scratch = mkdtempSync(join(tmpdir(), "rerail-pack-fixtures-"));
  try {
    const staged = join(scratch, "upload");
    for (const path of tracked) {
      mkdirSync(dirname(join(staged, path)), { recursive: true });
      copyFileSync(join(root, path), join(staged, path));
    }
    awsChecked(aws, ["s3", "cp", staged, `s3://${BUCKET}/${KEY_PREFIX}`, "--recursive", "--region", REGION, "--only-show-errors"]);

    const fetched = join(scratch, "download");
    for (const [prefix, paths] of byToolDirectory(tracked)) {
      for (let start = 0; start < paths.length; start += FILTER_CHUNK) {
        const includes = paths.slice(start, start + FILTER_CHUNK).flatMap((path) => ["--include", literalFilter(path.slice(prefix.length))]);
        awsChecked(aws, ["s3", "cp", `s3://${BUCKET}/${KEY_PREFIX}${prefix}`, join(fetched, prefix), "--recursive", "--region", REGION, "--only-show-errors", "--exclude", "*", ...includes]);
      }
    }
    const differing = tracked.filter((path) => !existsSync(join(fetched, path)) || !readFileSync(join(fetched, path)).equals(readFileSync(join(root, path))));
    if (differing.length > 0) {
      throw new Error(`the round trip through S3 changed ${differing.length} file(s); nothing was removed from git:\n${differing.map((path) => `  ${path}`).join("\n")}`);
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  runGit(root, ["rm", "--cached", "-q", "--pathspec-from-file=-", "--pathspec-file-nul", "--"], `${tracked.join("\0")}\0`);
  for (const path of tracked) log(`removed from the index: ${path}`);
  log(`uploaded and verified ${tracked.length} authored file(s) in s3://${BUCKET}/${KEY_PREFIX}; they are no longer tracked (still on disk, ignored). Commit the removal.`);
  return { pushed: tracked };
}

const USAGE = "usage: node scripts/s3/pack-fixtures.mjs <pull|push> [--dry-run]";

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [command, ...rest] = process.argv.slice(2);
  const unknown = rest.filter((argument) => argument !== "--dry-run");
  if ((command !== "pull" && command !== "push") || unknown.length > 0) {
    console.error(USAGE);
    process.exitCode = 2;
  } else {
    try {
      (command === "pull" ? pull : push)({ dryRun: rest.includes("--dry-run") });
    } catch (error) {
      console.error(`fixtures:${command}: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 1;
    }
  }
}
