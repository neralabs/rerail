/**
 * The gates recorder: run a tool-pack's live adapter over every upstream rule the pack's conformance manifests name for
 * the tool, and write what the gates read (conformance/port-readiness/recording/): recording.json and the corpus
 * answers archive into the cache, the `recording.lock` naming them into `packs/<pack>/upstream/<tool>/`, and every case
 * source the recording names that the directory lacks. The corpus answers are recorded incrementally: upstream is
 * asked only for the answers the locked archive lacks (corpus-answers.ts says what names an answer).
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, extname, join, resolve } from "node:path";
import { missingPackFixture, recordings as fetchRecordings } from "../../../scripts/s3/pack-fixtures.mjs";

import { buildCorpus, buildRerailCorpora, conformancePaths, failedTasksMessage, readCorpusTasks, readRerailCorpora, wantedKey, wantedPlaces, type CorpusFile, type RerailCorpus } from "../../port-readiness/corpus.js";
import { readGoldens, type Golden } from "../../port-readiness/goldens.js";
import { readGatesManifest, recordedRuntimes, toolPackDirectory, type GatesManifest } from "../../port-readiness/manifest.js";
import type { CommandRunner } from "../../port-readiness/process.js";
import {
  answerText, ASKS_EVERY_RULE, BATCHED_ADAPTERS, cachedCorpusTree, cacheCorpusArchive, CONTEXT_FILES, corpusCommits, CORPUS_ARCHIVE, fetchedContext, memoryTree, missingAnswers, neededAnswers, nextTree, packCorpusTree,
  treeFiles, verdictText, type NeededAnswer, type NeededAnswers, type PackedCorpus,
} from "../../port-readiness/recording/corpus-answers.js";
import {
  canonicalText, cleanVerdicts, compareSpans, RECORDING_FILE, RECORDING_FORMAT, gatesRecordingSchema, isCleanVerdict, recordedParsers, spanTuple,
  type GatesRecording, type RecordedCase, type RecordedRule, type RecordedSpan,
} from "../../port-readiness/recording/schema.js";
import { diffRecordings, type RecordedPair, type RecordingChanges, type ResolvedRuntime, type UnreadRecording } from "../../port-readiness/recording/diff.js";
import {
  buildRecordingLock, readCachedRecording, readRecordingLock, RECORDING_LOCK, recordingLockSchema, recordingLockText, writeCachedRecording, type RecordingLock,
} from "../../port-readiness/recording/store.js";
import { licenseFiles, rebuildFixtureTexts, upstreamCheckoutDirectory, type RecordedFixtureTexts } from "../../port-readiness/recorded-adapter.js";
import type { PackAdapter, PackManifest, Span, UpstreamCase, UpstreamFile } from "../../port-readiness/types.js";
import { createBanditAdapter } from "./adapters/bandit.js";
import { createBladeParserAdapter } from "./adapters/blade-parser.js";
import { createBladestanAdapter } from "./adapters/bladestan.js";
import { createClippyAdapter } from "./adapters/clippy.js";
import { createEslintAdapter } from "./adapters/eslint/eslint.js";
import { createPhpcsAdapter } from "./adapters/phpcs.js";
import { createPhpstanAdapter } from "./adapters/phpstan.js";
import { createPsalmAdapter } from "./adapters/psalm.js";
import { createRuffAdapter } from "./adapters/ruff.js";
import { createSqlfluffAdapter } from "./adapters/sqlfluff.js";
import { createTlintAdapter } from "./adapters/tlint.js";
import { resolvedComposerRuntimes, runtimePinDigests } from "./runtimes.js";

export type AdapterFactory = (pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner) => Promise<PackAdapter>;

export const LIVE_ADAPTERS: Record<GatesManifest["adapter"], AdapterFactory> = {
  bandit: createBanditAdapter,
  ruff: createRuffAdapter,
  eslint: createEslintAdapter,
  phpcs: createPhpcsAdapter,
  tlint: createTlintAdapter,
  "blade-parser": createBladeParserAdapter,
  phpstan: createPhpstanAdapter,
  psalm: createPsalmAdapter,
  bladestan: createBladestanAdapter,
  sqlfluff: createSqlfluffAdapter,
  clippy: createClippyAdapter,
};

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");
const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);
const sortedUnique = (values: Iterable<string>): string[] => [...new Set([...values].filter((value) => value !== ""))].sort(byText);

/** An UPSTREAM.md line naming an upstream path prefix nothing is copied from (conformance/pack-validation.ts reads it too). */
const NOT_COPIED = /^- Not copied: `([^`]+)`/gmu;

/**
 * The upstream path prefixes the pack's `licenses/<project>/UPSTREAM.md` list as "Not copied": material an upstream
 * derives from a copyleft project (Ruff's fixtures for its pylint, flake8-builtins, flake8-django, flake8-executable and
 * refurb ports). No case source under one is committed, and no recording holds such a file's text.
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
 * The live adapter's fixtureTextSources as recording.json records them: files sorted by path, inline texts sorted. The
 * list the gates rebuild from it (rebuildFixtureTexts, on the live adapter's checkout) must equal the live
 * fixtureTexts() as a multiset, or nothing is written.
 */
export function recordFixtureTexts(adapter: PackAdapter, cacheRoot: string, pins: { repository: string; commit: string }, notCopied: readonly string[]): RecordedFixtureTexts | undefined {
  const origins = adapter.fixtureTextSources?.();
  if (origins === undefined) return undefined;
  const live = adapter.fixtureTexts();
  if (origins.length !== live.length) throw new Error(`the adapter gives ${origins.length} fixture text origin(s) for ${live.length} fixture text(s)`);
  const files = new Map<string, { path: string; sha256: string; transform?: "notebook-code-cells" }>();
  const inline: { text: string }[] = [];
  for (const origin of origins) {
    if ("inline" in origin) {
      inline.push({ text: origin.inline });
      continue;
    }
    if (files.has(origin.path)) throw new Error(`fixture text origin ${origin.path} is listed twice`);
    files.set(origin.path, { path: origin.path, sha256: origin.sha256, ...(origin.transform === undefined ? {} : { transform: origin.transform }) });
  }
  const recorded: RecordedFixtureTexts = {
    files: [...files.keys()].sort(byText).map((path) => files.get(path)!),
    inline: inline.sort((a, b) => byText(a.text, b.text)),
  };
  // A not-copied (copyleft-derived) fixture's text must never be inline; it is fetched from upstream by path instead.
  const notCopiedTexts = new Set(live.filter((_, index) => { const origin = origins[index]!; return !("inline" in origin) && notCopied.some((prefix) => origin.path.startsWith(prefix)); }));
  const leaked = recorded.inline.filter(({ text }) => text.length > 0 && notCopiedTexts.has(text)).length;
  if (leaked > 0) throw new Error(`${leaked} inline fixture text(s) are the text of a fixture UPSTREAM.md lists as not copied`);
  const rebuilt = rebuildFixtureTexts(upstreamCheckoutDirectory(cacheRoot, pins), recorded);
  const counts = new Map<string, number>();
  for (const text of live) counts.set(text, (counts.get(text) ?? 0) + 1);
  for (const text of rebuilt) counts.set(text, (counts.get(text) ?? 0) - 1);
  const differing = [...counts.values()].filter((count) => count !== 0).length;
  if (rebuilt.length !== live.length || differing > 0) throw new Error(`fixtureTexts rebuilt from their recorded origins differ from the live adapter's (${rebuilt.length} vs ${live.length} texts, ${differing} differing)`);
  return recorded;
}

/** A case as the layout places it: its upstream test file (`fixture`), the path it is read at, and its source's bytes. */
export interface CaseSource {
  fixture: string;
  path: string;
  bytes: Buffer;
}

/** The files under the tool-pack directory's fixtures/ with these bytes, relative to the tool-pack directory, sorted. */
export type CommittedSources = (bytes: Buffer) => string[];

