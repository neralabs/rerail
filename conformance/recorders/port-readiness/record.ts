/**
 * The gates' generator: run a tool-pack's live adapter over every upstream rule the pack's conformance manifests name
 * for the tool, and build what the gates read (conformance/port-readiness/recording/schema.ts). `npm run generate`
 * (expected-cli.ts) splits it into the expected answers (recording/expected.ts), which it keeps in the local cache and
 * uploads to Azure Blob Storage, and copies every case source the answers name that the directory lacks.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, extname, join } from "node:path";

import { buildCorpus, buildRerailCorpora, conformancePaths, CORPUS_TASKS, failedTasksMessage, readRerailCorpora, wantedKey, wantedPlaces, type CorpusFile, type RerailCorpus, type Wanted } from "../../port-readiness/corpus.js";
import { readGoldens } from "../../port-readiness/goldens.js";
import { readGatesManifest, recordedRuntimes, toolPackDirectory, type GatesManifest } from "../../port-readiness/manifest.js";
import type { CommandRunner } from "../../port-readiness/process.js";
import {
  CORPUS_FORMAT, RECORDING_FORMAT, gatesCorpusSchema, gatesRecordingSchema, isCleanVerdict, recordedParsers, spanTuple,
  type GatesCorpusRecording, type GatesRecording, type RecordedCase, type RecordedRule, type RecordedSpan,
} from "../../port-readiness/recording/schema.js";
import type { ResolvedRuntime } from "../../port-readiness/recording/diff.js";
import { expectedFiles, testFileOf } from "../../port-readiness/recording/expected.js";
import { decodeSource, rebuildFixtureTexts, upstreamCheckoutDirectory, type RecordedFixtureTexts } from "../../port-readiness/upstream-files.js";
import type { PackAdapter, PackManifest, Span, UpstreamCase } from "../../port-readiness/types.js";
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
 * The live adapter's fixtureTextSources as the recording records them: files sorted by path, inline texts sorted; and
 * the upstream test file each inline text was read from, in the same order (expected/texts/ keeps them by it). The
 * list the gates rebuild from it (rebuildFixtureTexts, on the live adapter's checkout) must equal the live
 * fixtureTexts() as a multiset, or nothing is written.
 */
