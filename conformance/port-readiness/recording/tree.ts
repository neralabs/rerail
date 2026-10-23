/**
 * The part of a tool-pack's recording archive (archive.ts) that answers for upstream's own tests, and the loader that
 * reads it back as the `GatesRecording` the gates judge with (schema.ts):
 *
 *   tool/<config-id>/<field>.json                  one per TOOL_FIELDS: identity, packChecks, license, the words and
 *                                                  texts not-overfit reads, absent, parsers, pins, runtimes, ...
 *   tool/<config-id>/name-sets/<kind>~<id>.json    names and regexes several rules share, stored once
 *   rules/<config-id>/<rule>/rule.json             the rule's facts, names and case index
 *   rules/<config-id>/<rule>/cases/<key>.json      upstream's answer on one case: `{expected, skipped}`
 *   texts/<sha256>.json                            one inline upstream test text, a JSON string
 *
 * `<rule>` is the rule's name with `/` written `~`. A case index entry is the case's identity (fixture, variant, path,
 * parsedAs, script), its committed source (`file`), upstream's written expectation (`markup`, parsed from the test,
 * not a run) and the `key` its answer is filed under (caseKey); a case whose source cannot be committed (`not-copied`)
 * has no source to key by, and carries its answer in the index. An answer is upstream's run on the case: what it
 * found (`expected`) and why it did not lint it (`skipped`, or null).
 *
 * Whole-tool facts are under tool/: not-overfit judges every rule against every upstream test text and word, which
 * cover every upstream test file, not one rule's. A text is named by the sha256 of its canonical JSON, which keeps a
 * lone UTF-16 surrogate (an ESLint test's `code` may hold one) as its escape.
 *
 * ESLint's spanParts is no file: a rule's end positions are compared only when every finding on its own tests carries
 * them, which its answers say (`eslintSpanParts`); every other adapter's is a fact of the rule, in rule.json.
 */
import { createHash } from "node:crypto";
import { extname } from "node:path";

import { z } from "zod";

import type { GatesManifest } from "../manifest.js";
import { eslintSpanParts } from "../span-units/eslint.js";
import { configPath, type RecordingTree } from "./archive.js";
import { parseJson } from "./corpus-answers.js";
import {
  canonicalText, caseSchema, checkSchema, gatesRecordingSchema, nameSetSchema, nameSourceSchema, parserSchema, recordedSpanSchema, ruleFactsSchema, spanPartsSchema,
  type GatesRecording, type RecordedCase, type RecordedRule,
} from "./schema.js";

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");
const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);
const json = (value: unknown): Buffer => Buffer.from(canonicalText(value), "utf8");

/** How many hex digits of the digest of a case's other inputs its key carries. */
const KEY_DIGITS = 12;

/** The whole-tool facts, one file each. `license` is licenseSha256 with licenseText; `fixtureTexts` names its inline texts by digest. */
export const TOOL_FIELDS = [
  "tool", "adapter", "pins", "runtimes", "identity", "packChecks", "license", "inScopeSamples", "corpus", "keywords", "commentVocabulary", "fixtureWords", "fixtureTexts", "absent", "parsers",
] as const;
type ToolField = (typeof TOOL_FIELDS)[number];

export const toolPath = (config: string, field: ToolField): string => `tool/${config}/${field}.json`;

/** A name set's id is `<kind>:<16 hex>` (record.ts's setId); its file writes the `:` as `~`, which no file system refuses. */
const SET_ID = /^[a-z-]+:[0-9a-f]{16}$/u;
export function nameSetPath(config: string, id: string): string {
  if (!SET_ID.test(id)) throw new Error(`name set id ${id} is not <kind>:<16 hex digits>`);
  return `tool/${config}/name-sets/${id.replace(":", "~")}.json`;
}

export const ruleDirectory = (config: string, rule: string): string => `rules/${config}/${rule.replaceAll("/", "~")}/`;
export const caseAnswerPath = (config: string, rule: string, key: string): string => `${ruleDirectory(config, rule)}cases/${key}.json`;
export const textPath = (digest: string): string => `texts/${digest}.json`;
/** A text's name: the sha256 of its file's bytes. */
export const textDigest = (text: string): string => sha256(canonicalText(text));

/**
 * Adapters whose answer on a case depends on the other rules recorded with it, so a case key carries the digest of the
 * recorded rules (corpus-answers.ts's rulesDigest): stillat/blade-parser leaves a template out for every validator when
 * one throws, PHPStan picks a data file's test method and levels among the ported rules, SQLFluff lints with every
 * recorded code selected, and PHP_CodeSniffer's and tlint's skips come from linting the ported group at once.
 */
