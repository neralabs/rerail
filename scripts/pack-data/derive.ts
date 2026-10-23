/**
 * `npm run fixtures:derive`: regenerate every tool-pack's derived pack test data from its pins (CONTRIBUTING.md, "Pack
 * test data"). It is the maintainer's regeneration and the bump producer's (port-readiness-bump.yml): the result is
 * archived with the tool-pack's authored data by `npm run fixtures:push` (`pack` in the bump workflow), whose
 * test-data.lock names the archive. Pull request CI runs it only to check that a changed test-data.lock reproduces
 * (`--compare`, "Test data reproduces from the pins" in ci.yml), never to make the data it uses. Derived data is
 * a tool's test files copied or derived from its pinned upstream and the tool's recorded answers; what Rerail wrote for
 * the tool to score is authored data, which `npm run fixtures:pull` fetches and this only reads. For each tool-pack it
 *
 *   1. removes the tool directory's derived files, so nothing stale survives and nothing is read before it is made;
 *   2. runs the tool-pack's recipe (scripts/pack-data/recipes/): the pinned inputs, then the pack's generator;
 *   3. for a gated tool-pack (gates.yaml), writes the case sources the port readiness recorder names that the generator
 *      did not (CASE_LAYOUTS reuses a generator's file of the same bytes, so this runs after it), and checks every case
 *      source against the recording recording.lock names when the recorder's cache holds it;
 *   4. fails when a tracked file changed (a licence table, UPSTREAM.md), an authored file changed, or an untracked file
 *      appeared beside them: a derivation of a commit makes the derived data and nothing else. `--update` (a pin bump)
 *      keeps those changes for the commit instead, and reports them.
 *
 * With `--compare <dir>` it then compares the tool directory's pack test data with the same paths under <dir> (a pulled
 * copy of a complete checkout's data), byte for byte.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, rmdirSync, rmSync, statSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { spawnSync } from "node:child_process";

import { authoredListing, isPackFixture, packFixtureKind } from "../s3/pack-fixtures.mjs";
import { DEFAULT_CACHE_ROOT } from "../../conformance/port-readiness/corpus.js";
import { readGatesManifest } from "../../conformance/port-readiness/manifest.js";
import { ProcessCommandRunner } from "../../conformance/port-readiness/process.js";
import { CASE_LAYOUTS, hasRecordingNote, inlineTextsSource, listedCopies, readLockedPair, recordCaseSources, withRecordingNote, withUpstreamRows } from "../../conformance/recorders/port-readiness/record.js";
import { licenseFiles } from "../../conformance/port-readiness/recorded-adapter.js";
import { createContext, type DeriveRecipe } from "./context.js";
import { RECIPES } from "./recipes/index.js";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const DEFAULT_CACHE = join(ROOT, ".rerail", "pack-data");

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");
const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

function git(args: string[]): string {
  const run = spawnSync("git", ["--literal-pathspecs", ...args], { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28 });
  if (run.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${run.stderr.trim()}`);
  return run.stdout;
}

const nulList = (text: string): string[] => text.split("\0").filter((path) => path !== "");
const tracked = (...paths: string[]): string[] => nulList(git(["ls-files", "-z", "--", ...paths]));

const toolPath = (recipe: Pick<DeriveRecipe, "pack" | "tool">): string => `packs/${recipe.pack}/upstream/${recipe.tool}`;
const isGated = (recipe: DeriveRecipe): boolean => existsSync(join(ROOT, toolPath(recipe), "gates.yaml"));

/** Every file under a repository directory, as repository paths, sorted. */
function filesUnder(directory: string): string[] {
  const absolute = join(ROOT, directory);
  if (!existsSync(absolute)) return [];
  return readdirSync(absolute, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => relative(ROOT, join(entry.parentPath, entry.name)).split("\\").join("/"))
    .sort(byText);
}

/** A tool directory's pack test data on disk, as repository paths, sorted. */
const packData = (directory: string): string[] => filesUnder(directory).filter((path) => isPackFixture(path));

// ---------------------------------------------------------------------------------------------------------------
// The derivation.

/** Each file's sha256, by repository path. */
function digests(paths: readonly string[]): Map<string, string> {
  return new Map(paths.map((path) => [path, existsSync(join(ROOT, path)) ? sha256(readFileSync(join(ROOT, path))) : "(deleted)"]));
}