/** Where a case's upstream test file is committed under the tool-pack directory, or null when it must not be copied. */
export interface CaseLayout {
  /** The committed path of a case's source, relative to the tool-pack directory; null for one that cannot be copied. */
  file(testCase: CaseSource, committed: CommittedSources): string | null;
  /** How UPSTREAM.md lists a new copy: the upstream file it comes from, and whether it is that file verbatim; verbatim when omitted. */
  row?(fixture: string): { upstream: string; copy: "verbatim" | "derived" };
}

const RUFF_FIXTURES = "crates/ruff_linter/resources/test/fixtures/";
const RUFF_TESTS = "crates/ruff_linter/resources/test/";
/** Ruff's notebook fixtures, which its linter tests read from the ruff_notebook crate. */
const RUFF_NOTEBOOK_FIXTURES = "crates/ruff_notebook/resources/test/fixtures/";
/** An ESLint-family case's test file, without the run and case it names (`tests/lib/rules/eqeqeq.js#0/valid[3]`). */
const eslintTestFile = (fixture: string): string => fixture.replace(/#.*$/u, "");
/** Where PHP_CodeSniffer keeps its standards; each sniff's unit tests are under <standard>/Tests/<category>/. */
const PHPCS_STANDARDS = "src/Standards/";
/** A tlint case's test file, without the test method it names (`tests/Linting/Linters/NoDumpDirectivesTest.php#catches_dd_directive_usage`). */
const tlintTestFile = (fixture: string): string => fixture.replace(/#.*$/u, "");
/** Where phpstan-src keeps its tests; a rule test's data files are under tests/PHPStan/Rules/<category>/data/. */
const PHPSTAN_TESTS = "tests/PHPStan/";
/** Where Bladestan's rule test keeps the templates of its application (views/ and the Test namespace's). */
const BLADESTAN_TEMPLATES = "tests/skeleton/resources/";

/**
 * The existing fixtures/ layouts: Ruff's resources/test/fixtures/<dir>/… and Bandit's examples/… map to fixtures/….
 * An ESLint-family case is not a file but a RuleTester case's `code`, so each case is its own source: one the tool-pack
 * already commits byte for byte (upstream/generate.mjs's fixtures/<rule>/<n>.<ext>) is reused, from the test file's own
 * directory and with the case's file type first; any other is copied to fixtures/<test file>/<sha256 prefix><ext>, the
 * extension of the path the case is read at, in lower case. PHP_CodeSniffer's unit test files,
 * src/Standards/<standard>/Tests/<category>/<file>.inc, are fixtures/<standard>/<category>/<file>.inc.php, as
 * upstream/phpcs/generate.py copied them: verbatim, with a `.php` suffix so Rerail routes them to PHP. A tlint case is
 * the template a linter test's method hands the linter, so, as for ESLint, one the tool-pack commits byte for byte
 * (upstream/tlint/generate.py's fixtures/<test class>/<n>.blade.php) is reused, from the test's own directory first;
 * any other is copied to fixtures/<test class>/<sha256 prefix>.blade.php. A stillat/blade-parser case is the n-th
 * distinct template a validator test (tests/Validation/<X>Test.php, `#<n>`) hands Document::fromText(), which
 * upstream/blade-parser/generate.py wrote to fixtures/<X>Test/<n>.blade.php.
 * PHPStan's test data files, tests/PHPStan/<path>, are fixtures/<path>, as upstream/phpstan/generate.py copied them.
 * A Psalm case is the `code` of one case of a test class's providerValidCodeParse() or providerInvalidCodeParse()
 * (`tests/<X>Test.php#<provider>/<case>`), which upstream/psalm/generate.py wrote to fixtures/<X>Test/<provider>/<case>.php.
 * Bladestan's test application's templates, tests/skeleton/resources/<path>, are fixtures/skeleton/resources/<path>,
 * as upstream/bladestan/generate.py copied them.
 */
export const CASE_LAYOUTS: Record<GatesManifest["adapter"], CaseLayout> = {
  ruff: {
    file({ fixture }) {
      if (fixture.startsWith(RUFF_NOTEBOOK_FIXTURES)) return `fixtures/ruff_notebook/${fixture.slice(RUFF_NOTEBOOK_FIXTURES.length)}`;
      const rest = fixture.startsWith(RUFF_FIXTURES) ? fixture.slice(RUFF_FIXTURES.length) : fixture.startsWith(RUFF_TESTS) ? fixture.slice(RUFF_TESTS.length) : null;
      if (rest === null) return null;
      return `fixtures/${rest}`;
    },
  },
  bandit: {
    file: ({ fixture }) => (fixture.startsWith("examples/") ? `fixtures/${fixture.slice("examples/".length)}` : null),
  },
  eslint: {
    file({ fixture, path, bytes }, committed) {
      const directory = `fixtures/${(eslintTestFile(fixture).split("/").pop() ?? "").replace(/(?:\.test)?\.[cm]?[jt]sx?$/u, "")}/`;
      const extension = extname(path).toLowerCase();
      const same = committed(bytes);
      const own = same.filter((file) => file.startsWith(directory));
      const typed = (file: string) => extname(file).toLowerCase() === extension;
      return own.find(typed) ?? own[0] ?? same.find(typed) ?? same[0]
        ?? `${directory}${sha256(bytes).slice(0, 12)}${extension}`;
    },
    row: (fixture) => ({ upstream: eslintTestFile(fixture), copy: "derived" }),
  },
  phpcs: {
    file({ fixture }) {
      const parts = fixture.startsWith(PHPCS_STANDARDS) ? fixture.slice(PHPCS_STANDARDS.length).split("/") : [];
      return parts.length === 4 && parts[1] === "Tests" ? `fixtures/${parts[0]}/${parts[2]}/${parts[3]}.php` : null;
    },
  },
  tlint: {
    file({ fixture, bytes }, committed) {
      const directory = `fixtures/${(tlintTestFile(fixture).split("/").pop() ?? "").replace(/\.php$/u, "")}/`;
      const same = committed(bytes);
      return same.find((file) => file.startsWith(directory)) ?? same[0] ?? `${directory}${sha256(bytes).slice(0, 12)}.blade.php`;
    },
    row: (fixture) => ({ upstream: tlintTestFile(fixture), copy: "derived" }),
  },
  "blade-parser": {
    file({ fixture }) {
      const match = /^tests\/Validation\/(\w+Test)\.php#(\d+)$/u.exec(fixture);
      return match === null ? null : `fixtures/${match[1]}/${match[2]}.blade.php`;
    },
    row: (fixture) => ({ upstream: fixture.replace(/#\d+$/u, ""), copy: "derived" }),
  },
  phpstan: {
    file: ({ fixture }) => (fixture.startsWith(PHPSTAN_TESTS) ? `fixtures/${fixture.slice(PHPSTAN_TESTS.length)}` : null),
  },
  psalm: {
    file({ fixture }) {
      const match = /^tests\/(\w+Test)\.php#(valid|invalid)\/([\w-]+)$/u.exec(fixture);
      return match === null ? null : `fixtures/${match[1]}/${match[2]}/${match[3]}.php`;
    },
    row: (fixture) => ({ upstream: fixture.replace(/#.*$/u, ""), copy: "derived" }),
  },
  bladestan: {
    file: ({ fixture }) => (fixture.startsWith(BLADESTAN_TEMPLATES) ? `fixtures/${fixture.slice("tests/".length)}` : null),
  },
  // A SQLFluff case is one case of a rule's YAML test file (`test/fixtures/rules/std_rule_cases/AL04.yml#<case>`), read
  // at `<dialect>/<CODE>/<case>.sql`; its `fail_str` or `pass_str` is committed byte for byte at fixtures/<that path>,
  // where packs/sql/upstream/sqlfluff/generate.py writes the conformance fixtures it records.
  sqlfluff: {
    file: ({ path }) => `fixtures/${path}`,
    row: (fixture) => ({ upstream: fixture.replace(/#.*$/u, ""), copy: "verbatim" }),
  },
  // Clippy's UI tests keep their upstream paths under fixtures/ (tests/ui/…, tests/ui-toml/<dir>/…), where
  // packs/rust/upstream/clippy/generate.py copies them.
  clippy: {
    file: ({ fixture }) => (fixture.startsWith("tests/") ? `fixtures/${fixture}` : null),
  },
};

/**
 * Every file under the tool-pack directory's fixtures/, by the sha256 of its bytes, and every case source `listed` (the
 * tool's rows in its licence's UPSTREAM.md, tool-relative) that a layout named for its bytes (`<sha256 prefix><ext>`),
 * on disk or not. A layout reuses a committed file of the same bytes, so where a case's source went depends on the
 * files there when it was first recorded; the listed names keep that placement when the derived files are made afresh
 * (npm run fixtures:derive), where only the generators' files are on disk.
 */
export function committedSources(directory: string, listed: readonly string[] = []): CommittedSources {
  const byDigest = new Map<string, string[]>();
  const walk = (relativeDirectory: string): void => {
    const absolute = join(directory, relativeDirectory);
    if (!existsSync(absolute)) return;
    for (const entry of readdirSync(absolute, { withFileTypes: true })) {
      const file = `${relativeDirectory}/${entry.name}`;
      if (entry.isDirectory()) walk(file);
      else if (entry.isFile()) {
        const digest = sha256(readFileSync(join(directory, file)));
        byDigest.set(digest, [...(byDigest.get(digest) ?? []), file]);
      }
    }
  };
  walk("fixtures");
  const byPrefix = new Map<string, string[]>();
  for (const file of listed) {
    const prefix = /^([0-9a-f]{12})(?:\.[^/]*)?$/u.exec(file.split("/").pop() ?? "")?.[1];
    if (prefix !== undefined && file.startsWith("fixtures/")) byPrefix.set(prefix, [...(byPrefix.get(prefix) ?? []), file]);
  }
  return (bytes) => {
    const digest = sha256(bytes);
    return [...new Set([...(byDigest.get(digest) ?? []), ...(byPrefix.get(digest.slice(0, 12)) ?? [])])].sort(byText);
  };
}

/** The tool's files a licence's UPSTREAM.md table lists (`| \`upstream/<tool>/<file>\` |`), tool-relative. */
export function listedCopies(text: string, tool: string): string[] {
  const prefix = `upstream/${tool}/`;
  return text.split("\n").flatMap((line) => {
    const file = /^\|\s*`([^`]+)`/u.exec(line)?.[1];
    return file !== undefined && file.startsWith(prefix) ? [file.slice(prefix.length)] : [];
  });
}

/**
 * Extensions a corpus could hold, probed against the adapter's inCorpus. `.blade.php` is a type of its own (a Blade
 * template), which a tool-pack that lints `.php` leaves out.
 */
const PROBED_EXTENSIONS = [".blade.php", ".cjs", ".cts", ".inc", ".ipynb", ".js", ".jsx", ".mjs", ".mts", ".php", ".py", ".pyi", ".rs", ".sql", ".ts", ".tsx", ".vue"];

/** A case source the recording needs committed: upstream's bytes, and where they go. */
export interface CaseCopy {
  /** Relative to the tool-pack directory. */
  file: string;
  fixture: string;
  bytes: Buffer;
}

/** The corpus answers a record leaves: the tree, packed, and how much of it upstream was just asked for. */
export interface RecordedCorpus {
  /** The tree's files by path. */
  tree: Map<string, Buffer>;
  packed: PackedCorpus;
  needed: NeededAnswers;
  /** The answers and verdicts upstream was asked for this time: those the tree started without. */
  asked: { answers: number; verdicts: number };
  /** The fresh files, by path, for --check to compare with the locked tree's. */
  written: Map<string, Buffer>;
}

export interface Recorded {
  recording: GatesRecording;
  corpus: RecordedCorpus;
  /** Every case source the recording names, with upstream's bytes; none for a corpus-only record. */
  sources: CaseCopy[];
  /** The Composer runtimes the adapter used and what each resolved: for the summary, not for the recording or its lock. */
  resolved: ResolvedRuntime[];
}

/**
 * How the corpus answers are recorded. By default upstream is asked only for the answers the locked archive lacks.
 * `only` keeps the locked recording.json and records nothing else (`--corpus`); `full` starts from an empty tree, so
 * every needed answer is asked again (`--full`, and `--check`, which compares them with the locked ones); `prune`
 * drops answers no golden needs (`--prune`).
 */
export interface CorpusMode {
  only?: boolean;
  full?: boolean;
  prune?: boolean;
}

export interface RecordOptions {
  root: string;
  pack: string;
  tool: string;
  cacheRoot: string;
  runner: CommandRunner;
  corpus?: CorpusMode;
  /** The live adapters by gates.yaml `adapter`; tests pass a fake. */
  adapters?: Partial<Record<GatesManifest["adapter"], AdapterFactory>>;
  /** Upstream's bytes of a test file, when the adapter's checkout has it; defaults to the checkout under the cache root. */
  upstreamBytes?: (fixture: string) => Buffer | undefined;
  /**
   * The golden corpus files to ask upstream about, each its own golden; defaults to the files the tool-pack's goldens
   * name, and the context files beside them, fetched into the cache.
   */
  corpusFiles?: (inCorpus: (path: string) => boolean) => Promise<CorpusFile[]>;
  /** Pin file digests of a runtime; defaults to the committed pin files'. */
  runtimeDigests?: (runtime: string) => Record<string, string>;
}

interface ConformanceEntry { upstream?: { tool?: string; rule?: string } }

/** Every upstream rule the pack's conformance manifests name for the tool (shipped, candidate, listed without a document). */
export async function conformanceRuleNames(root: string, pack: string, tool: string): Promise<string[]> {
  const names: string[] = [];
  for (const path of conformancePaths(pack)) {
    const text = await readFile(join(root, path), "utf8").catch(() => null);
    if (text === null) continue;
    const rules = (JSON.parse(text) as { rules?: ConformanceEntry[] }).rules ?? [];
    for (const entry of rules) if (entry.upstream?.tool === tool && typeof entry.upstream.rule === "string") names.push(entry.upstream.rule);
  }
  return sortedUnique(names);
}

/**
 * A case path the recording can hold. Ruff's linter tests read its notebook fixtures from a sibling crate
 * (`../ruff_notebook/…`); recording/schema.ts takes no `..` segment, so such a path is recorded as the fixture's
 * repository path. Those cases are notebooks, which no Rerail dialect reads, so no gate result depends on the path.
 */
function recordedPath(path: string, fixture: string): string {
  return path.split("/").includes("..") ? fixture : path;
}

/** The engine path a case is read at (the gates' readPath). */
function readPath(testCase: Pick<UpstreamCase, "path" | "parsedAs">): string {
  return testCase.parsedAs === undefined ? testCase.path : testCase.path.replace(/\.[^./]*$/u, testCase.parsedAs);
}

const spans = (found: Span[]): RecordedSpan[] => found.map(spanTuple).sort(compareSpans);

/** A set id from its content, so a set several rules share is stored once and named the same on every machine. */
const setId = (kind: string, value: unknown): string => `${kind}:${sha256(JSON.stringify(value)).slice(0, 16)}`;

/**
 * The corpus files the tool-pack's goldens name, and the context files upstream's answer on them depends on
 * (CONTEXT_FILES), and no others: the gates read upstream's answers on goldens alone, so upstream never runs over the
 * rest of the corpus (that is NRV's real-code gate). `goldens` are the goldens whose fetched file is their sha256, the
 * ones the gates judge. A task that could not be fetched throws: the recording would lack its answers.
 */
async function goldenCorpus(directory: string, cacheRoot: string, inCorpus: (path: string) => boolean, runner: CommandRunner, corpora: RerailCorpus[], context: (path: string) => string[]): Promise<{ files: CorpusFile[]; goldens: Golden[] }> {
  const named = [...(await readGoldens(directory)).values()].flat();
  const wanted = wantedPlaces([...named, ...named.flatMap((entry) => context(entry.path).map((path) => ({ task: entry.task, path })))]);
  const catalog = await buildCorpus(cacheRoot, inCorpus, runner, wanted);
  const failed = catalog.tasks.filter((task) => task.status === "failed");
  if (failed.length > 0) throw new Error(`${failedTasksMessage(failed)}: the recording would lack their answers; record again`);
  const files = [...catalog.files, ...(await buildRerailCorpora(corpora, cacheRoot, inCorpus, runner, wanted)).files];
  const fetched = new Map(files.map((file) => [wantedKey(file.task, file.path), file.sha256]));
  return { files, goldens: named.filter((golden) => fetched.get(wantedKey(golden.task, golden.path)) === golden.sha256) };
}

/** A tool-pack's live adapter and where its cases' sources go: `place` gives a case's file (null: not copied) and keeps its source. */
interface CasePlacement {
  manifest: GatesManifest;
  names: string[];
  adapter: PackAdapter;
  notCopied: string[];
  place(testCase: UpstreamCase): string | null;
  /** Every source placed so far, by file: the first case placed at a file supplies its bytes. */
  sources: Map<string, CaseCopy>;
}

async function casePlacement(options: RecordOptions): Promise<CasePlacement> {
  const { root, pack: packId, tool, cacheRoot, runner } = options;
  const manifest = readGatesManifest(root, packId, tool);
  // A tool-pack recorded before (it has a lock) has its case sources derived beside the lock (npm run fixtures:derive
  // runs the generators first): over a checkout that lacks them the recorder would take every source for new.
  const directory = toolPackDirectory(root, packId, tool);
  const caseSources = join(root, "packs", packId, "upstream", tool, "fixtures");
  if (existsSync(join(directory, RECORDING_LOCK)) && !existsSync(caseSources)) throw new Error(missingPackFixture(caseSources));
  const layout = CASE_LAYOUTS[manifest.adapter];
  const names = await conformanceRuleNames(root, packId, tool);
  const factory = options.adapters?.[manifest.adapter] ?? LIVE_ADAPTERS[manifest.adapter];
  const adapter = await factory(manifest, names, cacheRoot, runner);
  // The checkout the live adapters make, under the repository's name (rebuildFixtureTexts reads the same one).
  const checkout = upstreamCheckoutDirectory(cacheRoot, manifest.upstream);
  const upstreamBytes = options.upstreamBytes ?? ((fixture: string) => {
    const path = join(checkout, fixture);
    return existsSync(path) ? readFileSync(path) : undefined;
  });
  const fixtureBytes = new Map<string, Buffer | undefined>();
  const upstreamBytesOf = (fixture: string): Buffer | undefined => {
    if (!fixtureBytes.has(fixture)) fixtureBytes.set(fixture, upstreamBytes(fixture));
    return fixtureBytes.get(fixture);
  };
  const licence = join(root, "packs", packId, "licenses", manifest.rerail.project, "UPSTREAM.md");
  const committed = committedSources(directory, existsSync(licence) ? listedCopies(readFileSync(licence, "utf8"), tool) : []);
  const notCopied = notCopiedPrefixes(root, packId);
  const sources = new Map<string, CaseCopy>();
  return {
    manifest,
    names,
    adapter,
    notCopied,
    sources,
    place(testCase) {
      const bytes = upstreamBytesOf(testCase.fixture) ?? Buffer.from(testCase.source, "utf8");
      const file = notCopied.some((prefix) => testCase.fixture.startsWith(prefix)) ? null : layout.file({ fixture: testCase.fixture, path: readPath(testCase), bytes }, committed);
      if (file !== null && !sources.has(file)) sources.set(file, { file, fixture: testCase.fixture, bytes });
      return file;
    },
  };
}

/**
 * The case sources a recording of the tool-pack names, with upstream's bytes, sorted by file: what recordToolPack's
 * `sources` would be, without asking upstream about any corpus file or parser. `npm run fixtures:derive` writes them
 * after the tool-pack's generators.
 */
export async function recordCaseSources(options: RecordOptions): Promise<CaseCopy[]> {
  const { names, adapter, place, sources } = await casePlacement(options);
  for (const name of names) {
    if (adapter.rule(name) === undefined) continue;
    for (const testCase of adapter.cases(name)) place(testCase);
  }
  return [...sources.values()].sort((a, b) => byText(a.file, b.file));
}

/** Run the live adapter and build the recording and the corpus answers; writes nothing. */
export async function recordToolPack(options: RecordOptions): Promise<Recorded> {
  const placement = await casePlacement(options);
  const only = options.corpus?.only === true;
  const recording = only ? await lockedRecording(options, placement.manifest) : await recordRecording(options, placement);
  const corpus = await recordCorpus(options, placement.manifest, placement.adapter, recording);
  return { recording, corpus, sources: only ? [] : [...placement.sources.values()].sort((a, b) => byText(a.file, b.file)), resolved: resolvedComposerRuntimes() };
}

/** recording.json from the live adapter: every rule's facts, names, cases and the parsers' verdicts on their sources. */
async function recordRecording(options: RecordOptions, placement: CasePlacement): Promise<GatesRecording> {
  const { cacheRoot, tool } = options;
  const { manifest, names, adapter, notCopied, place, sources } = placement;

  const nameSets: GatesRecording["nameSets"] = {};
  const rules: Record<string, RecordedRule> = {};
  const absent: string[] = [];
  const verdictInputs = new Map<string, { path: string; script: boolean; source: string; sha256: string }>();
  const allCases: UpstreamCase[] = [];
  for (const name of names) {
    const facts = adapter.rule(name);
    if (facts === undefined) {
      absent.push(name);
      continue;
    }
    const regexes = adapter.implementationRegexes(name).map((regex) => ({ source: regex.source, flags: regex.flags }));
    const shared = sortedUnique(adapter.sharedImplementationNames(name));
    const implementationSets: string[] = [];
    const sharedSets: string[] = [];
    if (regexes.length > 0) {
      const id = setId("regexes", regexes);
      nameSets[id] = { names: [], regexes };
      implementationSets.push(id);
    }
    if (shared.length > 0) {
      const id = setId("shared-names", shared);
      nameSets[id] = { names: shared, regexes: [] };
      sharedSets.push(id);
    }
    const probe = adapter.toSpan(name, "a\nb", 0, 3);
    const cases: RecordedCase[] = adapter.cases(name).map((testCase) => {
      allCases.push(testCase);
      const file = place(testCase);
      if (file !== null) {
        const entry = { path: readPath({ ...testCase, path: recordedPath(testCase.path, testCase.fixture) }), script: testCase.script === true, source: testCase.source, sha256: sha256(testCase.source) };
        verdictInputs.set(`${entry.path}\0${entry.script}\0${entry.sha256}`, entry);
      }
      return {
        fixture: testCase.fixture,
        variant: testCase.variant,
        path: recordedPath(testCase.path, testCase.fixture),
        ...(testCase.parsedAs === undefined ? {} : { parsedAs: testCase.parsedAs }),
        ...(testCase.script === undefined ? {} : { script: testCase.script }),
        skipped: file === null ? "not-copied" : testCase.skipped,
        expected: spans(testCase.expected),
        markup: testCase.markup === null ? null : spans(testCase.markup),
        file,
        ...(file === null ? {} : { sha256: sha256(sources.get(file)!.bytes) }),
      };
    });
    rules[name] = {
      facts: {
        name: facts.name,
        implementation: facts.implementation,
        tests: facts.tests,
        deprecation: facts.deprecation,
        recommended: facts.recommended === null ? null : { config: facts.recommended.config, options: facts.recommended.options as never[] },
        notOnRealCode: facts.notOnRealCode,
      },
      names: {
        implementation: { sets: implementationSets, names: sortedUnique(adapter.implementationNames(name)), regexes: [] },
        shared: { sets: sharedSets, names: [], regexes: [] },
      },
      spanParts: { endLine: probe.endLine !== null, endCol: probe.endCol !== null },
      cases,
    };
  }

  // Every known gap's probe, read as the file type its fixtures share (the gates' provenGaps).
  const caseByFixture = new Map(allCases.map((testCase) => [testCase.fixture, testCase]));
  for (const gap of manifest.known_gaps) {
    const extensions = [...new Set(gap.fixtures.map((fixture) => caseByFixture.get(fixture)).filter((testCase) => testCase !== undefined).map((testCase) => extname(readPath(testCase))))];
    if (extensions.length !== 1 || extensions[0] === "") continue;
    const script = gap.fixtures.every((fixture) => caseByFixture.get(fixture)?.script === true);
    const entry = { path: `probe${extensions[0]}`, script, source: gap.probe, sha256: sha256(gap.probe) };
    verdictInputs.set(`${entry.path}\0${entry.script}\0${entry.sha256}`, entry);
  }
  const verdictKeys = [...verdictInputs.keys()].sort(byText);
  const verdictFiles = verdictKeys.map((key) => verdictInputs.get(key)!);
  const verdicts = await adapter.parserVerdicts(verdictFiles.map(({ path, source, script }) => ({ path, source, script })));

  // fixtureTexts by origin, rebuilt from upstream's checkout exactly as the gates will, and checked against the live list.
  const fixtureTexts = recordFixtureTexts(adapter, cacheRoot, manifest.upstream, notCopied);

  const extensions = PROBED_EXTENSIONS.filter((extension) => adapter.inCorpus(`src/example${extension}`));
  return gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
    tool,
    adapter: manifest.adapter,
    pins: { repository: manifest.upstream.repository, commit: manifest.upstream.commit, version: manifest.upstream.version, runtime: manifest.upstream.runtime },
    runtimes: currentRuntimes(options, manifest),
    identity: adapter.identity,
    packChecks: adapter.packChecks,
    licenseSha256: sha256(adapter.licenseText),
    licenseText: adapter.licenseText,
    inScopeSamples: adapter.inScopeSamples,
    corpus: {
      extensions,
      caseSensitive: extensions.length === 0 || !adapter.inCorpus(`src/example${extensions[0]!.toUpperCase()}`),
    },
    keywords: sortedUnique(adapter.keywords),
    commentVocabulary: sortedUnique(adapter.commentVocabulary),
    fixtureWords: sortedUnique(adapter.fixtureWords()),
    ...(fixtureTexts === undefined ? {} : { fixtureTexts }),
    nameSets,
    rules,
    absent: sortedUnique(absent),
    parsers: recordedParsers(verdicts),
    // A list every parser was silent on is left off: the entry keeps its identity, and an absent list means silence.
    parserVerdicts: verdictFiles.map(({ path, script, sha256: digest }, index) => ({
      path,
      script,
      sha256: digest,
      ...(verdicts[index]!.every(isCleanVerdict) ? {} : { verdicts: verdicts[index]! }),
    })),
  });
}

/** The recorder's runtime pin digests for every runtime gates.yaml installs, as recording.json and recording.lock record them. */
function currentRuntimes(options: RecordOptions, manifest: GatesManifest): GatesRecording["runtimes"] {
  const runtimeDigests = options.runtimeDigests ?? ((runtime: string) => runtimePinDigests(runtime));
  return Object.fromEntries(recordedRuntimes(manifest).map((runtime) => [runtime, runtimeDigests(runtime)]));
}

/**
 * Fetch the recordings the committed lock names from S3 into the recorder's cache (`<cacheRoot>/recordings`, which is
 * where it reads them); undefined when the fetch ran (S3 may still lack them), else why it could not (no aws, or not
 * signed in, as in CI's record legs, which hold no credentials).
 */
function fetchLocked(root: string, cacheRoot: string, pack: string, tool: string): string | undefined {
  try {
    fetchRecordings({ root, pack, tool, cache: join(cacheRoot, "recordings") });
    return undefined;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

/**
 * The recording.json the committed lock names, for a corpus-only record: fetched from S3 when the cache lacks it. Its
 * pins and runtime pin digests must be gates.yaml's and the committed runtime pins', since the answers are filed under
 * them (the config id): a tool-pack whose pins or runtimes moved is recorded whole.
 */
async function lockedRecording(options: RecordOptions, manifest: GatesManifest): Promise<GatesRecording> {
  const { root, pack, tool, cacheRoot } = options;
  const whole = `record the tool-pack whole (npm run port-readiness:record -- --pack ${pack} --tool ${tool})`;
  const lock = readLock(root, pack, tool);
  if (lock === null) throw new Error(`packs/${pack}/upstream/${tool} has no ${RECORDING_LOCK} to record the corpus answers of: ${whole}`);
  let text = readCachedRecording(cacheRoot, lock.files[RECORDING_FILE].sha256);
  let failure: string | undefined;
  if (text === undefined) {
    failure = fetchLocked(root, cacheRoot, pack, tool);
    text = readCachedRecording(cacheRoot, lock.files[RECORDING_FILE].sha256);
  }
  if (text === undefined) throw new Error(`the ${RECORDING_FILE} packs/${pack}/upstream/${tool}/${RECORDING_LOCK} names is not in ${cacheRoot}/recordings, and ${failure === undefined ? "S3 does not hold it" : `S3 could not be read (${failure})`}: ${whole}`);
  const recording = gatesRecordingSchema.parse(JSON.parse(text));
  const moved = (["repository", "commit", "version", "runtime"] as const).filter((field) => recording.pins[field] !== manifest.upstream[field]);
  if (moved.length > 0) throw new Error(`gates.yaml's upstream ${moved.join(", ")} moved since ${RECORDING_FILE} was recorded: ${whole}`);
  const runtimes = currentRuntimes(options, manifest);
  const runtimesMoved = sortedUnique([...Object.keys(runtimes), ...Object.keys(recording.runtimes)])
    .filter((runtime) => canonicalText(runtimes[runtime] ?? null) !== canonicalText(recording.runtimes[runtime] ?? null));
  if (runtimesMoved.length > 0) throw new Error(`the runtime pins of ${runtimesMoved.join(", ")} moved since ${RECORDING_FILE} was recorded, and new answers would be filed under the old runtimes' config: ${whole}`);
  return recording;
}

/** The committed lock, or null when there is none; one of another format is refused (readRecordingLock). */
function readLock(root: string, pack: string, tool: string): RecordingLock | null {
  return existsSync(join(toolPackDirectory(root, pack, tool), RECORDING_LOCK)) ? readRecordingLock(root, pack, tool) : null;
}

/**
 * The tree the committed lock names, from the cache or else S3; an empty one when there is no lock (nothing to
 * build on: every needed answer is asked). A lock whose archive is in neither, or that S3 cannot be read for,
 * throws, saying to record from no answers with --full.
 */
function lockedTree(root: string, cacheRoot: string, pack: string, tool: string): Map<string, Buffer> {
  const lock = readLock(root, pack, tool);
  if (lock === null) return new Map();
  const pin = lock.files[CORPUS_ARCHIVE];
  const label = `packs/${pack}/upstream/${tool}/${CORPUS_ARCHIVE}`;
  let directory = cachedCorpusTree(cacheRoot, pin, label);
  let failure: string | undefined;
  if (directory === undefined) {
    failure = fetchLocked(root, cacheRoot, pack, tool);
    directory = cachedCorpusTree(cacheRoot, pin, label);
  }
  if (directory === undefined) {
    throw new Error(`${label} (sha256 ${pin.sha256.slice(0, 12)}) is not in ${cacheRoot}/recordings, and ${failure === undefined ? "S3 does not hold it" : `S3 could not be read (${failure})`}: `
      + `record with --full to start from no answers (npm run port-readiness:record -- --pack ${pack} --tool ${tool} --full)`);
  }
  return treeFiles(directory);
}

/**
 * Upstream's answers on the golden corpus files, recorded incrementally: the needed answers (every golden's file for
 * every recorded rule at this config, and the parsers' verdicts on each) the starting tree lacks are asked of upstream
 * and written into the tree, which is packed canonically. Upstream is handed only those files, with the context files
 * beside them, and asked for each only its missing rules (every recorded rule, and every answer on the file written
 * again, for an adapter that ASKS_EVERY_RULE); for an adapter whose answers depend on the batch (BATCHED_ADAPTERS),
 * every fetched file of each task that lacks an answer is handed at once, for every recorded rule, and every answer of
 * the task is written again, as a full record writes it.
 */
async function recordCorpus(options: RecordOptions, manifest: GatesManifest, adapter: PackAdapter, recording: GatesRecording): Promise<RecordedCorpus> {
  const { root, pack, tool, cacheRoot, runner } = options;
  const mode = options.corpus ?? {};
  const inCorpus = (path: string) => adapter.inCorpus(path);
  const corpora = await readRerailCorpora(root, pack);
  const fetched = options.corpusFiles !== undefined
    ? await options.corpusFiles(inCorpus).then((files) => ({ files, goldens: files }))
    : await goldenCorpus(toolPackDirectory(root, pack, tool), cacheRoot, inCorpus, runner, corpora, CONTEXT_FILES[manifest.adapter]);
  const byPlace = new Map(fetched.files.map((file) => [wantedKey(file.task, file.path), file]));
  const rules = Object.keys(recording.rules).sort(byText);
  const context = fetchedContext([...byPlace.values()], corpusCommits(corpora, readCorpusTasks()), rules);
  const needed = neededAnswers({ manifest, runtimes: recording.runtimes, parsers: recording.parsers, rules, files: fetched.goldens, context });
  const previous = mode.full === true ? new Map<string, Buffer>() : lockedTree(root, cacheRoot, pack, tool);
  const missing = missingAnswers(memoryTree(previous), needed);
  const written = new Map<string, Buffer>();
  let askedAnswers = 0;

  // Upstream lints `linted` for `asked` rules, and each of `answers` is written from its answer on its file.
  const ask = async (linted: CorpusFile[], asked: string[], answers: readonly NeededAnswer[]): Promise<void> => {
    const upstream = new Map<string, UpstreamFile>((await adapter.lintCorpus(linted, asked)).map((answer) => [wantedKey(answer.file.task, answer.file.path), answer]));
    for (const { rule, file, path } of answers) {
      const answer = upstream.get(wantedKey(file.task, file.path));
      if (answer === undefined) throw new Error(`${pack}/${tool}: upstream gave no answer for corpus file ${file.task}/${file.path}`);
      written.set(path, Buffer.from(answerText({ excluded: answer.excluded, findings: answer.findings.get(rule) ?? [] }), "utf8"));
    }
    askedAnswers += answers.length;
  };

  if (BATCHED_ADAPTERS[manifest.adapter]) {
    for (const task of sortedUnique(missing.answers.map((answer) => answer.file.task))) {
      const linted = [...byPlace.values()].filter((file) => file.task === task).sort((a, b) => byText(a.path, b.path));
      await ask(linted, rules, needed.answers.filter((answer) => answer.file.task === task));
    }
  } else {
    // Each file is asked for only its missing rules (all of them, for an adapter that ASKS_EVERY_RULE): files that lack
    // the same rules are asked together.
    const neededAt = (answer: NeededAnswer) => wantedKey(answer.file.task, answer.file.path);
    const missingByPlace = new Map<string, NeededAnswer[]>();
    for (const answer of missing.answers) missingByPlace.set(neededAt(answer), [...(missingByPlace.get(neededAt(answer)) ?? []), answer]);
    if (ASKS_EVERY_RULE[manifest.adapter]) {
      const lacking = new Set(missing.answers.map((answer) => answer.path));
      for (const answer of needed.answers) if (!lacking.has(answer.path)) missingByPlace.get(neededAt(answer))?.push(answer);
      for (const answers of missingByPlace.values()) answers.sort((a, b) => byText(a.rule, b.rule));
    }
    const groups = new Map<string, { rules: string[]; places: string[] }>();
    for (const [place, answers] of missingByPlace) {
      const lacking = answers.map((answer) => answer.rule);
      const group = groups.get(lacking.join("\0")) ?? { rules: lacking, places: [] };
      group.places.push(place);
      groups.set(lacking.join("\0"), group);
    }
    for (const group of groups.values()) {
      const asked = group.places.map((place) => byPlace.get(place)!);
      // Upstream reads the context files beside the goldens' files, as it read them in the whole corpus.
      const contextFiles = asked.flatMap((file) => CONTEXT_FILES[manifest.adapter](file.path).map((path) => byPlace.get(wantedKey(file.task, path)))).filter((file): file is CorpusFile => file !== undefined);
      const linted = [...new Map([...asked, ...contextFiles].map((file) => [wantedKey(file.task, file.path), file])).values()].sort((a, b) => byText(a.task, b.task) || byText(a.path, b.path));
      await ask(linted, group.rules, group.places.flatMap((place) => missingByPlace.get(place)!));
    }
  }

  if (missing.verdicts.length > 0) {
    const verdicts = await adapter.parserVerdicts(missing.verdicts.map(({ file }) => ({ path: file.path, source: byPlace.get(wantedKey(file.task, file.path))!.source, script: false })));
    // One recorder run asks one set of parsers about cases and corpus files alike: recording.json names them.
    recordedParsers([cleanVerdicts(recording.parsers), ...verdicts]);
    missing.verdicts.forEach(({ path }, index) => written.set(path, Buffer.from(verdictText(verdicts[index]!), "utf8")));
  }
  const tree = nextTree(previous, needed, written, mode.prune === true);
  return { tree, packed: packCorpusTree(tree), needed, asked: { answers: askedAnswers, verdicts: missing.verdicts.length }, written };
}

/** What differs between a fresh recording and what the tool-pack's committed `recording.lock` names. */
export interface RecordingDiff {
  /** recording.json or corpus.tar.zst when the lock names other answers for it, or no lock is committed. */
  changed: string[];
  /** How the fresh corpus answers compare with the locked archive's: null when the lock names this very archive. */
  corpusNote: string | null;
  /** Case sources the directory lacks. */
  missing: CaseCopy[];
  /** Case sources the directory has with other bytes than upstream's. */
  conflicting: string[];
}

/**
 * A fresh recording against the committed lock. recording.json is compared by digest: the recordings are not in the
 * tree, and the lock is what the commit pins, so a lock naming other bytes is exactly what a stale recording is. The
 * corpus answers are compared file by file: the locked archive holds while it has every answer upstream was just
 * asked for, with the same content (answers no golden names any more do not make it stale).
 */
export function diffRecording(root: string, pack: string, tool: string, recorded: Recorded, cacheRoot: string): RecordingDiff {
  const directory = toolPackDirectory(root, pack, tool);
  const lock = readLock(root, pack, tool);
  const changed: string[] = [];
  if (lock === null || lock.files[RECORDING_FILE].sha256 !== sha256(Buffer.from(canonicalText(recorded.recording), "utf8"))) changed.push(RECORDING_FILE);
  let corpusNote: string | null = null;
  if (lock === null) {
    corpusNote = `no ${RECORDING_LOCK} names corpus answers`;
    changed.push(CORPUS_ARCHIVE);
  } else if (lock.files[CORPUS_ARCHIVE].sha256 !== recorded.corpus.packed.sha256) {
    let locked: Map<string, Buffer> | undefined;
    try {
      locked = lockedTree(root, cacheRoot, pack, tool);
    } catch (error) {
      corpusNote = `the locked ${CORPUS_ARCHIVE} could not be compared: ${error instanceof Error ? error.message : String(error)}`;
    }
    const differing = locked === undefined ? [] : [...recorded.corpus.written].filter(([path, bytes]) => !locked.get(path)?.equals(bytes)).map(([path]) => path);
    if (locked !== undefined && differing.length === 0) {
      corpusNote = `the locked ${CORPUS_ARCHIVE} holds all ${recorded.corpus.written.size} answer(s) and verdict(s) just recorded, identically`;
    } else {
      if (locked !== undefined) corpusNote = `the locked ${CORPUS_ARCHIVE} lacks or differs on ${differing.length} of the ${recorded.corpus.written.size} answer(s) and verdict(s) just recorded, e.g. ${differing[0]}`;
      changed.push(CORPUS_ARCHIVE);
    }
  }
  const missing: CaseCopy[] = [];
  const conflicting: string[] = [];
  for (const source of recorded.sources) {
    const path = join(directory, source.file);
    if (!existsSync(path)) missing.push(source);
    else if (!readFileSync(path).equals(source.bytes)) conflicting.push(source.file);
  }
  return { changed, corpusNote, missing, conflicting };
}

/** What `--check` makes of a fresh recording against the committed lock. */
export interface RecordingCheck extends RecordingDiff {
  /** What a record of the fresh answers would commit (over the base's tree, when it is given), and the archive it names. */
  lock: RecordingLock;
  packed: PackedCorpus;
  /** The committed archive, when a record of the fresh answers made it: what the cache lacks once the check holds. */
  committed: PackedCorpus | null;
  /** The per-rule summary against the base's recording, or the committed lock's when no base is given. */
  changes: RecordingChanges;
}

/**
 * `--check`: a recording that asked every needed answer again (`corpus.full`) against the committed lock. The corpus
 * answers hold when the committed archive is one a record of these answers makes, which needs none of its bytes: the
 * fresh tree itself, as a `--full` or `--prune` record leaves it, or the fresh answers over the tree the `base` lock
 * names, as an incremental record on that lock leaves it (the answers no golden needs kept). Failing both, they hold
 * when the committed archive, read and verified from the cache or S3, has every fresh answer identically
 * (diffRecording), so extra answers no golden names never make it stale. `base` is the lock the commit's was recorded
 * on (a pull request's base's), null when the base has none, and its archive is read from the cache alone.
 */
export function checkRecording(root: string, pack: string, tool: string, recorded: Recorded, options: { cacheRoot: string; base?: RecordingLock | null | undefined }): RecordingCheck {
  const { base, cacheRoot } = options;
  const diff = diffRecording(root, pack, tool, recorded, cacheRoot);
  const committed = readLock(root, pack, tool)?.files[CORPUS_ARCHIVE];
  const label = `the base's packs/${pack}/upstream/${tool}/${CORPUS_ARCHIVE}`;
  let tree = recorded.corpus.tree;
  let packed = recorded.corpus.packed;
  let baseNote: string | null = null;
  if (base != null) {
    const pin = base.files[CORPUS_ARCHIVE];
    let directory: string | undefined;
    try {
      directory = cachedCorpusTree(cacheRoot, pin, label);
    } catch (error) {
      baseNote = `${label} does not verify (${error instanceof Error ? error.message : String(error)})`;
    }
    if (directory !== undefined) {
      tree = nextTree(treeFiles(directory), recorded.corpus.needed, recorded.corpus.written, false);
      packed = packCorpusTree(tree);
    } else {
      baseNote ??= `${label} (sha256 ${pin.sha256.slice(0, 12)}) is not in the cache to build on`;
    }
  }
  const isCommitted = (candidate: PackedCorpus) => committed !== undefined && candidate.sha256 === committed.sha256 && candidate.files === committed.files && candidate.bytes === committed.bytes;
  const madeBy = isCommitted(recorded.corpus.packed) ? recorded.corpus.packed : isCommitted(packed) ? packed : null;
  let changed = diff.changed;
  let corpusNote = baseNote === null ? diff.corpusNote : `${diff.corpusNote === null ? "" : `${diff.corpusNote}; `}${baseNote}`;
  if (madeBy !== null && madeBy !== recorded.corpus.packed) {
    changed = changed.filter((file) => file !== CORPUS_ARCHIVE);
    corpusNote = `the locked ${CORPUS_ARCHIVE} is the ${recorded.corpus.written.size} answer(s) and verdict(s) just recorded over the base's archive (sha256 ${base!.files[CORPUS_ARCHIVE].sha256.slice(0, 12)}), as an incremental record on it makes them`;
  }
  const before = base === undefined ? readLockedPair(root, cacheRoot, pack, tool) : base === null ? null : readPair(cacheRoot, base, label);
  return {
    ...diff,
    changed,
    corpusNote,
    lock: buildRecordingLock(tool, recorded.recording.pins, recorded.recording.runtimes, canonicalText(recorded.recording), packed),
    packed,
    committed: madeBy,
    changes: diffRecordings(pack, tool, before, { recording: recorded.recording, corpus: tree }, recorded.resolved),
  };
}

/**
 * Add a row for each new copy to licenses/<project>/UPSTREAM.md's file table (`| \`upstream/<tool>/<file>\` |
 * \`<upstream path>\` | verbatim |`, or the layout's row). A table sorted as text stays sorted; in any other (the ESLint
 * table, by rule and case number), the new rows go after the last row of their directory, in order, or at the end.
 */
export function withUpstreamRows(text: string, tool: string, copies: CaseCopy[], layout?: CaseLayout): string {
  const lines = text.split("\n");
  const isRow = (line: string) => line.startsWith("| `upstream/");
  const first = lines.findIndex(isRow);
  if (first < 0) throw new Error("UPSTREAM.md has no file table to add the copies to");
  let last = first;
  while (last + 1 < lines.length && isRow(lines[last + 1]!)) last += 1;
  const existing = lines.slice(first, last + 1);
  const known = new Set(existing);
  const added = sortedUnique(copies.map((copy) => {
    const { upstream, copy: kind } = layout?.row?.(copy.fixture) ?? { upstream: copy.fixture, copy: "verbatim" };
    return `| \`upstream/${tool}/${copy.file}\` | \`${upstream}\` | ${kind} |`;
  })).filter((row) => !known.has(row));
  let table: string[];
  if (existing.every((row, index) => index === 0 || byText(existing[index - 1]!, row) <= 0)) {
    table = [...existing, ...added].sort(byText);
  } else {
    const directory = (row: string) => dirname(/^\| `([^`]*)`/u.exec(row)?.[1] ?? "");
    table = [...existing];
    for (const row of added) {
      const at = table.findLastIndex((line) => directory(line) === directory(row));
      if (at < 0) table.push(row);
      else table.splice(at + 1, 0, row);
    }
  }
  return [...lines.slice(0, first), ...table, ...lines.slice(last + 1)].join("\n");
}

/**
 * Where a recording's inline fixture texts come from: upstream test text that is not a file of its own (Ruff's test-module
 * string literals, the `code` of ESLint's RuleTester cases), kept verbatim in recording.json.
 */
export function inlineTextsSource(manifest: GatesManifest): string {
  if (manifest.adapter === "ruff") return "`crates/ruff_linter/src/rules/*/mod.rs`, `crates/ruff_linter/src/linter.rs`: their test modules' string literals";
  if (manifest.eslint !== undefined) return `${manifest.eslint.tests.map((glob) => `\`${glob}\``).join(", ")}: the \`code\` of each RuleTester case`;
  if (manifest.adapter === "psalm") return "`tests/**/*.php`: the `code` of each providerValidCodeParse() and providerInvalidCodeParse() case";
  if (manifest.adapter === "sqlfluff") return "`test/fixtures/rules/std_rule_cases/*.yml`: the `fail_str` and `pass_str` of each case";
  return "upstream's test files";
}

/** How the note that lists the recording in UPSTREAM.md starts; the recorder finds an earlier note by it. */
const RECORDING_NOTE_START = `> \`${RECORDING_FILE}\` is a port readiness recording`;

/** Whether an UPSTREAM.md holds the note that lists the recording (withRecordingNote's). */
export function hasRecordingNote(text: string): boolean {
  return text.split("\n").some((line) => line.startsWith(RECORDING_NOTE_START));
}

/** Width a written note is wrapped to, `> ` included. */
const NOTE_WIDTH = 120;

/**
 * The note that lists the recording in UPSTREAM.md when it holds upstream test text inline, as a Markdown blockquote
 * wrapped to NOTE_WIDTH: recording.json is then partly upstream's material (excerpts), like a copied fixture. It names
 * the licence files kept beside UPSTREAM.md (recorded-adapter.ts's licenseFiles), as they are named.
 */
export function recordingNote(source: string, licenses: string[]): string[] {
  if (licenses.length === 0) throw new Error("UPSTREAM.md has no licence file beside it for the recording note to name");
  const named = licenses.map((name) => `\`${name}\``);
  const kept = named.length === 1 ? `${named[0]!} beside this file is` : `${named.slice(0, -1).join(", ")} and ${named.at(-1)!} beside this file are`;
  const text = `${RECORDING_NOTE_START.slice(2)}, and no file of this pack, so it is recorded here rather than in the table above: `
    + `the tool-pack's \`${RECORDING_LOCK}\` names it by its sha256, and the bytes are an object in S3, not a file in git (docs/port-readiness.md). It carries `
    + `excerpts of this upstream's tests (${source}), redistributed under the licence above, together with Rerail's Apache-2.0; `
    + `${kept} this upstream's own, at the pinned commit.`;
  const lines: string[] = [];
  let line = ">";
  for (const word of text.split(" ")) {
    if (line !== ">" && line.length + 1 + word.length > NOTE_WIDTH) {
      lines.push(line);
      line = ">";
    }
    line = `${line} ${word}`;
  }
  return [...lines, line];
}

/**
 * List the recording in UPSTREAM.md, in a note under the file table and never as a row of it: pack check 6 reads the
 * table as the files the pack carries, and the recording is not one of them, so a row naming it fails the check. An
 * earlier note is replaced, and left as it is when it already says the same in other line breaks; without one, the
 * note goes after the table.
 */
export function withRecordingNote(text: string, source: string, licenses: string[]): string {
  const lines = text.split("\n");
  const isRow = (line: string) => line.startsWith("| `upstream/");
  const first = lines.findIndex(isRow);
  if (first < 0) throw new Error("UPSTREAM.md has no file table to list the recording under");
  let last = first;
  while (last + 1 < lines.length && isRow(lines[last + 1]!)) last += 1;
  const note = recordingNote(source, licenses);
  const at = lines.findIndex((line) => line.startsWith(RECORDING_NOTE_START));
  if (at < 0) return [...lines.slice(0, last + 1), "", ...note, ...lines.slice(last + 1)].join("\n");
  let end = at;
  while (end + 1 < lines.length && lines[end + 1]!.startsWith(">")) end += 1;
  const words = (quote: string[]) => quote.map((line) => line.replace(/^>\s?/u, "")).join(" ").split(/\s+/u).filter((word) => word !== "").join(" ");
  if (words(lines.slice(at, end + 1)) === words(note)) return text;
  return [...lines.slice(0, at), ...note, ...lines.slice(end + 1)].join("\n");
}

/** What a written recording left behind, for the caller to commit. */
export interface WrittenRecording extends RecordingDiff {
  /** The lock as it is now committed. */
  lock: RecordingLock;
  /** The per-rule summary against the recording the lock named before, a first recording, or why it was not compared. */
  changes: RecordingChanges;
}

/**
 * Write the recording into the local cache, the `recording.lock` that names it, and the missing case sources; list the
 * copies in UPSTREAM.md.
 *
 * The lock and UPSTREAM.md are committed. The copied case sources are derived pack test data, never committed:
 * `npm run fixtures:derive` writes them from the pins as this would (CONTRIBUTING.md, "Pack test data"). The
 * recordings are neither; they go to the cache the gates read, so iterating locally needs no network:
 * record, then run the gates, and they read what was just recorded because the lock names it.
 */
export async function writeRecording(root: string, pack: string, tool: string, recorded: Recorded, options: { cacheRoot: string }): Promise<WrittenRecording> {
  const directory = toolPackDirectory(root, pack, tool);
  const diff = diffRecording(root, pack, tool, recorded, options.cacheRoot);
  if (diff.conflicting.length > 0) throw new Error(`packs/${pack}/upstream/${tool}: the checkout's case sources differ from upstream's (derive them again: npm run fixtures:derive -- --pack ${pack} --tool ${tool}): ${diff.conflicting.slice(0, 5).join(", ")}`);

  // The summary compares against the recording the committed lock named, where it is still in the cache. Without a
  // lock the report says it is a first recording; with one whose recording cannot be read, that what moved is unknown.
  const before = readLockedPair(root, options.cacheRoot, pack, tool);
  const text = canonicalText(recorded.recording);
  const lock = buildRecordingLock(tool, recorded.recording.pins, recorded.recording.runtimes, text, recorded.corpus.packed);
  await writeCachedRecording(options.cacheRoot, text);
  cacheCorpusArchive(options.cacheRoot, recorded.corpus.packed);
  await writeFile(join(directory, RECORDING_LOCK), recordingLockText(lock));

  for (const copy of diff.missing) {
    await mkdir(dirname(join(directory, copy.file)), { recursive: true });
    await writeFile(join(directory, copy.file), copy.bytes);
  }
  const inline = recorded.recording.fixtureTexts?.inline.length ?? 0;
  if (diff.missing.length > 0 || inline > 0) {
    const manifest = readGatesManifest(root, pack, tool);
    const upstreamMd = join(root, "packs", pack, "licenses", manifest.rerail.project, "UPSTREAM.md");
    let text = await readFile(upstreamMd, "utf8");
    if (diff.missing.length > 0) text = withUpstreamRows(text, tool, diff.missing, CASE_LAYOUTS[manifest.adapter]);
    if (inline > 0) text = withRecordingNote(text, inlineTextsSource(manifest), licenseFiles(dirname(upstreamMd)));
    await writeFile(upstreamMd, text);
  }
  return { ...diff, lock, changes: diffRecordings(pack, tool, before, { recording: recorded.recording, corpus: recorded.corpus.tree }, recorded.resolved) };
}

/**
 * The recording the committed lock names, read from the cache: null when there is no lock, and why not when the lock
 * is there but its recording.json or corpus answers archive cannot be read (never recorded or fetched here:
 * `pack-fixtures.mjs recordings`, or CI's cache and S3 lacked it; or a lock of another format), which is not a
 * first recording. It is what a re-record is summarized against; a maintainer re-recording has usually just run the
 * gates on it.
 */
export function readLockedPair(root: string, cacheRoot: string, pack: string, tool: string): RecordedPair | UnreadRecording | null {
  const lockPath = join(toolPackDirectory(root, pack, tool), RECORDING_LOCK);
  if (!existsSync(lockPath)) return null;
  const lock = recordingLockSchema.safeParse(JSON.parse(readFileSync(lockPath, "utf8")));
  if (!lock.success) return { unavailable: `${RECORDING_LOCK} does not match its schema` };
  return readPair(cacheRoot, lock.data, `packs/${pack}/upstream/${tool}/${CORPUS_ARCHIVE}`);
}

/** The two recordings `lock` names, from the cache alone, or why they cannot be read (readLockedPair). */
function readPair(cacheRoot: string, lock: RecordingLock, label: string): RecordedPair | UnreadRecording {
  const recordingPin = lock.files[RECORDING_FILE];
  const corpusPin = lock.files[CORPUS_ARCHIVE];
  const text = readCachedRecording(cacheRoot, recordingPin.sha256);
  let tree: string | undefined;
  try {
    tree = cachedCorpusTree(cacheRoot, corpusPin, label);
  } catch (error) {
    return { unavailable: `the locked ${CORPUS_ARCHIVE} does not verify (${error instanceof Error ? error.message : String(error)})` };
  }
  const missing = [
    ...(text === undefined ? [`the locked ${RECORDING_FILE} (sha256 ${recordingPin.sha256.slice(0, 12)})`] : []),
    ...(tree === undefined ? [`the locked ${CORPUS_ARCHIVE} (sha256 ${corpusPin.sha256.slice(0, 12)})`] : []),
  ];
  if (text === undefined || tree === undefined) return { unavailable: `${missing.join(" and ")} ${missing.length === 1 ? "is" : "are"} not in the cache` };
  const recording = gatesRecordingSchema.safeParse(JSON.parse(text));
  if (!recording.success) return { unavailable: "the cached recording does not parse" };
  return { recording: recording.data, corpus: treeFiles(tree) };
}
