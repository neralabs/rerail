import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { readdir, readFile } from "node:fs/promises";
import { join, matchesGlob, relative, sep } from "node:path";
import { z } from "zod";
import type { CodeRuleSourceLanguage } from "../../src/engine/index.js";
import {
  IGNORED_DIRECTORIES,
  languageFor,
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  nativePlatformSuffix,
  type NativeCompiledCodeRuleDsl,
  CODE_RULE_AST_LANGUAGES,
  CODE_RULE_DIALECTS,
  codeRuleLanguageFamily,
  compareDeterministicStrings,
} from "../../src/engine/index.js";
import { capabilitiesJson, INFORMATIONAL_FAILURE_CODES } from "../engine.js";
import {
  corpusRecordingPath,
  defaultCorpusRoot,
  fixturesDirectory,
  fixturesRecordingPath,
  loadRecording,
  coversFixture,
  documentPath,
  isCandidate,
  MINIMUM_RULES,
  RERAIL_ROOT,
  type CorpusRecording,
  type Divergence,
  type FixtureRecording,
  type LoadedPack,
  type PackCorpus,
  type PackRule,
  type Recording,
} from "./pack.js";
import { EXAMPLE_LIMIT, REPORT_FORMAT, type RuleRow, type SourceSummary, type SyntaxErrorLocation, type T4Report } from "./report.js";
import { precisionRecall, scoreSource, type Point } from "./score.js";

/** The production repository audit's batching and paging (`code-rule-enforcer.ts`). */
const AUDIT_BATCH_FILES = 32;
const AUDIT_CHUNK_BYTES = 1024 * 1024;
const RESULT_PAGE_BYTES = 8 * 1024 * 1024;

interface SourceFile {
  path: string;
  language: CodeRuleSourceLanguage;
  source: string;
}

const pageSchema = z.object({
  cursor: z.number().int().nonnegative(),
  done: z.boolean(),
  violations: z.array(
    z.object({
      ruleId: z.string(),
      code: z.string(),
      path: z.string().optional(),
      span: z.object({ startByte: z.number().int().nonnegative(), startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }).optional(),
    }),
  ),
  failures: z.array(z.object({ ruleId: z.string().optional(), code: z.string(), message: z.string(), path: z.string().optional() })),
});
type Page = z.infer<typeof pageSchema>;

interface Evaluation {
  violations: Page["violations"];
  failures: Page["failures"];
  ms: number;
}

/**
 * The repository audit's path through the production workspace: every code rule added to one workspace, sources applied
 * in 32-file / 1 MiB batches with global relations deferred, one closing apply, then results paged out.
 */
async function evaluateWorkspace(codeRules: readonly NativeCompiledCodeRuleDsl[], files: readonly SourceFile[]): Promise<Evaluation> {
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  const started = performance.now();
  try {
    for (const codeRule of codeRules) workspace.addCodeRule(codeRule);
    let revision = 0;
    let batch: SourceFile[] = [];
    let bytes = 0;
    const flush = async (): Promise<void> => {
      revision += 1;
      await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision, files: batch, removed: [], deferGlobal: true, quiet: true }));
      batch = [];
      bytes = 0;
    };
    for (const file of files) {
      batch.push(file);
      bytes += Buffer.byteLength(file.source);
      if (batch.length >= AUDIT_BATCH_FILES || bytes >= AUDIT_CHUNK_BYTES) await flush();
    }
    if (batch.length > 0) await flush();
    revision += 1;
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision, files: [], removed: [], quiet: true }));
    const violations: Page["violations"] = [];
    const failures: Page["failures"] = [];
    for (let cursor = 0; ; ) {
      const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
      violations.push(...page.violations);
      failures.push(...page.failures);
      if (page.done) break;
      if (page.cursor <= cursor) throw new Error("native result stream did not advance");
      cursor = page.cursor;
    }
    return { violations, failures, ms: performance.now() - started };
  } finally {
    workspace.close();
  }
}

