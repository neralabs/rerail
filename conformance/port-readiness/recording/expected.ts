/**
 * A tool-pack's expected answers: the files of the `expected/` layout, which only the generator (`npm run generate`,
 * conformance/recorders/port-readiness/expected-cli.ts) writes and the gates read. schema.ts describes each file. None
 * of them is in the tree: the generator stores them, under its key (generator-key.ts), as one blob in Azure Blob
 * Storage (expected-store.ts) and in the local cache, and the gates read the local cache.
 *
 * The generator runs the live adapter as the recorder always has, which gives one GatesRecording and one
 * GatesCorpusRecording, and `expectedFiles` splits them: tool facts into upstream.json, each rule's facts into
 * rules/<rule>.json, each upstream test file's cases into cases/<test file>.json with every finding stored once and
 * keyed by rule, the inline test texts read from it into texts/<test file>.txt, and the corpus answers into one file per
 * corpus task. `joinExpected` puts them back together exactly, which the generator checks before it stores anything,
 * so the gates answer from the same recording whichever way it was stored. Two generations are compared file by file
 * (recording/diff.ts compares them rule by rule).
 */
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { gunzipSync, gzipSync } from "node:zlib";

import {
  CORPUS_FORMAT, EXPECTED_FORMAT, RECORDING_FORMAT, expectedCorpusSchema, expectedRuleSchema, expectedTestFileSchema, expectedUpstreamSchema, gatesCorpusSchema,
  gatesRecordingSchema, recordingText,
  type ExpectedCase, type ExpectedCorpus, type ExpectedRule, type ExpectedTestFile, type ExpectedUpstream, type GatesCorpusRecording, type GatesRecording, type RecordedCase,
} from "./schema.js";

/** The layout the answers are split into; messages name a file of the answers by its path under it (`expected/upstream.json`). */
export const EXPECTED_DIRECTORY = "expected";
export const UPSTREAM_FILE = "upstream.json";

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");
const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** A path the layout may name: `/`-separated, relative, without `.` or `..` segments or a NUL. */
const SAFE_PATH = /^(?!\/)(?!.*(?:^|\/)\.\.?(?:\/|$))[^\0\\]+$/u;

