import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";
import { z } from "zod";
import { isPackFixture, packFixtureAbsence, requirePackFixture, samplesTree } from "../../scripts/s3/pack-fixtures.mjs";
import { LABELS_SUFFIX, SAMPLE_ID, sampleLabelsSchema } from "../../scripts/samples/format.js";
import type { CodeRuleSourceLanguage } from "../../src/engine/index.js";
import {
  languageFor,
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  nativePlatformSuffix,
  type NativeCompiledCodeRuleDsl,
  CODE_RULE_AST_LANGUAGES,
  CODE_RULE_DIALECTS,
  codeRuleLanguageFamily,
  codeRuleLanguageEmbedVias,
  compareDeterministicStrings,
  isUnreadFailure,
  UNREAD_FAILURE_HEAD,
} from "../../src/engine/index.js";
import { capabilitiesJson, INFORMATIONAL_FAILURE_CODES } from "../engine.js";
import {
  fixturesDirectory,
  fixturesExpectedPath,
  loadExpectedFindings,
  coversFixture,
  documentPath,
  isCandidate,
  MINIMUM_RULES,
  packFamilies,
  RERAIL_ROOT,
  unitKey,
  type Divergence,
  type FixtureExpectedFindings,
  type LoadedPack,
  type PackRule,
  type ExpectedFindings,
  type UnitLanguage,
  type UnitOrigin,
  type UpstreamFinding,
} from "./pack.js";
import { GATES_MANIFEST, readGatesManifest, toolPackDirectory } from "./manifest.js";
import { EXAMPLE_LIMIT, REPORT_FORMAT, type RuleRow, type SourceSummary, type SyntaxErrorLocation, type T4Report } from "./report.js";
import { precisionRecall, scoreSamples, scoreSource, type JudgedSample, type Point, type SourceScore } from "./score.js";

/** The production repository audit's batching and paging (`code-rule-enforcer.ts`). */
const AUDIT_BATCH_FILES = 32;
const AUDIT_CHUNK_BYTES = 1024 * 1024;
const RESULT_PAGE_BYTES = 8 * 1024 * 1024;

export interface SourceFile {
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
 * in 32-file / 1 MiB batches with global relations deferred, one closing apply, then results paged out. `root` is the
 * folder the files' paths are relative to, which import targets resolve against as the daemon resolves them against
 * its workspace root.
 */
export async function evaluateWorkspace(codeRules: readonly NativeCompiledCodeRuleDsl[], files: readonly SourceFile[], root?: string): Promise<Evaluation> {
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  const started = performance.now();
  try {
    for (const codeRule of codeRules) workspace.addCodeRule(codeRule);
    if (root !== undefined) workspace.setRoot(resolve(root));
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
 * Every AST clause skips a file whose error regions cover half of it or more (`isUnreadFailure`): the grammar did not
 * read it, which is T1's too. The message names the first region (`FIRST_SYNTAX_ERROR`), so no probe is needed.
 */
function unreadLocation(failure: Page["failures"][number]): SyntaxErrorLocation | undefined {
  const match = isUnreadFailure(failure) && failure.path !== undefined ? FIRST_SYNTAX_ERROR.exec(failure.message) : null;
  return match === null ? undefined : { path: failure.path!, line: Number(match[1]), column: Number(match[2]) };
}

/**
 * Whether `failure` is a `source-incomplete` the engine reports because a file was read with syntax errors, in this or
 * another file, rather than because a file could not be read at all. The code alone cannot tell them apart: the engine
 * also reports unreadable, binary and released sources (`cannot read workspace source: …`, `Binary source cannot be
 * evaluated`, `source released while the content store was busy …`) as `source-incomplete`, and those stay engine
 * failures. So the message decides, one entry per engine source of a parse-induced message (native/src/code_rule_dsl/):
 * - "file has syntax errors": a clause unsound under errors skipped the file (workspace/evaluate.rs), here without the
 *   probe's confirmation (`SYNTAX_ERROR_FAILURE` otherwise);
 * - "file did not parse: …" (`UNREAD_FAILURE_HEAD`): the parser did not read the file (soundness.rs `UNREAD`), here
 *   without the first error's location (`unreadLocation` otherwise);
 * - "query source is incomplete": a `select_when` row in a file read around errors (eval/mod.rs);
 * - "A cross-file query source is incomplete": a cross-file query's row is unknown (workspace/global.rs);
 * - "subject evidence in this file is incomplete", "evidence is incomplete in …" and "count N of `key` is a lower
 *   bound: evidence is incomplete in …": a `require_each` or `budget` over evidence some file did not yield in full
 *   (eval/evidence.rs).
 * Only the samples treat these as reading in part; on the fixtures they stay engine failures.
 */
function isParseIncomplete(failure: Page["failures"][number]): boolean {
  if (failure.code !== "source-incomplete") return false;
  const { message } = failure;
  return (
    message === SYNTAX_ERROR_FAILURE.message ||
    message.startsWith(`${UNREAD_FAILURE_HEAD}: `) ||
    message === "query source is incomplete" ||
    message === "A cross-file query source is incomplete" ||
    message === "subject evidence in this file is incomplete" ||
    message.startsWith("evidence is incomplete in ") ||
    /^count \d+ of `.*` is a lower bound: evidence is incomplete in /su.test(message)
  );
}

/**
 * The first syntax error the grammar reports in each of `files` that skipped a clause with `SYNTAX_ERROR_FAILURE`.
 * A probe clause sound under errors (it matches `ERROR` nodes, which are never witnesses, so it reports nothing) is
 * evaluated over those files through the same workspace path; its `source-partial` failure confirms the syntax error
 * and locates it. A language whose files embed a program probes each document (`ast.embedded`): each has its own
 * syntax errors.
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
  const violation = { anchor: "witness.span", code: "SYNTAX-PROBE", message: "syntax probe" };
  const families: readonly string[] = packFamilies(loaded.pack);
  const probes = CODE_RULE_AST_LANGUAGES.filter(language => families.includes(codeRuleLanguageFamily(language))).map(language =>
    runtime.compile(
      JSON.stringify({
        format: "rerail.policy/v1",
        id: `conformance-syntax-probe:${language}`,
        revision: 1,
        mode: "log",
        description: "Locates the grammar's syntax errors; reports nothing.",
        scope: { include: ["**/*"], exclude: [] },
        clauses: [
          { id: "probe", forbid: { ast: { language, rule: { kind: "ERROR" } } }, violation },
          ...codeRuleLanguageEmbedVias(language).map(via => ({
            id: `probe-${via}`,
            forbid: { ast: { language, embedded: via, rule: { kind: "ERROR" } } },
            violation,
          })),
        ],
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
  /** The code rule document's text, for its applicability probe (`applicabilityProbe`). */
  document: string;
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
      return [{ rule, policyId: document.id, document: text, compiled: runtime.compile(text, "json", capabilitiesJson()) }];
    } catch (error) {
      errors.push(`${rule.id}: ${relative(RERAIL_ROOT, path)}: ${error instanceof Error ? error.message : String(error)}`);
      return [];
    }
  });
}

