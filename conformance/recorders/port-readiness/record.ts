/**
 * The gates recorder: run a tool-pack's live adapter over the upstream rules the pack's conformance manifests name for
 * the tool, and write what the gates read (conformance/port-readiness/recording/): the recording archive into the
 * cache, the `recording.lock` naming it into `packs/<pack>/upstream/<tool>/`, and every case source the recording names
 * that the directory lacks. It records incrementally: upstream is asked only for what the locked archive lacks under
 * this branch's config (archive.ts says what names an answer) — the whole-tool facts when they are missing, the rules
 * without a complete record (all of them, for an adapter whose answers depend on the recorded rules, when those
 * changed: tree.ts's RECORDS_PER_RULE), and the parsers' verdicts no file holds yet — so a record with nothing missing
 * runs no upstream at all.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, extname, join } from "node:path";
import { missingPackFixture, recordings as fetchRecordings } from "../../../scripts/s3/pack-fixtures.mjs";

import { conformancePaths } from "./corpus.js";
import { readGatesManifest, recordedRuntimes, toolPackDirectory, type GatesManifest } from "../../languages/manifest.js";
import type { CommandRunner } from "../../process.js";
import {
  answerConfig, cacheArchive, cachedTree, configId, configPath, memoryTree, notCopiedPrefixes, packTree, parsersId, RECORDING_ARCHIVE, treeConfigs, treeFiles, type PackedArchive,
} from "../../port-readiness/recording/archive.js";
import { rulesDigest, verdictPath, verdictText } from "../../port-readiness/recording/verdicts.js";
import {
  canonicalText, cleanVerdicts, compareSpans, gatesRecordingSchema, recordedParsers, spanTuple,
  type GatesRecording, type RecordedCase, type RecordedRule, type RecordedSpan,
} from "../../port-readiness/recording/schema.js";
import { diffRecordings, type RecordingChanges, type ResolvedRuntime, type UnreadRecording } from "../../port-readiness/recording/diff.js";
import { buildRecordingLock, readRecordingLock, RECORDING_LOCK, recordingLockSchema, recordingLockText, type RecordingLock } from "../../port-readiness/recording/store.js";
import {
  loadRecording, loadRule, loadTool, nameSetPath, recordedRuleNames, RECORDS_PER_RULE, ruleDirectory, ruleFiles, textDigest, textPath, toolFiles, toolPath,
} from "../../port-readiness/recording/tree.js";
import { decodeSource, licenseFiles, rebuildFixtureTexts, upstreamCheckoutDirectory, type RecordedFixtureTexts } from "./recorded-adapter.js";
import type { PackAdapter, PackManifest, ParserVerdict, Span, UpstreamCase } from "./types.js";
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

/**
 * The live adapter's fixtureTextSources as the recording archive records them: files sorted by path, inline texts sorted. The
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

/** A case source the recording needs committed: upstream's bytes, and where they go. */
export interface CaseCopy {
  /** Relative to the tool-pack directory. */
  file: string;
  fixture: string;
  bytes: Buffer;
}

/** What a record leaves: the recording archive's tree, packed, read back, and what upstream was asked for. */
export interface Recorded {
  /** The tree's files by path. */
  tree: Map<string, Buffer>;
  packed: PackedArchive;
  /** The tree read back at its config, as the gates read it. */
  recording: GatesRecording;
  configId: string;
  /** What upstream was asked this time: the rules recorded, and the parsers' verdicts the tree lacked. */
  asked: { rules: string[]; verdicts: number };
  /** Every file this record wrote from upstream's answers, by path: for --check to compare with the locked tree's. */
  written: Map<string, Buffer>;
  /** Cases filed under an answer file another case of their rule has. */
  deduplicated: number;
  /** Every case source of the rules recorded this time, with upstream's bytes. */
  sources: CaseCopy[];
  /** The Composer runtimes the adapter used and what each resolved: for the summary, not for the recording or its lock. */
  resolved: ResolvedRuntime[];
}

/**
 * How a record is made. By default upstream is asked only for what the locked archive lacks. `full` starts from an
 * empty tree, so everything is asked again (`--full`, and `--check`, which compares it with the locked archive);
 * `prune` drops the files no recorded rule names (`--prune`).
 */
export interface RecordMode {
  full?: boolean;
  prune?: boolean;
}