/**
 * How a clause that is unsound under syntax errors (an error region could hide a binding it reads) skips a file the
 * grammar reads with errors. Those errors are T1's to classify, so such a failure is attributed to T1, not counted
 * as an engine failure, once the probe below confirms the grammar reports a syntax error in that file.
 */
const SYNTAX_ERROR_FAILURE = { code: "source-incomplete", message: "file has syntax errors" } as const;
/** The engine's `source-partial` message names the first error region (`first at line L, column C`, both 1-based). */
const FIRST_SYNTAX_ERROR = /first at line (\d+), column (\d+)$/u;
/**
 * How every AST clause skips a file whose error regions cover half of it or more: the grammar did not read it, which is
 * T1's too. The message names the first region, so no probe is needed.
 */
const UNREAD_FAILURE = /^file did not parse: error regions cover \d+% of it, first at line (\d+), column (\d+)$/u;

function unreadLocation(failure: Page["failures"][number]): SyntaxErrorLocation | undefined {
  const match = failure.code === "source-incomplete" && failure.path !== undefined ? UNREAD_FAILURE.exec(failure.message) : null;
  return match === null ? undefined : { path: failure.path!, line: Number(match[1]), column: Number(match[2]) };
}

/**
 * The first syntax error the grammar reports in each of `files` that skipped a clause with `SYNTAX_ERROR_FAILURE`.
 * A probe clause sound under errors (it matches `ERROR` nodes, which are never witnesses, so it reports nothing) is
 * evaluated over those files through the same workspace path; its `source-partial` failure confirms the syntax error
 * and locates it.
 */
async function syntaxErrorLocations(loaded: LoadedPack, evaluation: Evaluation, files: readonly SourceFile[]): Promise<Map<string, SyntaxErrorLocation>> {
  const failed = new Set(
    evaluation.failures.flatMap(failure =>
      failure.code === SYNTAX_ERROR_FAILURE.code && failure.message === SYNTAX_ERROR_FAILURE.message && failure.path !== undefined ? [failure.path] : [],
    ),
  );
  const locations = new Map<string, SyntaxErrorLocation>();
  if (failed.size === 0) return locations;
  const runtime = loadNativeBinding().CodeRuleDslRuntime;
  const probes = CODE_RULE_AST_LANGUAGES.filter(language => codeRuleLanguageFamily(language) === loaded.pack.family).map(language =>
    runtime.compile(
      JSON.stringify({
        format: "rerail.policy/v1",
        id: `conformance-syntax-probe:${language}`,
        revision: 1,
        mode: "log",
        description: "Locates the grammar's syntax errors; reports nothing.",
        scope: { include: ["**/*"], exclude: [] },
        clauses: [{ id: "probe", forbid: { ast: { language, rule: { kind: "ERROR" } } }, violation: { anchor: "witness.span", code: "SYNTAX-PROBE", message: "syntax probe" } }],
      }),
      "json",
      capabilitiesJson(),
    ),
  );
  const probed = await evaluateWorkspace(probes, files.filter(file => failed.has(file.path)));
  for (const failure of probed.failures) {
    const match = failure.code === "source-partial" && failure.path !== undefined ? FIRST_SYNTAX_ERROR.exec(failure.message) : null;
    if (match !== null && !locations.has(failure.path!)) locations.set(failure.path!, { path: failure.path!, line: Number(match[1]), column: Number(match[2]) });
  }
  return locations;
}

interface CompiledRule {
  rule: PackRule;
  /** The code rule document's `id`, which every violation and failure of the rule carries as `ruleId`. */
  policyId: string;
  compiled: NativeCompiledCodeRuleDsl;
}

const codeRuleDocumentSchema = z.object({
  id: z.string(),
  clauses: z.array(z.object({ id: z.string(), violation: z.object({ code: z.string() }).optional() })).min(1),
});