/** Whether an AST query's `language` is an ECMA one, whose queries take ESLint's `languageOptions`. */
const isEcmaQuery = (language: unknown): boolean =>
  typeof language === "string" && (CODE_RULE_AST_LANGUAGES as readonly string[]).includes(language) && codeRuleLanguageFamily(language as (typeof CODE_RULE_AST_LANGUAGES)[number]) === "ecma";

/**
 * What a code rule's AST query spells of a unit's language (`UnitLanguage`): `ecmaVersion` and `sourceType` as they are,
 * `parserOptions.ecmaFeatures`' `globalReturn` and `impliedStrict` (the engine reads no other parser option of
 * espree's), and typescript-eslint's `jsxPragma`, `jsxFragmentName` and `lib`, which a query spells at the top of its
 * `languageOptions`. Anything else is the parser's alone (`ecmaFeatures.jsx` is the extension's).
 */
export function queryLanguageOptions(language: UnitLanguage): JsonObject {
  const options: JsonObject = {};
  if (language.ecmaVersion !== undefined) options.ecmaVersion = language.ecmaVersion;
  if (language.sourceType !== undefined) options.sourceType = language.sourceType;
  const parserOptions = language.parserOptions ?? {};
  const features = isJsonObject(parserOptions.ecmaFeatures) ? parserOptions.ecmaFeatures : {};
  const ecmaFeatures = Object.fromEntries(["globalReturn", "impliedStrict"].filter(key => typeof features[key] === "boolean").map(key => [key, features[key]]));
  if (Object.keys(ecmaFeatures).length > 0) options.parserOptions = { ecmaFeatures };
  for (const key of ["jsxPragma", "jsxFragmentName", "lib"]) if (parserOptions[key] !== undefined) options[key] = parserOptions[key];
  return options;
}

/**
 * Marks every `resolves` under `value` that lists `global` names, without `declared` or `directives`, as
 * `declared: false`: what such a read means in a query without `languageOptions` (the name is unbound, whether or not
 * the configuration declares it), which giving the query `languageOptions` would otherwise turn into "a declared global".
 */
function keepUndeclaredGlobals(value: unknown): void {
  if (Array.isArray(value)) {
    for (const item of value) keepUndeclaredGlobals(item);
    return;
  }
  if (!isJsonObject(value)) return;
  const resolves = value.resolves;
  if (isJsonObject(resolves) && Array.isArray(resolves.global) && resolves.declared === undefined && resolves.directives === undefined) resolves.declared = false;
  for (const inner of Object.values(value)) keepUndeclaredGlobals(inner);
}

/**
 * A code rule document with `language` (a unit's) merged into the `languageOptions` of each ECMA AST clause, every
 * case of a clause chosen by a document option included: the unit's language replaces the rule's `ecmaVersion`,
 * `sourceType` and typescript-eslint options and adds to its `ecmaFeatures`; the rule's `globals` stay. A query that
 * had no `languageOptions` keeps reading its listed globals as unbound names (`keepUndeclaredGlobals`); shared
 * definitions compile apart from any query's options, so they need none. A language with nothing the engine reads
 * leaves the document as it is.
 */
export function inUnitLanguage(documentText: string, language: UnitLanguage): string {
  const options = queryLanguageOptions(language);
  // Nothing of the unit's language is the engine's to read (its parserOptions are the parser's alone).
  if (Object.keys(options).length === 0) return documentText;
  const document = JSON.parse(documentText) as JsonObject;
  const merge = (query: JsonObject): void => {
    if (!isEcmaQuery(query.language)) return;
    if (!isJsonObject(query.languageOptions)) keepUndeclaredGlobals(query);
    const own = isJsonObject(query.languageOptions) ? query.languageOptions : {};
    const merged: JsonObject = { ...own, ...options };
    if (isJsonObject(options.parserOptions) && isJsonObject(own.parserOptions)) {
      const ownFeatures = isJsonObject(own.parserOptions.ecmaFeatures) ? own.parserOptions.ecmaFeatures : {};
      merged.parserOptions = { ...own.parserOptions, ecmaFeatures: { ...ownFeatures, ...(options.parserOptions.ecmaFeatures as JsonObject) } };
    }
    query.languageOptions = merged;
  };
  for (const clause of Array.isArray(document.clauses) ? document.clauses.filter(isJsonObject) : []) {
    const body = isJsonObject(clause.forbid) ? clause.forbid : isJsonObject(clause.require) ? clause.require : undefined;
    if (!isJsonObject(body?.ast)) continue;
    const ast = body.ast;
    if (ast.$option !== undefined && isJsonObject(ast.cases)) for (const variant of [...Object.values(ast.cases), ast.default].filter(isJsonObject)) merge(variant);
    else merge(ast);
  }
  return JSON.stringify(document);
}

/**
 * The rules as a unit with a recorded language (`source.units[key].languageOptions`) is evaluated with: each rule's code
 * rule recompiled with that language in its AST clauses (`inUnitLanguage`), once per rule and distinct language. The
 * engine's languageOptions are per query, not per file, so a unit of its own language takes its own copy of the rule.
 * A copy that does not compile is an error, and the rule is not evaluated on the unit.
 */
function unitLanguageRules(errors: string[]): (rules: readonly CompiledRule[], language: UnitLanguage | undefined, unit: string) => CompiledRule[] {
  const cache = new Map<string, CompiledRule | string>();
  return (rules, language, unit) => {
    if (language === undefined) return [...rules];
    const key = JSON.stringify(language);
    return rules.flatMap(entry => {
      const cacheKey = `${entry.policyId}\0${key}`;
      let compiled = cache.get(cacheKey);
      if (compiled === undefined) {
        const document = inUnitLanguage(entry.document, language);
        try {
          compiled = { ...entry, compiled: loadNativeBinding().CodeRuleDslRuntime.compile(document, "json", capabilitiesJson()) };
        } catch (error) {
          compiled = error instanceof Error ? error.message : String(error);
        }
        cache.set(cacheKey, compiled);
      }
      if (typeof compiled !== "string") return [compiled];
      errors.push(`${entry.rule.id}: does not compile in the language of fixture unit ${unit} (${key}): ${compiled}`);
      return [];
    });
  };
}

