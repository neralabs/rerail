/**
 * Upstream's answers on the golden corpus files, and independent parsers' verdicts on every source the gates read, in
 * a tool-pack's recording archive (archive.ts says how the archive is named, packed and cached):
 *
 *   answers/<config-id>/<rule>/<task>/<path>@<key>.json       upstream's answer for one rule on one corpus file
 *   verdicts/<parsers-id>/<sha256><type>[.script].json        independent parsers' verdicts on that content
 *
 * It replaced corpus.json, one file of every answer pinned to the corpus pins (corpus-tasks.json's sha256 and every
 * pinned corpus's paths): any change to them made every answer stale and forced a full re-record, though each answer
 * was already keyed by the file's content. An answer is valid exactly while its name is, so the recorder asks upstream
 * only about the names a tree lacks (`port-readiness:record -- --corpus`), and the gates check by existence:
 *
 * - `<rule>` is the rule as the recording names it with `/` written `~`; `<task>` the golden's task or rerail corpus
 *   with `:` written `~`; `<path>` the corpus-relative path as it is.
 * - `<key>` is the file's sha256, plus, where upstream's answer depends on other files, `+` and 12 hex digits of what
 *   it depends on: for Ruff the context files (CONTEXT_FILES) the corpus holds, as the sha256 of their sorted
 *   `<path>\0<sha256>\n` lines; for PHPStan and Psalm (BATCHED_ADAPTERS), which analyse the files they are handed as
 *   one project, the same digest of every corpus file of the task the goldens name, the batch the recorder hands them;
 *   for Clippy, which builds whole crates, the corpus's commit. Where the answer for one rule depends on the other
 *   rules asked with it (ASKS_EVERY_RULE: stillat/blade-parser and PHPStan), the digest is also of the sorted rules the
 *   tool-pack records, so a rule added or removed asks every answer again. Anything else: none.
 * - `<parsers-id>` is the sha256 of the recorded parser list (the tool's `parsers`, canonical JSON), of the adapter
 *   code's digest, which runs the parsers, and of the recorder code's (archive.ts's RECORDER_SOURCES), which files
 *   their verdicts. A verdict is named by the source's sha256, its file type (engine.ts's
 *   fileTypeOf) and whether it is read as an ECMAScript script: the rest of the path never decides a verdict, so a case
 *   source and a corpus file with the same text share one.
 *
 * An answer file is `{"findings": [[startLine, startCol, endLine, endCol], ...]}` (spans sorted; `[]` is upstream
 * asked and silent) or `{"excluded": "<reason>"}` (upstream does not lint it); a verdict file is the verdict list, `[]`
 * when every parser was silent. The corpus pins name none of it: a golden whose file changed has a new key, and an
 * answer no golden names any more stays until `--prune`.
 */
import { z } from "zod";

import { sha256 } from "../../../scripts/s3/test-data-archive.mjs";
import { rerailCorpusId, type PinnedTask, type RerailCorpus } from "../corpus.js";
import { fileTypeOf } from "../engine.js";
import type { GatesManifest } from "../manifest.js";
import type { ParserVerdict, Span } from "../types.js";
import { answerConfig, configId, parsersId, type RecordingTree, type RuntimeDigests } from "./archive.js";
import { canonicalText, cleanVerdicts, compareSpans, isCleanVerdict, parserVerdictSchema, recordedSpanSchema, spanTuple, type RecordedParser } from "./schema.js";

/** How many hex digits of a context digest or corpus commit a key carries. */
const CONTEXT_DIGITS = 12;

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** A path's directories, innermost first: `a/b/c.py` gives `a/b` and `a`. */
function directoriesOf(path: string): string[] {
  const parts = path.split("/").slice(0, -1);
  return parts.map((_, index) => parts.slice(0, parts.length - index).join("/"));
}