/** Compiles each rule's code rule; a code rule whose clauses report another code than the upstream rule is an error. */
function compileRules(loaded: LoadedPack, rules: readonly PackRule[], errors: string[]): CompiledRule[] {
  const runtime = loadNativeBinding().CodeRuleDslRuntime;
  const seen = new Map<string, string>();
  return rules.flatMap(rule => {
    const path = join(loaded.directory, rule.policy);
    try {
      const text = readFileSync(path, "utf8");
      const document = codeRuleDocumentSchema.parse(JSON.parse(text));
      const other = seen.get(document.id);
      if (other !== undefined) throw new Error(`policy id ${document.id} is also ${other}'s`);
      seen.set(document.id, rule.id);
      for (const clause of document.clauses) {
        if (clause.violation !== undefined && clause.violation.code !== rule.upstream.rule)
          throw new Error(`clause ${clause.id} reports code ${clause.violation.code}, not the upstream rule ${rule.upstream.rule}`);
      }
      return [{ rule, policyId: document.id, compiled: runtime.compile(text, "json", capabilitiesJson()) }];
    } catch (error) {
      errors.push(`${rule.id}: ${relative(RERAIL_ROOT, path)}: ${error instanceof Error ? error.message : String(error)}`);
      return [];
    }
  });
}

/** Byte offset → upstream coordinates: the engine's line (the family's linter's terminators), code points from line start. */
function pointOf(violation: Page["violations"][number], sources: ReadonlyMap<string, Buffer>): Point {
  if (violation.path === undefined || violation.span === undefined) throw new Error(`${violation.ruleId}: a finding without a path or span cannot be scored`);
  const source = sources.get(violation.path);
  if (source === undefined) throw new Error(`${violation.ruleId}: finding in ${violation.path}, which was not evaluated`);
  const lineStart = violation.span.startByte - violation.span.startColumn;
  const column = [...source.subarray(lineStart, violation.span.startByte).toString("utf8")].length + 1;
  return { path: violation.path, line: violation.span.startLine + 1, column };
}

/** What Rerail reported for one pack rule over one source. */
interface RuleActual {
  points: Point[];
  /** Failures other than `source-partial` and the syntax-error skips below. */
  engineFailures: number;
  partialPaths: Set<string>;
  /** Files this rule's clauses skipped with `SYNTAX_ERROR_FAILURE` or as unread (`UNREAD_FAILURE`), at the first syntax error. */
  syntaxErrorFiles: Map<string, SyntaxErrorLocation>;
}

interface SourceRun {
  /** `fixtures` or the corpus name, as divergences name it. */
  name: string;
  /** `fixtures` or `corpus:<name>`, as report rows name it. */
  label: string;
  /** Per pack rule id. */
  actual: Map<string, RuleActual>;
  /** Per tool: its recording, or the reason there is none. */
  recordings: Map<string, Recording | string>;
  /** Paths evaluated; upstream findings elsewhere are out of scope. */
  scored: ReadonlySet<string>;
  summary: SourceSummary;
}

/** Collects each rule's violations and failures; `syntaxErrors` holds the probe-confirmed syntax-error files. */
function collectActual(
  compiled: readonly CompiledRule[],
  evaluation: Evaluation,
  sources: ReadonlyMap<string, Buffer>,
  syntaxErrors: ReadonlyMap<string, SyntaxErrorLocation>,
  actual: SourceRun["actual"],
): void {
  const byCodeRule = new Map(compiled.map(entry => [entry.policyId, entry.rule.id]));
  const slot = (rule: string): RuleActual => {
    let entry = actual.get(rule);
    if (entry === undefined) actual.set(rule, (entry = { points: [], engineFailures: 0, partialPaths: new Set(), syntaxErrorFiles: new Map() }));
    return entry;
  };
  for (const entry of compiled) slot(entry.rule.id);
  for (const violation of evaluation.violations) {
    const rule = byCodeRule.get(violation.ruleId);
    if (rule !== undefined) slot(rule).points.push(pointOf(violation, sources));
  }
  for (const failure of evaluation.failures) {
    const rule = failure.ruleId === undefined ? undefined : byCodeRule.get(failure.ruleId);
    if (rule === undefined) continue;
    const located = failure.path === undefined ? undefined : syntaxErrors.get(failure.path);
    const unread = unreadLocation(failure);
    if (INFORMATIONAL_FAILURE_CODES.has(failure.code)) {
      if (failure.path !== undefined) slot(rule).partialPaths.add(failure.path);
    } else if (unread !== undefined) {
      slot(rule).syntaxErrorFiles.set(unread.path, unread);
    } else if (located !== undefined && failure.code === SYNTAX_ERROR_FAILURE.code && failure.message === SYNTAX_ERROR_FAILURE.message) {
      slot(rule).syntaxErrorFiles.set(located.path, located);
    } else slot(rule).engineFailures += 1;
  }
}