/** Byte offset → upstream coordinates: the engine's line (the family's linter's terminators), code points from line start. */
export function pointOf(violation: Page["violations"][number], sources: ReadonlyMap<string, Buffer>): Point {
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
  /** Engine failures, other than `source-partial`, the syntax-error skips below and `isParseIncomplete`, per path. */
  engineFailurePaths: Map<string, number>;
  /** Of those, the ones naming no file: rule-level, so they count wherever the rule is scored. */
  pathlessEngineFailures: number;
  partialPaths: Set<string>;
  /** `isParseIncomplete` failures other than the syntax-error skips below, per path, and those naming no file. */
  incompletePaths: Map<string, number>;
  pathlessIncomplete: number;
  /** Files this rule's clauses skipped with `SYNTAX_ERROR_FAILURE` or as unread (`isUnreadFailure`), at the first syntax error. */
  syntaxErrorFiles: Map<string, SyntaxErrorLocation>;
  /** Fixture units (`unitKey`) whose evaluation reported a failure of this rule naming no file. */
  pathlessUnits: Set<string>;
}

const noActual = (): RuleActual => ({
  points: [],
  engineFailurePaths: new Map(),
  pathlessEngineFailures: 0,
  partialPaths: new Set(),
  incompletePaths: new Map(),
  pathlessIncomplete: 0,
  syntaxErrorFiles: new Map(),
  pathlessUnits: new Set(),
});

const total = (counts: ReadonlyMap<string, number>, paths: (path: string) => boolean = () => true): number =>
  [...counts].reduce((sum, [path, count]) => (paths(path) ? sum + count : sum), 0);

/** A scored fixture unit: its key (`unitKey`), its source paths, and its upstream origin when `source.units` has one. */
interface ScoredUnit {
  key: string;
  paths: readonly string[];
  origin: UnitOrigin | undefined;
}

interface FixturesRun {
  /** Per pack rule id, over the original units. */
  actual: Map<string, RuleActual>;
  /** Per pack rule id, over the rename variants (`source.units[key].renamedFrom`), scored apart from the originals. */
  renamedActual: Map<string, RuleActual>;
  /** Per tool: its expected findings, or the reason there are none. */
  expectedByTool: Map<string, ExpectedFindings | string>;
  /** `tool\0path` of every file evaluated; the expected findings list every file upstream ran on. */
  scored: ReadonlySet<string>;
  /** Per tool, every unit evaluated, rename variants included. */
  units: Map<string, ScoredUnit[]>;
  summary: SourceSummary;
}

/** The real-code samples as a source, as divergences and report rows name it. */
export const SAMPLES_SOURCE = "samples";

interface SamplesRun {
  /** Per pack rule id. */
  actual: Map<string, RuleActual>;
  /** Per pack rule id, the samples judged for it; a rule no sample has a key for has no samples row. */
  judged: Map<string, JudgedSample[]>;
  /** Per pack rule id, the clean samples its scope and its clauses' `file` conditions admit (`applicabilityProbe`). */
  admitted: Map<string, ReadonlySet<string>>;
  /** Rules whose tool's samples tree is missing: the run's errors name each, and none of their divergences is stale. */
  unscored: ReadonlySet<string>;
  summary: SourceSummary;
}

type JsonObject = Record<string, unknown>;
const isJsonObject = (value: unknown): value is JsonObject => typeof value === "object" && value !== null && !Array.isArray(value);

/** Matches a file's root node alone, whatever its grammar: the one node inside no other. */
const ROOT_NODE = { not: { inside: { regex: "[\\s\\S]" } } };

/**
 * The applicability probe of a code rule document: the same document (scope, options) whose AST clauses each match
 * only the file's root node, keeping the clause's language and `file` condition. It reports a file exactly when the
 * rule's scope and some clause's `file` condition admit it, whatever the file holds. Undefined when the document has
 * no AST clause: its scope alone decides then, and the rule's own evaluation applies it.
 */
export function applicabilityProbe(documentText: string): string | undefined {
  const document = JSON.parse(documentText) as JsonObject;
  const clauses = Array.isArray(document.clauses) ? document.clauses.filter(isJsonObject) : [];
  const probes = clauses.flatMap((clause, index) => {
    const body = isJsonObject(clause.forbid) ? clause.forbid : isJsonObject(clause.require) ? clause.require : undefined;
    if (!isJsonObject(body?.ast)) return [];
    const ast = body.ast;
    // A clause chosen by a document option (`$option`) applies where any of its cases does.
    const variants = ast.$option !== undefined && isJsonObject(ast.cases) ? [...Object.values(ast.cases), ast.default].filter(isJsonObject) : [ast];
    return variants.flatMap((variant, position) => {
      if (typeof variant.language !== "string") return [];
      const kept: JsonObject = { language: variant.language, rule: ROOT_NODE };
      for (const key of ["file", "languageOptions", "embedded"] as const) if (variant[key] !== undefined) kept[key] = variant[key];
      return [{ id: `applies-${index}-${position}`, forbid: { ast: kept }, violation: { anchor: "witness.span", code: "APPLIES-PROBE", message: "the rule applies to this file" } }];
    });
  });
  if (probes.length === 0) return undefined;
  // Clauses that only define (options, utilities) stay: the probe's options resolve as the rule's do.
  const defining = clauses.filter(clause => clause.forbid === undefined && clause.require === undefined);
  return JSON.stringify({ ...document, clauses: [...defining, ...probes] });
}

/** The paths of `files` the rule's applicability probe reports: where its scope and `file` conditions admit it. */
async function admittedPaths(entry: CompiledRule, files: readonly SourceFile[], judged: readonly string[], errors: string[]): Promise<ReadonlySet<string>> {
  const probe = applicabilityProbe(entry.document);
  if (probe === undefined) return new Set(judged);
  let compiled: NativeCompiledCodeRuleDsl;
  try {
    compiled = loadNativeBinding().CodeRuleDslRuntime.compile(probe, "json", capabilitiesJson());
  } catch (error) {
    // Fail closed: no clean sample counts as silence for a rule whose applicability cannot be asked.
    errors.push(`${entry.rule.id} (${SAMPLES_SOURCE}): its applicability probe does not compile: ${error instanceof Error ? error.message : String(error)}`);
    return new Set();
  }
  const evaluation = await evaluateWorkspace([compiled], files);
  const reported = new Set(evaluation.violations.flatMap(violation => (violation.path === undefined ? [] : [violation.path])));
  return new Set(judged.filter(path => reported.has(path)));
}

/** Collects each rule's violations and failures; `syntaxErrors` holds the probe-confirmed syntax-error files. */
function collectActual(
  compiled: readonly CompiledRule[],
  evaluation: Evaluation,
  sources: ReadonlyMap<string, Buffer>,
  syntaxErrors: ReadonlyMap<string, SyntaxErrorLocation>,
  actual: Map<string, RuleActual>,
  unit?: string,
): void {
  const byCodeRule = new Map(compiled.map(entry => [entry.policyId, entry.rule.id]));
  const slot = (rule: string): RuleActual => {
    let entry = actual.get(rule);
    if (entry === undefined) actual.set(rule, (entry = noActual()));
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
    } else if (isParseIncomplete(failure)) {
      const entry = slot(rule);
      if (failure.path === undefined) entry.pathlessIncomplete += 1;
      else entry.incompletePaths.set(failure.path, (entry.incompletePaths.get(failure.path) ?? 0) + 1);
    } else {
      const entry = slot(rule);
      if (failure.path === undefined) entry.pathlessEngineFailures += 1;
      else entry.engineFailurePaths.set(failure.path, (entry.engineFailurePaths.get(failure.path) ?? 0) + 1);
    }
    if (failure.path === undefined && unit !== undefined) slot(rule).pathlessUnits.add(unit);
  }
}