/**
 * The corpus files whose presence changes upstream's answer on a file, by adapter. Ruff resolves a file's package, and
 * so its module name (N999), from the `__init__.py` files in the directories above it; the whole corpus held them, so
 * the recorder fetches them and hands them to upstream beside the goldens' files, records answers for the goldens'
 * files alone, and keys each answer by the ones present. Bandit, ESLint, PHP_CodeSniffer, tlint, stillat/blade-parser
 * and SQLFluff read each file on its own; so does Bladestan, which renders each template alone, with only templates
 * under its task's directory to find by view name, which no corpus template names. PHPStan and Psalm analyse what they
 * are handed as one project (BATCHED_ADAPTERS). Clippy lints whole crates, so its adapter checks out each pinned corpus
 * whole for cargo, and keys an answer by the corpus's commit instead.
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

/** One corpus file an answer is about: where it is, and its content's sha256. */
export interface CorpusPlace {
  task: string;
  path: string;
  sha256: string;
}

/**
 * Whether an adapter's answer on a file depends on every other file upstream is handed with it: PHPStan and Psalm each
 * analyse the files as one project, where a symbol is found only among them, and PHPStan leaves a file it cannot
 * analyse out of the project at every level the asked rules need. The recorder hands such an adapter every fetched
 * file of a task at once, for every recorded rule, and keys each answer by that batch (KeyContext.batchOf).
 */
export const BATCHED_ADAPTERS: Record<GatesManifest["adapter"], boolean> = {
  ruff: false,
  bandit: false,
  eslint: false,
  phpcs: false,
  tlint: false,
  "blade-parser": false,
  phpstan: true,
  psalm: true,
  bladestan: false,
  sqlfluff: false,
  clippy: false,
};

/**
 * Whether an adapter's answer for one rule on a file depends on the other rules asked with it, so that the recorder
 * asks it every recorded rule about any file it asks about, and the answer is keyed by the rules it records
 * (KeyContext.rules): stillat/blade-parser leaves a file out for every validator asked when one of them throws on it,
 * and PHPStan leaves a file out at every level the asked rules need when one level cannot analyse it (and is
 * BATCHED_ADAPTERS besides). The others answer a rule whatever else is asked: PHP_CodeSniffer and tlint run every
 * sniff and linter the pack names, and the rest exclude a file only when upstream's parser rejects it.
 */
export const ASKS_EVERY_RULE: Record<GatesManifest["adapter"], boolean> = {
  bandit: false,
  ruff: false,
  eslint: false,
  phpcs: false,
  tlint: false,
  "blade-parser": true,
  phpstan: true,
  psalm: false,
  bladestan: false,
  sqlfluff: false,
  clippy: false,
};

/** What an answer's key reads beyond the file: the other corpus files beside it, the corpus's commit, and the recorded rules. */
export interface KeyContext {
  /** sha256 of the corpus file at `task`/`path`, or undefined when the corpus holds none there. */
  sha256Of(task: string, path: string): string | undefined;
  /** The commit a task's or rerail corpus's files are read at. */
  commitOf(task: string): string;
  /** The digest of every fetched corpus file of the task, as places: the batch a BATCHED_ADAPTERS adapter is handed. */
  batchOf(task: string): string;
  /** The sha256 of the sorted rules the tool-pack records, one per line: what an ASKS_EVERY_RULE answer is asked with. */
  rules: string;
}

/** The sha256 of `<path>\0<sha256>\n` lines of corpus files, sorted: the same files name the same digest. */
function placesDigest(files: Iterable<{ path: string; sha256: string }>): string {
  return sha256([...files].map((file) => `${file.path}\0${file.sha256}\n`).sort(byText).join(""));
}

/**
 * The key context over the corpus files a run fetched (the goldens' files and the context files beside them), which
 * the gates and the recorder fetch alike, and the rules the tool-pack records: what the keys read of them, and the
 * commits.
 */
