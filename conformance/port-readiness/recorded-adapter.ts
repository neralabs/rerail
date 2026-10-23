import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { extname, join, posix, resolve } from "node:path";
import { isPackFixture, packFixtureAbsence, requirePackFixture } from "../../scripts/s3/pack-fixtures.mjs";

import { recordMissingAnswers } from "./checks/matches-upstream-tests.js";
import { checkoutPinned, readCorpusTasks, readRerailCorpora, wantedKey, type CorpusFile } from "./corpus.js";
import { fileTypeOf } from "./engine.js";
import type { Golden } from "./goldens.js";
import { readGatesManifest, recordedRuntimes, toolPackDirectory, type GatesManifest } from "./manifest.js";
import type { CommandRunner } from "./process.js";
import { answerConfig, cachedTree, configId, directoryTree, parsersId, RECORDING_ARCHIVE, treeConfigs, type RecordingTree } from "./recording/archive.js";
import { answerKey, answerPath, corpusCommits, fetchedContext, missingAnswers, neededAnswers, parseAnswer, parseVerdicts, verdictPath, type KeyContext } from "./recording/corpus-answers.js";
import { spanOf, type GatesRecording } from "./recording/schema.js";
import { readRecordingLock, RECORDING_LOCK, type RecordingLock } from "./recording/store.js";
import { loadRecording } from "./recording/tree.js";
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
import type { Check, PackAdapter, PackManifest, Span, SpanParts, UpstreamCase, UpstreamFile } from "./types.js";

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
  sqlfluff: sqlfluffSpan,
  clippy: clippySpan,
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

/** Upstream's answers on the corpus and the parsers' verdicts as the gates read them: the archive's tree, and the commits its keys name. */
export interface RecordedCorpus {
  tree: RecordingTree;
  commitOf: KeyContext["commitOf"];
}

/** Everything a recorded adapter is read from, so tests can build one without a checkout. */
export interface RecordedInputs {
  root: string;
  manifest: PackManifest;
  /** The tool-pack's committed manifest, or null for a test that builds the recordings itself. */
  lock: RecordingLock | null;
  recording: GatesRecording;
  /** The config id the archive holds the recording under: this branch's, or the recording is stale. */
  config: string;
  corpus: RecordedCorpus | null;
  /** conformance.json `tools[]` of the pack. */
  tools: { name: string; version: string }[];
  /** The upstream rules the run asks about. */
  names: string[];
}

/** What to do when the archive the lock names is not in the cache: fetch it from S3, or record it, which writes the lock it names. */
function missingMessage(pack: string, tool: string): string {
  return `the recording packs/${pack}/upstream/${tool}/${RECORDING_LOCK} names (${RECORDING_ARCHIVE}) is not in the cache:`
    + ` fetch it with node scripts/s3/pack-fixtures.mjs recordings --pack ${pack} --tool ${tool}, or record it with npm run port-readiness:record -- --pack ${pack} --tool ${tool}`
    + ` (CI restores it from the Actions cache or S3, or records it)`;
}

/**
 * Read a gated tool-pack's `gates.yaml` and `recording.lock` from the working tree at `root`, and the recording archive
 * the lock names from the cache, extracted to its tree. It is not in the tree: `port-readiness:record` puts it in the
 * cache, and `pack-fixtures.mjs recordings` fetches it there from S3 (recording/store.ts). The archive is checked
 * against the lock's tar sha256, file count and bytes before it is extracted, and its tree is reused only while the
 * marker the extraction wrote names that archive and the canonical tar of the tree's files is still that archive
 * (archive.ts cachedTree), so a rule is never judged against an archive this commit did not name. The recording is
 * read at the one config the archive holds; corpus-answers fails every rule when it is not this branch's.
 */