function familyLanguages(loaded: LoadedPack): Set<string> {
  const families: readonly string[] = packFamilies(loaded.pack);
  return new Set(Object.values(CODE_RULE_DIALECTS).filter(dialect => families.includes(dialect.family)).map(dialect => dialect.label));
}

/** A fixture's text; pack test data missing from the checkout fails with the instruction to pull it. */
function readCaseSource(path: string): string {
  return readFileSync(isPackFixture(path) ? requirePackFixture(path) : path, "utf8");
}

/** The expected findings at `path`, checked against the pack's pins, or why it cannot be used. */
function pinnedExpectedFindings(loaded: LoadedPack, tool: string, path: string): ExpectedFindings | string {
  const where = relative(RERAIL_ROOT, path);
  if (!existsSync(path)) return packFixtureAbsence(path) ?? `no expected findings at ${where}`;
  let findingsFile: ExpectedFindings;
  try {
    findingsFile = loadExpectedFindings(path);
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  const version = loaded.pack.tools.find(entry => entry.name === tool)?.version;
  if (findingsFile.tool !== tool) return `${where} records ${findingsFile.tool}, not ${tool}`;
  if (findingsFile.version !== version) return `${where} records ${tool} ${findingsFile.version}; the pack pins ${version}`;
  if (findingsFile.source.kind !== "fixtures") return `${where} does not hold fixtures expected findings`;
  return findingsFile;
}

/**
 * Shipped ports and candidates, each evaluated in a workspace of their own: a file's AST work budget is shared by every
 * code rule in a workspace, so candidates evaluated beside the shipped ports would spend the shipped ports' budget and
 * change their verdicts, which a candidate must never do. Production never runs candidates beside shipped rules either.
 */
async function evaluateApart(rules: readonly CompiledRule[], files: readonly SourceFile[], root?: string): Promise<[CompiledRule[], Evaluation][]> {
  const groups = [rules.filter(entry => !isCandidate(entry.rule)), rules.filter(entry => isCandidate(entry.rule))].filter(group => group.length > 0);
  const evaluations: [CompiledRule[], Evaluation][] = [];
  for (const group of groups) evaluations.push([group, await evaluateWorkspace(group.map(entry => entry.compiled), files, root)]);
  return evaluations;
}

async function runFixtures(loaded: LoadedPack, compiled: readonly CompiledRule[], errors: string[]): Promise<FixturesRun> {
  const languages = familyLanguages(loaded);
  const actual: FixturesRun["actual"] = new Map();
  const renamedActual: FixturesRun["renamedActual"] = new Map();
  const expectedByTool: FixturesRun["expectedByTool"] = new Map();
  const scored = new Set<string>();
  const units: FixturesRun["units"] = new Map();
  const summary: SourceSummary = { source: "fixtures", units: 0, files: 0, bytes: 0, syntaxErrorFiles: { count: 0, files: [] }, evaluationMs: 0 };
  const inLanguage = unitLanguageRules(errors);
  for (const tool of toolsOf(compiled)) {
    const toolExpected = pinnedExpectedFindings(loaded, tool, fixturesExpectedPath(loaded, tool));
    expectedByTool.set(tool, toolExpected);
    if (typeof toolExpected === "string") continue;
    const source = (toolExpected as FixtureExpectedFindings).source;
    const rules = compiled.filter(entry => entry.rule.upstream.tool === tool);
    const toolUnits: ScoredUnit[] = [];
    units.set(tool, toolUnits);
    for (const unit of source.files) {
      const paths = typeof unit === "string" ? [unit] : unit;
      const covering = rules.filter(entry => paths.some(path => coversFixture(entry.rule, path)));
      if (covering.length === 0) continue;
      const files: SourceFile[] = [];
      // A unit's package.json and module configurations are no sources: they ride along so the engine reads the
      // sources' manifest facts and import aliases.
      const manifests: SourceFile[] = [];
      for (const path of paths) {
        const language = languageFor(path);
        if (language !== undefined && ridesAlong(path)) {
          manifests.push({ path, language, source: readCaseSource(join(fixturesDirectory(loaded, tool), path)) });
          continue;
        }
        if (language === undefined || !languages.has(language)) {
          errors.push(`fixtures ${tool}: ${path} is not a ${packFamilies(loaded.pack).join(" or ")} source by the host's routing`);
          continue;
        }
        files.push({ path, language, source: readCaseSource(join(fixturesDirectory(loaded, tool), path)) });
      }
      if (files.length === 0) continue;
      const key = unitKey(unit);
      const origin = source.units?.[key];
      // A unit linted under its test case's own language is evaluated under it too.
      const unitRules = inLanguage(covering, origin?.languageOptions, key);
      // A rename variant's findings are scored apart from the originals' (`renamed` on the row).
      const into = origin?.renamedFrom === undefined ? actual : renamedActual;
      const sources = new Map(files.map(file => [file.path, Buffer.from(file.source, "utf8")]));
      for (const [group, evaluation] of await evaluateApart(unitRules, [...manifests, ...files], fixturesDirectory(loaded, tool))) {
        collectActual(group, evaluation, sources, await syntaxErrorLocations(loaded, evaluation, files), into, key);
        summary.evaluationMs += evaluation.ms;
      }
      for (const file of files) scored.add(`${tool}\0${file.path}`);
      toolUnits.push({ key, paths: files.map(file => file.path), origin });
      summary.units += 1;
      summary.files += files.length;
      summary.bytes += [...sources.values()].reduce((total, bytes) => total + bytes.length, 0);
    }
  }
  summary.evaluationMs = Math.round(summary.evaluationMs);
  summary.syntaxErrorFiles = syntaxErrorSummary(actual);
  return { actual, renamedActual, expectedByTool, scored, units, summary };
}

/** The tools of `compiled`'s rules, each once, in order. */
function toolsOf(compiled: readonly CompiledRule[]): string[] {
  return [...new Set(compiled.map(entry => entry.rule.upstream.tool))].sort(compareDeterministicStrings);
}

/** Every file of a source some rule skipped for its syntax errors, each once, in path order. */
function syntaxErrorSummary(actual: Map<string, RuleActual>): SourceSummary["syntaxErrorFiles"] {
  const files = new Map<string, SyntaxErrorLocation>();
  for (const rule of actual.values()) for (const [path, point] of rule.syntaxErrorFiles) files.set(path, point);
  const sorted = [...files.values()].sort((left, right) => compareDeterministicStrings(left.path, right.path));
  return { count: sorted.length, files: sorted };
}

/** Every regular file under `directory`, as a posix path relative to `root`, in path order. */
function filesUnder(root: string, directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true })
    .flatMap(entry => {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) return filesUnder(root, path);
      return entry.isFile() ? [relative(root, path).split(sep).join("/")] : [];
    })
    .sort(compareDeterministicStrings);
}