export const RULE_SET_KEYED: Record<GatesManifest["adapter"], boolean> = {
  bandit: false,
  ruff: false,
  eslint: false,
  phpcs: true,
  tlint: true,
  "blade-parser": true,
  phpstan: true,
  psalm: false,
  bladestan: false,
  sqlfluff: true,
  clippy: false,
};

/**
 * Adapters the recorder can ask about some rules alone: a rule's cases and answers, and the whole-tool facts, do not
 * depend on which other rules are recorded (ESLint loads every test file and lints the asked rules' cases, Ruff runs
 * each code over its own fixtures, Bandit runs every test over every example). Every other adapter's case set or
 * answers depend on the recorded rules (a ported rule's tests are cases of every rule, or the rules are linted
 * together), so a change to the recorded rules records the whole tool-pack again.
 */
export const RECORDS_PER_RULE: Record<GatesManifest["adapter"], boolean> = {
  bandit: true,
  ruff: true,
  eslint: true,
  phpcs: false,
  tlint: false,
  "blade-parser": false,
  phpstan: false,
  psalm: false,
  bladestan: false,
  sqlfluff: false,
  clippy: false,
};

type CaseIdentity = Pick<RecordedCase, "fixture" | "variant" | "path" | "parsedAs" | "script">;

/**
 * What upstream's answer on a case depends on besides its source and the config, by adapter: every input the adapter
 * feeds upstream that is not positional.
 *
 * - ESLint: the case's own test configuration (its languageOptions, globals, parser, settings, and its run's tester
 *   configuration) decides the answer, and no recorded field holds it: two cases of one rule with the same code,
 *   options, file name and source type can be answered differently. So the key takes the positional fixture id
 *   (`<test file>#<run>/<kind>[<index>]`), with the rest of the case's identity. Positions cannot move within one
 *   config id, which pins upstream's commit; a bump is a new config id, recorded whole. A per-case digest of that
 *   configuration, written by the driver, would let equal cases share an answer.
 * - Psalm: the case's PHP version, ignored issues and its test class's settings, which only the fixture
 *   (`tests/<X>Test.php#<provider>/<case>`) names. SQLFluff: the case's own rule configuration, in its YAML case
 *   (`<CODE>.yml#<case>`), and the dialect its path names.
 * - Ruff: the fixture's path (`.py` or `.pyi`, and the `__init__.py` files above it, which path-dependent rules read).
 *   Clippy: the UI test's path (its aux crates and a ui-toml directory's clippy.toml). Bladestan: the template's path
 *   (its view name). PHP_CodeSniffer and PHPStan: the path (sniffs read the file name; the data file) and the variant
 *   (the owner test's CLI values; the test method).
 * - tlint and stillat/blade-parser: the template's file type and the variant (the owner test's preset group).
 * - Bandit: the example's file type.
 *
 * RULE_SET_KEYED adapters add the recorded rules.
 */
function caseInputs(adapter: GatesManifest["adapter"], testCase: CaseIdentity): Record<string, unknown> {
  const type = extname(testCase.path);
  switch (adapter) {
    case "eslint":
      return { fixture: testCase.fixture, variant: testCase.variant, path: testCase.path, parsedAs: testCase.parsedAs, script: testCase.script };
    case "psalm":
    case "sqlfluff":
      return { fixture: testCase.fixture, variant: testCase.variant, path: testCase.path };
    case "ruff":
    case "clippy":
    case "bladestan":
      return { path: testCase.path };
    case "phpcs":
    case "phpstan":
      return { path: testCase.path, variant: testCase.variant };
    case "tlint":
    case "blade-parser":
      return { type, variant: testCase.variant };
    case "bandit":
      return { type };
  }
}

/** The key a case's answer is filed under: its source's sha256, `+`, and 12 hex digits of its other inputs (caseInputs). */
export function caseKey(adapter: GatesManifest["adapter"], testCase: CaseIdentity, source: string, rules: string): string {
  const inputs = { ...caseInputs(adapter, testCase), ...(RULE_SET_KEYED[adapter] ? { rules } : {}) };
  return `${source}+${sha256(canonicalText(inputs)).slice(0, KEY_DIGITS)}`;
}

const sha256Schema = z.string().regex(/^[0-9a-f]{64}$/u);
const keySchema = z.string().regex(/^[0-9a-f]{64}\+[0-9a-f]{12}$/u);

