import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { extname, join, posix } from "node:path";

import { checkoutPinned, type CorpusFile } from "./corpus.js";
import { fileTypeOf } from "./engine.js";
import { readGatesManifest, recordedRuntimes, toolPackDirectory } from "./manifest.js";
import type { CommandRunner } from "./process.js";
import { EXPECTED_DIRECTORY, readExpected, UPSTREAM_FILE } from "./recording/expected.js";
import { cleanVerdicts, spanOf, type GatesCorpusRecording, type GatesRecording } from "./recording/schema.js";
import { banditSpan } from "./span-units/bandit.js";
import { bladeParserSpan } from "./span-units/blade-parser.js";
import { bladestanSpan } from "./span-units/bladestan.js";
import { clippySpan } from "./span-units/clippy.js";
import { eslintSpan } from "./span-units/eslint.js";
import { phpcsSpan } from "./span-units/phpcs.js";
import { phpstanSpan } from "./span-units/phpstan.js";
import { psalmSpan } from "./span-units/psalm.js";
import { ruffSpan } from "./span-units/ruff.js";
import { sqlfluffSpan } from "./span-units/sqlfluff.js";
import { tlintSpan } from "./span-units/tlint.js";
import type { Check, PackAdapter, PackManifest, ParserVerdict, Span, SpanParts, UpstreamCase, UpstreamFile } from "./types.js";
import { decodeSource, keptLicenseDirectory, licenseBytes, licenseFiles, rebuildFixtureTexts, RUNTIMES_DIRECTORY, upstreamCheckoutDirectory, type RecordedFixtureTexts } from "./upstream-files.js";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** Two values compared as data, for a check that reports a disagreement rather than relying on reference equality. */
const canonical = (value: unknown): string => JSON.stringify(value, (_key, inner: unknown) => (inner !== null && typeof inner === "object" && !Array.isArray(inner) ? Object.fromEntries(Object.entries(inner as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : 1))) : inner));

/**
 * What a failing tool-pack check tells the reader to do. The answers are keyed by everything the generator reads
 * (recording/generator-key.ts), so answers that disagree with the tree were made from an input the key misses, or by a generator
 * that is not deterministic: generating them again for this key is the fix, and the miss is worth finding.
 */
export function staleMessage(pack: string, tool: string): string {
  return `the generated answers are stale: run npm run generate -- --pack ${pack} --tool ${tool} --force`;
}

const SPAN_UNITS: Record<GatesRecording["adapter"], (source: string, startByte: number, endByte: number, parts: SpanParts) => Span> = {
  ruff: ruffSpan,
  bandit: banditSpan,
  eslint: eslintSpan,
  phpcs: phpcsSpan,
  tlint: tlintSpan,
  "blade-parser": bladeParserSpan,
  phpstan: phpstanSpan,
  psalm: psalmSpan,
  bladestan: bladestanSpan,
  sqlfluff: sqlfluffSpan,
  clippy: clippySpan,
};

/**
 * A directory as a non-cone sparse-checkout pattern. Those are gitignore patterns, so glob characters in a name
 * (Next.js's `custom-pages/[profile]/`) are escaped to match only that directory.
 */
export function sparseDirectoryPattern(path: string): string {
  return `/${posix.dirname(path)}/`.replace(/^\/\.\/$/u, "/").replace(/[\\*?[\]]/gu, "\\$&");
}

/** A sparse, shallow, blobless checkout of the directories of every recorded upstream file, at the pinned commit. */
export async function fetchFixtureTexts(cacheRoot: string, pins: { repository: string; commit: string }, recorded: RecordedFixtureTexts, runner: CommandRunner): Promise<string[]> {
  if (recorded.files.length === 0) return rebuildFixtureTexts(cacheRoot, recorded);
  const directories = [...new Set(recorded.files.map(({ path }) => sparseDirectoryPattern(path)))].sort();
  const checkout = await checkoutPinned(pins.repository, pins.commit, upstreamCheckoutDirectory(cacheRoot, pins), directories, runner, { blobless: true });
  return rebuildFixtureTexts(checkout, recorded);
}

/** Everything a recorded adapter is read from, so tests can build one without a checkout. */
export interface RecordedInputs {
  root: string;
  manifest: PackManifest;
  /** expectedDigest of the tool-pack's answers, which the result cache keys on; tests that build the recordings themselves give any. */
  digest: string;
  recording: GatesRecording;
  corpus: GatesCorpusRecording | null;
  /** conformance.json `tools[]` of the pack. */
  tools: { name: string; version: string }[];
  /** The upstream rules the run asks about. */
  names: string[];
}

