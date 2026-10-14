import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { extname, join, posix, resolve } from "node:path";

import { checkoutPinned, type CorpusFile } from "./corpus.js";
import { readGatesManifest, toolPackDirectory } from "./manifest.js";
import type { CommandRunner } from "./process.js";
import { GATES_CORPUS, GATES_RECORDING, gatesCorpusSchema, gatesRecordingSchema, spanOf, type GatesCorpusRecording, type GatesRecording } from "./recording.js";
import { banditSpan } from "./span-units/bandit.js";
import { eslintSpan } from "./span-units/eslint.js";
import { ruffSpan } from "./span-units/ruff.js";
import type { Check, PackAdapter, PackManifest, ParserVerdict, Span, SpanParts, UpstreamCase, UpstreamFile } from "./types.js";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** Where the recorder's runtimes and their lockfiles are committed, relative to the repository root. */
export const RUNTIMES_DIRECTORY = "conformance/recorders/gates/runtimes";

/** What a failing tool-pack check tells the reader to do. */
export function staleMessage(pack: string, tool: string): string {
  return `the recording is stale: run npm run gates:record -- --pack ${pack} --tool ${tool}`;
}

const SPAN_UNITS: Record<GatesRecording["adapter"], (source: string, startByte: number, endByte: number, parts: SpanParts) => Span> = {
  ruff: ruffSpan,
  bandit: banditSpan,
  eslint: eslintSpan,
};

function parse<T>(schema: { safeParse(value: unknown): { success: true; data: T } | { success: false; error: { issues: { path: PropertyKey[]; message: string }[] } } }, path: string): T {
  const parsed = schema.safeParse(JSON.parse(readFileSync(path, "utf8")));
  if (!parsed.success) throw new Error(`${path}: ${parsed.error.issues.slice(0, 5).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  return parsed.data;
}

/**
 * A committed case source as upstream's adapter decoded it: UTF-8, or, for bytes that are not UTF-8, the Latin-1 a PEP 263
 * coding line declares (Bandit's trojansource_latin1.py); recorded parser verdicts are keyed by the decoded text.
 */
export function decodeSource(bytes: Buffer): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    const head = bytes.subarray(0, 400).toString("latin1").split(/\r?\n/u).slice(0, 2).join("\n");
    const coding = /coding[:=][ \t]*([-\w.]+)/u.exec(head)?.[1]?.toLowerCase();
    if (coding !== undefined && /^(latin-?1|iso-?8859-1|l1|cp819)$/u.test(coding.replaceAll("_", "-"))) return bytes.toString("latin1");
    return bytes.toString("utf8");
  }
}

/**
 * The code cells of a notebook joined, as the words of upstream's notebook test files; a text that is not a notebook is
 * returned unchanged. The Ruff adapter's `notebook-code-cells` transform, shared so the gates rebuild its texts exactly.
 */
export function notebookCodeCells(text: string): string {
  try {
    const notebook = JSON.parse(text) as { cells?: { cell_type?: string; source?: string | string[] }[] };
    return (notebook.cells ?? []).filter((cell) => cell.cell_type === "code")
      .map((cell) => (Array.isArray(cell.source) ? cell.source.join("") : cell.source ?? "")).join("\n");
  } catch {
    return text;
  }
}

export type RecordedFixtureTexts = NonNullable<GatesRecording["fixtureTexts"]>;

/** Where the gates and the recorder keep upstream's checkout at the pinned commit: `<cache>/upstream/<repository name>/<commit>`. */
export function upstreamCheckoutDirectory(cacheRoot: string, pins: { repository: string; commit: string }): string {
  return resolve(cacheRoot, "upstream", pins.repository.slice(pins.repository.lastIndexOf("/") + 1), pins.commit);
}

/**
 * fixtureTexts rebuilt from a checkout of upstream at the pinned commit: each recorded file read (UTF-8, as the live
 * adapters read it), its transform applied and its sha256 checked, then the inline texts. A missing file or another
 * digest throws: the recording is stale, and a silent narrowing would let not-overfit pass what it should not.
 */
export function rebuildFixtureTexts(checkout: string, recorded: RecordedFixtureTexts): string[] {
  const texts = recorded.files.map(({ path, sha256: digest, transform }) => {
    const file = join(checkout, path);
    if (!existsSync(file)) throw new Error(`upstream's ${path} is missing from the checkout at ${checkout}`);
    const raw = readFileSync(file, "utf8");
    const text = transform === "notebook-code-cells" ? notebookCodeCells(raw) : raw;
    const actual = sha256(text);
    if (actual !== digest) throw new Error(`upstream's ${path}${transform === undefined ? "" : ` (${transform})`} has sha256 ${actual.slice(0, 12)}, recorded ${digest.slice(0, 12)}: the recording is stale`);
    return text;
  });
  return [...texts, ...recorded.inline.map(({ text }) => text)];
}