/** The paths whose digest differs between two snapshots, sorted. */
function changedBetween(before: Map<string, string>, after: Map<string, string>): string[] {
  return [...new Set([...before.keys(), ...after.keys()])].filter((path) => before.get(path) !== after.get(path)).sort(byText);
}

/** Untracked files git does not ignore, under `paths`. */
const untracked = (...paths: string[]): string[] => nulList(git(["ls-files", "-z", "--others", "--exclude-standard", "--", ...paths])).sort(byText);

function removeEmptyDirectories(directory: string): void {
  if (!existsSync(directory) || !statSync(directory).isDirectory()) return;
  for (const entry of readdirSync(directory)) removeEmptyDirectories(join(directory, entry));
  if (readdirSync(directory).length === 0) rmdirSync(directory);
}

export interface DeriveOptions {
  cacheRoot: string;
  recorderCache: string;
  update: boolean;
  compare?: string;
  log: (message: string) => void;
}

export interface DeriveResult {
  label: string;
  derived: number;
  authored: number;
  problems: string[];
  /** Tracked files the derivation changed, kept for the commit (`--update` only). */
  changed: string[];
  notes: string[];
}

export async function deriveToolPack(recipe: DeriveRecipe, options: DeriveOptions): Promise<DeriveResult> {
  const label = toolPath(recipe);
  const started = Date.now();
  const problems: string[] = [];
  const notes: string[] = [];
  const packDirectory = `packs/${recipe.pack}`;
  if (recipe.platforms !== undefined && !recipe.platforms.only.includes(process.platform)) {
    notes.push(`derived on ${process.platform}, not ${recipe.platforms.only.join(" or ")}, so these are not the bytes the nightly producer archives: ${recipe.platforms.reason}`);
  }

  // Authored data is an input: it must be here (pulled, or the pull request's), and it must come out unchanged.
  const listing = authoredListing(ROOT);
  const authored = [...new Set([...[...listing].filter((path) => path.startsWith(`${label}/`)), ...packData(label).filter((path) => packFixtureKind(path, listing) === "authored")])].sort(byText);
  const absent = authored.filter((path) => !existsSync(join(ROOT, path)));
  if (absent.length > 0) throw new Error(`${label}: ${absent.length} authored file(s) are missing, e.g. ${absent[0]}: run \`npm run fixtures:pull\` first`);
  const authoredBefore = digests(authored);
  const trackedBefore = digests(tracked(packDirectory));
  const untrackedBefore = new Set(untracked(packDirectory));

  const stale = packData(label).filter((path) => packFixtureKind(path, listing) === "derived");
  for (const path of stale) rmSync(join(ROOT, path));
  removeEmptyDirectories(join(ROOT, label));
  options.log(`${label}: removed ${stale.length} derived file(s); deriving${isGated(recipe) ? " (generator, then the recorder's case sources)" : ""}`);

  // A generator that rewrites the licence record writes its table, not the note the recorder keeps under it.
  const manifest = isGated(recipe) ? readGatesManifest(ROOT, recipe.pack, recipe.tool) : undefined;
  const record = manifest === undefined ? undefined : join(ROOT, packDirectory, "licenses", manifest.rerail.project, "UPSTREAM.md");
  const recordBefore = record !== undefined && existsSync(record) ? readFileSync(record, "utf8") : undefined;
  const noted = recordBefore !== undefined && hasRecordingNote(recordBefore);

  const runner = new ProcessCommandRunner();
  const context = createContext({ root: ROOT, pack: recipe.pack, tool: recipe.tool, cacheRoot: options.cacheRoot, recorderCache: options.recorderCache, runner, log: (message) => options.log(`${label}: ${message}`) });
  await mkdir(options.cacheRoot, { recursive: true });
  await recipe.derive(context);

  if (manifest !== undefined && record !== undefined) {
    const sources = await recordCaseSources({ root: ROOT, pack: recipe.pack, tool: recipe.tool, cacheRoot: options.recorderCache, runner });
    const missing = sources.filter((source) => !existsSync(join(ROOT, label, source.file)));
    const conflicting = sources.filter((source) => existsSync(join(ROOT, label, source.file)) && !readFileSync(join(ROOT, label, source.file)).equals(source.bytes));
    for (const source of conflicting.slice(0, 10)) problems.push(`${label}/${source.file}: the generator wrote other bytes than the recorder's case source (${source.fixture})`);
    if (conflicting.length > 10) problems.push(`… and ${conflicting.length - 10} more case sources the generator wrote differently`);
    for (const source of missing) {
      await mkdir(dirname(join(ROOT, label, source.file)), { recursive: true });
      await writeFile(join(ROOT, label, source.file), source.bytes);
    }
    // The rows the recorder adds for the sources it copied (a source the table lists already has its row, whichever
    // case placed it), and its note on the recording's inline texts, as writeRecording writes them. The note goes after
    // the table, where the generator ended the file, and the file keeps the ending it had.
    const text = await readFile(record, "utf8");
    const known = new Set(listedCopies(text, recipe.tool));
    const copied = missing.filter((source) => !known.has(source.file));
    const rows = copied.length === 0 ? text : withUpstreamRows(text, recipe.tool, copied, CASE_LAYOUTS[manifest.adapter]);
    const listed = noted ? withRecordingNote(rows, inlineTextsSource(manifest), licenseFiles(dirname(record))).replace(/\n*$/u, /\n*$/u.exec(recordBefore!)![0]) : rows;
    if (listed !== text) await writeFile(record, listed);
    options.log(`${label}: ${sources.length} case source(s), ${missing.length} not written by the generator`);

    // The recording the lock names, when the recorder's cache holds it, names every case source by sha256.
    const locked = readLockedPair(ROOT, options.recorderCache, recipe.pack, recipe.tool);
    if (locked === null) notes.push("no recording.lock: the case sources are not checked against a recording");
    else if ("unavailable" in locked) notes.push(`the case sources are not checked against the recording: ${locked.unavailable}`);
    else {
      const wrong = new Set<string>();
      for (const rule of Object.values(locked.recording.rules)) {
        for (const testCase of rule.cases) {
          if (testCase.file === null || testCase.sha256 === undefined) continue;
          const path = join(ROOT, label, testCase.file);
          if (!existsSync(path) || sha256(readFileSync(path)) !== testCase.sha256) wrong.add(testCase.file);
        }
      }
      // After a pin change (--update) the lock still names the old release's recording: the case sources differing from
      // it is what the bump changed, and recording the tool-pack again (port-readiness:record) is what checks them.
      const differing = [...wrong].sort(byText);
      if (options.update) {
        if (differing.length > 0) notes.push(`${differing.length} case source(s) differ from the recording recording.lock names (${manifest.upstream.version} at its last record), e.g. ${differing.slice(0, 3).join(", ")}: record the tool-pack again (npm run port-readiness:record) before pushing`);
      } else {
        for (const file of differing.slice(0, 10)) problems.push(`${label}/${file}: not the case source the recording ${manifest.upstream.version} names (sha256 in the recording recording.lock names)`);
        if (wrong.size > 10) problems.push(`… and ${wrong.size - 10} more case sources the recording names differently`);
      }
      if (wrong.size === 0) notes.push("every case source is the one recording.lock's recording names");
    }
  }

  const authoredChanged = changedBetween(authoredBefore, digests(authored));
  const trackedChanged = changedBetween(trackedBefore, digests(tracked(packDirectory)));
  const appeared = untracked(packDirectory).filter((path) => !untrackedBefore.has(path));
  const extra = packData(label).filter((path) => packFixtureKind(path, listing) === "authored" && !authoredBefore.has(path));
  if (options.update) {
    for (const path of authoredChanged) notes.push(`changed authored file ${path}: check it in with git add -f`);
    for (const path of extra) notes.push(`new authored file ${path}: check it in with git add -f`);
  } else {
    for (const path of authoredChanged) problems.push(`${path}: authored data the derivation changed (a generator's own cases changed: check the new file in with git add -f)`);
    for (const path of extra) problems.push(`${path}: authored data the derivation wrote that was not there before`);
    for (const path of trackedChanged) problems.push(`${path}: a tracked file the derivation changed (derive with --update after changing a pin, and commit it)`);
    for (const path of appeared) problems.push(`${path}: an untracked file the derivation wrote outside the pack test data`);
  }

  const derived = packData(label).filter((path) => packFixtureKind(path, listing) === "derived");
  if (options.compare !== undefined) problems.push(...compareWith(options.compare, label));
  options.log(`${label}: ${derived.length} derived file(s) beside ${authored.length} authored, in ${((Date.now() - started) / 1000).toFixed(0)} s`);
  return { label, derived: derived.length, authored: authored.length, problems, changed: options.update ? [...trackedChanged, ...appeared] : [], notes };
}