/** A judged file of a sample with its labels; the sample's other files ride along as its context, never judged. */
interface Sample {
  /** `<sample-id>/<source.path>`: the workspace path it is evaluated under and divergences name it by. */
  path: string;
  /** Pack rule id → labelled lines; `[]` is clean for the rule, and a rule with no key does not judge the sample. */
  expected: Record<string, number[]>;
  context: string[];
}

/**
 * The samples in a tool-pack's samples tree (scripts/samples/format.ts): each directory at its top is a sample, and each
 * file in it with a labels file beside it is judged. Files at the top (LICENSES.md, the extraction marker) are no
 * samples. A labels key that is no rule of the tool in the pack is an error: the labels name rules by their pack id.
 */
function readSamples(loaded: LoadedPack, tool: string, directory: string, errors: string[]): Sample[] {
  const where = `${SAMPLES_SOURCE} ${tool}`;
  const toolRules = new Set(loaded.pack.rules.filter(rule => rule.upstream.tool === tool).map(rule => rule.id));
  const ids = readdirSync(directory, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort(compareDeterministicStrings);
  const samples: Sample[] = [];
  for (const id of ids) {
    if (!SAMPLE_ID.test(id)) {
      errors.push(`${where}: ${id} is no sample id (<source>-<path slug>-<first 12 of its sha256>)`);
      continue;
    }
    const files = filesUnder(directory, join(directory, id));
    const judged = new Set(files.filter(path => path.endsWith(LABELS_SUFFIX)).map(path => path.slice(0, -LABELS_SUFFIX.length)));
    const context = files.filter(path => !path.endsWith(LABELS_SUFFIX) && !judged.has(path));
    for (const path of judged) {
      const labelsPath = `${path}${LABELS_SUFFIX}`;
      let json: unknown;
      try {
        json = JSON.parse(readFileSync(join(directory, labelsPath), "utf8"));
      } catch (error) {
        errors.push(`${where}: ${labelsPath}: ${error instanceof Error ? error.message : String(error)}`);
        continue;
      }
      const parsed = sampleLabelsSchema.safeParse(json);
      if (!parsed.success) {
        errors.push(`${where}: ${labelsPath}: ${z.prettifyError(parsed.error)}`);
        continue;
      }
      if (path !== `${id}/${parsed.data.source.path}`) {
        errors.push(`${where}: ${labelsPath} labels ${parsed.data.source.path}, which is not the file beside it`);
        continue;
      }
      if (!files.includes(path)) {
        errors.push(`${where}: ${labelsPath} labels a file the sample does not hold`);
        continue;
      }
      const unknown = Object.keys(parsed.data.expected).filter(rule => !toolRules.has(rule));
      if (unknown.length > 0) errors.push(`${where}: ${path} is labelled for ${unknown.join(", ")}, no ${tool} rule of the ${loaded.pack.pack} pack`);
      const expected = Object.fromEntries(Object.entries(parsed.data.expected).map(([rule, labels]) => [rule, labels.map(label => label.line)]));
      samples.push({ path, expected, context });
    }
  }
  return samples;
}

/**
 * The pack's real-code samples, per tool-pack from its samples tree (`directoryOf`; undefined when it has none). Each
 * rule is evaluated alone over the samples labelled for it, their context files riding along first as a fixture unit's
 * manifests do: a file's AST work budget is shared by the rules beside it, so a rule's verdict never depends on which
 * other rules a sample is labelled for. A tree that cannot be had fails the rules of its tool alone, since a
 * conformance leg restores only its own tool-pack's data.
 */
async function runSamples(loaded: LoadedPack, compiled: readonly CompiledRule[], directoryOf: (tool: string) => string | undefined, errors: string[]): Promise<SamplesRun> {
  const languages = familyLanguages(loaded);
  const actual: SamplesRun["actual"] = new Map();
  const judged: SamplesRun["judged"] = new Map();
  const admitted: SamplesRun["admitted"] = new Map();
  const unscored = new Set<string>();
  const evaluated = new Map<string, number>();
  const judgedPaths = new Set<string>();
  let evaluationMs = 0;
  for (const tool of toolsOf(compiled)) {
    const rules = compiled.filter(entry => entry.rule.upstream.tool === tool);
    let directory: string | undefined;
    try {
      directory = directoryOf(tool);
      if (directory !== undefined && !existsSync(directory)) throw new Error(`no samples tree at ${directory}`);
    } catch (error) {
      for (const entry of rules) {
        errors.push(`${entry.rule.id} (${SAMPLES_SOURCE}): ${error instanceof Error ? error.message : String(error)}`);
        unscored.add(entry.rule.id);
      }
      continue;
    }
    if (directory === undefined) continue;
    const tree = directory;
    const samples = readSamples(loaded, tool, tree, errors).filter(sample => {
      const language = languageFor(sample.path);
      if (language !== undefined && languages.has(language)) return true;
      errors.push(`${SAMPLES_SOURCE} ${tool}: ${sample.path} is not a ${packFamilies(loaded.pack).join(" or ")} source by the host's routing`);
      return false;
    });
    const texts = new Map<string, string>();
    const fileOf = (path: string, language: CodeRuleSourceLanguage): SourceFile => {
      let source = texts.get(path);
      if (source === undefined) texts.set(path, (source = readFileSync(join(tree, path), "utf8")));
      return { path, language, source };
    };
    for (const entry of rules) {
      const labelled = samples.filter(sample => sample.expected[entry.rule.id] !== undefined);
      if (labelled.length === 0) continue;
      judged.set(entry.rule.id, labelled.map(sample => ({ path: sample.path, lines: sample.expected[entry.rule.id]! })));
      const context = labelled.flatMap(sample =>
        sample.context.flatMap(path => {
          const language = languageFor(path);
          return language === undefined ? [] : [fileOf(path, language)];
        }),
      );
      const files = labelled.map(sample => fileOf(sample.path, languageFor(sample.path)!));
      const all = [...context, ...files];
      const sources = new Map(all.map(file => [file.path, Buffer.from(file.source, "utf8")]));
      const evaluation = await evaluateWorkspace([entry.compiled], all);
      collectActual([entry], evaluation, sources, await syntaxErrorLocations(loaded, evaluation, all), actual);
      evaluationMs += evaluation.ms;
      for (const [path, bytes] of sources) evaluated.set(path, bytes.length);
      for (const file of files) judgedPaths.add(file.path);
      const clean = labelled.filter(sample => sample.expected[entry.rule.id]!.length === 0).map(sample => sample.path);
      if (clean.length > 0) admitted.set(entry.rule.id, await admittedPaths(entry, all, clean, errors));
    }
  }
  const summary: SourceSummary = {
    source: SAMPLES_SOURCE,
    units: judgedPaths.size,
    files: evaluated.size,
    bytes: [...evaluated.values()].reduce((total, bytes) => total + bytes, 0),
    syntaxErrorFiles: syntaxErrorSummary(actual),
    evaluationMs: Math.round(evaluationMs),
  };
  return { actual, judged, admitted, unscored, summary };
}

/**
 * Whether a unit's file rides along with its sources rather than being one: a package.json, a `.npmignore` or
 * `.gitignore` (as the engine's `ignore_file_directory` names them), or a project configuration (a `.json` file named
 * `tsconfig…` or `jsconfig…`, as the engine's `is_module_config` names them).
 */
function ridesAlong(path: string): boolean {
  return /^(?:package\.json|\.npmignore|\.gitignore|(?:ts|js)config[^/]*\.json)$/.test(basename(path));
}

export interface RunOptions {
  /** Pack rule ids to score; all when omitted. */
  rules?: readonly string[];
  fixtures: boolean;
  /** Score the pack's real-code samples (every tool with a samples.lock or samples staged under `publish/samples/`). */
  samples: boolean;
  /** Test seam: a tool's samples tree; default samplesTree() from pack-fixtures.mjs, the locked set overlaid with the staged one. */
  samplesDirectory?: (tool: string) => string | undefined;
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

/** A row's fields that come from its scoring rather than from the rule. */
interface Scored {
  score: SourceScore;
  reviewed: readonly Point[];
  cleanSamples: number;
  silentCleanSamples: number;
  judgedSamples: number;
  ownUnits: number;
  silentOwnUnits: number;
  upstreamFindings: number;
  renamed: RuleRow["renamed"];
  engineFailures: number;
  contextFailures: number;
  partialFiles: number;
  syntaxErrorFiles: readonly SyntaxErrorLocation[];
}

function rowOf({ rule }: CompiledRule, loaded: LoadedPack, source: string, scored: Scored, used: Set<Divergence>): RuleRow {
  const { score } = scored;
  for (const entry of score.explained) used.add(entry.divergence);
  return {
    rule: rule.id,
    candidate: isCandidate(rule),
    examples: loaded.exampled.has(documentPath(rule.policy)),
    upstream: rule.upstream,
    capabilities: rule.capabilities,
    source,
    tp: score.tp,
    fp: score.falsePositives.length,
    fn: score.falseNegatives.length,
    ...precisionRecall(score),
    reviewed: scored.reviewed.length,
    reviewedFindings: scored.reviewed.slice(0, EXAMPLE_LIMIT),
    cleanSamples: scored.cleanSamples,
    silentCleanSamples: scored.silentCleanSamples,
    judgedSamples: scored.judgedSamples,
    ownUnits: scored.ownUnits,
    silentOwnUnits: scored.silentOwnUnits,
    upstreamFindings: scored.upstreamFindings,
    renamed: scored.renamed,
    engineFailures: scored.engineFailures,
    contextFailures: scored.contextFailures,
    partialFiles: scored.partialFiles,
    syntaxErrorFiles: {
      count: scored.syntaxErrorFiles.length,
      files: [...scored.syntaxErrorFiles].sort((left, right) => compareDeterministicStrings(left.path, right.path)).slice(0, EXAMPLE_LIMIT),
    },
    divergences: score.explained.map(({ side, point, divergence }) => ({ kind: divergence.kind, side, ...point, evidence: divergence.evidence })),
    falsePositives: score.falsePositives.slice(0, EXAMPLE_LIMIT),
    falseNegatives: score.falseNegatives.slice(0, EXAMPLE_LIMIT),
  };
}

function fixturesRow(entry: CompiledRule, run: FixturesRun, loaded: LoadedPack, used: Set<Divergence>, errors: string[]): RuleRow | undefined {
  const { rule } = entry;
  const { tool } = rule.upstream;
  const toolExpected = run.expectedByTool.get(tool);
  if (toolExpected === undefined) return undefined;
  if (typeof toolExpected === "string") {
    errors.push(`${rule.id} (fixtures): ${toolExpected}`);
    return undefined;
  }
  if (!toolExpected.source.rules.includes(rule.upstream.rule)) {
    errors.push(`${rule.id} (fixtures): the ${tool} expected findings do not cover ${rule.upstream.rule}`);
    return undefined;
  }
  // A rule scoped to part of the fixtures (`fixtures`) is scored only there; other units ran another configuration.
  const findings = toolExpected.findings.filter(finding => finding.rule === rule.upstream.rule && coversFixture(rule, finding.path));
  const units = run.units.get(tool) ?? [];
  const variantPaths = new Set(units.filter(unit => unit.origin?.renamedFrom !== undefined).flatMap(unit => unit.paths));
  const expected: Point[] = [];
  const expectedRenamed: Point[] = [];
  for (const finding of findings) {
    if (!run.scored.has(`${tool}\0${finding.path}`)) {
      errors.push(`${rule.id} (fixtures): ${finding.path} is not among the expected findings' files`);
      continue;
    }
    if (rule.match === "line+column" && finding.column === undefined) {
      errors.push(`${rule.id} (fixtures): ${finding.path}:${finding.line} has no column; the rule matches line+column`);
      continue;
    }
    (variantPaths.has(finding.path) ? expectedRenamed : expected).push({ path: finding.path, line: finding.line, ...(rule.match === "line+column" ? { column: finding.column! } : {}) });
  }
  const pointsOf = (rerail: RuleActual): Point[] => rerail.points.map(point => (rule.match === "line" ? { path: point.path, line: point.line } : point));
  const rerail = run.actual.get(rule.id) ?? noActual();
  const applicable = loaded.divergences.filter(divergence => divergence.rule === rule.id && (divergence.source === undefined || divergence.source === "fixtures"));
  return rowOf(entry, loaded, "fixtures", {
    score: scoreSource(expected, pointsOf(rerail), rule.match, applicable),
    reviewed: [],
    cleanSamples: 0,
    silentCleanSamples: 0,
    judgedSamples: 0,
    ...ownUnits(rule, units, findings, rerail),
    upstreamFindings: expected.length,
    renamed: variantPaths.size === 0 ? null : {
      ...renamedScore(rule, units, findings, expectedRenamed, run.renamedActual.get(rule.id) ?? noActual(), pointsOf, applicable, used),
      ...renameExemption(loaded, tool, rule.upstream.rule),
    },
    // On the fixtures, which the port must read as upstream does, every failure but `source-partial` and the T1
    // syntax-error skips is an engine failure, parse-induced `source-incomplete` included.
    engineFailures: total(rerail.engineFailurePaths) + rerail.pathlessEngineFailures + total(rerail.incompletePaths) + rerail.pathlessIncomplete,
    contextFailures: 0,
    partialFiles: rerail.partialPaths.size,
    syntaxErrorFiles: [...rerail.syntaxErrorFiles.values()],
  }, used);
}

/**
 * A rule's own upstream units and, of those, the silent ones: an original unit (no `renamedFrom`) the rule is scored on
 * whose origin names the rule's upstream rule, where upstream reports the rule nowhere and Rerail reads every file in
 * full and reports nothing. A unit with no origin is never the rule's own: its silence proves nothing about the rule.
 */
function ownUnits(rule: PackRule, units: readonly ScoredUnit[], findings: readonly UpstreamFinding[], rerail: RuleActual): { ownUnits: number; silentOwnUnits: number } {
  const own = units.filter(
    unit => unit.origin !== undefined && unit.origin.renamedFrom === undefined && unit.origin.rules.includes(rule.upstream.rule) && unit.paths.some(path => coversFixture(rule, path)),
  );
  const upstreamReports = new Set(findings.map(finding => finding.path));
  const reported = new Set(rerail.points.map(point => point.path));
  const unsure = (path: string): boolean =>
    reported.has(path) || rerail.engineFailurePaths.has(path) || rerail.incompletePaths.has(path) || rerail.partialPaths.has(path) || rerail.syntaxErrorFiles.has(path);
  const silent = own.filter(unit => !rerail.pathlessUnits.has(unit.key) && unit.paths.every(path => !upstreamReports.has(path) && !unsure(path)));
  return { ownUnits: own.length, silentOwnUnits: silent.length };
}

/** gates.yaml's `rename_exempt` for an upstream rule, as the row's `renamed.exempt`; none when the tool has no gates.yaml. */
function renameExemption(loaded: LoadedPack, tool: string, upstreamRule: string): { exempt?: string } {
  const { pack } = loaded.pack;
  if (!existsSync(join(toolPackDirectory(RERAIL_ROOT, pack, tool), GATES_MANIFEST))) return {};
  const exempt = readGatesManifest(RERAIL_ROOT, pack, tool).rules[upstreamRule]?.rename_exempt;
  return exempt === undefined ? {} : { exempt };
}

/**
 * The rule scored on its tool's rename variants alone, as the originals are scored (holds-under-rename). An engine
 * failure on a variant fails the rule as on an original; a parse-induced `source-incomplete` is only counted, as a
 * sample's is. A variant whose original upstream reports the rule in, and which upstream reports it nowhere in, is
 * counted as renamed away: the renaming took a name the rule keys on, so the variant tests only its silence.
 */
function renamedScore(
  rule: PackRule,
  units: readonly ScoredUnit[],
  findings: readonly UpstreamFinding[],
  expected: readonly Point[],
  rerail: RuleActual,
  pointsOf: (rerail: RuleActual) => Point[],
  applicable: readonly Divergence[],
  used: Set<Divergence>,
): NonNullable<RuleRow["renamed"]> {
  const score = scoreSource([...expected], pointsOf(rerail), rule.match, [...applicable]);
  for (const entry of score.explained) used.add(entry.divergence);
  const variants = units.filter(unit => unit.origin?.renamedFrom !== undefined && unit.paths.some(path => coversFixture(rule, path)));
  const pathsOf = new Map(units.map(unit => [unit.key, unit.paths]));
  const reported = new Set(findings.map(finding => finding.path));
  return {
    units: variants.length,
    tp: score.tp,
    fp: score.falsePositives.length,
    fn: score.falseNegatives.length,
    falsePositives: score.falsePositives.slice(0, EXAMPLE_LIMIT),
    falseNegatives: score.falseNegatives.slice(0, EXAMPLE_LIMIT),
    engineFailures: total(rerail.engineFailurePaths) + rerail.pathlessEngineFailures,
    incomplete: total(rerail.incompletePaths) + rerail.pathlessIncomplete,
    renamedAway: variants.filter(unit => (pathsOf.get(unit.origin!.renamedFrom!) ?? []).some(path => reported.has(path)) && !unit.paths.some(path => reported.has(path))).length,
  };
}

/**
 * Scores a rule on the samples judged for it (`scoreSamples`). Only the judged files' findings and failures count, and
 * failures naming no file, which are the rule's; a context file is never judged, so its engine failures are listed
 * (`contextFailures`) and fail nothing. A file read in part (a T1 syntax error, `source-partial`, a parse-induced
 * `source-incomplete`) is no engine failure: the sample stays judged, and its labels decide.
 */
function samplesRow(entry: CompiledRule, run: SamplesRun, loaded: LoadedPack, used: Set<Divergence>): RuleRow | undefined {
  const { rule } = entry;
  const samples = run.judged.get(rule.id);
  if (samples === undefined) return undefined;
  const judged = new Set(samples.map(sample => sample.path));
  const rerail = run.actual.get(rule.id) ?? noActual();
  const actual = rerail.points.filter(point => judged.has(point.path)).map(({ path, line }) => ({ path, line }));
  const reported = new Set(actual.map(point => point.path));
  const clean = samples.filter(sample => sample.lines.length === 0);
  const applicable = loaded.divergences.filter(divergence => divergence.rule === rule.id && (divergence.source === undefined || divergence.source === SAMPLES_SOURCE));
  const score = scoreSamples(samples, actual, applicable);
  const readInPart = new Set([...rerail.partialPaths, ...rerail.incompletePaths.keys()].filter(path => judged.has(path)));
  // A clean sample is stays-silent evidence only where the rule applies and read it in full and finished: its scope and
  // a clause's `file` condition admit the file (silence where the rule never looks proves nothing), no report, no read
  // in part, no engine failure there, and no failure naming no file, which leaves every file's answer in doubt.
  const finished = rerail.pathlessEngineFailures + rerail.pathlessIncomplete === 0;
  const admitted = run.admitted.get(rule.id) ?? new Set<string>();
  const silent = clean.filter(
    ({ path }) => finished && admitted.has(path) && !reported.has(path) && !rerail.syntaxErrorFiles.has(path) && !readInPart.has(path) && !rerail.engineFailurePaths.has(path),
  );
  return rowOf(entry, loaded, SAMPLES_SOURCE, {
    score,
    reviewed: score.reviewed,
    cleanSamples: clean.length,
    silentCleanSamples: silent.length,
    judgedSamples: samples.length,
    ownUnits: 0,
    silentOwnUnits: 0,
    upstreamFindings: 0,
    renamed: null,
    engineFailures: total(rerail.engineFailurePaths, path => judged.has(path)) + rerail.pathlessEngineFailures,
    contextFailures: total(rerail.engineFailurePaths, path => !judged.has(path)),
    partialFiles: readInPart.size + rerail.pathlessIncomplete,
    syntaxErrorFiles: [...rerail.syntaxErrorFiles.values()].filter(file => judged.has(file.path)),
  }, used);
}

/** The sources a divergence with no `source` applies to; it is stale only once each of them scored its rule. */
const ALL_SOURCES = ["fixtures", SAMPLES_SOURCE] as const;

/** T4: score the pack's rules over its upstream fixtures and/or its real-code samples. */
export async function runPack(loaded: LoadedPack, options: RunOptions): Promise<T4Report> {
  const { pack } = loaded;
  const errors: string[] = [];
  const unknown = (options.rules ?? []).filter(id => !pack.rules.some(rule => rule.id === id));
  if (unknown.length > 0) throw new Error(`unknown rule(s) in ${pack.pack}: ${unknown.join(", ")}`);
  const selected = options.rules === undefined ? pack.rules : pack.rules.filter(rule => options.rules!.includes(rule.id));
  const compiled = compileRules(loaded, selected, errors);

  const fixtures = options.fixtures && compiled.length > 0 ? await runFixtures(loaded, compiled, errors) : undefined;
  const directoryOf = options.samplesDirectory ?? ((tool: string) => samplesTree({ root: RERAIL_ROOT, pack: pack.pack, tool }));
  const samples = options.samples && compiled.length > 0 ? await runSamples(loaded, compiled, directoryOf, errors) : undefined;

  const used = new Set<Divergence>();
  const rows = [
    ...(fixtures === undefined ? [] : compiled.flatMap(entry => fixturesRow(entry, fixtures, loaded, used, errors) ?? [])),
    ...(samples === undefined ? [] : compiled.flatMap(entry => samplesRow(entry, samples, loaded, used) ?? [])),
  ];
  // A source scored a rule when it ran and had the rule's data. A divergence naming any other source applies to nothing
  // and is never stale.
  const scored = (source: string, rule: string): boolean =>
    source === "fixtures" ? fixtures !== undefined : source === SAMPLES_SOURCE && samples !== undefined && !samples.unscored.has(rule);
  const selectedIds = new Set(selected.map(rule => rule.id));
  const stale = loaded.divergences.filter(
    divergence =>
      selectedIds.has(divergence.rule) &&
      !used.has(divergence) &&
      (divergence.source === undefined ? ALL_SOURCES.every(source => scored(source, divergence.rule)) : scored(divergence.source, divergence.rule)),
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
    tools: pack.tools,
    scope: { rules: selected.map(rule => rule.id), fixtures: options.fixtures, samples: options.samples },
    coverage: {
      rules: pack.rules.length,
      minimumRules: MINIMUM_RULES,
      capabilities,
      uncovered: pack.capabilities.filter(capability => capabilities[capability]!.length === 0 && !exempt.has(capability)),
      exempt: pack.exemptCapabilities,
    },
    sources: [fixtures, samples].flatMap(run => (run === undefined ? [] : [run.summary])),
    rules: rows,
    staleDivergences: stale.map(({ rule, source, path, line, kind }) => ({ rule, ...(source === undefined ? {} : { source }), path, line, kind })),
    errors,
    pass: false,
  };
  report.pass = reportFailures(report).length === 0;
  return report;
}

/**
 * Why a report fails, one line per problem; empty when it passes. On the fixtures, which the port must reproduce, an
 * unexplained FP or FN fails, and so does a shipped port with no fixture TP unless pack.json gives its rule examples:
 * divergences can excuse every finding of its fixtures, and a port that matches nothing must still show it catches
 * something. On the samples, which show a rule works on real code as intended, a missed label fails, as does a finding
 * on a clean sample; a report on a labelled sample's unlabelled line is only reviewed. A sample the grammar reads only
 * in part (a T1 syntax error, `source-partial`, a parse-induced `source-incomplete`: `isParseIncomplete`) is still
 * judged but fails nothing by itself: a sample judges the rule, and a label the partial read hides is a missed label.
 * A sample's context files are never judged, so their engine failures fail nothing either. Other engine failures fail
 * on both. On the fixtures' rename variants, scored apart, an unexplained FP or FN fails too, and so does an engine
 * failure other than a parse-induced `source-incomplete` (holds-under-rename). A candidate's row (a port under
 * `candidates/`) is reported but fails nothing; its errors and stale divergences still do.
 */
export function reportFailures(report: T4Report): string[] {
  return [
    ...report.errors,
    ...report.rules.flatMap(row => {
      const problems: string[] = [];
      if (row.candidate) return problems;
      const where = `${row.rule} (${row.source})`;
      if (row.source === SAMPLES_SOURCE) {
        if (row.fn > 0) problems.push(`${where}: ${row.fn} missed label(s), first ${JSON.stringify(row.falseNegatives[0])}`);
        if (row.fp > 0) problems.push(`${where}: ${row.fp} finding(s) on clean samples, first ${JSON.stringify(row.falsePositives[0])}`);
      } else {
        if (row.fp > 0) problems.push(`${where}: ${row.fp} unexplained FP, first ${JSON.stringify(row.falsePositives[0])}`);
        if (row.fn > 0) problems.push(`${where}: ${row.fn} unexplained FN, first ${JSON.stringify(row.falseNegatives[0])}`);
        if (row.tp === 0 && !row.examples) problems.push(`${where}: 0 TP and no examples in pack.json; nothing shows it catches anything`);
        if (row.renamed !== null && row.renamed.fp > 0) problems.push(`${where}: ${row.renamed.fp} unexplained FP on rename variants, first ${JSON.stringify(row.renamed.falsePositives[0])}`);
        if (row.renamed !== null && row.renamed.fn > 0) problems.push(`${where}: ${row.renamed.fn} unexplained FN on rename variants, first ${JSON.stringify(row.renamed.falseNegatives[0])}`);
        if (row.renamed !== null && row.renamed.engineFailures > 0) problems.push(`${where}: ${row.renamed.engineFailures} engine failure(s) on rename variants`);
        if (row.renamed !== null && row.renamed.tp === 0 && row.renamed.renamedAway > 0 && row.renamed.exempt === undefined) problems.push(`${where}: the renaming took every finding from its ${row.renamed.renamedAway} rename variant(s) and none survives`);
      }
      if (row.engineFailures > 0) problems.push(`${where}: ${row.engineFailures} engine failure(s)`);
      return problems;
    }),
    ...report.staleDivergences.map(divergence => `${divergence.rule}: stale divergence ${divergence.path}:${divergence.line} (${divergence.kind}) explains nothing`),
  ];
}