/**
 * Read a gated tool-pack's `gates.yaml` from the working tree at `root`, and its expected answers from the local cache
 * under `key`, the generator key of that tree (recording/generator-key.ts), joined into the recording and corpus answers
 * recording/expected.ts splits them from. Nothing is downloaded: `npm run generate` puts the answers in the cache.
 */
export function readRecordedInputs(root: string, pack: string, tool: string, names: string[], answers: { cacheRoot: string; key: string }): RecordedInputs {
  const manifest = readGatesManifest(root, pack, tool);
  const { recording, corpus, digest } = readExpected(answers.cacheRoot, pack, tool, answers.key);
  const conformance = JSON.parse(readFileSync(join(root, "packs", pack, "conformance.json"), "utf8")) as { tools?: { name: string; version: string }[] };
  return { root, manifest, digest, recording, corpus, tools: conformance.tools ?? [], names };
}

/**
 * `loads`'s in-scope samples: upstream's (one example file per type its approved configuration lints), narrowed to the
 * types gates.yaml's `rerail.in_scope` names when the pack deliberately covers fewer.
 */
export function inScopeSamples(samples: readonly string[], inScope: readonly string[] | undefined): string[] {
  return inScope === undefined ? [...samples] : samples.filter((sample) => inScope.includes(extname(sample)));
}

/**
 * The tool-pack checks the gates make on its expected answers: pins agree (gates.yaml, expected/upstream.json,
 * conformance.json), upstream.json pins every runtime gates.yaml installs and no other (recordedRuntimes), the runtimes'
 * pins (package.json, pyproject.toml, .python-version, composer.json, .php-version, PHP's extensions.txt) are the
 * committed ones, the kept licence (licenseBytes of keptLicenseDirectory) is the recorded one, every committed case
 * source has its recorded sha256, and the answers cover every upstream rule the pack names. Any failure means the
 * answers are stale (staleMessage): a pull request whose pins moved and whose answers did not fails on its own commit.
 */