/** A sparse, shallow, blobless checkout of the directories of every recorded upstream file, at the pinned commit. */
export async function fetchFixtureTexts(cacheRoot: string, pins: { repository: string; commit: string }, recorded: RecordedFixtureTexts, runner: CommandRunner): Promise<string[]> {
  if (recorded.files.length === 0) return rebuildFixtureTexts(cacheRoot, recorded);
  const directories = [...new Set(recorded.files.map(({ path }) => `/${posix.dirname(path)}/`.replace(/^\/\.\/$/u, "/")))].sort();
  const checkout = await checkoutPinned(pins.repository, pins.commit, upstreamCheckoutDirectory(cacheRoot, pins), directories, runner, { blobless: true });
  return rebuildFixtureTexts(checkout, recorded);
}

/** Everything a recorded adapter is read from, so tests can build one without a checkout. */
export interface RecordedInputs {
  root: string;
  manifest: PackManifest;
  recording: GatesRecording;
  corpus: GatesCorpusRecording | null;
  /** conformance.json `tools[]` of the pack. */
  tools: { name: string; version: string }[];
  /** The upstream rules the run asks about. */
  names: string[];
}

/** Read a gated tool-pack's manifest and recordings from the working tree at `root`. */
export function readRecordedInputs(root: string, pack: string, tool: string, names: string[]): RecordedInputs {
  const directory = toolPackDirectory(root, pack, tool);
  const manifest = readGatesManifest(root, pack, tool);
  const recordingPath = join(directory, GATES_RECORDING);
  if (!existsSync(recordingPath)) throw new Error(`packs/${pack}/upstream/${tool}/${GATES_RECORDING} is missing: ${staleMessage(pack, tool)}`);
  const recording = parse(gatesRecordingSchema, recordingPath);
  const corpusPath = join(directory, GATES_CORPUS);
  const corpus = existsSync(corpusPath) ? parse(gatesCorpusSchema, corpusPath) : null;
  const conformance = JSON.parse(readFileSync(join(root, "packs", pack, "conformance.json"), "utf8")) as { tools?: { name: string; version: string }[] };
  return { root, manifest, recording, corpus, tools: conformance.tools ?? [], names };
}

/**
 * The tool-pack checks the gates make on a recording: pins agree (gates.yaml, gates.json, conformance.json), the
 * runtimes' lockfiles are the committed ones, the kept LICENSE is the recorded one, and the recording covers every
 * upstream rule the pack names. Any failure means the recording is stale.
 */
