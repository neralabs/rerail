/**
 * The gates recorder: run a tool-pack's live adapter over every upstream rule the pack's conformance manifests name for
 * the tool, and write what the gates read (conformance/port-readiness/recording/schema.ts): recording.json and
 * corpus.json into the cache, the `recording.lock` naming them into `packs/<pack>/upstream/<tool>/`, and every case
 * source the recording names that the directory lacks.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, extname, join, resolve } from "node:path";

import { buildCorpus, buildRerailCorpora, conformancePaths, CORPUS_TASKS, failedTasksMessage, readRerailCorpora, wantedKey, wantedPlaces, type CorpusFile, type RerailCorpus, type Wanted } from "../../port-readiness/corpus.js";
import { readGoldens } from "../../port-readiness/goldens.js";
import { readGatesManifest, toolPackDirectory, type GatesManifest } from "../../port-readiness/manifest.js";
import type { CommandRunner } from "../../port-readiness/process.js";
import {
  CORPUS_FILE, CORPUS_FORMAT, RECORDING_FILE, RECORDING_FORMAT, gatesCorpusSchema, gatesRecordingSchema, isCleanVerdict, recordedParsers, recordingText, spanTuple,
  type GatesCorpusRecording, type GatesRecording, type RecordedCase, type RecordedRule, type RecordedSpan,
} from "../../port-readiness/recording/schema.js";
import { diffRecordings, type RecordedPair, type RecordingChanges, type UnreadRecording } from "../../port-readiness/recording/diff.js";
import {
  buildRecordingLock, DEFAULT_RELEASE, gzipRecording, readCachedRecording, RECORDING_LOCK, recordingLockSchema, recordingLockText, writeCachedRecording, type RecordingLock,
} from "../../port-readiness/recording/store.js";
import { rebuildFixtureTexts, upstreamCheckoutDirectory, type RecordedFixtureTexts } from "../../port-readiness/recorded-adapter.js";
import type { PackAdapter, PackManifest, Span, UpstreamCase } from "../../port-readiness/types.js";
import { createBanditAdapter } from "./adapters/bandit.js";
import { createEslintAdapter } from "./adapters/eslint.js";
import { createRuffAdapter } from "./adapters/ruff.js";
import { runtimePinDigests } from "./runtimes.js";

export type AdapterFactory = (pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner) => Promise<PackAdapter>;

export const LIVE_ADAPTERS: Record<GatesManifest["adapter"], AdapterFactory> = {
  bandit: createBanditAdapter,
  ruff: createRuffAdapter,
  eslint: createEslintAdapter,
};

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");
const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);
const sortedUnique = (values: Iterable<string>): string[] => [...new Set([...values].filter((value) => value !== ""))].sort(byText);

/** Ruff's fixture directories whose files derive from GPL-licensed projects: their text is never written into a recording. */
const GPL_DERIVED = /(?:^|\/)resources\/test\/fixtures\/(?:pylint|flake8_builtins)\//u;

/**
 * The live adapter's fixtureTextSources as recording.json records them: files sorted by path, inline texts sorted. The
 * list the gates rebuild from it (rebuildFixtureTexts, on the live adapter's checkout) must equal the live
 * fixtureTexts() as a multiset, or nothing is written.
 */