export function toolPackChecks(inputs: RecordedInputs): Check[] {
  const { root, manifest, recording, corpus, tools, names } = inputs;
  const pack = manifest.rerail.pack;
  const tool = recording.tool;
  const stale = staleMessage(pack, tool);
  const checks: Check[] = [];
  const pins = recording.pins;
  const upstreamFile = `${EXPECTED_DIRECTORY}/${UPSTREAM_FILE}`;
  const wantedRuntimes = recordedRuntimes(manifest);
  const pinnedRuntimes = Object.keys(recording.runtimes).sort((a, b) => (a < b ? -1 : 1));
  const conformanceVersion = tools.find((entry) => entry.name === tool)?.version ?? null;
  const pinMismatches = [
    ...(manifest.upstream.version === pins.version ? [] : [`gates.yaml upstream.version ${manifest.upstream.version} but ${upstreamFile} pins.version ${pins.version}`]),
    ...(conformanceVersion === manifest.upstream.version ? [] : [`conformance.json tools[${tool}].version ${conformanceVersion ?? "missing"} but gates.yaml upstream.version ${manifest.upstream.version}`]),
    ...(manifest.upstream.commit === pins.commit ? [] : [`gates.yaml upstream.commit ${manifest.upstream.commit} but ${upstreamFile} pins.commit ${pins.commit}`]),
    ...(manifest.upstream.repository === pins.repository ? [] : [`gates.yaml upstream.repository ${manifest.upstream.repository} but ${upstreamFile} pins.repository ${pins.repository}`]),
    ...(manifest.upstream.runtime === pins.runtime ? [] : [`gates.yaml upstream.runtime ${manifest.upstream.runtime} but ${upstreamFile} pins.runtime ${pins.runtime}`]),
    ...(manifest.adapter === recording.adapter ? [] : [`gates.yaml adapter ${manifest.adapter} but ${upstreamFile} adapter ${recording.adapter}`]),
    ...(canonical(pinnedRuntimes) === canonical(wantedRuntimes) ? [] : [`gates.yaml's adapter and environment install the runtimes ${wantedRuntimes.join(", ")} but ${upstreamFile} pins ${pinnedRuntimes.join(", ") || "none"}`]),
  ];
  checks.push({ id: "pins-agree", ok: pinMismatches.length === 0, detail: pinMismatches.length === 0 ? `${tool} ${pins.version} at ${pins.commit} in gates.yaml, ${upstreamFile} and conformance.json` : `${pinMismatches.join("; ")}: ${stale}` });

  const pinFileMismatches: string[] = [];
  for (const [runtime, files] of Object.entries(recording.runtimes).sort(([a], [b]) => (a < b ? -1 : 1))) {
    for (const [file, digest] of Object.entries(files).sort(([a], [b]) => (a < b ? -1 : 1))) {
      const path = join(root, RUNTIMES_DIRECTORY, runtime, file);
      const actual = existsSync(path) ? sha256(readFileSync(path)) : null;
      if (actual !== digest) pinFileMismatches.push(`${runtime}/${file} ${actual === null ? "is not committed" : `is ${actual.slice(0, 12)}, recorded ${digest.slice(0, 12)}`}`);
    }
  }
  checks.push({ id: "runtime-pins", ok: pinFileMismatches.length === 0, detail: pinFileMismatches.length === 0 ? `${pinnedRuntimes.length} runtime(s) match their committed pins` : `${pinFileMismatches.slice(0, 3).join("; ")}: ${stale}` });

  const keptDirectory = keptLicenseDirectory(manifest);
  const kept = licenseFiles(join(root, keptDirectory));
  const keptLabel = kept.length === 0 ? `${keptDirectory}/ has no LICENSE or LICENCE file, and` : kept.map((name) => `${keptDirectory}/${name}`).join(" + ");
  const keptBytes = licenseBytes(join(root, keptDirectory));
  const licenseSha = keptBytes === null ? null : sha256(keptBytes);
  checks.push({
    id: "license-sha256",
    ok: licenseSha === recording.licenseSha256,
    detail: licenseSha === recording.licenseSha256
      ? `${keptLabel} is upstream's licence at the pinned commit (sha256 ${licenseSha.slice(0, 12)})`
      : `${keptLabel} ${licenseSha === null ? "" : `has sha256 ${licenseSha.slice(0, 12)}, `}upstream's recorded licence ${recording.licenseSha256.slice(0, 12)}: ${stale}`,
  });

  // Upstream's answers are committed beside the sources they were made on, so those must be the same bytes: an edited
  // case source would be judged against answers upstream gave on other bytes.
  const directory = toolPackDirectory(root, pack, tool);
  const recordedDigests = new Map<string, Set<string | undefined>>();
  for (const rule of Object.values(recording.rules)) {
    for (const testCase of rule.cases) if (testCase.file !== null) recordedDigests.set(testCase.file, (recordedDigests.get(testCase.file) ?? new Set()).add(testCase.sha256));
  }
  const sourceMismatches: string[] = [];
  for (const [file, digests] of [...recordedDigests].sort(([a], [b]) => (a < b ? -1 : 1))) {
    const recorded = digests.size === 1 ? [...digests][0] : undefined;
    const path = join(directory, file);
    const actual = existsSync(path) ? sha256(readFileSync(path)) : null;
    if (digests.size > 1) sourceMismatches.push(`${file} has cases that disagree about its sha256`);
    else if (actual === null) sourceMismatches.push(`${file} is not committed`);
    else if (recorded === undefined) sourceMismatches.push(`${file} has no recorded sha256`);
    else if (actual !== recorded) sourceMismatches.push(`${file} is ${actual.slice(0, 12)}, recorded ${recorded.slice(0, 12)}`);
  }
  checks.push({ id: "case-sources", ok: sourceMismatches.length === 0, detail: sourceMismatches.length === 0 ? `${recordedDigests.size} committed case source(s) are the bytes upstream's answers were recorded on` : `${sourceMismatches.length} case source(s) differ: ${sourceMismatches.slice(0, 3).join("; ")}: ${stale}` });

  // A narrowing may only leave types out: a type upstream does not lint would hold the ports to nothing upstream checks.
  const inScope = manifest.rerail.in_scope;
  if (inScope !== undefined) {
    const linted = new Set(recording.inScopeSamples.map((sample) => extname(sample)));
    const unlinted = inScope.filter((type) => !linted.has(type));
    checks.push({
      id: "in-scope-narrowing",
      ok: unlinted.length === 0,
      detail: unlinted.length === 0
        ? `gates.yaml's rerail.in_scope holds the ports to ${inScope.join(", ")} of the ${[...linted].join(", ")} upstream lints`
        : `gates.yaml's rerail.in_scope names ${unlinted.join(", ")}, which upstream does not lint (it lints ${[...linted].join(", ")})`,
    });
  }

  const absent = new Set(recording.absent);
  const unrecorded = [...new Set(names)].filter((name) => !(name in recording.rules) && !absent.has(name)).sort();
  checks.push({ id: "recording-covers-rules", ok: unrecorded.length === 0, detail: unrecorded.length === 0 ? `every one of ${new Set(names).size} upstream rule(s) is recorded` : `${unrecorded.length} upstream rule(s) never recorded (${unrecorded.slice(0, 5).join(", ")}): ${stale}` });

  if (corpus !== null) {
    const missing = names.filter((name) => name in recording.rules && !corpus.rules.includes(name));
    checks.push({ id: "corpus-covers-rules", ok: missing.length === 0, detail: missing.length === 0 ? `${EXPECTED_DIRECTORY}/corpus/ has upstream's answers for every recorded rule` : `${EXPECTED_DIRECTORY}/corpus/ lacks ${missing.slice(0, 5).join(", ")}: ${stale}` });
  } else {
    checks.push({ id: "corpus-covers-rules", ok: false, detail: `upstream's corpus answers are missing: ${stale}` });
  }
  return checks;
}