/** A rule.json case index entry: a copied case names its answer by `key`; a not-copied one holds its answer. */
const indexEntrySchema = z.strictObject({
  fixture: caseSchema.shape.fixture,
  variant: caseSchema.shape.variant,
  path: caseSchema.shape.path,
  parsedAs: caseSchema.shape.parsedAs,
  script: caseSchema.shape.script,
  markup: caseSchema.shape.markup,
  file: caseSchema.shape.file,
  key: keySchema.optional(),
  skipped: caseSchema.shape.skipped.optional(),
  expected: caseSchema.shape.expected.optional(),
}).refine((entry) => (entry.file === null) === (entry.key === undefined) && (entry.key === undefined) === (entry.expected !== undefined && entry.skipped !== undefined), "a case with a committed source has a key and no answer of its own; one without has its answer and no key");

const ruleFileSchema = z.strictObject({
  /** The rule as the pack's conformance.json names it, which its directory writes with `/` as `~`. */
  name: z.string().min(1),
  facts: ruleFactsSchema,
  names: z.strictObject({ implementation: nameSourceSchema, shared: nameSourceSchema }),
  /** Absent for ESLint, whose span parts its answers say. */
  spanParts: spanPartsSchema.optional(),
  cases: z.array(indexEntrySchema),
});

const caseAnswerSchema = z.strictObject({ expected: z.array(recordedSpanSchema), skipped: z.string().min(1).nullable() });

const fixtureTextsFileSchema = z.strictObject({
  files: gatesRecordingSchema.shape.fixtureTexts.unwrap().shape.files,
  /** Each inline text's digest (textDigest), in the order of the texts, each as often as upstream has it. */
  inline: z.array(sha256Schema),
}).nullable();

const TOOL_SCHEMAS: Record<ToolField, z.ZodType> = {
  tool: gatesRecordingSchema.shape.tool,
  adapter: gatesRecordingSchema.shape.adapter,
  pins: gatesRecordingSchema.shape.pins,
  runtimes: gatesRecordingSchema.shape.runtimes,
  identity: gatesRecordingSchema.shape.identity,
  packChecks: z.array(checkSchema),
  license: z.strictObject({ sha256: sha256Schema, text: z.string().min(1).optional() }),
  inScopeSamples: gatesRecordingSchema.shape.inScopeSamples,
  corpus: gatesRecordingSchema.shape.corpus,
  keywords: gatesRecordingSchema.shape.keywords,
  commentVocabulary: gatesRecordingSchema.shape.commentVocabulary,
  fixtureWords: gatesRecordingSchema.shape.fixtureWords,
  fixtureTexts: fixtureTextsFileSchema,
  absent: gatesRecordingSchema.shape.absent,
  parsers: z.array(parserSchema).min(1),
};

/** A rule's files, and how many of its cases share an answer file with another of them. */
interface RuleFiles {
  files: Map<string, Buffer>;
  deduplicated: number;
}

/**
 * One rule's files: rule.json and an answer per key. Two cases of the rule under one key must have the same answer,
 * or the key lacks an input the answer depends on, and this throws rather than file one answer for both.
 */
export function ruleFiles(config: string, adapter: GatesManifest["adapter"], rules: string, name: string, rule: RecordedRule): RuleFiles {
  const files = new Map<string, Buffer>();
  let deduplicated = 0;
  const cases = rule.cases.map((testCase) => {
    const identity = {
      fixture: testCase.fixture,
      variant: testCase.variant,
      path: testCase.path,
      ...(testCase.parsedAs === undefined ? {} : { parsedAs: testCase.parsedAs }),
      ...(testCase.script === undefined ? {} : { script: testCase.script }),
      markup: testCase.markup,
      file: testCase.file,
    };
    if (testCase.file === null) return { ...identity, skipped: testCase.skipped, expected: testCase.expected };
    if (testCase.sha256 === undefined) throw new Error(`${name}: ${testCase.fixture} has a committed source and no sha256 of it`);
    const key = caseKey(adapter, testCase, testCase.sha256, rules);
    const path = caseAnswerPath(config, name, key);
    const answer = json({ expected: testCase.expected, skipped: testCase.skipped });
    const filed = files.get(path);
    if (filed !== undefined) {
      if (!filed.equals(answer)) throw new Error(`${name}: two cases are filed under the key ${key} with different answers (${testCase.fixture} among them): the key lacks an input upstream's answer depends on`);
      deduplicated += 1;
    }
    files.set(path, answer);
    return { ...identity, key };
  });
  const spanParts = adapter === "eslint" ? undefined : rule.spanParts;
  files.set(`${ruleDirectory(config, name)}rule.json`, json({ name, facts: rule.facts, names: rule.names, ...(spanParts === undefined ? {} : { spanParts }), cases }));
  return { files, deduplicated };
}

