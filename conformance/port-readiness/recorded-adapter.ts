import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { extname, join, posix, resolve } from "node:path";

import { checkoutPinned, type CorpusFile } from "./corpus.js";
import { fileTypeOf } from "./engine.js";
import { readGatesManifest, recordedRuntimes, toolPackDirectory, type GatesManifest } from "./manifest.js";
import type { CommandRunner } from "./process.js";
import { cleanVerdicts, CORPUS_FILE, RECORDING_FILE, gatesCorpusSchema, gatesRecordingSchema, spanOf, type GatesCorpusRecording, type GatesRecording } from "./recording/schema.js";
import { caseSourcesDigest, LIFTED_CASE_SOURCES, LIFTED_RUNTIMES, readCachedRecording, readRecordingLock, RECORDING_LOCK, type RecordingLock } from "./recording/store.js";
import { banditSpan } from "./span-units/bandit.js";
import { bladeParserSpan } from "./span-units/blade-parser.js";
import { bladestanSpan } from "./span-units/bladestan.js";
import { eslintSpan } from "./span-units/eslint.js";
import { phpcsSpan } from "./span-units/phpcs.js";
import { phpstanSpan } from "./span-units/phpstan.js";
import { psalmSpan } from "./span-units/psalm.js";
import { ruffSpan } from "./span-units/ruff.js";
import { tlintSpan } from "./span-units/tlint.js";
import type { Check, PackAdapter, PackManifest, ParserVerdict, Span, SpanParts, UpstreamCase, UpstreamFile } from "./types.js";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** Two values compared as data, for a check that reports a disagreement rather than relying on reference equality. */
const canonical = (value: unknown): string => JSON.stringify(value, (_key, inner: unknown) => (inner !== null && typeof inner === "object" && !Array.isArray(inner) ? Object.fromEntries(Object.entries(inner as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : 1))) : inner));

/** Where the recorder's runtimes and their pins are committed, relative to the repository root. */
export const RUNTIMES_DIRECTORY = "conformance/recorders/port-readiness/runtimes";

/** What a failing tool-pack check tells the reader to do. */
export function staleMessage(pack: string, tool: string): string {
  return `the recording is stale: run npm run port-readiness:record -- --pack ${pack} --tool ${tool}`;
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
};

/**
 * A licence file's name: LICENSE or LICENCE in any letter case, bare or with any extension or suffix (`licence.txt`,
 * `license.md`, `LICENSE-MIT`). Every such file at the root of upstream's repository is part of its licence, and the
 * pack keeps each under the same name in licenses/<project>/.
 */
const LICENSE_FILE = /^licen[cs]e(?:[.-].*)?$/iu;

/** The licence files' names (LICENSE_FILE) as non-cone sparse-checkout patterns, for a checkout of upstream's root. */
export const LICENSE_SPARSE_PATTERNS = ["/[Ll][Ii][Cc][Ee][Nn][CcSs][Ee]", "/[Ll][Ii][Cc][Ee][Nn][CcSs][Ee][.-]*"];

/** The licence files directly in `directory`, in name order (code units: `LICENSE-APACHE` before `LICENSE-MIT`). */
export function licenseFiles(directory: string): string[] {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && LICENSE_FILE.test(entry.name))
    .map((entry) => entry.name)
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/** A licence as the gates compare it: `directory`'s licence files, their bytes one after another; null when it has none. */
export function licenseBytes(directory: string): Buffer | null {
  const names = licenseFiles(directory);
  return names.length === 0 ? null : Buffer.concat(names.map((name) => readFileSync(join(directory, name))));
}

/** Where the pack keeps upstream's licence and UPSTREAM.md, relative to the repository root. */
export function keptLicenseDirectory(manifest: Pick<GatesManifest, "rerail">): string {
  return `packs/${manifest.rerail.pack}/licenses/${manifest.rerail.project}`;
}

/** Upstream's licence text at the root of its checkout at the pinned commit (licenseBytes), as a live adapter records it. */
export function upstreamLicenseText(checkout: string): string {
  const bytes = licenseBytes(checkout);
  if (bytes === null) throw new Error(`upstream's checkout at ${checkout} has no LICENSE or LICENCE file at its root`);
  return bytes.toString("utf8");
}