export function fetchedContext(files: readonly CorpusPlace[], commitOf: KeyContext["commitOf"], rules: readonly string[]): KeyContext {
  const byPlace = new Map(files.map((file) => [`${file.task}\0${file.path}`, file]));
  const byTask = new Map<string, CorpusPlace[]>();
  for (const file of byPlace.values()) byTask.set(file.task, [...(byTask.get(file.task) ?? []), file]);
  const batches = new Map([...byTask].map(([task, placed]) => [task, placesDigest(placed)]));
  return {
    sha256Of: (task, path) => byPlace.get(`${task}\0${path}`)?.sha256,
    commitOf,
    batchOf: (task) => batches.get(task) ?? placesDigest([]),
    rules: rulesDigest(rules),
  };
}

/** The commits files are read at: a rerail corpus's pinned commit, a catalog task's base commit. */
export function corpusCommits(corpora: readonly RerailCorpus[], tasks: readonly PinnedTask[]): KeyContext["commitOf"] {
  const commits = new Map<string, string>([...corpora.map((corpus) => [rerailCorpusId(corpus), corpus.commit] as const), ...tasks.map((task) => [task.task, task.baseSha] as const)]);
  return (task) => {
    const commit = commits.get(task);
    if (commit === undefined) throw new Error(`${task} is neither a pinned catalog task nor a corpus the pack pins`);
    return commit;
  };
}

/** The key an answer on `file` is filed under (the module comment says what each adapter's depends on). */
export function answerKey(adapter: GatesManifest["adapter"], file: CorpusPlace, context: KeyContext): string {
  if (adapter === "clippy") return `${file.sha256}+${context.commitOf(file.task).slice(0, CONTEXT_DIGITS)}`;
  if (BATCHED_ADAPTERS[adapter]) {
    const batch = context.batchOf(file.task);
    return `${file.sha256}+${(ASKS_EVERY_RULE[adapter] ? sha256(`${batch}\0${context.rules}`) : batch).slice(0, CONTEXT_DIGITS)}`;
  }
  if (ASKS_EVERY_RULE[adapter]) return `${file.sha256}+${context.rules.slice(0, CONTEXT_DIGITS)}`;
  if (adapter !== "ruff") return file.sha256;
  const present = CONTEXT_FILES.ruff(file.path).flatMap((path) => {
    const digest = context.sha256Of(file.task, path);
    return digest === undefined ? [] : [{ path, sha256: digest }];
  });
  return `${file.sha256}+${placesDigest(present).slice(0, CONTEXT_DIGITS)}`;
}

/** The sha256 of the sorted rules, one per line: what an answer asked with every recorded rule is keyed by. */
export function rulesDigest(rules: readonly string[]): string {
  return sha256([...new Set(rules)].sort(byText).map((rule) => `${rule}\n`).join(""));
}

export function answerPath(config: string, rule: string, file: Pick<CorpusPlace, "task" | "path">, key: string): string {
  return `answers/${config}/${rule.replaceAll("/", "~")}/${file.task.replaceAll(":", "~")}/${file.path}@${key}.json`;
}

export function verdictPath(parsers: string, file: { path: string; sha256: string; script: boolean }): string {
  return `verdicts/${parsers}/${file.sha256}${fileTypeOf(file.path)}${file.script ? ".script" : ""}.json`;
}

const answerSchema = z.union([z.strictObject({ findings: z.array(recordedSpanSchema) }), z.strictObject({ excluded: z.string().min(1) })]);
export type Answer = z.infer<typeof answerSchema>;

/** An answer file's text: the excluded reason, or the findings sorted by position. */
export function answerText(answer: { excluded: string | null; findings: readonly Span[] }): string {
  return canonicalText(answer.excluded !== null ? { excluded: answer.excluded } : { findings: answer.findings.map(spanTuple).sort(compareSpans) });
}

/** A verdict file's text: the verdict list, `[]` when every parser was silent. */
export function verdictText(verdicts: readonly ParserVerdict[]): string {
  return canonicalText(verdicts.every(isCleanVerdict) ? [] : verdicts);
}