/**
 * A PackAdapter answering from a tool-pack's expected answers and committed case sources; it never runs upstream. Its
 * fixtureTexts are upstream's test files fetched at the pinned commit (`upstream` says where to cache them):
 * `loadFixtureTexts` fetches them, once, and must be awaited before fixtureTexts is read.
 */
export function createRecordedAdapter(
  inputs: RecordedInputs,
  upstream?: { cacheRoot: string; runner: CommandRunner },
): PackAdapter & { recordedCorpusPins: GatesCorpusRecording["pins"] | null; loadFixtureTexts(): Promise<void> } {
  const { root, manifest, recording, corpus } = inputs;
  let fetchedTexts: string[] | undefined;
  const directory = toolPackDirectory(root, manifest.rerail.pack, recording.tool);
  const sources = new Map<string, string>();
  const source = (file: string): string => {
    let text = sources.get(file);
    if (text === undefined) {
      text = decodeSource(readFileSync(join(directory, file)));
      sources.set(file, text);
    }
    return text;
  };
  const setNames = (ids: string[]) => ids.flatMap((id) => recording.nameSets[id]!.names);
  const setRegexes = (ids: string[]) => ids.flatMap((id) => recording.nameSets[id]!.regexes);
  const regex = ({ source: pattern, flags }: { source: string; flags: string }) => new RegExp(pattern, flags);
  const toSpan = SPAN_UNITS[recording.adapter];
  const extensions = new Set(recording.corpus.caseSensitive ? recording.corpus.extensions : recording.corpus.extensions.map((ext) => ext.toLowerCase()));
  // An entry without `verdicts` is every parser silent; an entry that is not there at all is a source nobody recorded.
  const silent = cleanVerdicts(recording.parsers);
  const corpusSilent = corpus === null ? [] : cleanVerdicts(corpus.parsers);
  const verdictsByKey = new Map<string, ParserVerdict[]>(recording.parserVerdicts.map((entry) => [`${entry.path}\0${entry.script}\0${entry.sha256}`, entry.verdicts ?? silent]));
  const corpusVerdicts = new Map<string, ParserVerdict[]>((corpus?.files ?? []).map((file) => [`${file.path}\0${file.sha256}`, file.verdicts ?? corpusSilent]));
  const corpusByPlace = new Map((corpus?.files ?? []).map((file) => [`${file.task}\0${file.path}`, file]));

  return {
    identity: recording.identity,
    packChecks: [...recording.packChecks, ...toolPackChecks(inputs)],
    // The text is the kept licence; license-sha256 checks it is upstream's.
    licenseText: licenseBytes(join(root, keptLicenseDirectory(manifest)))?.toString("utf8") ?? "",
    recordedCorpusPins: corpus?.pins ?? null,
    rule(name) {
      const recorded = recording.rules[name];
      return recorded === undefined ? undefined : { ...recorded.facts };
    },
    implementationNames(name) {
      const names = recording.rules[name]?.names.implementation;
      return new Set(names === undefined ? [] : [...setNames(names.sets), ...names.names]);
    },
    sharedImplementationNames(name) {
      const names = recording.rules[name]?.names.shared;
      return new Set(names === undefined ? [] : [...setNames(names.sets), ...names.names]);
    },
    implementationRegexes(name) {
      const names = recording.rules[name]?.names;
      if (names === undefined) return [];
      return [...names.implementation.regexes, ...setRegexes(names.implementation.sets), ...names.shared.regexes, ...setRegexes(names.shared.sets)].map(regex);
    },
    fixtureWords: () => new Set(recording.fixtureWords),
    async loadFixtureTexts() {
      if (recording.fixtureTexts === undefined || fetchedTexts !== undefined) return;
      if (upstream === undefined) throw new Error(`${EXPECTED_DIRECTORY}/${UPSTREAM_FILE} records upstream's fixture texts by origin, and this recorded adapter has no cache to fetch them into`);
      fetchedTexts = await fetchFixtureTexts(upstream.cacheRoot, recording.pins, recording.fixtureTexts, upstream.runner);
    },
    fixtureTexts() {
      if (recording.fixtureTexts !== undefined) {
        if (fetchedTexts === undefined) throw new Error("fixtureTexts read before loadFixtureTexts fetched upstream's test files");
        return fetchedTexts;
      }
      const files = new Set(Object.values(recording.rules).flatMap((rule) => rule.cases.map((testCase) => testCase.file).filter((file): file is string => file !== null)));
      return [...files].sort().map(source);
    },
    keywords: new Set(recording.keywords),
    commentVocabulary: new Set(recording.commentVocabulary),
    inScopeSamples: inScopeSamples(recording.inScopeSamples, manifest.rerail.in_scope),
    ...(manifest.rerail.in_scope === undefined ? {} : { inScopeTypes: manifest.rerail.in_scope }),
    cases(name): UpstreamCase[] {
      return (recording.rules[name]?.cases ?? []).map((testCase) => ({
        fixture: testCase.fixture,
        variant: testCase.variant,
        path: testCase.path,
        ...(testCase.parsedAs === undefined ? {} : { parsedAs: testCase.parsedAs }),
        ...(testCase.script === undefined ? {} : { script: testCase.script }),
        source: testCase.file === null ? "" : source(testCase.file),
        skipped: testCase.skipped,
        expected: testCase.expected.map(spanOf),
        markup: testCase.markup === null ? null : testCase.markup.map(spanOf),
      }));
    },
    toSpan(name, text, startByte, endByte) {
      const parts = recording.rules[name]?.spanParts ?? { endLine: true, endCol: true };
      return toSpan(text, startByte, endByte, parts);
    },
    inCorpus(path) {
      // A path's type is its longest dialect suffix where that is longer than its extension: `.blade.php` is no `.php`.
      const type = fileTypeOf(path);
      return extensions.has(recording.corpus.caseSensitive ? type : type.toLowerCase());
    },
    async lintCorpus(files: CorpusFile[], names: string[]): Promise<UpstreamFile[]> {
      const asked = new Set(names);
      return files.map((file) => {
        const recorded = corpusByPlace.get(`${file.task}\0${file.path}`);
        if (recorded === undefined || recorded.sha256 !== file.sha256) {
          // Without upstream's recorded answer the file has no ground truth; treating it as silent would invent agreement.
          return { file, excluded: `${EXPECTED_DIRECTORY}/corpus/ has no answer for it with this sha256: regenerate the tool-pack (docs/port-readiness.md, "Goldens")`, findings: new Map() };
        }
        const findings = new Map<string, Span[]>(Object.entries(recorded.findings).filter(([rule]) => asked.has(rule)).map(([rule, spans]) => [rule, spans.map(spanOf)]));
        return { file, excluded: recorded.excluded, findings };
      });
    },
    async parserVerdicts(files) {
      // Independent parsers do not run in PR CI: answer from the recording where it has the file, else no verdict.
      return files.map((file) => {
        const digest = sha256(file.source);
        return verdictsByKey.get(`${file.path}\0${file.script === true}\0${digest}`) ?? corpusVerdicts.get(`${file.path}\0${digest}`) ?? [];
      });
    },
  };
}