function familyLanguages(loaded: LoadedPack): Set<string> {
  return new Set(Object.values(CODE_RULE_DIALECTS).filter(dialect => dialect.family === loaded.pack.family).map(dialect => dialect.label));
}

/** The recording at `path`, checked against the pack's pins, or why it cannot be used. */
function pinnedRecording(loaded: LoadedPack, tool: string, path: string, corpus: PackCorpus | undefined): Recording | string {
  const where = relative(RERAIL_ROOT, path);
  if (!existsSync(path)) return `no recording at ${where}`;
  let recording: Recording;
  try {
    recording = loadRecording(path);
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  const version = loaded.pack.tools.find(entry => entry.name === tool)?.version;
  if (recording.tool !== tool) return `${where} records ${recording.tool}, not ${tool}`;
  if (recording.version !== version) return `${where} records ${tool} ${recording.version}; the pack pins ${version}`;
  if (corpus === undefined && recording.source.kind !== "fixtures") return `${where} is not a fixtures recording`;
  if (corpus !== undefined) {
    if (recording.source.kind !== "corpus" || recording.source.name !== corpus.name) return `${where} is not a recording of corpus ${corpus.name}`;
    if (recording.source.commit !== corpus.commit) return `${where} records ${corpus.name} at ${recording.source.commit}; the pack pins ${corpus.commit}`;
  }
  return recording;
}

/**
 * Shipped ports and candidates, each evaluated in a workspace of their own: a file's AST work budget is shared by every
 * code rule in a workspace, so candidates evaluated beside the shipped ports would spend the shipped ports' budget and
 * change their verdicts, which a candidate must never do. Production never runs candidates beside shipped rules either.
 */
async function evaluateApart(rules: readonly CompiledRule[], files: readonly SourceFile[]): Promise<[CompiledRule[], Evaluation][]> {
  const groups = [rules.filter(entry => !isCandidate(entry.rule)), rules.filter(entry => isCandidate(entry.rule))].filter(group => group.length > 0);
  const evaluations: [CompiledRule[], Evaluation][] = [];
  for (const group of groups) evaluations.push([group, await evaluateWorkspace(group.map(entry => entry.compiled), files)]);
  return evaluations;
}

async function runFixtures(loaded: LoadedPack, compiled: readonly CompiledRule[], errors: string[]): Promise<SourceRun> {
  const languages = familyLanguages(loaded);
  const actual: SourceRun["actual"] = new Map();
  const recordings: SourceRun["recordings"] = new Map();
  const scored = new Set<string>();
  const summary: SourceSummary = { source: "fixtures", units: 0, files: 0, bytes: 0, skipped: 0, outsideScope: 0, syntaxErrorFiles: { count: 0, files: [] }, evaluationMs: 0 };
  for (const tool of [...new Set(compiled.map(entry => entry.rule.upstream.tool))].sort(compareDeterministicStrings)) {
    const recording = pinnedRecording(loaded, tool, fixturesRecordingPath(loaded, tool), undefined);
    recordings.set(tool, recording);
    if (typeof recording === "string") continue;
    const source = (recording as FixtureRecording).source;
    const rules = compiled.filter(entry => entry.rule.upstream.tool === tool);
    for (const unit of source.files) {
      const paths = typeof unit === "string" ? [unit] : unit;
      const unitRules = rules.filter(entry => paths.some(path => coversFixture(entry.rule, path)));
      if (unitRules.length === 0) continue;
      const files: SourceFile[] = [];
      for (const path of paths) {
        const language = languageFor(path);
        if (language === undefined || !languages.has(language)) {
          errors.push(`fixtures ${tool}: ${path} is not a ${loaded.pack.family} source by the host's routing`);
          continue;
        }
        files.push({ path, language, source: readFileSync(join(fixturesDirectory(loaded, tool), path), "utf8") });
      }
      if (files.length === 0) continue;
      const sources = new Map(files.map(file => [file.path, Buffer.from(file.source, "utf8")]));
      for (const [group, evaluation] of await evaluateApart(unitRules, files)) {
        collectActual(group, evaluation, sources, await syntaxErrorLocations(loaded, evaluation, files), actual);
        summary.evaluationMs += evaluation.ms;
      }
      for (const file of files) scored.add(`${tool}\0${file.path}`);
      summary.units += 1;
      summary.files += files.length;
      summary.bytes += [...sources.values()].reduce((total, bytes) => total + bytes.length, 0);
    }
  }
  summary.evaluationMs = Math.round(summary.evaluationMs);
  summary.syntaxErrorFiles = syntaxErrorSummary(actual);
  return { name: "fixtures", label: "fixtures", actual, recordings, scored, summary };
}

/** Every file of a source some rule skipped for its syntax errors, each once, in path order. */
function syntaxErrorSummary(actual: SourceRun["actual"]): SourceSummary["syntaxErrorFiles"] {
  const files = new Map<string, SyntaxErrorLocation>();
  for (const rule of actual.values()) for (const [path, point] of rule.syntaxErrorFiles) files.set(path, point);
  const sorted = [...files.values()].sort((left, right) => compareDeterministicStrings(left.path, right.path));
  return { count: sorted.length, files: sorted };
}

async function walk(root: string, directory: string, files: string[]): Promise<void> {
  const entries = await readdir(directory, { withFileTypes: true }).catch((error: NodeJS.ErrnoException) => {
    if (error.code === "ENOTDIR") return undefined;
    throw error;
  });
  if (entries === undefined) {
    files.push(relative(root, directory).split(sep).join("/"));
    return;
  }
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!IGNORED_DIRECTORIES.has(entry.name)) await walk(root, path, files);
    } else if (entry.isFile()) files.push(relative(root, path).split(sep).join("/"));
  }
}