export function recordFixtureTexts(adapter: PackAdapter, cacheRoot: string, pins: { repository: string; commit: string }): RecordedFixtureTexts | undefined {
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
  // A GPL-derived fixture's text must never be inline; it is fetched from upstream by path instead.
  const gplTexts = new Set(live.filter((_, index) => { const origin = origins[index]!; return !("inline" in origin) && GPL_DERIVED.test(origin.path); }));
  const leaked = recorded.inline.filter(({ text }) => text.length > 0 && gplTexts.has(text)).length;
  if (leaked > 0) throw new Error(`${leaked} inline fixture text(s) are the text of a GPL-derived fixture`);
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
/** Ruff's fixtures of pylint-derived (PL*) and flake8-builtins (A*) rules are GPL-derived and are never copied. */
const RUFF_NOT_COPIED = ["pylint/", "flake8_builtins/"];

/** An ESLint-family case's test file, without the run and case it names (`tests/lib/rules/eqeqeq.js#0/valid[3]`). */
const eslintTestFile = (fixture: string): string => fixture.replace(/#.*$/u, "");

/**
 * The existing fixtures/ layouts: Ruff's resources/test/fixtures/<dir>/… and Bandit's examples/… map to fixtures/….
 * An ESLint-family case is not a file but a RuleTester case's `code`, so each case is its own source: one the tool-pack
 * already commits byte for byte (upstream/generate.mjs's fixtures/<rule>/<n>.<ext>) is reused, from the test file's own
 * directory and with the case's file type first; any other is copied to fixtures/<test file>/<sha256 prefix><ext>, the
 * extension of the path the case is read at, in lower case.
 */
export const CASE_LAYOUTS: Record<GatesManifest["adapter"], CaseLayout> = {
  ruff: {
    file({ fixture }) {
      if (fixture.startsWith(RUFF_NOTEBOOK_FIXTURES)) return `fixtures/ruff_notebook/${fixture.slice(RUFF_NOTEBOOK_FIXTURES.length)}`;
      const rest = fixture.startsWith(RUFF_FIXTURES) ? fixture.slice(RUFF_FIXTURES.length) : fixture.startsWith(RUFF_TESTS) ? fixture.slice(RUFF_TESTS.length) : null;
      if (rest === null || RUFF_NOT_COPIED.some((prefix) => rest.startsWith(prefix))) return null;
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

/** The runtimes each adapter runs besides upstream's own (gates.yaml `upstream.runtime`). */
const EXTRA_RUNTIMES: Record<GatesManifest["adapter"], string[]> = { bandit: ["python"], ruff: ["python"], eslint: ["parsers"] };

/** A path's directories, innermost first: `a/b/c.py` gives `a/b` and `a`. */
function directoriesOf(path: string): string[] {
  const parts = path.split("/").slice(0, -1);
  return parts.map((_, index) => parts.slice(0, parts.length - index).join("/"));
}

/**
 * The corpus files whose presence changes upstream's answer on a file, by adapter. Ruff resolves a file's package, and
 * so its module name (N999), from the `__init__.py` files in the directories above it; the whole corpus held them, so
 * the recorder fetches them and hands them to upstream beside the goldens' files, and records answers for the goldens'
 * files alone. Bandit and ESLint read each file on its own.
 */
export const CONTEXT_FILES: Record<GatesManifest["adapter"], (path: string) => string[]> = {
  ruff: (path) => directoriesOf(path).flatMap((directory) => [`${directory}/__init__.py`, `${directory}/__init__.pyi`]),
  bandit: () => [],
  eslint: () => [],
};

/** Extensions a corpus could hold, probed against the adapter's inCorpus. */
const PROBED_EXTENSIONS = [".cjs", ".cts", ".ipynb", ".js", ".jsx", ".mjs", ".mts", ".py", ".pyi", ".ts", ".tsx", ".vue"];

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
  const checkout = resolve(cacheRoot, "upstream", tool, manifest.upstream.commit);
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
      const file = layout.file({ fixture: testCase.fixture, path: readPath(testCase), bytes }, committed);
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
  const fixtureTexts = recordFixtureTexts(adapter, cacheRoot, manifest.upstream);

  const extensions = PROBED_EXTENSIONS.filter((extension) => adapter.inCorpus(`src/example${extension}`));
  const runtimeDigests = options.runtimeDigests ?? ((runtime: string) => runtimePinDigests(runtime));
  const recording = gatesRecordingSchema.parse({
    format: RECORDING_FORMAT,
    tool,
    adapter: manifest.adapter,
    pins: { repository: manifest.upstream.repository, commit: manifest.upstream.commit, version: manifest.upstream.version, runtime: manifest.upstream.runtime },
    runtimes: Object.fromEntries([manifest.upstream.runtime, ...EXTRA_RUNTIMES[manifest.adapter]].sort(byText).map((runtime) => [runtime, runtimeDigests(runtime)])),
    identity: adapter.identity,
    packChecks: adapter.packChecks,
    licenseSha256: sha256(adapter.licenseText),
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
    // The same parsers recording.json names: one recorder run asked one set of parsers about cases and corpus files
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

  return { recording, corpus, sources: [...sources.values()].sort((a, b) => byText(a.file, b.file)) };
}

/** What differs between a fresh recording and what the tool-pack's committed `recording.lock` names. */
export interface RecordingDiff {
  /** recording.json or corpus.json when the lock names other bytes for it, or no lock is committed. */
  changed: string[];
  /**
   * How corpus.json was judged when the lock names other bytes for it: null when the digests agree; otherwise why the
   * locked corpus.json still holds (it answers every file the fresh one does, identically, and more) or why it does not.
   */
  corpusNote: string | null;
  /** Case sources the directory lacks. */
  missing: CaseCopy[];
  /** Case sources the directory has with other bytes than upstream's. */
  conflicting: string[];
}

/**
 * Whether a locked corpus.json still answers for a fresh one: the same tool, pins, rules and parsers, and every file of
 * the fresh recording answered identically. A locked corpus.json recorded over more files (the whole corpus, before
 * the recorder narrowed to the goldens' files) holds while it agrees on these; null when it holds, else why not.
 */
export function corpusSupersetProblem(locked: GatesCorpusRecording, fresh: GatesCorpusRecording): string | null {
  for (const key of ["format", "tool", "pins", "rules", "parsers"] as const) {
    if (JSON.stringify(locked[key]) !== JSON.stringify(fresh[key])) return `its ${key} differ from a fresh recording's`;
  }
  const lockedFiles = new Map(locked.files.map((file) => [`${file.task}\0${file.path}`, JSON.stringify(file)]));
  const differing = fresh.files.filter((file) => lockedFiles.get(`${file.task}\0${file.path}`) !== JSON.stringify(file));
  if (differing.length === 0) return null;
  return `${differing.length} of the ${fresh.files.length} file(s) the goldens name are answered differently or not at all, e.g. ${differing[0]!.task}/${differing[0]!.path}`;
}

/**
 * A fresh recording against the committed lock. recording.json is compared by digest: the recordings are not in the
 * tree, and the lock is what the commit pins, so a lock naming other bytes is exactly what a stale recording is.
 * corpus.json is too, except that a locked corpus.json recorded over more files than the goldens name still holds
 * while it answers theirs identically (corpusSupersetProblem); reading it needs the locked recording in `cacheRoot`.
 */
export function diffRecording(root: string, pack: string, tool: string, recorded: Recorded, cacheRoot: string): RecordingDiff {
  const directory = toolPackDirectory(root, pack, tool);
  const lockPath = join(directory, RECORDING_LOCK);
  const lock = existsSync(lockPath) ? recordingLockSchema.safeParse(JSON.parse(readFileSync(lockPath, "utf8"))) : null;
  const changed: string[] = [];
  let corpusNote: string | null = null;
  for (const [file, value] of [[RECORDING_FILE, recorded.recording], [CORPUS_FILE, recorded.corpus]] as const) {
    const digest = sha256(Buffer.from(recordingText(value), "utf8"));
    if (lock !== null && lock.success && lock.data.files[file].sha256 === digest) continue;
    if (file === CORPUS_FILE && lock !== null && lock.success) {
      const locked = readLockedPair(root, cacheRoot, pack, tool);
      if (locked === null || "unavailable" in locked) {
        corpusNote = `the locked ${CORPUS_FILE} could not be compared: ${locked === null ? "no lock" : locked.unavailable} (run npm run port-readiness -- fetch first)`;
      } else if (locked.corpus === null) {
        corpusNote = `the lock names no ${CORPUS_FILE}`;
      } else {
        const problem = corpusSupersetProblem(locked.corpus, recorded.corpus);
        if (problem === null) {
          corpusNote = `the locked ${CORPUS_FILE} answers all ${recorded.corpus.files.length} file(s) the goldens name, as a fresh recording does, and ${locked.corpus.files.length - recorded.corpus.files.length} more; the next re-record drops those`;
          continue;
        }
        corpusNote = `the locked ${CORPUS_FILE} no longer holds: ${problem}`;
      }
    }
    changed.push(file);
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
  return "upstream's test files";
}

/** How the note that lists the recording in UPSTREAM.md starts; the recorder finds an earlier note by it. */
const RECORDING_NOTE_START = `> \`${RECORDING_FILE}\` is a port readiness recording`;

/** Width a written note is wrapped to, `> ` included. */
const NOTE_WIDTH = 120;

/**
 * The note that lists the recording in UPSTREAM.md when it holds upstream test text inline, as a Markdown blockquote
 * wrapped to NOTE_WIDTH: recording.json is then partly upstream's material (excerpts), like a copied fixture.
 */
export function recordingNote(source: string): string[] {
  const text = `${RECORDING_NOTE_START.slice(2)}, and no file of this pack, so it is recorded here rather than in the table above: `
    + `the tool-pack's \`${RECORDING_LOCK}\` names it and a GitHub release holds the bytes (docs/port-readiness.md). It carries `
    + `excerpts of this upstream's tests (${source}), redistributed under the licence above, together with Rerail's Apache-2.0; `
    + "the LICENSE beside this file is this upstream's own, at the pinned commit.";
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
export function withRecordingNote(text: string, source: string): string {
  const lines = text.split("\n");
  const isRow = (line: string) => line.startsWith("| `upstream/");
  const first = lines.findIndex(isRow);
  if (first < 0) throw new Error("UPSTREAM.md has no file table to list the recording under");
  let last = first;
  while (last + 1 < lines.length && isRow(lines[last + 1]!)) last += 1;
  const note = recordingNote(source);
  const at = lines.findIndex((line) => line.startsWith(RECORDING_NOTE_START));
  if (at < 0) return [...lines.slice(0, last + 1), "", ...note, ...lines.slice(last + 1)].join("\n");
  let end = at;
  while (end + 1 < lines.length && lines[end + 1]!.startsWith(">")) end += 1;
  const words = (quote: string[]) => quote.map((line) => line.replace(/^>\s?/u, "")).join(" ").split(/\s+/u).filter((word) => word !== "").join(" ");
  if (words(lines.slice(at, end + 1)) === words(note)) return text;
  return [...lines.slice(0, at), ...note, ...lines.slice(end + 1)].join("\n");
}

/** What a written recording left behind, for the caller to upload and to commit. */
export interface WrittenRecording extends RecordingDiff {
  /** The lock as it is now committed. */
  lock: RecordingLock;
  /** The gzipped recordings, by their asset name, ready for the release; written beside the cache. */
  assets: { name: string; path: string; bytes: number }[];
  /** The per-rule summary against the recording the lock named before, a first recording, or why it was not compared. */
  changes: RecordingChanges;
}

/**
 * Write the recording into the local cache, the `recording.lock` that names it, the gzipped assets a trusted job
 * uploads, and the missing case sources; list the copies in UPSTREAM.md.
 *
 * The lock, the copied case sources and UPSTREAM.md are committed; the recordings are not. They go to the cache the
 * gates read, so iterating locally needs no release and no network: record, then run the gates, and they read what was
 * just recorded because the lock names it.
 */
export async function writeRecording(root: string, pack: string, tool: string, recorded: Recorded, options: { cacheRoot: string; release?: string }): Promise<WrittenRecording> {
  const directory = toolPackDirectory(root, pack, tool);
  const diff = diffRecording(root, pack, tool, recorded, options.cacheRoot);
  if (diff.conflicting.length > 0) throw new Error(`packs/${pack}/upstream/${tool}: committed case sources differ from upstream's: ${diff.conflicting.slice(0, 5).join(", ")}`);

  // The summary compares against the recording the committed lock named, where it is still in the cache. Without a
  // lock the report says it is a first recording; with one whose recording cannot be read, that what moved is unknown.
  const before = readLockedPair(root, options.cacheRoot, pack, tool);
  const texts = { [RECORDING_FILE]: recordingText(recorded.recording), [CORPUS_FILE]: recordingText(recorded.corpus) };
  const lock = buildRecordingLock(pack, tool, options.release ?? DEFAULT_RELEASE, recorderVersion(root), recorded.recording.pins, recorded.recording.runtimes, texts);
  for (const text of Object.values(texts)) await writeCachedRecording(options.cacheRoot, text);
  await writeFile(join(directory, RECORDING_LOCK), recordingLockText(lock));

  const uploads = join(options.cacheRoot, "recordings", "upload");
  await mkdir(uploads, { recursive: true });
  const assets = [];
  for (const file of [RECORDING_FILE, CORPUS_FILE] as const) {
    const gzipped = gzipRecording(texts[file]);
    const path = join(uploads, lock.files[file].asset);
    await writeFile(path, gzipped);
    assets.push({ name: lock.files[file].asset, path, bytes: gzipped.length });
  }

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
    if (inline > 0) text = withRecordingNote(text, inlineTextsSource(manifest));
    await writeFile(upstreamMd, text);
  }
  return { ...diff, lock, assets, changes: diffRecordings(pack, tool, before, { recording: recorded.recording, corpus: recorded.corpus }) };
}

/**
 * The recording the committed lock names, read from the cache: null when there is no lock, and why not when the lock
 * is there but its recording cannot be read (not fetched, or a fetch that failed), which is not a first recording. It
 * is what a re-record is summarized against; nothing fetches it, because a summary is not worth a download and a
 * maintainer re-recording has usually just run the gates.
 */
export function readLockedPair(root: string, cacheRoot: string, pack: string, tool: string): RecordedPair | UnreadRecording | null {
  const lockPath = join(toolPackDirectory(root, pack, tool), RECORDING_LOCK);
  if (!existsSync(lockPath)) return null;
  const lock = recordingLockSchema.safeParse(JSON.parse(readFileSync(lockPath, "utf8")));
  if (!lock.success) return { unavailable: `${RECORDING_LOCK} does not match its schema` };
  const locked = [RECORDING_FILE, CORPUS_FILE].map((file) => lock.data.files[file as typeof RECORDING_FILE]);
  const texts = locked.map((file) => readCachedRecording(cacheRoot, file.sha256));
  const missing = locked.filter((_, index) => texts[index] === undefined).map((file) => file.asset);
  if (missing.length > 0) return { unavailable: `${missing.join(" and ")} ${missing.length === 1 ? "is" : "are"} not in the cache` };
  const recording = gatesRecordingSchema.safeParse(JSON.parse(texts[0]!));
  const corpus = gatesCorpusSchema.safeParse(JSON.parse(texts[1]!));
  if (!recording.success || !corpus.success) return { unavailable: "the cached recording does not parse" };
  return { recording: recording.data, corpus: corpus.data };
}

/** The repository version the recorder ran from, written into the lock so a reader can date a recording. */
function recorderVersion(root: string): string {
  const path = join(root, "package.json");
  if (!existsSync(path)) return "0.0.0";
  return (JSON.parse(readFileSync(path, "utf8")) as { version?: string }).version ?? "0.0.0";
}