export function toolPackChecks(inputs: RecordedInputs): Check[] {
  const { root, manifest, recording, corpus, tools, names } = inputs;
  const pack = manifest.rerail.pack;
  const tool = recording.tool;
  const stale = staleMessage(pack, tool);
  const checks: Check[] = [];
  const conformanceVersion = tools.find((entry) => entry.name === tool)?.version ?? null;
  const pinMismatches = [
    ...(manifest.upstream.version === recording.pins.version ? [] : [`gates.yaml upstream.version ${manifest.upstream.version} but gates.json pins.version ${recording.pins.version}`]),
    ...(conformanceVersion === manifest.upstream.version ? [] : [`conformance.json tools[${tool}].version ${conformanceVersion ?? "missing"} but gates.yaml upstream.version ${manifest.upstream.version}`]),
    ...(manifest.upstream.commit === recording.pins.commit ? [] : [`gates.yaml upstream.commit ${manifest.upstream.commit} but gates.json pins.commit ${recording.pins.commit}`]),
    ...(manifest.upstream.repository === recording.pins.repository ? [] : [`gates.yaml upstream.repository ${manifest.upstream.repository} but gates.json pins.repository ${recording.pins.repository}`]),
    ...(manifest.upstream.runtime === recording.pins.runtime ? [] : [`gates.yaml upstream.runtime ${manifest.upstream.runtime} but gates.json pins.runtime ${recording.pins.runtime}`]),
    ...(manifest.adapter === recording.adapter ? [] : [`gates.yaml adapter ${manifest.adapter} but gates.json adapter ${recording.adapter}`]),
  ];
  checks.push({ id: "pins-agree", ok: pinMismatches.length === 0, detail: pinMismatches.length === 0 ? `${tool} ${recording.pins.version} at ${recording.pins.commit} in gates.yaml, gates.json and conformance.json` : `${pinMismatches.join("; ")}: ${stale}` });

  const lockMismatches: string[] = [];
  for (const [runtime, files] of Object.entries(recording.runtimes).sort(([a], [b]) => (a < b ? -1 : 1))) {
    for (const [file, digest] of Object.entries(files).sort(([a], [b]) => (a < b ? -1 : 1))) {
      const path = join(root, RUNTIMES_DIRECTORY, runtime, file);
      const actual = existsSync(path) ? sha256(readFileSync(path)) : null;
      if (actual !== digest) lockMismatches.push(`${runtime}/${file} ${actual === null ? "is not committed" : `is ${actual.slice(0, 12)}, recorded ${digest.slice(0, 12)}`}`);
    }
  }
  checks.push({ id: "runtime-lockfiles", ok: lockMismatches.length === 0, detail: lockMismatches.length === 0 ? `${Object.keys(recording.runtimes).length} runtime(s) match their committed lockfiles` : `${lockMismatches.slice(0, 3).join("; ")}: ${stale}` });

  const licensePath = join(root, "packs", pack, "licenses", manifest.rerail.project, "LICENSE");
  const licenseSha = existsSync(licensePath) ? sha256(readFileSync(licensePath)) : null;
  checks.push({
    id: "license-sha256",
    ok: licenseSha === recording.licenseSha256,
    detail: licenseSha === recording.licenseSha256
      ? `packs/${pack}/licenses/${manifest.rerail.project}/LICENSE is upstream's LICENSE at the pinned commit (sha256 ${licenseSha.slice(0, 12)})`
      : `packs/${pack}/licenses/${manifest.rerail.project}/LICENSE ${licenseSha === null ? "is missing" : `has sha256 ${licenseSha.slice(0, 12)}`}, upstream's recorded LICENSE ${recording.licenseSha256.slice(0, 12)}: ${stale}`,
  });

  const absent = new Set(recording.absent);
  const unrecorded = [...new Set(names)].filter((name) => !(name in recording.rules) && !absent.has(name)).sort();
  checks.push({ id: "recording-covers-rules", ok: unrecorded.length === 0, detail: unrecorded.length === 0 ? `every one of ${new Set(names).size} upstream rule(s) is recorded` : `${unrecorded.length} upstream rule(s) never recorded (${unrecorded.slice(0, 5).join(", ")}): ${stale}` });

  if (corpus !== null) {
    const missing = names.filter((name) => name in recording.rules && !corpus.rules.includes(name));
    checks.push({ id: "corpus-covers-rules", ok: missing.length === 0, detail: missing.length === 0 ? `${GATES_CORPUS} has upstream's answers for every recorded rule` : `${GATES_CORPUS} lacks ${missing.slice(0, 5).join(", ")}: ${stale}` });
  } else {
    checks.push({ id: "corpus-covers-rules", ok: false, detail: `${GATES_CORPUS} is missing: ${stale}` });
  }
  return checks;
}

/**
 * A PackAdapter answering from a tool-pack's recordings and committed case sources; it never runs upstream. Its
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
  const verdictsByKey = new Map<string, ParserVerdict[]>(recording.parserVerdicts.map((entry) => [`${entry.path}\0${entry.script}\0${entry.sha256}`, entry.verdicts]));
  const corpusVerdicts = new Map<string, ParserVerdict[]>((corpus?.files ?? []).map((file) => [`${file.path}\0${file.sha256}`, file.verdicts]));
  const corpusByPlace = new Map((corpus?.files ?? []).map((file) => [`${file.task}\0${file.path}`, file]));

  return {
    identity: recording.identity,
    packChecks: [...recording.packChecks, ...toolPackChecks(inputs)],
    // The text is the kept LICENSE; license-sha256 checks it is upstream's.
    licenseText: (() => {
      const path = join(root, "packs", manifest.rerail.pack, "licenses", manifest.rerail.project, "LICENSE");
      return existsSync(path) ? readFileSync(path, "utf8") : "";
    })(),
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
      if (upstream === undefined) throw new Error(`${GATES_RECORDING} records upstream's fixture texts by origin, and this recorded adapter has no cache to fetch them into`);
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
    inScopeSamples: recording.inScopeSamples,
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
      const ext = extname(path);
      return extensions.has(recording.corpus.caseSensitive ? ext : ext.toLowerCase());
    },
    async lintCorpus(files: CorpusFile[], names: string[]): Promise<UpstreamFile[]> {
      const asked = new Set(names);
      return files.map((file) => {
        const recorded = corpusByPlace.get(`${file.task}\0${file.path}`);
        if (recorded === undefined || recorded.sha256 !== file.sha256) {
          // Without upstream's recorded answer the file has no ground truth; treating it as silent would invent agreement.
          return { file, excluded: `not in ${GATES_CORPUS} with this sha256 (the corpus recording is stale)`, findings: new Map() };
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