/** How the tool directory's pack test data differs from the same paths under `oracle`, a copy of complete data. */
export function compareWith(oracle: string, label: string): string[] {
  const mine = new Set(packData(label));
  const theirs = existsSync(join(oracle, label))
    ? readdirSync(join(oracle, label), { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) => relative(oracle, join(entry.parentPath, entry.name)).split("\\").join("/"))
      .filter((path) => isPackFixture(path))
    : [];
  const missing = theirs.filter((path) => !mine.has(path)).sort(byText);
  const extra = [...mine].filter((path) => !theirs.includes(path)).sort(byText);
  const differing = theirs.filter((path) => mine.has(path) && !readFileSync(join(ROOT, path)).equals(readFileSync(join(oracle, path)))).sort(byText);
  const report = (what: string, paths: string[]) => (paths.length === 0 ? [] : [`${label}: ${paths.length} file(s) ${what} ${oracle}, e.g. ${paths.slice(0, 5).join(", ")}`]);
  return [...report("missing that are in", missing), ...report("extra that are not in", extra), ...report("differing from", differing)];
}

// ---------------------------------------------------------------------------------------------------------------
// The CLI.

const USAGE = `usage:
  npm run fixtures:derive [-- --pack <id> [--tool <tool>]] [--update] [--compare <dir>] [--cache <dir>] [--recorder-cache <dir>]
`;