function parseText<T>(schema: { safeParse(value: unknown): { success: true; data: T } | { success: false; error: { issues: { path: PropertyKey[]; message: string }[] } } }, label: string, text: string): T {
  const parsed = schema.safeParse(JSON.parse(text));
  if (!parsed.success) throw new Error(`${label}: ${parsed.error.issues.slice(0, 5).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
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
  /** The tool-pack's committed manifest, or null for a test that builds the recordings itself. */
  lock: RecordingLock | null;
  recording: GatesRecording;
  corpus: GatesCorpusRecording | null;
  /** conformance.json `tools[]` of the pack. */
  tools: { name: string; version: string }[];
  /** The upstream rules the run asks about. */
  names: string[];
}

/** What to do when a recording the lock names is not in the cache; it is a fetch away, not a re-recording. */
function missingMessage(pack: string, tool: string, file: string): string {
  return `the recording packs/${pack}/upstream/${tool}/${RECORDING_LOCK} names for ${file} is not in the cache:`
    + ` run npm run port-readiness -- fetch --pack ${pack} --tool ${tool}, or record it with npm run port-readiness:record -- --pack ${pack} --tool ${tool}`;
}

/**
 * Read a gated tool-pack's `gates.yaml` and `recording.lock` from the working tree at `root`, and the two recordings
 * the lock names from the cache. The recordings are not in the tree: `gates fetch` puts them in the cache from the
 * release, `port-readiness:record` puts them there directly, and either way they are checked against the lock's digests, so a
 * rule is never judged against bytes this commit did not name.
 */
export function readRecordedInputs(root: string, cacheRoot: string, pack: string, tool: string, names: string[]): RecordedInputs {
  const manifest = readGatesManifest(root, pack, tool);
  const lock = readRecordingLock(root, pack, tool);
  const read = <T>(schema: Parameters<typeof parseText<T>>[0], file: typeof RECORDING_FILE | typeof CORPUS_FILE): T => {
    const text = readCachedRecording(cacheRoot, lock.files[file].sha256);
    if (text === undefined) throw new Error(missingMessage(pack, tool, file));
    return parseText(schema, `packs/${pack}/upstream/${tool}/${file}`, text);
  };
  const conformance = JSON.parse(readFileSync(join(root, "packs", pack, "conformance.json"), "utf8")) as { tools?: { name: string; version: string }[] };
  return {
    root,
    manifest,
    lock,
    recording: read(gatesRecordingSchema, RECORDING_FILE),
    corpus: read(gatesCorpusSchema, CORPUS_FILE),
    tools: conformance.tools ?? [],
    names,
  };
}

/**
 * `loads`'s in-scope samples: upstream's (one example file per type its approved configuration lints), narrowed to the
 * types gates.yaml's `rerail.in_scope` names when the pack deliberately covers fewer.
 */
export function inScopeSamples(samples: readonly string[], inScope: readonly string[] | undefined): string[] {
  return inScope === undefined ? [...samples] : samples.filter((sample) => inScope.includes(extname(sample)));
}

/**
 * The tool-pack checks the gates make on a recording: pins agree (gates.yaml, recording.lock, recording.json,
 * conformance.json), the lock pins every runtime gates.yaml installs and no other (recordedRuntimes, or with
 * `liftedRuntimes` for a recording uploaded by hand before the recorder pinned them all), the runtimes' pins
 * (package.json, pyproject.toml, .python-version, composer.json, .php-version, PHP's extensions.txt) are the committed ones, the kept licence (licenseBytes of keptLicenseDirectory) is the recorded one, every committed case source has its recorded sha256 (or, in a recording uploaded by
 * hand before cases carried one, the digest `liftedCaseSources` pins them all by), and the recording covers every
 * upstream rule the pack names. Any failure means the recording is stale.
 *
 * The pins and the runtime digests are read from `recording.lock`, which is committed, and the downloaded recording
 * must agree with it: a pull request whose pins moved is stale on its own commit, before any byte is fetched, and a
 * lock that disagrees with the recording it names means the two were written by different runs.
 */
export function toolPackChecks(
  inputs: RecordedInputs,
  liftedCaseSources: ReadonlyMap<string, string> = LIFTED_CASE_SOURCES,
  liftedRuntimes: ReadonlyMap<string, RecordingLock["runtimes"]> = LIFTED_RUNTIMES,
): Check[] {
  const { root, manifest, lock, recording, corpus, tools, names } = inputs;
  const pack = manifest.rerail.pack;
  const tool = recording.tool;
  const stale = staleMessage(pack, tool);
  const checks: Check[] = [];
  const pins = lock?.pins ?? recording.pins;
  const liftedAsset = lock?.files[RECORDING_FILE].asset;
  // A recording uploaded by hand before the recorder pinned every runtime it installs: the ones its lock doesn't name
  // are pinned in recording/store.ts (liftedRuntimes, by the lock's asset name) and checked as if the lock named them.
  const lifted = liftedAsset === undefined ? undefined : liftedRuntimes.get(liftedAsset);
  const runtimes = { ...lifted, ...(lock?.runtimes ?? recording.runtimes) };
  const wantedRuntimes = recordedRuntimes(manifest);
  const pinnedRuntimes = Object.keys(runtimes).sort((a, b) => (a < b ? -1 : 1));
  const conformanceVersion = tools.find((entry) => entry.name === tool)?.version ?? null;
  const pinMismatches = [
    ...(manifest.upstream.version === pins.version ? [] : [`gates.yaml upstream.version ${manifest.upstream.version} but ${RECORDING_LOCK} pins.version ${pins.version}`]),
    ...(conformanceVersion === manifest.upstream.version ? [] : [`conformance.json tools[${tool}].version ${conformanceVersion ?? "missing"} but gates.yaml upstream.version ${manifest.upstream.version}`]),
    ...(manifest.upstream.commit === pins.commit ? [] : [`gates.yaml upstream.commit ${manifest.upstream.commit} but ${RECORDING_LOCK} pins.commit ${pins.commit}`]),
    ...(manifest.upstream.repository === pins.repository ? [] : [`gates.yaml upstream.repository ${manifest.upstream.repository} but ${RECORDING_LOCK} pins.repository ${pins.repository}`]),
    ...(manifest.upstream.runtime === pins.runtime ? [] : [`gates.yaml upstream.runtime ${manifest.upstream.runtime} but ${RECORDING_LOCK} pins.runtime ${pins.runtime}`]),
    ...(manifest.adapter === recording.adapter ? [] : [`gates.yaml adapter ${manifest.adapter} but recording.json adapter ${recording.adapter}`]),
    ...(lock === null || lock.tool === tool ? [] : [`${RECORDING_LOCK} tool ${lock.tool} but recording.json tool ${tool}`]),
    ...(canonical(pins) === canonical(recording.pins) ? [] : [`${RECORDING_LOCK} and recording.json were written by different runs: pins ${canonical(pins)} and ${canonical(recording.pins)}`]),
    ...(canonical(lock?.runtimes ?? recording.runtimes) === canonical(recording.runtimes) ? [] : [`${RECORDING_LOCK} and recording.json disagree about the recorder's runtimes`]),
    ...(canonical(pinnedRuntimes) === canonical(wantedRuntimes) ? [] : [`gates.yaml's adapter and environment install the runtimes ${wantedRuntimes.join(", ")} but ${RECORDING_LOCK}${lifted === undefined ? "" : " with recording/store.ts's LIFTED_RUNTIMES"} pins ${pinnedRuntimes.join(", ") || "none"}`]),
  ];
  checks.push({ id: "pins-agree", ok: pinMismatches.length === 0, detail: pinMismatches.length === 0 ? `${tool} ${pins.version} at ${pins.commit} in gates.yaml, ${RECORDING_LOCK}, recording.json and conformance.json` : `${pinMismatches.join("; ")}: ${stale}` });

  const pinFileMismatches: string[] = [];
  for (const [runtime, files] of Object.entries(runtimes).sort(([a], [b]) => (a < b ? -1 : 1))) {
    for (const [file, digest] of Object.entries(files).sort(([a], [b]) => (a < b ? -1 : 1))) {
      const path = join(root, RUNTIMES_DIRECTORY, runtime, file);
      const actual = existsSync(path) ? sha256(readFileSync(path)) : null;
      if (actual !== digest) pinFileMismatches.push(`${runtime}/${file} ${actual === null ? "is not committed" : `is ${actual.slice(0, 12)}, recorded ${digest.slice(0, 12)}`}`);
    }
  }
  const liftedRuntimesNote = lifted === undefined ? "" : ` (${Object.keys(lifted).sort().join(", ")} pinned in recording/store.ts's LIFTED_RUNTIMES: ${RECORDING_FILE} was uploaded by hand before its lock named every runtime)`;
  checks.push({ id: "runtime-pins", ok: pinFileMismatches.length === 0, detail: pinFileMismatches.length === 0 ? `${pinnedRuntimes.length} runtime(s) match their committed pins${liftedRuntimesNote}` : `${pinFileMismatches.slice(0, 3).join("; ")}: ${stale}` });

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

  // Upstream's answers are pinned by the lock, so the sources they were recorded on must be too: an edited case source
  // would be judged against answers upstream gave on other bytes.
  const directory = toolPackDirectory(root, pack, tool);
  const recordedDigests = new Map<string, Set<string | undefined>>();
  for (const rule of Object.values(recording.rules)) {
    for (const testCase of rule.cases) if (testCase.file !== null) recordedDigests.set(testCase.file, (recordedDigests.get(testCase.file) ?? new Set()).add(testCase.sha256));
  }
  const sourceMismatches: string[] = [];
  // A recording uploaded by hand before cases carried a sha256 pins its case sources as one digest instead
  // (liftedCaseSources, by the lock's asset name); any other recording without a case's sha256 is stale.
  const undigested: [file: string, actual: string][] = [];
  for (const [file, digests] of [...recordedDigests].sort(([a], [b]) => (a < b ? -1 : 1))) {
    const recorded = digests.size === 1 ? [...digests][0] : undefined;
    const path = join(directory, file);
    const actual = existsSync(path) ? sha256(readFileSync(path)) : null;
    if (digests.size > 1) sourceMismatches.push(`${file} has cases that disagree about its sha256`);
    else if (actual === null) sourceMismatches.push(`${file} is not committed`);
    else if (recorded === undefined) undigested.push([file, actual]);
    else if (actual !== recorded) sourceMismatches.push(`${file} is ${actual.slice(0, 12)}, recorded ${recorded.slice(0, 12)}`);
  }
  const pinned = liftedAsset === undefined ? undefined : liftedCaseSources.get(liftedAsset);
  if (undigested.length > 0 && pinned === undefined) {
    sourceMismatches.push(...undigested.map(([file]) => `${file} has no recorded sha256`));
  } else if (undigested.length > 0 && caseSourcesDigest(undigested) !== pinned) {
    sourceMismatches.push(`the ${undigested.length} case source(s) ${liftedAsset} records no sha256 for are ${caseSourcesDigest(undigested).slice(0, 12)} together, pinned ${pinned!.slice(0, 12)} in recording/store.ts's LIFTED_CASE_SOURCES (git log -p -- packs/${pack}/upstream/${tool}/fixtures shows what changed)`);
  }
  const liftedNote = undigested.length === 0 ? "" : ` (${undigested.length} of them pinned together: ${RECORDING_FILE} was uploaded by hand before cases carried a sha256)`;
  checks.push({ id: "case-sources", ok: sourceMismatches.length === 0, detail: sourceMismatches.length === 0 ? `${recordedDigests.size} committed case source(s) are the bytes upstream's answers were recorded on${liftedNote}` : `${sourceMismatches.length} case source(s) differ: ${sourceMismatches.slice(0, 3).join("; ")}: ${stale}` });

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
    checks.push({ id: "corpus-covers-rules", ok: missing.length === 0, detail: missing.length === 0 ? `${CORPUS_FILE} has upstream's answers for every recorded rule` : `${CORPUS_FILE} lacks ${missing.slice(0, 5).join(", ")}: ${stale}` });
  } else {
    checks.push({ id: "corpus-covers-rules", ok: false, detail: `${CORPUS_FILE} is missing: ${stale}` });
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
      if (upstream === undefined) throw new Error(`${RECORDING_FILE} records upstream's fixture texts by origin, and this recorded adapter has no cache to fetch them into`);
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
          return { file, excluded: `${CORPUS_FILE} has no answer for it with this sha256: re-record the tool-pack (docs/port-readiness.md, "Goldens")`, findings: new Map() };
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