export async function readRecordedInputs(root: string, cacheRoot: string, pack: string, tool: string, names: string[]): Promise<RecordedInputs> {
  const manifest = readGatesManifest(root, pack, tool);
  const lock = readRecordingLock(root, pack, tool);
  const label = `packs/${pack}/upstream/${tool}/${RECORDING_ARCHIVE}`;
  const directory = cachedTree(cacheRoot, lock.files[RECORDING_ARCHIVE], label);
  if (directory === undefined) throw new Error(missingMessage(pack, tool));
  const tree = directoryTree(directory);
  const configs = treeConfigs(tree);
  if (configs.length !== 1) throw new Error(`${label} holds ${configs.length} configs, not one: record the tool-pack again`);
  const conformance = JSON.parse(readFileSync(join(root, "packs", pack, "conformance.json"), "utf8")) as { tools?: { name: string; version: string }[] };
  return {
    root,
    manifest,
    lock,
    recording: loadRecording(tree, configs[0]!, label),
    config: configs[0]!,
    corpus: { tree, commitOf: corpusCommits(await readRerailCorpora(root, pack), readCorpusTasks()) },
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
 * The tool-pack checks the gates make on a recording: pins agree (gates.yaml, recording.lock, the recording archive,
 * conformance.json), the lock pins every runtime gates.yaml installs and no other (recordedRuntimes), the runtimes' pins
 * (package.json, pyproject.toml, .python-version, composer.json, .php-version, PHP's extensions.txt) are the committed ones, the kept licence (licenseBytes of keptLicenseDirectory) is the recorded one, every committed case source has its recorded sha256, and the recording covers every
 * upstream rule the pack names. Any failure means the recording is stale.
 *
 * The pins and the runtime digests are read from `recording.lock`, which is committed, and the cached recording
 * must agree with it: a pull request whose pins moved is stale on its own commit, whatever recording it reads, and a
 * lock that disagrees with the recording it names means the two were written by different runs.
 */
export function toolPackChecks(inputs: RecordedInputs): Check[] {
  const { root, manifest, lock, recording, tools, names } = inputs;
  const pack = manifest.rerail.pack;
  const tool = recording.tool;
  const stale = staleMessage(pack, tool);
  const checks: Check[] = [];
  const pins = lock?.pins ?? recording.pins;
  const runtimes = lock?.runtimes ?? recording.runtimes;
  const wantedRuntimes = recordedRuntimes(manifest);
  const pinnedRuntimes = Object.keys(runtimes).sort((a, b) => (a < b ? -1 : 1));
  const conformanceVersion = tools.find((entry) => entry.name === tool)?.version ?? null;
  const pinMismatches = [
    ...(manifest.upstream.version === pins.version ? [] : [`gates.yaml upstream.version ${manifest.upstream.version} but ${RECORDING_LOCK} pins.version ${pins.version}`]),
    ...(conformanceVersion === manifest.upstream.version ? [] : [`conformance.json tools[${tool}].version ${conformanceVersion ?? "missing"} but gates.yaml upstream.version ${manifest.upstream.version}`]),
    ...(manifest.upstream.commit === pins.commit ? [] : [`gates.yaml upstream.commit ${manifest.upstream.commit} but ${RECORDING_LOCK} pins.commit ${pins.commit}`]),
    ...(manifest.upstream.repository === pins.repository ? [] : [`gates.yaml upstream.repository ${manifest.upstream.repository} but ${RECORDING_LOCK} pins.repository ${pins.repository}`]),
    ...(manifest.upstream.runtime === pins.runtime ? [] : [`gates.yaml upstream.runtime ${manifest.upstream.runtime} but ${RECORDING_LOCK} pins.runtime ${pins.runtime}`]),
    ...(manifest.adapter === recording.adapter ? [] : [`gates.yaml adapter ${manifest.adapter} but ${RECORDING_ARCHIVE} adapter ${recording.adapter}`]),
    ...(lock === null || lock.tool === tool ? [] : [`${RECORDING_LOCK} tool ${lock.tool} but ${RECORDING_ARCHIVE} tool ${tool}`]),
    ...(canonical(pins) === canonical(recording.pins) ? [] : [`${RECORDING_LOCK} and ${RECORDING_ARCHIVE} were written by different runs: pins ${canonical(pins)} and ${canonical(recording.pins)}`]),
    ...(canonical(lock?.runtimes ?? recording.runtimes) === canonical(recording.runtimes) ? [] : [`${RECORDING_LOCK} and ${RECORDING_ARCHIVE} disagree about the recorder's runtimes`]),
    ...(canonical(pinnedRuntimes) === canonical(wantedRuntimes) ? [] : [`gates.yaml's adapter and environment install the runtimes ${wantedRuntimes.join(", ")} but ${RECORDING_LOCK} pins ${pinnedRuntimes.join(", ") || "none"}`]),
  ];
  checks.push({ id: "pins-agree", ok: pinMismatches.length === 0, detail: pinMismatches.length === 0 ? `${tool} ${pins.version} at ${pins.commit} in gates.yaml, ${RECORDING_LOCK}, ${RECORDING_ARCHIVE} and conformance.json` : `${pinMismatches.join("; ")}: ${stale}` });

  const pinFileMismatches: string[] = [];
  for (const [runtime, files] of Object.entries(runtimes).sort(([a], [b]) => (a < b ? -1 : 1))) {
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

  // Upstream's answers are pinned by the lock, so the sources they were recorded on must be too: an edited case source
  // would be judged against answers upstream gave on other bytes.
  const directory = toolPackDirectory(root, pack, tool);
  const recordedDigests = new Map<string, Set<string | undefined>>();
  for (const rule of Object.values(recording.rules)) {
    for (const testCase of rule.cases) if (testCase.file !== null) recordedDigests.set(testCase.file, (recordedDigests.get(testCase.file) ?? new Set()).add(testCase.sha256));
  }
  // A recording whose case carries no sha256 of its source is stale.
  const sourceMismatches: string[] = [];
  // Case sources are pack test data, not in git: a checkout without them is told to pull and derive them, not to re-record.
  const unpulled: string[] = [];
  for (const [file, digests] of [...recordedDigests].sort(([a], [b]) => (a < b ? -1 : 1))) {
    const recorded = digests.size === 1 ? [...digests][0] : undefined;
    const path = join(directory, file);
    const actual = existsSync(path) ? sha256(readFileSync(path)) : null;
    if (digests.size > 1) sourceMismatches.push(`${file} has cases that disagree about its sha256`);
    else if (actual === null && packFixtureAbsence(path) !== undefined) unpulled.push(path);
    else if (actual === null) sourceMismatches.push(`${file} is not committed`);
    else if (recorded === undefined) sourceMismatches.push(`${file} has no recorded sha256`);
    else if (actual !== recorded) sourceMismatches.push(`${file} is ${actual.slice(0, 12)}, recorded ${recorded.slice(0, 12)}`);
  }
  if (unpulled.length > 0) sourceMismatches.unshift(`${unpulled.length} case source(s) are missing, such as ${packFixtureAbsence(unpulled[0]!)}`);
  checks.push({ id: "case-sources", ok: sourceMismatches.length === 0, detail: unpulled.length > 0 ? sourceMismatches[0]! : sourceMismatches.length === 0 ? `${recordedDigests.size} committed case source(s) are the bytes upstream's answers were recorded on` : `${sourceMismatches.length} case source(s) differ: ${sourceMismatches.slice(0, 3).join("; ")}: ${stale}` });

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
  return checks;
}

/**
 * The recorded adapter's inCorpus: whether a path is of a type upstream lints. A path's type is its longest dialect
 * suffix where that is longer than its extension: `.blade.php` is no `.php`.
 */
export function recordedInCorpus(recording: Pick<GatesRecording, "corpus">): (path: string) => boolean {
  const { caseSensitive, extensions } = recording.corpus;
  const types = new Set(caseSensitive ? extensions : extensions.map((ext) => ext.toLowerCase()));
  return (path) => types.has(caseSensitive ? fileTypeOf(path) : fileTypeOf(path).toLowerCase());
}

/**
 * A PackAdapter answering from a tool-pack's recordings and committed case sources; it never runs upstream. Its
 * fixtureTexts are upstream's test files fetched at the pinned commit (`upstream` says where to cache them):
 * `loadFixtureTexts` fetches them, once, and must be awaited before fixtureTexts is read. `corpusAnswers` is the
 * tool-pack check that the recording archive is filed under this branch's config and answers every golden.
 */
export function createRecordedAdapter(
  inputs: RecordedInputs,
  upstream?: { cacheRoot: string; runner: CommandRunner },
): PackAdapter & { corpusAnswers(files: readonly CorpusFile[], goldens: readonly Golden[]): Check; loadFixtureTexts(): Promise<void> } {
  const { root, manifest, recording, corpus } = inputs;
  let fetchedTexts: string[] | undefined;
  const pack = manifest.rerail.pack;
  const directory = toolPackDirectory(root, pack, recording.tool);
  const sources = new Map<string, string>();
  const source = (file: string): string => {
    let text = sources.get(file);
    if (text === undefined) {
      const path = join(directory, file);
      text = decodeSource(readFileSync(isPackFixture(path) ? requirePackFixture(path) : path));
      sources.set(file, text);
    }
    return text;
  };
  const setNames = (ids: string[]) => ids.flatMap((id) => recording.nameSets[id]!.names);
  const setRegexes = (ids: string[]) => ids.flatMap((id) => recording.nameSets[id]!.regexes);
  const regex = ({ source: pattern, flags }: { source: string; flags: string }) => new RegExp(pattern, flags);
  const toSpan = SPAN_UNITS[recording.adapter];
  const inCorpus = recordedInCorpus(recording);
  // The answers this branch reads are those filed under its own config: its pins, runtimes, gates.yaml and adapter code.
  const runtimes = inputs.lock?.runtimes ?? recording.runtimes;
  const config = configId(answerConfig(manifest, runtimes));
  const verdictsId = parsersId(manifest.adapter, recording.parsers);
  const label = `packs/${pack}/upstream/${recording.tool}/${RECORDING_ARCHIVE}`;

  return {
    identity: recording.identity,
    packChecks: [...recording.packChecks, ...toolPackChecks(inputs)],
    // The text is the kept licence; license-sha256 checks it is upstream's.
    licenseText: licenseBytes(join(root, keptLicenseDirectory(manifest)))?.toString("utf8") ?? "",
    corpusAnswers(files, goldens) {
      const command = recordMissingAnswers(pack, recording.tool);
      if (corpus === null) return { id: "corpus-answers", ok: false, detail: `no recording archive was read: ${command}` };
      // The whole recording is filed under the config it was made at: another config's answers say nothing of this branch's upstream.
      if (inputs.config !== config) return { id: "corpus-answers", ok: false, detail: `${label} holds upstream's answers at config ${inputs.config.slice(0, 12)}, not this branch's config ${config.slice(0, 12)}: ${staleMessage(pack, recording.tool)}` };
      // A golden whose fetched file is not its sha256 fails on its own (the goldens gate); its answer cannot be named.
      const fetched = new Map(files.map((file) => [wantedKey(file.task, file.path), file.sha256]));
      const judged = goldens.filter((golden) => fetched.get(wantedKey(golden.task, golden.path)) === golden.sha256);
      const needed = neededAnswers({ manifest, runtimes, parsers: recording.parsers, rules: Object.keys(recording.rules), files: judged, context: fetchedContext(files, corpus.commitOf, Object.keys(recording.rules)) });
      const missing = missingAnswers(corpus.tree, needed);
      const goldenFiles = new Set(judged.map((golden) => wantedKey(golden.task, golden.path))).size;
      if (missing.answers.length + missing.verdicts.length === 0) {
        return { id: "corpus-answers", ok: true, detail: `${label} answers all ${needed.answers.length} rule-and-golden pair(s) (${Object.keys(recording.rules).length} rule(s), ${goldenFiles} golden file(s)) at config ${needed.configId.slice(0, 12)}` };
      }
      const first = missing.answers[0];
      const example = first !== undefined ? `${first.rule} ${first.file.task}/${first.file.path}` : `the parsers' verdict on ${missing.verdicts[0]!.file.task}/${missing.verdicts[0]!.file.path}`;
      return {
        id: "corpus-answers",
        ok: false,
        detail: `${label} lacks ${missing.answers.length} of ${needed.answers.length} answer(s) and ${missing.verdicts.length} of ${needed.verdicts.length} verdict(s) at config ${needed.configId.slice(0, 12)}, such as ${example}: ${command}`,
      };
    },
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
      if (upstream === undefined) throw new Error(`${label} records upstream's fixture texts by origin, and this recorded adapter has no cache to fetch them into`);
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
    inCorpus,
    async lintCorpus(files: CorpusFile[], names: string[]): Promise<UpstreamFile[]> {
      // A rule the recording does not have (absent upstream) has no answer anywhere, as before the archive.
      const asked = names.filter((name) => name in recording.rules);
      const context = corpus === null ? undefined : fetchedContext(files, corpus.commitOf, Object.keys(recording.rules));
      return files.map((file) => {
        const findings = new Map<string, Span[]>();
        const unanswered = new Set<string>();
        // Each rule's answer says on its own whether upstream linted the file: one rule's exclusion hides no other's.
        const excludedFor = new Map<string, string>();
        const key = context === undefined ? undefined : answerKey(manifest.adapter, file, context);
        for (const name of asked) {
          const path = key === undefined ? undefined : answerPath(config, name, file, key);
          const text = path === undefined ? undefined : corpus?.tree.text(path);
          // Without upstream's recorded answer the rule has no ground truth here; treating it as silent would invent agreement.
          if (text === undefined) {
            unanswered.add(name);
            continue;
          }
          const answer = parseAnswer(text, `${label}: ${path}`);
          if ("excluded" in answer) excludedFor.set(name, answer.excluded);
          else findings.set(name, answer.findings.map(spanOf));
        }
        // The file is excluded as a whole when every answered rule's answer excludes it, for the same reason.
        const reasons = new Set(excludedFor.values());
        const excluded = reasons.size === 1 && findings.size === 0 ? [...reasons][0]! : null;
        return { file, excluded, findings, unanswered, ...(excluded === null && excludedFor.size > 0 ? { excludedFor } : {}) };
      });
    },
    async parserVerdicts(files) {
      // Independent parsers do not run in PR CI: answer from the archive where it has the source, else no verdict.
      return files.map((file) => {
        const path = verdictPath(verdictsId, { path: file.path, sha256: sha256(file.source), script: file.script === true });
        const text = corpus?.tree.text(path);
        return text === undefined ? [] : parseVerdicts(text, `${label}: ${path}`, recording.parsers);
      });
    },
  };
}