function selected(pack: string | undefined, tool: string | undefined): DeriveRecipe[] {
  const chosen = RECIPES.filter((recipe) => (pack === undefined || recipe.pack === pack) && (tool === undefined || recipe.tool === tool));
  if (chosen.length === 0) throw new Error(`no tool-pack ${pack ?? "*"}/${tool ?? "*"} has a derivation recipe (scripts/pack-data/recipes/)`);
  return chosen;
}

async function main(): Promise<number> {
  const { values, positionals } = parseArgs({
    options: {
      pack: { type: "string" },
      tool: { type: "string" },
      update: { type: "boolean", default: false },
      compare: { type: "string" },
      cache: { type: "string" },
      "recorder-cache": { type: "string" },
      help: { type: "boolean", default: false },
    },
    allowPositionals: true,
    strict: true,
  });
  if (values.help || positionals.length > 0 || (values.tool !== undefined && values.pack === undefined)) {
    process.stderr.write(USAGE);
    return values.help ? 0 : 2;
  }

  const options: DeriveOptions = {
    cacheRoot: resolve(values.cache ?? DEFAULT_CACHE),
    recorderCache: resolve(values["recorder-cache"] ?? DEFAULT_CACHE_ROOT),
    update: values.update,
    ...(values.compare === undefined ? {} : { compare: resolve(values.compare) }),
    log: (message) => process.stderr.write(`${message}\n`),
  };
  const results: DeriveResult[] = [];
  for (const recipe of selected(values.pack, values.tool)) {
    try {
      results.push(await deriveToolPack(recipe, options));
    } catch (error) {
      results.push({ label: toolPath(recipe), derived: 0, authored: 0, problems: [error instanceof Error ? error.stack ?? error.message : String(error)], changed: [], notes: [] });
    }
  }
  let failed = 0;
  for (const result of results) {
    const status = result.problems.length === 0 ? "ok" : "FAILED";
    if (result.problems.length > 0) failed += 1;
    process.stdout.write(`${result.label}: ${status}, ${result.derived} derived file(s), ${result.authored} authored\n`);
    for (const note of result.notes) process.stdout.write(`  note: ${note}\n`);
    for (const path of result.changed) process.stdout.write(`  changed (commit it): ${path}\n`);
    for (const problem of result.problems) process.stdout.write(`  ${problem.split("\n").join("\n  ")}\n`);
  }
  process.stdout.write(`${results.length - failed} of ${results.length} tool-pack(s) derived${failed === 0 ? "" : `; ${failed} failed`}\n`);
  return failed === 0 ? 0 : 1;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then((code) => {
    process.exitCode = code;
  }, (error: unknown) => {
    process.stderr.write(`${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
    process.exitCode = 2;
  });
}