/** The whole-tool files of a recording, its name sets and its inline texts. */
export function toolFiles(config: string, recording: GatesRecording): Map<string, Buffer> {
  const { fixtureTexts } = recording;
  const values: Record<ToolField, unknown> = {
    tool: recording.tool,
    adapter: recording.adapter,
    pins: recording.pins,
    runtimes: recording.runtimes,
    identity: recording.identity,
    packChecks: recording.packChecks,
    license: { sha256: recording.licenseSha256, ...(recording.licenseText === undefined ? {} : { text: recording.licenseText }) },
    inScopeSamples: recording.inScopeSamples,
    corpus: recording.corpus,
    keywords: recording.keywords,
    commentVocabulary: recording.commentVocabulary,
    fixtureWords: recording.fixtureWords,
    fixtureTexts: fixtureTexts === undefined ? null : { files: fixtureTexts.files, inline: fixtureTexts.inline.map(({ text }) => textDigest(text)) },
    absent: recording.absent,
    parsers: recording.parsers,
  };
  const files = new Map<string, Buffer>(TOOL_FIELDS.map((field) => [toolPath(config, field), json(values[field])]));
  for (const [id, set] of Object.entries(recording.nameSets)) files.set(nameSetPath(config, id), json(set));
  for (const { text } of fixtureTexts?.inline ?? []) {
    const bytes = json(text);
    // A text is written as JSON, and must read back as the same text.
    if (JSON.parse(bytes.toString("utf8")) !== text) throw new Error(`an inline fixture text does not survive JSON (sha256 ${sha256(bytes).slice(0, 12)})`);
    files.set(textPath(sha256(bytes)), bytes);
  }
  return files;
}

/** Every file of a recording's tests part at one config: config, tool facts, texts and every rule. */
export function recordingFiles(config: Record<string, unknown>, id: string, recording: GatesRecording, rules: string): RuleFiles {
  const files = new Map<string, Buffer>([[configPath(id), json(config)], ...toolFiles(id, recording)]);
  let deduplicated = 0;
  for (const [name, rule] of Object.entries(recording.rules)) {
    const written = ruleFiles(id, recording.adapter, rules, name, rule);
    for (const [path, bytes] of written.files) files.set(path, bytes);
    deduplicated += written.deduplicated;
  }
  return { files, deduplicated };
}

/** The rules a tree records at a config: those with a rule.json, by the name it gives. */
export function recordedRuleNames(tree: RecordingTree, config: string): string[] {
  return tree.list(`rules/${config}/`)
    .filter((path) => /^rules\/[0-9a-f]{64}\/[^/]+\/rule\.json$/u.test(path))
    .map((path) => parseJson(ruleFileSchema, tree.text(path)!, path).name)
    .sort(byText);
}

export type RecordedTool = Omit<GatesRecording, "rules">;

/** The whole-tool facts at a config, with the name sets and texts they name; undefined when a file is missing. */
export function loadTool(tree: RecordingTree, config: string): RecordedTool | undefined {
  const values = new Map<ToolField, unknown>();
  for (const field of TOOL_FIELDS) {
    const path = toolPath(config, field);
    const text = tree.text(path);
    if (text === undefined) return undefined;
    values.set(field, parseJson(TOOL_SCHEMAS[field], text, path));
  }
  const get = <T>(field: ToolField): T => values.get(field) as T;
  const recordedTexts = get<z.infer<typeof fixtureTextsFileSchema>>("fixtureTexts");
  let fixtureTexts: GatesRecording["fixtureTexts"];
  if (recordedTexts !== null) {
    const inline: { text: string }[] = [];
    for (const digest of recordedTexts.inline) {
      const text = tree.text(textPath(digest));
      if (text === undefined) return undefined;
      if (sha256(text) !== digest) throw new Error(`${textPath(digest)} is not the text its name says`);
      inline.push({ text: parseJson(z.string(), text, textPath(digest)) });
    }
    fixtureTexts = { files: recordedTexts.files, inline };
  }
  const nameSets: GatesRecording["nameSets"] = {};
  const prefix = `tool/${config}/name-sets/`;
  for (const path of tree.list(prefix)) {
    const id = path.slice(prefix.length).replace(/\.json$/u, "").replace("~", ":");
    nameSets[id] = parseJson(nameSetSchema, tree.text(path)!, path);
  }
  const license = get<{ sha256: string; text?: string }>("license");
  return {
    tool: get("tool"),
    adapter: get("adapter"),
    pins: get("pins"),
    runtimes: get("runtimes"),
    identity: get("identity"),
    packChecks: get("packChecks"),
    licenseSha256: license.sha256,
    ...(license.text === undefined ? {} : { licenseText: license.text }),
    inScopeSamples: get("inScopeSamples"),
    corpus: get("corpus"),
    keywords: get("keywords"),
    commentVocabulary: get("commentVocabulary"),
    fixtureWords: get("fixtureWords"),
    ...(fixtureTexts === undefined ? {} : { fixtureTexts }),
    nameSets,
    absent: get("absent"),
    parsers: get("parsers"),
  };
}