export function recordFixtureTexts(adapter: PackAdapter, cacheRoot: string, pins: { repository: string; commit: string }, notCopied: readonly string[]): { recorded: RecordedFixtureTexts; inlineTests: string[] } | undefined {
  const origins = adapter.fixtureTextSources?.();
  if (origins === undefined) return undefined;
  const live = adapter.fixtureTexts();
  if (origins.length !== live.length) throw new Error(`the adapter gives ${origins.length} fixture text origin(s) for ${live.length} fixture text(s)`);
  const files = new Map<string, { path: string; sha256: string; transform?: "notebook-code-cells" }>();
  const inline: { text: string; test: string }[] = [];
  for (const origin of origins) {
    if ("inline" in origin) {
      inline.push({ text: origin.inline, test: origin.test });
      continue;
    }
    if (files.has(origin.path)) throw new Error(`fixture text origin ${origin.path} is listed twice`);
    files.set(origin.path, { path: origin.path, sha256: origin.sha256, ...(origin.transform === undefined ? {} : { transform: origin.transform }) });
  }
  inline.sort((a, b) => byText(a.text, b.text) || byText(a.test, b.test));
  const recorded: RecordedFixtureTexts = {
    files: [...files.keys()].sort(byText).map((path) => files.get(path)!),
    inline: inline.map(({ text }) => ({ text })),
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
  return { recorded, inlineTests: inline.map(({ test }) => test) };
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

/** Every file under the tool-pack directory's fixtures/, by the sha256 of its bytes. */
export function committedSources(directory: string): CommittedSources {
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
  for (const files of byDigest.values()) files.sort(byText);
  return (bytes) => byDigest.get(sha256(bytes)) ?? [];
}

/** A path's directories, innermost first: `a/b/c.py` gives `a/b` and `a`. */
function directoriesOf(path: string): string[] {
  const parts = path.split("/").slice(0, -1);
  return parts.map((_, index) => parts.slice(0, parts.length - index).join("/"));
}

/**
 * The corpus files whose presence changes upstream's answer on a file, by adapter. Ruff resolves a file's package, and
 * so its module name (N999), from the `__init__.py` files in the directories above it; the whole corpus held them, so
 * the recorder fetches them and hands them to upstream beside the goldens' files, and records answers for the goldens'
 * files alone. Bandit, ESLint and SQLFluff read each file on its own. Clippy lints whole crates, so its adapter checks
 * out each pinned corpus whole for cargo, and needs no context file here.
 */
export const CONTEXT_FILES: Record<GatesManifest["adapter"], (path: string) => string[]> = {
  ruff: (path) => directoriesOf(path).flatMap((directory) => [`${directory}/__init__.py`, `${directory}/__init__.pyi`]),
  bandit: () => [],
  eslint: () => [],
  phpcs: () => [],
  tlint: () => [],
  "blade-parser": () => [],
  phpstan: () => [],
  psalm: () => [],
  bladestan: () => [],
  sqlfluff: () => [],
  clippy: () => [],
};

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

export interface Recorded {
  recording: GatesRecording;
  corpus: GatesCorpusRecording;
  /** Every case source the recording names, with upstream's bytes. */
  sources: CaseCopy[];
  /** The Composer runtimes the adapter used and what each resolved: for the summary, not for the answers or their key. */
  resolved: ResolvedRuntime[];
  /** The upstream test file each of `recording.fixtureTexts.inline` was read from, in that order. */
  inlineTests: string[];
  /** sha256 of the bytes of each upstream test file the cases come from, at the pinned commit; null where the checkout has no such file. */
  testFileDigests: ReadonlyMap<string, string | null>;
}

export interface RecordOptions {
  root: string;
  pack: string;
  tool: string;
  cacheRoot: string;
  runner: CommandRunner;
  /** The live adapters by gates.yaml `adapter`; tests pass a fake. */
  adapters?: Partial<Record<GatesManifest["adapter"], AdapterFactory>>;
  /** Upstream's bytes of a test file, when the adapter's checkout has it; defaults to the checkout under the cache root. */
  upstreamBytes?: (fixture: string) => Buffer | undefined;
  /** The corpus files to ask upstream about; defaults to the files the tool-pack's goldens name, fetched into the cache. */
  corpusFiles?: (inCorpus: (path: string) => boolean) => Promise<CorpusFile[]>;
  /** sha256 of corpus-tasks.json; defaults to the committed file's. */
  tasksSha256?: string;
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

const compareSpans = (a: RecordedSpan, b: RecordedSpan): number => {
  for (let index = 0; index < 4; index += 1) {
    const left = a[index] ?? -1;
    const right = b[index] ?? -1;
    if (left !== right) return left - right;
  }
  return 0;
};
const spans = (found: Span[]): RecordedSpan[] => found.map(spanTuple).sort(compareSpans);

/** A set id from its content, so a set several rules share is stored once and named the same on every machine. */
const setId = (kind: string, value: unknown): string => `${kind}:${sha256(JSON.stringify(value)).slice(0, 16)}`;

/**
 * The corpus files the tool-pack's goldens name, and the context files upstream's answer on them depends on
 * (CONTEXT_FILES), and no others: the gates read upstream's answers on goldens alone, so upstream never runs over the
 * rest of the corpus (that is NRV's real-code gate). `golden` is the places to record. A task that could not be fetched
 * throws: the recording would lack its answers.
 */
async function goldenCorpus(directory: string, cacheRoot: string, inCorpus: (path: string) => boolean, runner: CommandRunner, corpora: RerailCorpus[], context: (path: string) => string[]): Promise<{ files: CorpusFile[]; golden: Wanted }> {
  const named = [...(await readGoldens(directory)).values()].flat();
  const golden = wantedPlaces(named);
  const wanted = wantedPlaces([...named, ...named.flatMap((entry) => context(entry.path).map((path) => ({ task: entry.task, path })))]);
  const catalog = await buildCorpus(cacheRoot, inCorpus, runner, wanted);
  const failed = catalog.tasks.filter((task) => task.status === "failed");
  if (failed.length > 0) throw new Error(`${failedTasksMessage(failed)}: the recording would lack their answers; record again`);
  return { files: [...catalog.files, ...(await buildRerailCorpora(corpora, cacheRoot, inCorpus, runner, wanted)).files], golden };
}

/** Run the live adapter and build the recording; writes nothing. */
export async function recordToolPack(options: RecordOptions): Promise<Recorded> {
  const { root, pack: packId, tool, cacheRoot, runner } = options;
  const manifest = readGatesManifest(root, packId, tool);
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
  const committed = committedSources(toolPackDirectory(root, packId, tool));
  const notCopied = notCopiedPrefixes(root, packId);

  const nameSets: GatesRecording["nameSets"] = {};
  const rules: Record<string, RecordedRule> = {};
  const absent: string[] = [];
  const sources = new Map<string, CaseCopy>();
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
      const bytes = upstreamBytesOf(testCase.fixture) ?? Buffer.from(testCase.source, "utf8");
      const file = notCopied.some((prefix) => testCase.fixture.startsWith(prefix)) ? null : layout.file({ fixture: testCase.fixture, path: readPath(testCase), bytes }, committed);
      if (file !== null && !sources.has(file)) sources.set(file, { file, fixture: testCase.fixture, bytes });
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
  const testFileDigests = new Map<string, string | null>();
  for (const testCase of allCases) {
    const test = testFileOf(testCase.fixture);
    if (!testFileDigests.has(test)) {
      const bytes = upstreamBytesOf(test);
      testFileDigests.set(test, bytes === undefined ? null : sha256(bytes));
    }
  }

  const extensions = PROBED_EXTENSIONS.filter((extension) => adapter.inCorpus(`src/example${extension}`));
  const runtimeDigests = options.runtimeDigests ?? ((runtime: string) => runtimePinDigests(runtime));
  const recording = gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
    tool,
    adapter: manifest.adapter,
    pins: { repository: manifest.upstream.repository, commit: manifest.upstream.commit, version: manifest.upstream.version, runtime: manifest.upstream.runtime },
    runtimes: Object.fromEntries(recordedRuntimes(manifest).map((runtime) => [runtime, runtimeDigests(runtime)])),
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
    ...(fixtureTexts === undefined ? {} : { fixtureTexts: fixtureTexts.recorded }),
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

  // Upstream's answers on the files the goldens name, from the pinned tasks' solutions and the corpora the pack pins.
  const inCorpus = (path: string) => adapter.inCorpus(path);
  const corpora = await readRerailCorpora(root, packId);
  const fetched = options.corpusFiles !== undefined
    ? { files: await options.corpusFiles(inCorpus), golden: null }
    : await goldenCorpus(toolPackDirectory(root, packId, tool), cacheRoot, inCorpus, runner, corpora, CONTEXT_FILES[manifest.adapter]);
  const unique = new Map<string, CorpusFile>();
  for (const file of fetched.files) if (!unique.has(`${file.task}\0${file.path}`)) unique.set(`${file.task}\0${file.path}`, file);
  const linted = [...unique.keys()].sort(byText).map((key) => unique.get(key)!);
  const recorded = Object.keys(rules).sort(byText);
  // Upstream reads the context files beside the goldens' files, as it read them in the whole corpus; only the goldens'
  // files are recorded.
  const answers = await adapter.lintCorpus(linted, recorded);
  const answerOf = new Map(answers.map((answer) => [`${answer.file.task}\0${answer.file.path}`, answer]));
  const corpusFiles = fetched.golden === null ? linted : linted.filter((file) => fetched.golden.has(wantedKey(file.task, file.path)));
  const corpusVerdicts = await adapter.parserVerdicts(corpusFiles.map((file) => ({ path: file.path, source: file.source, script: false })));
  const corpus = gatesCorpusSchema.parse({
    format: CORPUS_FORMAT,
    tool,
    pins: {
      tasks: options.tasksSha256 ?? sha256(readFileSync(CORPUS_TASKS)),
      corpora: corpora.map(({ name, repository, commit, paths, exclude }) => ({ name, repository, commit, paths, exclude })),
    },
    rules: recorded,
    // The same parsers the recording names: one generator run asked one set of parsers about cases and corpus files
    // alike, and a tool upstream cannot lint real code for (javascript/eslint) has no corpus file to read them off.
    parsers: recording.parsers,
    files: corpusFiles.map((file, index) => {
      const answer = answerOf.get(`${file.task}\0${file.path}`);
      if (answer === undefined) throw new Error(`${packId}/${tool}: upstream gave no answer for corpus file ${file.task}/${file.path}`);
      const findings = answer.excluded !== null ? {} : Object.fromEntries([...answer.findings]
        .filter(([, found]) => found.length > 0)
        .sort(([a], [b]) => byText(a, b))
        .map(([rule, found]) => [rule, spans(found)]));
      return {
        task: file.task,
        path: file.path,
        sha256: file.sha256,
        excluded: answer.excluded,
        findings,
        ...(corpusVerdicts[index]!.every(isCleanVerdict) ? {} : { verdicts: corpusVerdicts[index]! }),
      };
    }),
  });

  return {
    recording,
    corpus,
    sources: [...sources.values()].sort((a, b) => byText(a.file, b.file)),
    resolved: resolvedComposerRuntimes(),
    inlineTests: fixtureTexts?.inlineTests ?? [],
    testFileDigests,
  };
}

/** The case sources a recording names that the tool-pack directory lacks, and those it has with other bytes than upstream's. */
export function caseSourceDiff(directory: string, recorded: Pick<Recorded, "sources">): { missing: CaseCopy[]; conflicting: string[] } {
  const missing: CaseCopy[] = [];
  const conflicting: string[] = [];
  for (const source of recorded.sources) {
    const path = join(directory, source.file);
    if (!existsSync(path)) missing.push(source);
    else if (!readFileSync(path).equals(source.bytes)) conflicting.push(source.file);
  }
  return { missing, conflicting };
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
 * The expected answers for a fresh recording, by path under expected/ (recording/expected.ts's expectedFiles). A case
 * source's text is read as the gates read it: the bytes the recording copies, or the committed file.
 */
export function generatedFiles(root: string, pack: string, tool: string, recorded: Recorded): Map<string, string> {
  const directory = toolPackDirectory(root, pack, tool);
  const copies = new Map(recorded.sources.map((source) => [source.file, source.bytes]));
  return expectedFiles({
    recording: recorded.recording,
    corpus: recorded.corpus,
    inlineTests: recorded.inlineTests,
    testFileDigest(test) {
      const digest = recorded.testFileDigests.get(test);
      if (digest === undefined) throw new Error(`the recorder read no digest of the test file ${test}`);
      return digest;
    },
    sourceText: (file) => decodeSource(copies.get(file) ?? readFileSync(join(directory, file))),
  });
}

/**
 * Copy into the tool-pack the case sources the recording names that it lacks, and list each copy in
 * licenses/<project>/UPSTREAM.md's file table; returns the copies. A committed case source with other bytes than
 * upstream's stops it before anything is written.
 */
export async function writeCaseSources(root: string, pack: string, tool: string, recorded: Pick<Recorded, "sources">): Promise<CaseCopy[]> {
  const directory = toolPackDirectory(root, pack, tool);
  const { missing, conflicting } = caseSourceDiff(directory, recorded);
  if (conflicting.length > 0) throw new Error(`packs/${pack}/upstream/${tool}: committed case sources differ from upstream's: ${conflicting.slice(0, 5).join(", ")}`);
  if (missing.length === 0) return [];
  for (const copy of missing) {
    await mkdir(dirname(join(directory, copy.file)), { recursive: true });
    await writeFile(join(directory, copy.file), copy.bytes);
  }
  const manifest = readGatesManifest(root, pack, tool);
  const upstreamMd = join(root, "packs", pack, "licenses", manifest.rerail.project, "UPSTREAM.md");
  await writeFile(upstreamMd, withUpstreamRows(await readFile(upstreamMd, "utf8"), tool, missing, CASE_LAYOUTS[manifest.adapter]));
  return missing;
}