async function runCorpus(loaded: LoadedPack, compiled: readonly CompiledRule[], corpus: PackCorpus, corpusRoot: string, errors: string[]): Promise<SourceRun | undefined> {
  const directory = join(corpusRoot, corpus.name);
  if (!existsSync(directory)) {
    errors.push(`corpus ${corpus.name}: no checkout at ${directory} (clone ${corpus.repository} at ${corpus.commit})`);
    return undefined;
  }
  if (existsSync(join(directory, ".git"))) {
    const head = execFileSync("git", ["-C", directory, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    if (head !== corpus.commit) {
      errors.push(`corpus ${corpus.name}: ${directory} is at ${head}; the pack pins ${corpus.commit}`);
      return undefined;
    }
  }
  const recordings: SourceRun["recordings"] = new Map();
  const skipped = new Set<string>();
  for (const tool of [...new Set(compiled.map(entry => entry.rule.upstream.tool))].sort(compareDeterministicStrings)) {
    const path = corpusRecordingPath(loaded, tool, corpus.name);
    // A corpus recording covers the shipped ports: a tool with only candidates may have none, and they skip the corpus.
    if (!existsSync(path) && compiled.every(entry => entry.rule.upstream.tool !== tool || isCandidate(entry.rule))) continue;
    const recording = pinnedRecording(loaded, tool, path, corpus);
    recordings.set(tool, recording);
    if (typeof recording !== "string") for (const path of (recording as CorpusRecording).source.skipped) skipped.add(path);
  }

  const languages = familyLanguages(loaded);
  const candidates: string[] = [];
  for (const path of corpus.paths) await walk(directory, join(directory, path), candidates);
  const files: SourceFile[] = [];
  for (const path of [...new Set(candidates)].sort(compareDeterministicStrings)) {
    const language = languageFor(path);
    if (language === undefined || !languages.has(language) || skipped.has(path)) continue;
    if (corpus.exclude.some(pattern => matchesGlob(path, pattern))) continue;
    files.push({ path, language, source: await readFile(join(directory, path), "utf8") });
  }
  const sources = new Map(files.map(file => [file.path, Buffer.from(file.source, "utf8")]));
  const actual: SourceRun["actual"] = new Map();
  let evaluationMs = 0;
  for (const [group, evaluation] of await evaluateApart(compiled, files)) {
    collectActual(group, evaluation, sources, await syntaxErrorLocations(loaded, evaluation, files), actual);
    evaluationMs += evaluation.ms;
  }
  const scored = new Set<string>();
  for (const tool of recordings.keys()) for (const file of files) scored.add(`${tool}\0${file.path}`);
  return {
    name: corpus.name,
    label: `corpus:${corpus.name}`,
    actual,
    recordings,
    scored,
    summary: {
      source: `corpus:${corpus.name}`,
      units: 1,
      files: files.length,
      bytes: [...sources.values()].reduce((total, bytes) => total + bytes.length, 0),
      skipped: skipped.size,
      outsideScope: 0,
      syntaxErrorFiles: syntaxErrorSummary(actual),
      evaluationMs: Math.round(evaluationMs),
    },
  };
}

export interface RunOptions {
  /** Pack rule ids to score; all when omitted. */
  rules?: readonly string[];
  fixtures: boolean;
  /** Corpus names to score. */
  corpora: readonly string[];
  corpusRoot?: string;
  date: string;
}

function engineInfo(): { path: string; version: string } {
  const path = process.env.RERAIL_ENGINE_PATH ?? join(RERAIL_ROOT, "native", `rerail_engine.${nativePlatformSuffix()}.node`);
  let built = "";
  try {
    built = `, built ${statSync(path).mtime.toISOString()}`;
  } catch {
    built = "";
  }
  return { path: relative(RERAIL_ROOT, path), version: `${loadNativeBinding().health().kernelVersion}${built}` };
}

function rowFor(entry: CompiledRule, run: SourceRun, loaded: LoadedPack, used: Set<Divergence>, errors: string[]): RuleRow | undefined {
  const { rule } = entry;
  const recording = run.recordings.get(rule.upstream.tool);
  if (recording === undefined) return undefined;
  if (typeof recording === "string") {
    errors.push(`${rule.id} (${run.label}): ${recording}`);
    return undefined;
  }
  if (!recording.source.rules.includes(rule.upstream.rule)) {
    // Candidates are scored on the fixtures, and on a corpus only where its recording covers them.
    if (isCandidate(rule) && recording.source.kind === "corpus") return undefined;
    errors.push(`${rule.id} (${run.label}): the ${rule.upstream.tool} recording does not cover ${rule.upstream.rule}`);
    return undefined;
  }
  // Lines the tool did not analyse: its silence there is no verdict, so neither side's findings are scored.
  const unanalysed = recording.source.kind === "corpus" ? recording.source.unanalysed : [];
  const analysed = (point: Point): boolean =>
    !unanalysed.some(region => region.path === point.path && region.startLine <= point.line && point.line <= region.endLine);
  let left = 0;
  const expected: Point[] = [];
  for (const finding of recording.findings) {
    if (finding.rule !== rule.upstream.rule) continue;
    // A rule scoped to part of the fixtures (`fixtures`) is scored only there; other units ran another configuration.
    if (run.name === "fixtures" && !coversFixture(rule, finding.path)) continue;
    if (!run.scored.has(`${rule.upstream.tool}\0${finding.path}`)) {
      // A corpus recording covers the whole checkout; a fixtures recording lists every file it ran on.
      if (run.name === "fixtures") errors.push(`${rule.id} (fixtures): ${finding.path} is not among the recording's files`);
      else run.summary.outsideScope += 1;
      continue;
    }
    if (rule.match === "line+column" && finding.column === undefined) {
      errors.push(`${rule.id} (${run.label}): ${finding.path}:${finding.line} has no column; the rule matches line+column`);
      continue;
    }
    const point = { path: finding.path, line: finding.line, ...(rule.match === "line+column" ? { column: finding.column! } : {}) };
    if (analysed(point)) expected.push(point);
    else left += 1;
  }
  const rerail: RuleActual = run.actual.get(rule.id) ?? { points: [], engineFailures: 0, partialPaths: new Set(), syntaxErrorFiles: new Map() };
  const claimed = rerail.points.map(point => (rule.match === "line" ? { path: point.path, line: point.line } : point));
  const actual = claimed.filter(analysed);
  left += claimed.length - actual.length;
  const applicable = loaded.divergences.filter(divergence => divergence.rule === rule.id && (divergence.source === undefined || divergence.source === run.name));
  const score = scoreSource(expected, actual, rule.match, applicable);
  for (const entry of score.explained) used.add(entry.divergence);
  return {
    rule: rule.id,
    candidate: isCandidate(rule),
    examples: loaded.exampled.has(documentPath(rule.policy)),
    upstream: rule.upstream,
    capabilities: rule.capabilities,
    source: run.label,
    tp: score.tp,
    fp: score.falsePositives.length,
    fn: score.falseNegatives.length,
    ...precisionRecall(score),
    unanalysed: left,
    engineFailures: rerail.engineFailures,
    partialFiles: rerail.partialPaths.size,
    syntaxErrorFiles: {
      count: rerail.syntaxErrorFiles.size,
      files: [...rerail.syntaxErrorFiles.values()].sort((left, right) => compareDeterministicStrings(left.path, right.path)).slice(0, EXAMPLE_LIMIT),
    },
    divergences: score.explained.map(({ side, point, divergence }) => ({ kind: divergence.kind, side, ...point, evidence: divergence.evidence })),
    falsePositives: score.falsePositives.slice(0, EXAMPLE_LIMIT),
    falseNegatives: score.falseNegatives.slice(0, EXAMPLE_LIMIT),
  };
}

/** T4: score the pack's rules over its upstream fixtures and/or pinned corpora. */
export async function runPack(loaded: LoadedPack, options: RunOptions): Promise<T4Report> {
  const { pack } = loaded;
  const errors: string[] = [];
  const unknown = (options.rules ?? []).filter(id => !pack.rules.some(rule => rule.id === id));
  if (unknown.length > 0) throw new Error(`unknown rule(s) in ${pack.pack}: ${unknown.join(", ")}`);
  const unknownCorpora = options.corpora.filter(name => !pack.corpora.some(corpus => corpus.name === name));
  if (unknownCorpora.length > 0) throw new Error(`unknown corpus/corpora in ${pack.pack}: ${unknownCorpora.join(", ")}`);
  const selected = options.rules === undefined ? pack.rules : pack.rules.filter(rule => options.rules!.includes(rule.id));
  const compiled = compileRules(loaded, selected, errors);

  const runs: SourceRun[] = [];
  if (options.fixtures && compiled.length > 0) runs.push(await runFixtures(loaded, compiled, errors));
  const corpusRoot = options.corpusRoot ?? defaultCorpusRoot(pack.pack);
  for (const name of options.corpora) {
    if (compiled.length === 0) break;
    const run = await runCorpus(loaded, compiled, pack.corpora.find(corpus => corpus.name === name)!, corpusRoot, errors);
    if (run !== undefined) runs.push(run);
  }

  const used = new Set<Divergence>();
  const rows = runs.flatMap(run => compiled.flatMap(entry => rowFor(entry, run, loaded, used, errors) ?? []));
  const scoredSources = new Set(runs.map(run => run.name));
  const allSources = ["fixtures", ...pack.corpora.map(corpus => corpus.name)];
  const selectedIds = new Set(selected.map(rule => rule.id));
  const stale = loaded.divergences.filter(
    divergence =>
      selectedIds.has(divergence.rule) &&
      !used.has(divergence) &&
      (divergence.source === undefined ? allSources.every(source => scoredSources.has(source)) : scoredSources.has(divergence.source)),
  );

  const capabilities: Record<string, string[]> = {};
  for (const capability of pack.capabilities) capabilities[capability] = pack.rules.filter(rule => rule.capabilities.includes(capability)).map(rule => rule.id);
  // An exemption stands only while no rule exercises the capability; once one does, the exemption is stale.
  for (const { capability } of pack.exemptCapabilities) {
    const rules = capabilities[capability]!;
    if (rules.length > 0) errors.push(`stale exemption: capability ${capability} is covered by rule(s) ${rules.join(", ")}`);
  }
  const exempt = new Set(pack.exemptCapabilities.map(exemption => exemption.capability));
  const report: T4Report = {
    format: REPORT_FORMAT,
    pack: pack.pack,
    tier: "t4-rules",
    date: options.date,
    kernel: engineInfo(),
    corpora: pack.corpora.map(({ name, repository, commit }) => ({ name, repository, commit })),
    tools: pack.tools,
    scope: { rules: selected.map(rule => rule.id), fixtures: options.fixtures, corpora: [...options.corpora] },
    coverage: {
      rules: pack.rules.length,
      minimumRules: MINIMUM_RULES,
      capabilities,
      uncovered: pack.capabilities.filter(capability => capabilities[capability]!.length === 0 && !exempt.has(capability)),
      exempt: pack.exemptCapabilities,
    },
    sources: runs.map(run => run.summary),
    rules: rows,
    staleDivergences: stale.map(({ rule, source, path, line, kind }) => ({ rule, ...(source === undefined ? {} : { source }), path, line, kind })),
    errors,
    pass: false,
  };
  report.pass = reportFailures(report).length === 0;
  return report;
}

/**
 * Why a report fails, one line per problem; empty when it passes. An unexplained FP fails on every source; an
 * unexplained FN only on the fixtures, which the port must reproduce: on a corpus, misses are the rule's recall. A
 * shipped port with no fixture TP fails unless pack.json gives its rule examples: divergences can excuse every
 * finding of its fixtures, and a port that matches nothing must still show it catches something. A candidate's row
 * (a port under `candidates/`) is reported but fails nothing; its errors and stale divergences still do.
 */
export function reportFailures(report: T4Report): string[] {
  return [
    ...report.errors,
    ...report.rules.flatMap(row => {
      const problems: string[] = [];
      if (row.candidate) return problems;
      if (row.fp > 0) problems.push(`${row.rule} (${row.source}): ${row.fp} unexplained FP, first ${JSON.stringify(row.falsePositives[0])}`);
      if (row.fn > 0 && row.source === "fixtures") problems.push(`${row.rule} (${row.source}): ${row.fn} unexplained FN, first ${JSON.stringify(row.falseNegatives[0])}`);
      if (row.tp === 0 && row.source === "fixtures" && !row.examples) problems.push(`${row.rule} (${row.source}): 0 TP and no examples in pack.json; nothing shows it catches anything`);
      if (row.engineFailures > 0) problems.push(`${row.rule} (${row.source}): ${row.engineFailures} engine failure(s)`);
      return problems;
    }),
    ...report.staleDivergences.map(divergence => `${divergence.rule}: stale divergence ${divergence.path}:${divergence.line} (${divergence.kind}) explains nothing`),
  ];
}