/** A case's upstream test file: its fixture without the run and case it names (`tests/lib/rules/eqeqeq.js#0/valid[3]`). */
export function testFileOf(fixture: string): string {
  return fixture.replace(/#.*$/u, "");
}

/** Where each kind of file is, relative to expected/. */
const rulePath = (name: string): string => `rules/${name}.json`;
const casesPath = (test: string): string => `cases/${test}.json`;
const textsPath = (test: string): string => `texts/${test}.txt`;
/** A pinned corpus's task `rerail:<name>` is corpus/rerail/<name>.json: Windows allows no `:` in a file name. */
const corpusPath = (task: string): string => `corpus/${task.replace(/^rerail:/u, "rerail/")}.json`;

function safe(path: string, what: string): string {
  if (!SAFE_PATH.test(path)) throw new Error(`${what} ${JSON.stringify(path)} cannot name a file under ${EXPECTED_DIRECTORY}/`);
  return path;
}

/**
 * A texts/ file's text. Each text is a header line and the text itself, then a newline: `### text <n>` and the text
 * verbatim, `<n>` its length in UTF-16 code units, so no content can end it early; or, for a text that a UTF-8 text file
 * cannot hold as it is (a lone surrogate, a NUL, which makes git read the file as binary, or a CR), `### json` and the
 * text as one JSON string. The texts are in code-unit order.
 */
export function textsFileText(texts: readonly string[]): string {
  return texts.map((text) => (text.isWellFormed() && !/[\0\r]/u.test(text) ? `### text ${text.length}\n${text}\n` : `### json\n${JSON.stringify(text)}\n`)).join("");
}

/** The texts a texts/ file holds (textsFileText's inverse). */
export function parseTextsFile(content: string, label: string): string[] {
  const texts: string[] = [];
  let at = 0;
  while (at < content.length) {
    const lineEnd = content.indexOf("\n", at);
    const header = lineEnd < 0 ? content.slice(at) : content.slice(at, lineEnd);
    const raw = /^### text (\d+)$/u.exec(header);
    if (raw !== null) {
      const end = lineEnd + 1 + Number(raw[1]);
      if (lineEnd < 0 || end >= content.length || content[end] !== "\n") throw new Error(`${label}: the text at ${at} is not ${raw[1]} code units long`);
      texts.push(content.slice(lineEnd + 1, end));
      at = end + 1;
    } else if (header === "### json") {
      const end = content.indexOf("\n", lineEnd + 1);
      if (lineEnd < 0 || end < 0) throw new Error(`${label}: the JSON text at ${at} has no line`);
      const text: unknown = JSON.parse(content.slice(lineEnd + 1, end));
      if (typeof text !== "string") throw new Error(`${label}: the JSON text at ${at} is no string`);
      texts.push(text);
      at = end + 1;
    } else {
      throw new Error(`${label}: ${JSON.stringify(header.slice(0, 40))} at ${at} is no "### text <n>" or "### json" header`);
    }
  }
  return texts;
}

/** What the generator splits into expected/. */
export interface ExpectedSource {
  recording: GatesRecording;
  corpus: GatesCorpusRecording;
  /** The upstream test file each of `recording.fixtureTexts.inline` was read from, in that list's order. */
  inlineTests: readonly string[];
  /** sha256 of an upstream test file's bytes at the pinned commit, or null where upstream's checkout lacks it. */
  testFileDigest(test: string): string | null;
  /** A committed case source's text as the gates read it (upstream-files.ts's decodeSource), by its tool-pack-relative path. */
  sourceText(file: string): string;
}

/** The fields that make two rules' entries for one test case the same case, as text. */
const caseIdentity = (testCase: RecordedCase): string => JSON.stringify([
  testCase.fixture, testCase.variant, testCase.path, testCase.parsedAs ?? null, testCase.script ?? null, testCase.skipped, testCase.file, testCase.sha256 ?? null,
]);

/** The key the gates look a case source's parser verdicts up by: its read path, whether it is a script, and the sha256 of its text. */
const verdictKey = (path: string, script: boolean, digest: string): string => `${path}\0${script}\0${digest}`;

/**
 * The expected/ files for a recording, by their path under expected/, each as its text. Throws when the files would not
 * join back into exactly this recording (joinExpected), which is what the gates would read.
 */
export function expectedFiles(source: ExpectedSource): Map<string, string> {
  const { recording, corpus } = source;
  const names = Object.keys(recording.rules).sort(byText);

  // A name set only one rule names goes in that rule's file, so a rule whose names move rewrites its own file alone.
  const users = new Map<string, Set<string>>();
  for (const name of names) {
    const { implementation, shared } = recording.rules[name]!.names;
    for (const id of [...implementation.sets, ...shared.sets]) users.set(id, (users.get(id) ?? new Set()).add(name));
  }
  const ownSets = (name: string): GatesRecording["nameSets"] => Object.fromEntries(Object.entries(recording.nameSets).filter(([id]) => users.get(id)?.size === 1 && users.get(id)!.has(name)));
  const sharedSets = Object.fromEntries(Object.entries(recording.nameSets).filter(([id]) => users.get(id)?.size !== 1));

  // Each test file's cases in the order the rules, taken by name, first list them; each rule's findings once per case.
  const testFiles = new Map<string, { entries: ExpectedCase[]; byIdentity: Map<string, ExpectedCase> }>();
  const ruleFiles = new Map<string, ExpectedRule>();
  for (const name of names) {
    const rule = recording.rules[name]!;
    const tests: string[] = [];
    for (const testCase of rule.cases) {
      const test = safe(testFileOf(testCase.fixture), "the test file");
      if (!tests.includes(test)) tests.push(test);
      let file = testFiles.get(test);
      if (file === undefined) {
        file = { entries: [], byIdentity: new Map() };
        testFiles.set(test, file);
      }
      const identity = caseIdentity(testCase);
      let entry = file.byIdentity.get(identity);
      if (entry === undefined) {
        entry = {
          fixture: testCase.fixture,
          variant: testCase.variant,
          path: testCase.path,
          ...(testCase.parsedAs === undefined ? {} : { parsedAs: testCase.parsedAs }),
          ...(testCase.script === undefined ? {} : { script: testCase.script }),
          skipped: testCase.skipped,
          file: testCase.file,
          ...(testCase.sha256 === undefined ? {} : { sha256: testCase.sha256 }),
          expected: {},
        };
        file.byIdentity.set(identity, entry);
        file.entries.push(entry);
      }
      if (name in entry.expected) throw new Error(`${name} lists the case ${testCase.fixture} (${testCase.variant}) twice, which ${EXPECTED_DIRECTORY}/ cannot hold`);
      entry.expected[name] = testCase.expected;
      if (testCase.markup !== null) entry.markup = { ...entry.markup, [name]: testCase.markup };
    }
    ruleFiles.set(name, { facts: rule.facts, names: rule.names, nameSets: ownSets(name), spanParts: rule.spanParts, cases: tests });
  }

  // Each parser verdict goes with the first test file (code-unit order) one of whose committed case sources it is on;
  // the rest (the known gaps' probes) are the tool's.
  const verdictTest = new Map<string, string>();
  for (const [test, { entries }] of testFiles) {
    for (const entry of entries) {
      if (entry.file === null) continue;
      const path = entry.parsedAs === undefined ? entry.path : entry.path.replace(/\.[^./]*$/u, entry.parsedAs);
      const key = verdictKey(path, entry.script === true, sha256(source.sourceText(entry.file)));
      const known = verdictTest.get(key);
      if (known === undefined || byText(test, known) < 0) verdictTest.set(key, test);
    }
  }
  const verdictsOf = new Map<string | null, GatesRecording["parserVerdicts"]>();
  for (const entry of recording.parserVerdicts) {
    const test = verdictTest.get(verdictKey(entry.path, entry.script, entry.sha256)) ?? null;
    verdictsOf.set(test, [...(verdictsOf.get(test) ?? []), entry]);
  }

  // The inline texts by the test file each was read from.
  const inline = recording.fixtureTexts?.inline ?? [];
  if (source.inlineTests.length !== inline.length) throw new Error(`${inline.length} inline fixture text(s) but ${source.inlineTests.length} test file(s) they were read from`);
  const textsOf = new Map<string, string[]>();
  inline.forEach(({ text }, index) => {
    const test = safe(source.inlineTests[index]!, "the test file");
    textsOf.set(test, [...(textsOf.get(test) ?? []), text]);
  });

  const files = new Map<string, string>();
  const upstream: ExpectedUpstream = {
    format: EXPECTED_FORMAT,
    tool: recording.tool,
    adapter: recording.adapter,
    pins: recording.pins,
    runtimes: recording.runtimes,
    identity: recording.identity,
    packChecks: recording.packChecks,
    licenseSha256: recording.licenseSha256,
    inScopeSamples: recording.inScopeSamples,
    corpus: recording.corpus,
    keywords: recording.keywords,
    commentVocabulary: recording.commentVocabulary,
    fixtureWords: recording.fixtureWords,
    ...(recording.fixtureTexts === undefined ? {} : { fixtureFiles: recording.fixtureTexts.files }),
    nameSets: sharedSets,
    absent: recording.absent,
    parsers: recording.parsers,
    parserVerdicts: verdictsOf.get(null) ?? [],
    corpusPins: corpus.pins,
  };
  files.set(UPSTREAM_FILE, recordingText(upstream));
  for (const [name, rule] of ruleFiles) files.set(rulePath(safe(name, "the rule name")), recordingText(rule));
  for (const [test, { entries }] of testFiles) {
    const testFile: ExpectedTestFile = { test, sha256: source.testFileDigest(test), parserVerdicts: verdictsOf.get(test) ?? [], cases: entries };
    files.set(casesPath(test), recordingText(testFile));
  }
  for (const [test, texts] of textsOf) files.set(textsPath(test), textsFileText(texts.sort(byText)));
  const tasks = new Map<string, ExpectedCorpus["files"]>();
  for (const { task, ...file } of corpus.files) tasks.set(task, [...(tasks.get(task) ?? []), file]);
  for (const [task, taskFiles] of tasks) {
    if (task.replace(/^rerail:/u, "").includes(":")) throw new Error(`the corpus task ${task} cannot name a file under ${EXPECTED_DIRECTORY}/corpus/`);
    files.set(safe(corpusPath(task), "the corpus file"), recordingText({ task, files: taskFiles } satisfies ExpectedCorpus));
  }

  // What the gates will read must be this recording, to the byte; licenseText is left out (the licence is kept in the pack).
  const joined = joinExpected(files, "the generated expected/");
  const { licenseText: _licenseText, ...recorded } = recording;
  if (recordingText(joined.recording) !== recordingText(recorded)) throw new Error(`the expected/ files do not join into the recording they were split from: ${firstDifference(recordingText(recorded), recordingText(joined.recording))}`);
  if (recordingText(joined.corpus) !== recordingText(corpus)) throw new Error(`the expected/ corpus files do not join into corpus.json: ${firstDifference(recordingText(corpus), recordingText(joined.corpus))}`);
  return files;
}

/** Where two texts first differ, for an error message. */
function firstDifference(a: string, b: string): string {
  let at = 0;
  while (at < a.length && at < b.length && a[at] === b[at]) at += 1;
  const line = a.slice(0, at).split("\n").length;
  return `at line ${line}: ${JSON.stringify(a.slice(at, at + 80))} became ${JSON.stringify(b.slice(at, at + 80))}`;
}

function parseFile<T>(schema: { safeParse(value: unknown): { success: true; data: T } | { success: false; error: { issues: { path: PropertyKey[]; message: string }[] } } }, label: string, text: string): T {
  const parsed = schema.safeParse(JSON.parse(text));
  if (!parsed.success) throw new Error(`${label}: ${parsed.error.issues.slice(0, 5).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  return parsed.data;
}

/** The recording and corpus answers expected/'s files hold, checked against recording/schema.ts. */
export interface JoinedExpected {
  recording: GatesRecording;
  corpus: GatesCorpusRecording;
}

/**
 * Join expected/'s files (by path under expected/) back into the recording and corpus answers they were split from.
 * Any file the layout does not name, a case no rule reads, or a test file a rule names that is missing throws: the
 * gates draw no conclusion from a half-written or hand-edited directory.
 */
export function joinExpected(files: ReadonlyMap<string, string>, label: string): JoinedExpected {
  const upstreamText = files.get(UPSTREAM_FILE);
  if (upstreamText === undefined) throw new Error(`${label} has no ${UPSTREAM_FILE}`);
  const upstream = parseFile(expectedUpstreamSchema, `${label}/${UPSTREAM_FILE}`, upstreamText);
  const rules = new Map<string, ExpectedRule>();
  const testFiles = new Map<string, ExpectedTestFile>();
  const texts: string[] = [];
  const textFiles: string[] = [];
  const corpusFiles: GatesCorpusRecording["files"] = [];
  for (const [path, text] of [...files].sort(([a], [b]) => byText(a, b))) {
    const where = `${label}/${path}`;
    if (path === UPSTREAM_FILE) continue;
    if (path.startsWith("rules/") && path.endsWith(".json")) {
      rules.set(path.slice("rules/".length, -".json".length), parseFile(expectedRuleSchema, where, text));
    } else if (path.startsWith("cases/") && path.endsWith(".json")) {
      const testFile = parseFile(expectedTestFileSchema, where, text);
      if (casesPath(testFile.test) !== path) throw new Error(`${where} holds the cases of ${testFile.test}, which belong in ${casesPath(testFile.test)}`);
      testFiles.set(testFile.test, testFile);
    } else if (path.startsWith("texts/") && path.endsWith(".txt")) {
      texts.push(...parseTextsFile(text, where));
      textFiles.push(path);
    } else if (path.startsWith("corpus/") && path.endsWith(".json")) {
      const corpus = parseFile(expectedCorpusSchema, where, text);
      if (corpusPath(corpus.task) !== path) throw new Error(`${where} holds the answers of ${corpus.task}, which belong in ${corpusPath(corpus.task)}`);
      corpusFiles.push(...corpus.files.map((file) => ({ task: corpus.task, ...file })));
    } else {
      throw new Error(`${where} is no file of the ${EXPECTED_DIRECTORY}/ layout (upstream.json, rules/, cases/, texts/, corpus/)`);
    }
  }
  if (upstream.fixtureFiles === undefined && textFiles.length > 0) throw new Error(`${label}/${textFiles[0]!} holds inline texts, and ${UPSTREAM_FILE} records no fixture texts`);

  const nameSets: GatesRecording["nameSets"] = { ...upstream.nameSets };
  for (const [name, rule] of rules) {
    for (const [id, set] of Object.entries(rule.nameSets)) {
      if (id in nameSets) throw new Error(`${label}/${rulePath(name)} holds the name set ${id}, which another file holds too`);
      nameSets[id] = set;
    }
  }
  const read = new Set<string>();
  const recordedRules: GatesRecording["rules"] = {};
  for (const [name, rule] of [...rules].sort(([a], [b]) => byText(a, b))) {
    const cases = rule.cases.flatMap((test) => {
      const testFile = testFiles.get(test);
      if (testFile === undefined) throw new Error(`${label}/${rulePath(name)} reads the cases of ${test}, and ${casesPath(test)} is missing`);
      return testFile.cases.filter((testCase) => name in testCase.expected).map((testCase): RecordedCase => {
        read.add(`${test}\0${name}`);
        return {
          fixture: testCase.fixture,
          variant: testCase.variant,
          path: testCase.path,
          ...(testCase.parsedAs === undefined ? {} : { parsedAs: testCase.parsedAs }),
          ...(testCase.script === undefined ? {} : { script: testCase.script }),
          skipped: testCase.skipped,
          expected: testCase.expected[name]!,
          markup: testCase.markup?.[name] ?? null,
          file: testCase.file,
          ...(testCase.sha256 === undefined ? {} : { sha256: testCase.sha256 }),
        };
      });
    });
    recordedRules[name] = { facts: rule.facts, names: rule.names, spanParts: rule.spanParts, cases };
  }
  for (const testFile of testFiles.values()) {
    for (const testCase of testFile.cases) {
      for (const name of Object.keys(testCase.expected)) {
        if (!read.has(`${testFile.test}\0${name}`)) throw new Error(`${label}/${casesPath(testFile.test)} has answers of ${name} for ${testCase.fixture}, and ${rulePath(name)} does not read ${testFile.test}`);
      }
    }
  }

  const parserVerdicts = [...upstream.parserVerdicts, ...[...testFiles.values()].flatMap((testFile) => testFile.parserVerdicts)]
    .sort((a, b) => byText(verdictKey(a.path, a.script, a.sha256), verdictKey(b.path, b.script, b.sha256)));
  const recording = parseFile(gatesRecordingSchema, `${label} (joined)`, JSON.stringify({
    format: RECORDING_FORMAT,
    tool: upstream.tool,
    adapter: upstream.adapter,
    pins: upstream.pins,
    runtimes: upstream.runtimes,
    identity: upstream.identity,
    packChecks: upstream.packChecks,
    licenseSha256: upstream.licenseSha256,
    inScopeSamples: upstream.inScopeSamples,
    corpus: upstream.corpus,
    keywords: upstream.keywords,
    commentVocabulary: upstream.commentVocabulary,
    fixtureWords: upstream.fixtureWords,
    ...(upstream.fixtureFiles === undefined ? {} : { fixtureTexts: { files: upstream.fixtureFiles, inline: texts.sort(byText).map((text) => ({ text })) } }),
    nameSets,
    rules: recordedRules,
    absent: upstream.absent,
    parsers: upstream.parsers,
    parserVerdicts,
  }));
  const corpus = parseFile(gatesCorpusSchema, `${label} (joined corpus)`, JSON.stringify({
    format: CORPUS_FORMAT,
    tool: upstream.tool,
    pins: upstream.corpusPins,
    rules: Object.keys(recordedRules).sort(byText),
    parsers: upstream.parsers,
    files: corpusFiles.sort((a, b) => byText(`${a.task}\0${a.path}`, `${b.task}\0${b.path}`)),
  }));
  return { recording, corpus };
}

/** One digest over the answers' files: sha256 of a `<sha256>  <path>\n` line per file, in path order. */
export function expectedDigest(files: ReadonlyMap<string, string>): string {
  return sha256([...files].sort(([a], [b]) => byText(a, b)).map(([path, text]) => `${sha256(text)}  ${path}\n`).join(""));
}

/** Tags a blob of answers, so one written in another layout is rejected rather than misread. */
export const EXPECTED_BLOB_FORMAT = "rerail.port-readiness-expected/v1";

/** A blob's name under its key, in the store and in the local cache. */
export const EXPECTED_BLOB = "expected.json.gz";

/** A blob of answers as it is stored: gzipped JSON, and the sha256 of the JSON text (the store keeps it as metadata). */
export interface EncodedExpected {
  bytes: Buffer;
  sha256: string;
}

/**
 * The blob for one generator key: gzip of `{"format","pack","tool","key","files"}` as compact JSON, `files` by path
 * under expected/ in code-unit order, so identical answers are identical bytes. The JSON is written field by field
 * because an object's own key order is not the insertion order for a path that reads as an integer.
 */
export function encodeExpected(pack: string, tool: string, key: string, files: ReadonlyMap<string, string>): EncodedExpected {
  const entries = [...files].sort(([a], [b]) => byText(a, b)).map(([path, text]) => `${JSON.stringify(path)}:${JSON.stringify(text)}`);
  const text = `{"format":${JSON.stringify(EXPECTED_BLOB_FORMAT)},"pack":${JSON.stringify(pack)},"tool":${JSON.stringify(tool)},"key":${JSON.stringify(key)},"files":{${entries.join(",")}}}`;
  return { bytes: gzipSync(Buffer.from(text, "utf8")), sha256: sha256(text) };
}

/**
 * The answers a blob holds, by path under expected/. A blob is rejected when it does not gunzip, when its JSON's sha256
 * is not `sha256` (when given: the digest the store's metadata carries), when it is another layout, or when it holds
 * the answers for another key than `key`: answers read under the wrong key would be judged as the tree's.
 */
export function decodeExpected(bytes: Buffer, key: string, sha256Of?: string): Map<string, string> {
  let text: string;
  try {
    text = gunzipSync(bytes).toString("utf8");
  } catch (error) {
    throw new Error(`the answers for ${key} are not gzip: ${error instanceof Error ? error.message : String(error)}`);
  }
  if (sha256Of !== undefined && sha256(text) !== sha256Of) throw new Error(`the answers for ${key} have sha256 ${sha256(text).slice(0, 12)}, the store says ${sha256Of.slice(0, 12)}`);
  const value: unknown = JSON.parse(text);
  const blob = (value ?? {}) as { format?: unknown; pack?: unknown; tool?: unknown; key?: unknown; files?: unknown };
  if (blob.format !== EXPECTED_BLOB_FORMAT) throw new Error(`the answers for ${key} are in the layout ${JSON.stringify(blob.format)}, not ${EXPECTED_BLOB_FORMAT}`);
  if (blob.key !== key) throw new Error(`the answers stored for ${key} are those for ${JSON.stringify(blob.key)}`);
  if (typeof blob.pack !== "string" || typeof blob.tool !== "string" || !key.startsWith(`${blob.pack}/${blob.tool}/`)) throw new Error(`the answers for ${key} name the tool-pack ${JSON.stringify(blob.pack)}/${JSON.stringify(blob.tool)}`);
  if (blob.files === null || typeof blob.files !== "object" || Array.isArray(blob.files)) throw new Error(`the answers for ${key} hold no files`);
  const files = new Map<string, string>();
  for (const [path, file] of Object.entries(blob.files)) {
    if (typeof file !== "string") throw new Error(`the answers for ${key} hold ${path} as no text`);
    files.set(safe(path, "the file"), file);
  }
  return files;
}

/** Where the local cache keeps the answers for a key: `<cacheRoot>/expected/<key>/expected.json.gz`. */
export function expectedCachePath(cacheRoot: string, key: string): string {
  return join(cacheRoot, EXPECTED_DIRECTORY, ...key.split("/"), EXPECTED_BLOB);
}

/** Put a blob in the local cache, through a temporary name so a reader never sees half of it. */
export async function writeCachedExpected(cacheRoot: string, key: string, bytes: Buffer): Promise<string> {
  const path = expectedCachePath(cacheRoot, key);
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, bytes);
  await rename(temporary, path);
  return path;
}

/** The answers the local cache holds for a key, or undefined when it holds none; a blob that does not decode throws. */
export function readCachedExpected(cacheRoot: string, key: string): Map<string, string> | undefined {
  const path = expectedCachePath(cacheRoot, key);
  return existsSync(path) ? decodeExpected(readFileSync(path), key) : undefined;
}

/**
 * A tool-pack's answers for the generator key of this tree, read from the local cache and joined, and the digest of
 * their files. The gates download nothing: `npm run generate` puts the answers in the cache (from the store, or by
 * generating them), and a miss names that command.
 */
export function readExpected(cacheRoot: string, pack: string, tool: string, key: string): JoinedExpected & { digest: string } {
  const files = readCachedExpected(cacheRoot, key);
  if (files === undefined) throw new Error(`answers for ${key} are not available: run npm run generate -- --pack ${pack} --tool ${tool}`);
  return { ...joinExpected(files, key), digest: expectedDigest(files) };
}