export interface RecordOptions {
  root: string;
  pack: string;
  tool: string;
  cacheRoot: string;
  runner: CommandRunner;
  mode?: RecordMode;
  /** The live adapters by gates.yaml `adapter`; tests pass a fake. */
  adapters?: Partial<Record<GatesManifest["adapter"], AdapterFactory>>;
  /** Upstream's bytes of a test file, when the adapter's checkout has it; defaults to the checkout under the cache root. */
  upstreamBytes?: (fixture: string) => Buffer | undefined;
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
function readPath(testCase: { path: string; parsedAs?: string | undefined }): string {
  return testCase.parsedAs === undefined ? testCase.path : testCase.path.replace(/\.[^./]*$/u, testCase.parsedAs);
}

const spans = (found: Span[]): RecordedSpan[] => found.map(spanTuple).sort(compareSpans);

/** A set id from its content, so a set several rules share is stored once and named the same on every machine. */
const setId = (kind: string, value: unknown): string => `${kind}:${sha256(JSON.stringify(value)).slice(0, 16)}`;

/** A tool-pack's live adapter over some of its rules, and where their cases' sources go: `place` gives a case's file (null: not copied) and keeps its source. */
interface CasePlacement {
  adapter: PackAdapter;
  /** The rules the adapter was made for. */
  names: string[];
  notCopied: string[];
  place(testCase: UpstreamCase): string | null;
  /** Every source placed so far, by file: the first case placed at a file supplies its bytes. */
  sources: Map<string, CaseCopy>;
}

async function casePlacement(options: RecordOptions, manifest: PackManifest, names: string[]): Promise<CasePlacement> {
  const { root, pack: packId, tool, cacheRoot, runner } = options;
  // A tool-pack recorded before (it has a lock) has its case sources derived beside the lock (npm run fixtures:derive
  // runs the generators first): over a checkout that lacks them the recorder would take every source for new.
  const directory = toolPackDirectory(root, packId, tool);
  const caseSources = join(root, "packs", packId, "upstream", tool, "fixtures");
  if (existsSync(join(directory, RECORDING_LOCK)) && !existsSync(caseSources)) throw new Error(missingPackFixture(caseSources));
  const layout = CASE_LAYOUTS[manifest.adapter];
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
    adapter,
    names,
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
 * The case sources a recording of the tool-pack names, with upstream's bytes, sorted by file: what a full record's
 * `sources` would be, without asking upstream's parsers about anything. `npm run fixtures:derive` writes them
 * after the tool-pack's generators.
 */
export async function recordCaseSources(options: RecordOptions): Promise<CaseCopy[]> {
  const manifest = readGatesManifest(options.root, options.pack, options.tool);
  const names = await conformanceRuleNames(options.root, options.pack, options.tool);
  const { adapter, place, sources } = await casePlacement(options, manifest, names);
  for (const name of names) {
    if (adapter.rule(name) === undefined) continue;
    for (const testCase of adapter.cases(name)) place(testCase);
  }
  return [...sources.values()].sort((a, b) => byText(a.file, b.file));
}

/** A source the independent parsers are asked about: as a case reads it (or a known gap's probe), and its text. */
interface VerdictInput {
  path: string;
  script: boolean;
  source: string;
  sha256: string;
}

/** What the live adapter answers for some rules: their records, the name sets they name, which are absent upstream, and their case sources. */
interface RecordedRules {
  rules: Record<string, RecordedRule>;
  nameSets: GatesRecording["nameSets"];
  absent: string[];
  verdictInputs: VerdictInput[];
}

/** Every asked rule's facts, names, span parts and cases from the live adapter, its case sources placed. */
function recordRules(placement: CasePlacement, names: readonly string[]): RecordedRules {
  const { adapter, place, sources } = placement;
  const nameSets: GatesRecording["nameSets"] = {};
  const rules: Record<string, RecordedRule> = {};
  const absent: string[] = [];
  const verdictInputs: VerdictInput[] = [];
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
      const file = place(testCase);
      const path = recordedPath(testCase.path, testCase.fixture);
      if (file !== null) verdictInputs.push({ path: readPath({ ...testCase, path }), script: testCase.script === true, source: testCase.source, sha256: sha256(testCase.source) });
      return {
        fixture: testCase.fixture,
        variant: testCase.variant,
        path,
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
  return { rules, nameSets, absent, verdictInputs };
}

/** Every known gap's probe, read as the file type its fixtures share, as the gates' provenGaps asks the parsers about it. */
function probeInputs(manifest: GatesManifest, rules: Readonly<Record<string, RecordedRule>>): VerdictInput[] {
  const caseByFixture = new Map(Object.values(rules).flatMap((rule) => rule.cases).map((testCase) => [testCase.fixture, testCase]));
  return manifest.known_gaps.flatMap((gap) => {
    const extensions = [...new Set(gap.fixtures.map((fixture) => caseByFixture.get(fixture)).filter((testCase) => testCase !== undefined).map((testCase) => extname(readPath(testCase))))];
    if (extensions.length !== 1 || extensions[0] === "") return [];
    const script = gap.fixtures.every((fixture) => caseByFixture.get(fixture)?.script === true);
    return [{ path: `probe${extensions[0]}`, script, source: gap.probe, sha256: sha256(gap.probe) }];
  });
}

/** The whole-tool facts from the live adapter (everything but the name sets, absent rules and parsers, which come with the rules and verdicts). */
function liveToolFacts(options: RecordOptions, manifest: GatesManifest, placement: CasePlacement): Omit<GatesRecording, "rules" | "nameSets" | "absent" | "parsers"> {
  const { adapter, notCopied } = placement;
  // fixtureTexts by origin, rebuilt from upstream's checkout exactly as the gates will, and checked against the live list.
  const fixtureTexts = recordFixtureTexts(adapter, options.cacheRoot, manifest.upstream, notCopied);
  return {
    tool: options.tool,
    adapter: manifest.adapter,
    pins: { repository: manifest.upstream.repository, commit: manifest.upstream.commit, version: manifest.upstream.version, runtime: manifest.upstream.runtime },
    runtimes: currentRuntimes(options, manifest),
    identity: adapter.identity,
    packChecks: adapter.packChecks,
    licenseSha256: sha256(adapter.licenseText),
    licenseText: adapter.licenseText,
    inScopeSamples: adapter.inScopeSamples,
    keywords: sortedUnique(adapter.keywords),
    commentVocabulary: sortedUnique(adapter.commentVocabulary),
    fixtureWords: sortedUnique(adapter.fixtureWords()),
    ...(fixtureTexts === undefined ? {} : { fixtureTexts }),
  };
}

/** The recorder's runtime pin digests for every runtime gates.yaml installs, as the recording and recording.lock record them. */
function currentRuntimes(options: RecordOptions, manifest: GatesManifest): GatesRecording["runtimes"] {
  const runtimeDigests = options.runtimeDigests ?? ((runtime: string) => runtimePinDigests(runtime));
  return Object.fromEntries(recordedRuntimes(manifest).map((runtime) => [runtime, runtimeDigests(runtime)]));
}

/**
 * Fetch the recording the committed lock names from S3 into the recorder's cache (`<cacheRoot>/recordings`, which is
 * where it reads it); undefined when the fetch ran (S3 may still lack it), else why it could not (no aws, or not
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

/** The committed lock, or null when there is none; one of another format is refused (readRecordingLock). */
function readLock(root: string, pack: string, tool: string): RecordingLock | null {
  return existsSync(join(toolPackDirectory(root, pack, tool), RECORDING_LOCK)) ? readRecordingLock(root, pack, tool) : null;
}

/**
 * The tree the committed lock names, from the cache or else S3; an empty one when there is no lock (nothing to
 * build on: everything is asked). A lock whose archive is in neither, or that S3 cannot be read for, throws, saying to
 * record from nothing with --full.
 */
function lockedTree(root: string, cacheRoot: string, pack: string, tool: string): Map<string, Buffer> {
  const lock = readLock(root, pack, tool);
  if (lock === null) return new Map();
  const pin = lock.files[RECORDING_ARCHIVE];
  const label = `packs/${pack}/upstream/${tool}/${RECORDING_ARCHIVE}`;
  let directory = cachedTree(cacheRoot, pin, label);
  let failure: string | undefined;
  if (directory === undefined) {
    failure = fetchLocked(root, cacheRoot, pack, tool);
    directory = cachedTree(cacheRoot, pin, label);
  }
  if (directory === undefined) {
    throw new Error(`${label} (sha256 ${pin.sha256.slice(0, 12)}) is not in ${cacheRoot}/recordings, and ${failure === undefined ? "S3 does not hold it" : `S3 could not be read (${failure})`}: `
      + `record with --full to start from nothing (npm run port-readiness:record -- --pack ${pack} --tool ${tool} --full)`);
  }
  return treeFiles(directory);
}

const fileText = (value: unknown): Buffer => Buffer.from(canonicalText(value), "utf8");

/**
 * Run the live adapter over what the locked archive lacks and build the archive a record leaves; writes nothing.
 *
 * The tests part first. At this branch's config (archive.ts), the locked tree's whole-tool facts and every rule with a
 * complete record (its rule.json, every answer its index names and the parsers' verdict on each case source) are kept.
 * Upstream is asked about the rest: for an adapter that records per rule (tree.ts's RECORDS_PER_RULE), the rules the
 * pack names without a complete record, with the whole-tool facts kept; for any other, every rule and the whole-tool
 * facts, once anything is missing or the recorded rules are not the ones the pack names. A rule the pack no longer
 * names is dropped; the parsers are asked only about the sources and known-gap probes with no verdict yet.
 */
export async function recordToolPack(options: RecordOptions): Promise<Recorded> {
  const { root, pack, tool, cacheRoot } = options;
  const mode = options.mode ?? {};
  const manifest = readGatesManifest(root, pack, tool);
  const names = await conformanceRuleNames(root, pack, tool);
  const config = answerConfig(root, manifest, currentRuntimes(options, manifest));
  const id = configId(config);
  const label = `packs/${pack}/upstream/${tool}/${RECORDING_ARCHIVE}`;
  const directory = toolPackDirectory(root, pack, tool);
  const previous = mode.full === true ? new Map<string, Buffer>() : lockedTree(root, cacheRoot, pack, tool);
  const before = memoryTree(previous);
  const lockedTool = loadTool(before, id);
  const lockedParsers = lockedTool === undefined ? undefined : parsersId(manifest.adapter, lockedTool.parsers);
  const lockedRules = new Set(recordedRuleNames(before, id));
  const lockedAbsent = new Set(lockedTool?.absent ?? []);
  const perRule = RECORDS_PER_RULE[manifest.adapter];

  // A kept rule's case sources are the committed ones, read as the gates read them.
  const committedText = (file: string): string | undefined => (existsSync(join(directory, file)) ? decodeSource(readFileSync(join(directory, file))) : undefined);
  const caseVerdicts = new Set<string>();
  const kept = new Map<string, RecordedRule>();
  for (const name of lockedRules) {
    const rule = lockedParsers === undefined || !names.includes(name) ? undefined : loadRule(before, id, manifest.adapter, name);
    if (rule === undefined) continue;
    const paths = rule.cases.flatMap((testCase) => {
      const text = testCase.file === null ? undefined : committedText(testCase.file);
      return testCase.file === null ? [] : [text === undefined ? undefined : verdictPath(lockedParsers!, { path: readPath(testCase), sha256: sha256(text), script: testCase.script === true })];
    });
    if (paths.some((path) => path === undefined || before.text(path) === undefined)) continue;
    for (const path of paths) caseVerdicts.add(path!);
    kept.set(name, rule);
  }
  const sameRules = canonicalText([...lockedRules, ...lockedAbsent].sort(byText)) === canonicalText(names);
  let toRecord: string[];
  if (lockedTool === undefined) toRecord = names;
  else if (perRule) toRecord = names.filter((name) => !lockedAbsent.has(name) && !kept.has(name));
  else toRecord = sameRules && names.every((name) => lockedAbsent.has(name) || kept.has(name)) ? [] : names;
  const rewriteTool = lockedTool === undefined || (!perRule && toRecord.length > 0);

  // The live adapter, made once for the rules asked (every rule, for an adapter that does not record per rule), and
  // again only when the parsers are asked about rules it was not made for.
  let placement: CasePlacement | undefined;
  const live = async (asked: readonly string[]): Promise<CasePlacement> => {
    const wanted = perRule ? sortedUnique(asked) : names;
    if (placement === undefined || !wanted.every((name) => placement!.names.includes(name))) placement = await casePlacement(options, manifest, wanted);
    return placement;
  };
  const fresh: RecordedRules = toRecord.length === 0 ? { rules: {}, nameSets: {}, absent: [], verdictInputs: [] } : recordRules(await live(toRecord), toRecord);
  const sources = placement === undefined ? [] : [...(placement as CasePlacement).sources.values()].sort((a, b) => byText(a.file, b.file));

  // The rules the archive records: every rule the pack names that upstream has.
  const rules: Record<string, RecordedRule> = {};
  for (const name of names) {
    const rule = fresh.rules[name] ?? (toRecord.includes(name) ? undefined : kept.get(name));
    if (rule !== undefined) rules[name] = rule;
  }
  const absent = sortedUnique([...names.filter((name) => lockedAbsent.has(name) && !toRecord.includes(name)), ...fresh.absent]);

  // The parsers' verdicts: on every source and probe when the whole tool is recorded, else on those without one.
  const probes = probeInputs(manifest, rules);
  const inputs = [...new Map([...fresh.verdictInputs, ...probes].map((input) => [`${input.path}\0${input.script}\0${input.sha256}`, input])).values()];
  const asked = rewriteTool ? inputs : inputs.filter((input) => before.text(verdictPath(lockedParsers!, input)) === undefined);
  const verdicts: ParserVerdict[][] = asked.length === 0 ? [] : await (await live(toRecord)).adapter.parserVerdicts(asked.map(({ path, source, script }) => ({ path, source, script })));
  // One recorder run asks one set of parsers about every source: the tool's parsers name them.
  const parsers = rewriteTool ? recordedParsers(verdicts) : lockedTool!.parsers;
  if (!rewriteTool && verdicts.length > 0) recordedParsers([cleanVerdicts(parsers), ...verdicts]);
  const verdictsId = parsersId(manifest.adapter, parsers);

  const written = new Map<string, Buffer>([[configPath(id), fileText(config)]]);
  asked.forEach((input, index) => written.set(verdictPath(verdictsId, input), Buffer.from(verdictText(verdicts[index]!), "utf8")));
  let recording: GatesRecording | undefined;
  if (rewriteTool) {
    recording = gatesRecordingSchema.parse({ ...liveToolFacts(options, manifest, await live(toRecord)), nameSets: fresh.nameSets, rules, absent, parsers });
    for (const [path, bytes] of toolFiles(id, recording)) written.set(path, bytes);
  } else {
    // A rule absent upstream that the pack no longer names leaves absent.json even when nothing is asked.
    if (canonicalText(absent) !== canonicalText(lockedTool!.absent)) written.set(toolPath(id, "absent"), fileText(absent));
    for (const [setName, set] of Object.entries(fresh.nameSets)) written.set(nameSetPath(id, setName), fileText(set));
  }
  let deduplicated = 0;
  const keyRules = rulesDigest(Object.keys(rules));
  for (const [name, rule] of Object.entries(fresh.rules)) {
    const files = ruleFiles(id, manifest.adapter, keyRules, name, rule);
    for (const [path, bytes] of files.files) written.set(path, bytes);
    deduplicated += files.deduplicated;
  }

  // The locked files still current: this config's, but not the whole-tool files when they were recorded again, nor
  // the directory of a rule recorded again or no longer recorded; the verdicts under these parsers.
  const keptRules = new Set(Object.keys(rules).filter((name) => fresh.rules[name] === undefined).map((name) => ruleDirectory(id, name)));
  const current = (path: string): boolean => {
    if (path.startsWith(`tool/${id}/`) || path.startsWith("texts/")) return !rewriteTool;
    if (path.startsWith(`rules/${id}/`)) return keptRules.has(path.slice(0, path.indexOf("/", `rules/${id}/`.length) + 1));
    return path.startsWith(`verdicts/${verdictsId}/`);
  };
  let tree = new Map([...[...previous].filter(([path]) => current(path)), ...written]);
  recording = loadRecording(memoryTree(tree), id, label);
  if (mode.prune === true) {
    for (const input of [...fresh.verdictInputs, ...probes]) caseVerdicts.add(verdictPath(verdictsId, input));
    tree = prunedTree(tree, id, recording, caseVerdicts);
  }
  return {
    tree,
    packed: packTree(tree),
    recording,
    configId: id,
    asked: { rules: toRecord, verdicts: asked.length },
    written,
    deduplicated,
    sources,
    resolved: resolvedComposerRuntimes(),
  };
}

/**
 * `--prune`: the tree without what no recorded rule names: rule.json's answers it does not index, name sets no rule
 * names, texts the tool facts do not list, and verdicts not in `wanted` (the case sources' and probes').
 */
function prunedTree(tree: ReadonlyMap<string, Buffer>, id: string, recording: GatesRecording, wanted: ReadonlySet<string>): Map<string, Buffer> {
  const files = memoryTree(tree);
  const named = new Set<string>([configPath(id)]);
  for (const path of files.list(`tool/${id}/`)) if (!path.startsWith(`tool/${id}/name-sets/`)) named.add(path);
  for (const rule of Object.values(recording.rules)) for (const set of [...rule.names.implementation.sets, ...rule.names.shared.sets]) named.add(nameSetPath(id, set));
  for (const { text } of recording.fixtureTexts?.inline ?? []) named.add(textPath(textDigest(text)));
  for (const name of Object.keys(recording.rules)) {
    const ruleJson = `${ruleDirectory(id, name)}rule.json`;
    named.add(ruleJson);
    const index = JSON.parse(files.text(ruleJson)!) as { cases: { key?: string }[] };
    for (const { key } of index.cases) if (key !== undefined) named.add(`${ruleDirectory(id, name)}cases/${key}.json`);
  }
  return new Map([...tree].filter(([path]) => named.has(path) || wanted.has(path)));
}

/** What differs between a fresh recording and what the tool-pack's committed `recording.lock` names. */
export interface RecordingDiff {
  /** True when the lock names another archive that does not hold every file just recorded identically, or no lock is committed. */
  changed: boolean;
  /** How the fresh files compare with the locked archive's: null when the lock names this very archive. */
  note: string | null;
  /** Case sources the directory lacks. */
  missing: CaseCopy[];
  /** Case sources the directory has with other bytes than upstream's. */
  conflicting: string[];
}

/**
 * A fresh recording against the committed lock, file by file: the locked archive holds while it has every file the
 * record just wrote from upstream's answers, with the same content (files no rule names any more do not make it
 * stale).
 */
export function diffRecording(root: string, pack: string, tool: string, recorded: Recorded, cacheRoot: string): RecordingDiff {
  const directory = toolPackDirectory(root, pack, tool);
  const lock = readLock(root, pack, tool);
  let changed = false;
  let note: string | null = null;
  if (lock === null) {
    note = `no ${RECORDING_LOCK} names a recording`;
    changed = true;
  } else if (lock.files[RECORDING_ARCHIVE].sha256 !== recorded.packed.sha256) {
    let locked: Map<string, Buffer> | undefined;
    try {
      locked = lockedTree(root, cacheRoot, pack, tool);
    } catch (error) {
      note = `the locked ${RECORDING_ARCHIVE} could not be compared: ${error instanceof Error ? error.message : String(error)}`;
    }
    const differing = locked === undefined ? [] : [...recorded.written].filter(([path, bytes]) => !locked.get(path)?.equals(bytes)).map(([path]) => path);
    if (locked !== undefined && differing.length === 0) {
      note = `the locked ${RECORDING_ARCHIVE} holds all ${recorded.written.size} file(s) just recorded, identically`;
    } else {
      if (locked !== undefined) note = `the locked ${RECORDING_ARCHIVE} lacks or differs on ${differing.length} of the ${recorded.written.size} file(s) just recorded, e.g. ${differing[0]}`;
      changed = true;
    }
  }
  const missing: CaseCopy[] = [];
  const conflicting: string[] = [];
  for (const source of recorded.sources) {
    const path = join(directory, source.file);
    if (!existsSync(path)) missing.push(source);
    else if (!readFileSync(path).equals(source.bytes)) conflicting.push(source.file);
  }
  return { changed, note, missing, conflicting };
}

/** What `--check` makes of a fresh recording against the committed lock. */
export interface RecordingCheck extends RecordingDiff {
  /** What a record of the fresh answers would commit (over the base's tree, when it is given), and the archive it names. */
  lock: RecordingLock;
  packed: PackedArchive;
  /** The committed archive, when a record of the fresh answers made it: what the cache lacks once the check holds. */
  committed: PackedArchive | null;
  /** The per-rule summary against the base's recording, or the committed lock's when no base is given. */
  changes: RecordingChanges;
}

/**
 * `--check`: a recording that asked everything again (`full`) against the committed lock. It holds when the committed
 * archive is one a record of these answers makes, which needs none of its bytes: the fresh tree itself, as a `--full`
 * or `--prune` record leaves it, or the fresh files over the verdicts of the tree the `base` lock names, as an
 * incremental record on that lock leaves them. Failing both, it holds when the committed archive, read and verified
 * from the cache or S3, has every fresh file identically (diffRecording), so extra files no rule names never make it
 * stale. `base` is the lock the commit's was recorded
 * on (a pull request's base's), null when the base has none, and its archive is read from the cache alone.
 */
export function checkRecording(root: string, pack: string, tool: string, recorded: Recorded, options: { cacheRoot: string; base?: RecordingLock | null | undefined }): RecordingCheck {
  const { base, cacheRoot } = options;
  const diff = diffRecording(root, pack, tool, recorded, cacheRoot);
  const committed = readLock(root, pack, tool)?.files[RECORDING_ARCHIVE];
  const label = `the base's packs/${pack}/upstream/${tool}/${RECORDING_ARCHIVE}`;
  let tree = recorded.tree;
  let packed = recorded.packed;
  let baseNote: string | null = null;
  if (base != null) {
    const pin = base.files[RECORDING_ARCHIVE];
    let directory: string | undefined;
    try {
      directory = cachedTree(cacheRoot, pin, label);
    } catch (error) {
      baseNote = `${label} does not verify (${error instanceof Error ? error.message : String(error)})`;
    }
    if (directory !== undefined) {
      const verdictsId = parsersId(recorded.recording.adapter, recorded.recording.parsers);
      const keep = (path: string) => path.startsWith(`verdicts/${verdictsId}/`);
      tree = new Map([...[...treeFiles(directory)].filter(([path]) => keep(path)), ...recorded.tree]);
      packed = packTree(tree);
    } else {
      baseNote ??= `${label} (sha256 ${pin.sha256.slice(0, 12)}) is not in the cache to build on`;
    }
  }
  const isCommitted = (candidate: PackedArchive) => committed !== undefined && candidate.sha256 === committed.sha256 && candidate.files === committed.files && candidate.bytes === committed.bytes;
  const madeBy = isCommitted(recorded.packed) ? recorded.packed : isCommitted(packed) ? packed : null;
  let changed = diff.changed;
  let note = baseNote === null ? diff.note : `${diff.note === null ? "" : `${diff.note}; `}${baseNote}`;
  if (madeBy !== null && madeBy !== recorded.packed) {
    changed = false;
    note = `the locked ${RECORDING_ARCHIVE} is the ${recorded.written.size} file(s) just recorded over the base's archive (sha256 ${base!.files[RECORDING_ARCHIVE].sha256.slice(0, 12)}), as an incremental record on it makes them`;
  }
  const before = base === undefined ? readLockedRecording(root, cacheRoot, pack, tool) : base === null ? null : readRecording(cacheRoot, base, label);
  return {
    ...diff,
    changed,
    note,
    lock: buildRecordingLock(tool, recorded.recording.pins, recorded.recording.runtimes, packed),
    packed,
    committed: madeBy,
    changes: diffRecordings(pack, tool, before, recorded.recording, recorded.resolved),
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
 * string literals, the `code` of ESLint's RuleTester cases), kept verbatim in the recording archive.
 */
export function inlineTextsSource(manifest: GatesManifest): string {
  if (manifest.adapter === "ruff") return "`crates/ruff_linter/src/rules/*/mod.rs`, `crates/ruff_linter/src/linter.rs`: their test modules' string literals";
  if (manifest.eslint !== undefined) return `${manifest.eslint.tests.map((glob) => `\`${glob}\``).join(", ")}: the \`code\` of each RuleTester case`;
  if (manifest.adapter === "psalm") return "`tests/**/*.php`: the `code` of each providerValidCodeParse() and providerInvalidCodeParse() case";
  if (manifest.adapter === "sqlfluff") return "`test/fixtures/rules/std_rule_cases/*.yml`: the `fail_str` and `pass_str` of each case";
  return "upstream's test files";
}

/** How the note that lists the recording in UPSTREAM.md starts; the recorder finds an earlier note by it. */
const RECORDING_NOTE_START = `> \`${RECORDING_ARCHIVE}\` is a port readiness recording`;

/** Whether an UPSTREAM.md holds the note that lists the recording (withRecordingNote's). */
export function hasRecordingNote(text: string): boolean {
  return text.split("\n").some((line) => line.startsWith(RECORDING_NOTE_START));
}

/** Width a written note is wrapped to, `> ` included. */
const NOTE_WIDTH = 120;

/**
 * The note that lists the recording in UPSTREAM.md when it holds upstream test text inline, as a Markdown blockquote
 * wrapped to NOTE_WIDTH: the recording archive is then partly upstream's material (excerpts), like a copied fixture. It names
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
 * Write the recording archive into the local cache, the `recording.lock` that names it, and the missing case sources;
 * list the copies in UPSTREAM.md.
 *
 * The lock and UPSTREAM.md are committed. The copied case sources are derived pack test data, never committed:
 * `npm run fixtures:derive` writes them from the pins as this would (CONTRIBUTING.md, "Pack test data"). The
 * archive is neither; it goes to the cache the gates read, so iterating locally needs no network: record, then run
 * the gates, and they read what was just recorded because the lock names it.
 */
export async function writeRecording(root: string, pack: string, tool: string, recorded: Recorded, options: { cacheRoot: string }): Promise<WrittenRecording> {
  const directory = toolPackDirectory(root, pack, tool);
  const diff = diffRecording(root, pack, tool, recorded, options.cacheRoot);
  if (diff.conflicting.length > 0) throw new Error(`packs/${pack}/upstream/${tool}: the checkout's case sources differ from upstream's (derive them again: npm run fixtures:derive -- --pack ${pack} --tool ${tool}): ${diff.conflicting.slice(0, 5).join(", ")}`);

  // The summary compares against the recording the committed lock named, where it is still in the cache. Without a
  // lock the report says it is a first recording; with one whose recording cannot be read, that what moved is unknown.
  const before = readLockedRecording(root, options.cacheRoot, pack, tool);
  const lock = buildRecordingLock(tool, recorded.recording.pins, recorded.recording.runtimes, recorded.packed);
  cacheArchive(options.cacheRoot, recorded.packed);
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
  return { ...diff, lock, changes: diffRecordings(pack, tool, before, recorded.recording, recorded.resolved) };
}

/**
 * The recording the committed lock names, read from the cache: null when there is no lock, and why not when the lock
 * is there but its archive cannot be read (never recorded or fetched here: `pack-fixtures.mjs recordings`, or CI's
 * cache and S3 lacked it; or a lock of another format), which is not a first recording. It is what a re-record is
 * summarized against; a maintainer re-recording has usually just run the gates on it.
 */
export function readLockedRecording(root: string, cacheRoot: string, pack: string, tool: string): GatesRecording | UnreadRecording | null {
  const lockPath = join(toolPackDirectory(root, pack, tool), RECORDING_LOCK);
  if (!existsSync(lockPath)) return null;
  const lock = recordingLockSchema.safeParse(JSON.parse(readFileSync(lockPath, "utf8")));
  if (!lock.success) return { unavailable: `${RECORDING_LOCK} does not match its schema` };
  return readRecording(cacheRoot, lock.data, `packs/${pack}/upstream/${tool}/${RECORDING_ARCHIVE}`);
}

/** The recording `lock` names, from the cache alone, read at the config it holds, or why it cannot be read (readLockedRecording). */
function readRecording(cacheRoot: string, lock: RecordingLock, label: string): GatesRecording | UnreadRecording {
  const pin = lock.files[RECORDING_ARCHIVE];
  let directory: string | undefined;
  try {
    directory = cachedTree(cacheRoot, pin, label);
  } catch (error) {
    return { unavailable: `the locked ${RECORDING_ARCHIVE} does not verify (${error instanceof Error ? error.message : String(error)})` };
  }
  if (directory === undefined) return { unavailable: `the locked ${RECORDING_ARCHIVE} (sha256 ${pin.sha256.slice(0, 12)}) is not in the cache` };
  const tree = memoryTree(treeFiles(directory));
  const configs = treeConfigs(tree);
  try {
    if (configs.length !== 1) throw new Error(`it holds ${configs.length} configs`);
    return loadRecording(tree, configs[0]!, label);
  } catch (error) {
    return { unavailable: `the locked ${RECORDING_ARCHIVE} does not read as a recording (${error instanceof Error ? error.message : String(error)})` };
  }
}