/** A tree file's value: valid under `schema`, and written in canonical form (one way to record a given answer). */
export function parseJson<T>(schema: z.ZodType<T>, text: string, label: string): T {
  const parsed = schema.safeParse(JSON.parse(text));
  if (!parsed.success) throw new Error(`${label}: ${parsed.error.issues.slice(0, 3).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  if (canonicalText(parsed.data) !== text) throw new Error(`${label} is not in canonical form`);
  return parsed.data;
}

export function parseAnswer(text: string, label: string): Answer {
  const answer = parseJson(answerSchema, text, label);
  if ("findings" in answer && !answer.findings.every((span, index) => index === 0 || compareSpans(answer.findings[index - 1]!, span) <= 0)) throw new Error(`${label}: findings are not sorted`);
  return answer;
}

/** A verdict file's list, `[]` read as every one of `parsers` silent. */
export function parseVerdicts(text: string, label: string, parsers: readonly RecordedParser[]): ParserVerdict[] {
  const verdicts = parseJson(z.array(parserVerdictSchema), text, label);
  if (verdicts.length > 0 && verdicts.every(isCleanVerdict)) throw new Error(`${label}: every parser is silent, which is written []`);
  return verdicts.length === 0 ? cleanVerdicts(parsers) : verdicts;
}

/** An answer or verdict a tool-pack needs, and the tree path it is filed under. */
export interface NeededAnswer {
  rule: string;
  file: CorpusPlace;
  path: string;
}
export interface NeededVerdict {
  file: CorpusPlace;
  path: string;
}

/** Every file a tree must hold for the goldens' corpus files at one config: what the gates read, and the recorder writes. */
export interface NeededAnswers {
  config: Record<string, unknown>;
  configId: string;
  parsersId: string;
  /** By rule, then task, then path. */
  answers: NeededAnswer[];
  verdicts: NeededVerdict[];
}

/**
 * The answers the goldens need: every golden corpus file (each place and content once) for every rule the tool-pack
 * records, at this config, and the parsers' verdicts on each file's content.
 */
export function neededAnswers(options: {
  /** The checkout whose pack's UPSTREAM.md files the config reads (answerConfig). */
  root: string;
  manifest: GatesManifest;
  runtimes: RuntimeDigests;
  parsers: readonly RecordedParser[];
  rules: readonly string[];
  files: readonly CorpusPlace[];
  context: KeyContext;
}): NeededAnswers {
  const config = answerConfig(options.root, options.manifest, options.runtimes);
  const id = configId(config);
  const parsers = parsersId(options.manifest.adapter, options.parsers);
  const files = [...new Map(options.files.map((file) => [`${file.task}\0${file.path}\0${file.sha256}`, file])).values()]
    .sort((a, b) => byText(a.task, b.task) || byText(a.path, b.path) || byText(a.sha256, b.sha256));
  const keyed = files.map((file) => ({ file, key: answerKey(options.manifest.adapter, file, options.context) }));
  const answers = [...new Set(options.rules)].sort(byText).flatMap((rule) => keyed.map(({ file, key }) => ({ rule, file, path: answerPath(id, rule, file, key) })));
  const verdicts = [...new Map(files.map((file) => {
    const path = verdictPath(parsers, { ...file, script: false });
    return [path, { file, path }] as const;
  })).values()];
  return { config, configId: id, parsersId: parsers, answers, verdicts };
}

/** The needed files a tree lacks: an answer exists exactly when its file does. */
export function missingAnswers(tree: RecordingTree, needed: NeededAnswers): { answers: NeededAnswer[]; verdicts: NeededVerdict[] } {
  return {
    answers: needed.answers.filter((answer) => tree.text(answer.path) === undefined),
    verdicts: needed.verdicts.filter((verdict) => tree.text(verdict.path) === undefined),
  };
}