/**
 * One rule at a config, its answers read in: undefined when it has no rule.json or lacks an answer its index names,
 * which the recorder then asks upstream for.
 */
export function loadRule(tree: RecordingTree, config: string, adapter: GatesManifest["adapter"], name: string): RecordedRule | undefined {
  const path = `${ruleDirectory(config, name)}rule.json`;
  const text = tree.text(path);
  if (text === undefined) return undefined;
  const file = parseJson(ruleFileSchema, text, path);
  if (file.name !== name) throw new Error(`${path} names the rule ${file.name}`);
  const cases: RecordedCase[] = [];
  for (const entry of file.cases) {
    const { key, skipped, expected, ...identity } = entry;
    if (key === undefined) {
      cases.push({ ...identity, skipped: skipped!, expected: expected! });
      continue;
    }
    const answerPath = caseAnswerPath(config, name, key);
    const answerText = tree.text(answerPath);
    if (answerText === undefined) return undefined;
    const answer = parseJson(caseAnswerSchema, answerText, answerPath);
    cases.push({ ...identity, skipped: answer.skipped, expected: answer.expected, sha256: key.slice(0, 64) });
  }
  // ESLint's end parts are compared only where every finding of the rule on its own tests has them.
  const spanParts = file.spanParts ?? eslintSpanParts(cases.every((testCase) => testCase.expected.every((span) => span[2] !== null && span[3] !== null)));
  if (file.spanParts === undefined && adapter !== "eslint") throw new Error(`${path} has no spanParts, which only an ESLint rule's answers say`);
  return { facts: file.facts, names: file.names, spanParts, cases };
}

/**
 * The recording a tree holds at a config, as the gates read it. Every file it names must be there: a tree the recorder
 * wrote is complete, so a missing one is a broken archive, and nothing is judged against part of a recording.
 */
export function loadRecording(tree: RecordingTree, config: string, label: string): GatesRecording {
  const tool = loadTool(tree, config);
  if (tool === undefined) throw new Error(`${label} has no complete tool facts at config ${config.slice(0, 12)}: record the tool-pack again`);
  const rules: Record<string, RecordedRule> = {};
  for (const name of recordedRuleNames(tree, config)) {
    const rule = loadRule(tree, config, tool.adapter, name);
    if (rule === undefined) throw new Error(`${label} lacks an answer the case index of ${name} names: record the tool-pack again`);
    rules[name] = rule;
  }
  const parsed = gatesRecordingSchema.safeParse({ ...tool, rules });
  if (!parsed.success) throw new Error(`${label}: ${parsed.error.issues.slice(0, 5).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  return parsed.data;
}

/**
 * The digest of what one rule's verdict reads of a tree at a config: the config, the whole-tool facts (not the name
 * sets other rules name), the name sets the rule names, its own directory (rule.json and its answers), its answers on
 * the corpus, and `extra`, the verdict files of its case sources and goldens, which the caller names. A file the tree
 * lacks counts as missing, so adding it changes the digest too. Adding another rule changes none of it, except for
 * an adapter whose answers are keyed by the recorded rules, where the rule's own answer files move.
 */
export function ruleTreeDigest(tree: RecordingTree, config: string, recording: GatesRecording, rule: string, extra: readonly string[]): string {
  const sets = recording.rules[rule] === undefined ? [] : [...recording.rules[rule].names.implementation.sets, ...recording.rules[rule].names.shared.sets];
  const paths = [
    configPath(config),
    ...TOOL_FIELDS.map((field) => toolPath(config, field)),
    ...sets.map((id) => nameSetPath(config, id)),
    ...tree.list(ruleDirectory(config, rule)),
    ...tree.list(`answers/${config}/${rule.replaceAll("/", "~")}/`),
    ...extra,
  ];
  return sha256([...new Set(paths)].sort(byText).map((path) => {
    const text = tree.text(path);
    return `${path}\0${text === undefined ? "missing" : sha256(text)}\n`;
  }).join(""));
}
