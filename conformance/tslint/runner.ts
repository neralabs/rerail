import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { z } from "zod";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl, type NativeCodeRuleDslWorkspace } from "../../src/native.js";
import { bundledCodeRuleDslDirectory } from "../../src/code-rule-dsl-catalog.js";
import type { ExpectedFailure } from "./lint-format.js";
import { checkFix, type EmittedFix, type FixCheck } from "./fix-check.js";
import { portSize, type PortSize } from "./port-size.js";
import { applyRepairs, FIXTURE_REPAIRS, type FixtureRepair } from "./repairs.js";
import { FIXTURE_RULES, loadVariants, TSLINT_PACK, type FixtureFile, type RuleConfig, type RuleInfo, type Variant } from "./fixtures.js";
import { evidenceSchema, goldenDocumentPath, goldenRules, isGoldenRule, loadGoldenManifest, type GoldenEntry } from "./golden.js";
import { protectionFor, renameIdentifiers, syntaxDiagnostics } from "./rename.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

/**
 * The TSLint pack's candidates: non-golden default docs, every rule's option-variant overrides, and non-golden verdict
 * sidecars. Scored, never shipped.
 */
export const COMMITTED_PORTS_DIRECTORY = join(TSLINT_PACK, "candidates");
/**
 * `RERAIL_TSLINT_PORTS` scores a scratch copy of the ports (e.g. drafts using unreleased primitives) without editing
 * `candidates/`. It replaces the non-golden default docs, the overrides and the sidecars; golden default docs always
 * come from the pack's `rules/` (`golden.ts`).
 */
export const PORTS_DIRECTORY = process.env.RERAIL_TSLINT_PORTS === undefined ? COMMITTED_PORTS_DIRECTORY : resolve(process.env.RERAIL_TSLINT_PORTS);

export const MANUAL_VERDICTS = ["BLOCKED", "INFEASIBLE", "AUTO"] as const;
export type AutoVerdict = "PASS" | "PARTIAL" | "COMPILE_ERROR" | "NO_PORT";
export type FinalVerdict = AutoVerdict | Exclude<(typeof MANUAL_VERDICTS)[number], "AUTO">;

const sidecarSchema = z.strictObject({
  verdict: z.enum(MANUAL_VERDICTS),
  /** The one primitive whose absence accounts for the most unmatched findings on valid code (see README). */
  primitive: z.string().min(1).optional(),
  /** Other primitives the port also needs, most decisive first. */
  secondary: z.array(z.string().min(1)).optional(),
  /** For rules that need the type layer: which checker questions the fixture lines ask. */
  needs: z.array(z.enum(["symbol-resolution", "overload-resolution", "expression-type", "lib-types", "compiler-options", "cross-file"])).optional(),
  reason: z.string().min(1),
  evidence: z.array(evidenceSchema),
});
export type VerdictSidecar = z.infer<typeof sidecarSchema>;

/** What the report shows beside a rule's score: a sidecar's notes, or a golden rule's manifest entry. */
export interface RuleAnnotation {
  primitive?: string | undefined;
  secondary?: string[] | undefined;
  needs?: string[] | undefined;
  reason?: string | undefined;
  evidence?: Array<z.infer<typeof evidenceSchema>> | undefined;
}

const spanSchema = z.object({ startByte: z.number().int().nonnegative(), endByte: z.number().int().nonnegative() });
const workspaceResultSchema = z.object({
  evaluation: z.object({
    violations: z.array(
      z.object({
        clauseId: z.string(),
        path: z.string().optional(),
        span: spanSchema.optional(),
        fix: z.object({ span: spanSchema, text: z.string() }).optional(),
      }),
    ),
    failures: z.array(z.object({ clauseId: z.string(), code: z.string(), message: z.string(), path: z.string().optional() })),
  }),
});

export interface ActualViolation {
  startLine: number;
  startCol: number;
  endLine: number;
  endCol: number;
  clauseId: string;
}

export interface FileResult {
  variant: string;
  lintPath: string;
  path: string;
  language: FixtureFile["language"];
  javascript: boolean;
  config: RuleConfig;
  /** Port document evaluated for this file, relative to the harness directory. */
  document: string | undefined;
  /** Why the file was not evaluated: TypeScript version gate, markup syntax error, or rule disabled in tslint.json. */
  skipped?: string;
  /** Outside a golden default doc's shipped scope: not evaluated, and its expected failures are not misses. */
  outOfScope?: true;
  expected: ExpectedFailure[];
  truePositives: number;
  endAgreements: number;
  missed: ExpectedFailure[];
  extra: ActualViolation[];
  endMismatches: Array<{ expected: ExpectedFailure; actual: ActualViolation }>;
  engineFailures: Array<{ clauseId: string; code: string; message: string }>;
  /** `source-partial` message: the file has syntax errors and AST clauses ran only outside them. Not an engine failure. */
  partial?: string | undefined;
  /** Only for ports that declare `violation.fix`, on fixtures that have a TSLint `.fix` file. */
  fix?: FixCheck;
  /** `--rename-identifiers` only: renamer problems for this file (parse changed, names left unrenamed). */
  renameIssues?: string[];
  source: string;
}

export interface EvaluateOptions {
  /** Score on fixtures whose user-chosen identifiers are renamed to opaque same-length names (`rename.ts`). */
  renameIdentifiers?: boolean;
}

interface PreparedSource {
  source: string;
  issues: string[];
}

function prepareSource(rule: string, file: FixtureFile, source: string, options: EvaluateOptions): PreparedSource {
  if (options.renameIdentifiers !== true) return { source, issues: [] };
  const renamed = renameIdentifiers(source, file.path, protectionFor(rule, file.config.options));
  const issues: string[] = [];
  const before = syntaxDiagnostics(source, file.path);
  const after = syntaxDiagnostics(renamed.source, file.path);
  if (before.join(",") !== after.join(",")) issues.push(`TypeScript syntax diagnostics changed: [${before.join(" ")}] -> [${after.join(" ")}]`);
  if (renamed.exhausted.length > 0) issues.push(`no free same-shape name for ${renamed.exhausted.join(", ")}`);
  return { source: renamed.source, issues };
}

/** Score over the same fixtures after `repairs.ts` edits; files without repairs keep their original result. */
export interface RepairedScore {
  repairedFiles: number;
  kinds: Array<FixtureRepair["kind"]>;
  truePositives: number;
  falsePositives: number;
  falseNegatives: number;
  engineFailures: number;
  partialFiles: number;
  droppedExpected: number;
  auto: "PASS" | "PARTIAL";
  errors: string[];
  /** The repaired files only. */
  results: FileResult[];
}

export interface RuleResult {
  rule: string;
  requiresTypeInfo: boolean;
  hasFixer: boolean;
  auto: AutoVerdict;
  verdict: FinalVerdict;
  /** Never read for a golden rule. */
  sidecar: VerdictSidecar | undefined;
  /** The default doc is the pack's (`golden.ts`). */
  golden: boolean;
  /** The rule's conformance.json entry; undefined for a golden rule the manifest does not list yet. */
  goldenEntry: GoldenEntry | undefined;
  compileError: string | undefined;
  notes: string[];
  variants: number;
  files: number;
  skippedFiles: number;
  /** Fixture files (`<variant>/<file>`) outside the golden default doc's shipped scope; never scored. */
  outOfScope: string[];
  truePositives: number;
  falsePositives: number;
  falseNegatives: number;
  endAgreements: number;
  engineFailures: number;
  /** Evaluated files the engine marked `source-partial`; informational, the verdict comes from TP/FP/FN. */
  partialFiles: number;
  precision: number | null;
  recall: number | null;
  evalMs: number | null;
  /** Bytes of the default doc plus every override doc. */
  documentBytes: number;
  size: PortSize;
  repaired: RepairedScore | undefined;
  /** `.fix` files whose text the port's emitted fixes reproduce, when the port declares `violation.fix`. */
  fixes: { files: number; matched: number } | undefined;
  results: FileResult[];
  variantInfo: Array<Pick<Variant, "id" | "config" | "otherRules" | "typeChecked">>;
}

export interface RulePort {
  /** The pack doc for a golden rule, else `candidates/<rule>.json`. */
  document: string | undefined;
  golden: boolean;
  /** Variant id (e.g. `single`, `tags/content`) to the override document for that variant. */
  overrides: Map<string, string>;
  /** Never set for a golden rule: conformance.json replaces its sidecar. */
  sidecar: string | undefined;
}

/** Verdict after fixture artifacts are repaired: a rule whose only blocker was invalid fixture syntax is a PASS. */
export type EffectiveVerdict = FinalVerdict | "PASS (repaired fixture)";

export function effectiveVerdict(rule: Pick<RuleResult, "verdict" | "repaired">): EffectiveVerdict {
  if (rule.verdict === "PASS") return "PASS";
  const repaired = rule.repaired;
  if (repaired !== undefined && repaired.auto === "PASS" && repaired.kinds.every(kind => kind === "fixture-artifact")) return "PASS (repaired fixture)";
  return rule.verdict;
}

export function annotationOf(rule: Pick<RuleResult, "sidecar" | "goldenEntry">): RuleAnnotation | undefined {
  return rule.sidecar ?? rule.goldenEntry;
}

const toPosix = (path: string): string => path.split(sep).join("/");

export function findPort(rule: string): RulePort {
  const golden = isGoldenRule(rule);
  const document = golden ? goldenDocumentPath(rule) : join(PORTS_DIRECTORY, `${rule}.json`);
  const sidecar = join(PORTS_DIRECTORY, `${rule}.verdict.json`);
  const overrideDirectory = join(PORTS_DIRECTORY, rule);
  const overrides = new Map<string, string>();
  const walk = (directory: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.name.endsWith(".json")) overrides.set(toPosix(relative(overrideDirectory, path)).replace(/\.json$/, ""), path);
    }
  };
  if (existsSync(overrideDirectory)) walk(overrideDirectory);
  return {
    document: existsSync(document) ? document : undefined,
    golden,
    overrides,
    sidecar: !golden && existsSync(sidecar) ? sidecar : undefined,
  };
}

/** Rules with a default doc: the golden set plus the non-golden ports. */
export function listPortedRules(): string[] {
  const ports = existsSync(PORTS_DIRECTORY)
    ? readdirSync(PORTS_DIRECTORY)
        .filter(name => name.endsWith(".json") && !name.endsWith(".verdict.json"))
        .map(name => name.replace(/\.json$/, ""))
    : [];
  return [...new Set([...goldenRules(), ...ports])].sort();
}

let capabilities: string | undefined;
export function capabilitiesJson(): string {
  capabilities ??= readFileSync(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  return capabilities;
}

export function compileDocument(path: string): NativeCompiledCodeRuleDsl {
  return loadNativeBinding().CodeRuleDslRuntime.compile(readFileSync(path, "utf8"), "json", capabilitiesJson());
}

/** Mirrors the enforcer: inline `tryApply` first, `applyAsync` when the workspace is busy. */
export async function applyToWorkspace(workspace: NativeCodeRuleDslWorkspace, payload: string): Promise<string> {
  return workspace.tryApply(payload) ?? (await workspace.applyAsync(payload));
}

function positionAt(source: Buffer, byte: number): { line: number; col: number } {
  const prefix = source.subarray(0, byte).toString("utf8");
  const lineStart = prefix.lastIndexOf("\n") + 1;
  let line = 0;
  for (let index = prefix.indexOf("\n"); index !== -1; index = prefix.indexOf("\n", index + 1)) line += 1;
  return { line, col: prefix.length - lineStart };
}

interface FileEvaluation {
  violations: ActualViolation[];
  fixes: EmittedFix[];
  failures: FileResult["engineFailures"];
  partial: string | undefined;
  ms: number;
}

/**
 * `source-partial` says the file has syntax errors and AST witnesses were reported only outside them. That is
 * the engine working as designed, so it is not an engine failure: missed violations inside the error region already
 * show up as FNs. `source-incomplete` (a rule unsound under errors skipped the file) stays an engine failure.
 */
export const INFORMATIONAL_FAILURE_CODES: ReadonlySet<string> = new Set(["source-partial"]);

/** Failure codes meaning the grammar could not parse the whole file, whatever the engine then did with it. */
export const PARSE_FAILURE_CODES: ReadonlySet<string> = new Set(["source-incomplete", "source-partial"]);

/** Each fixture is its own isolated workspace, as TSLint lints each fixture alone; a syntax oddity in one cannot poison another. */
async function evaluateFixture(compiled: NativeCompiledCodeRuleDsl, file: FixtureFile, source: string): Promise<FileEvaluation> {
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  try {
    workspace.addCodeRule(compiled);
    const payload = JSON.stringify({ revision: 1, files: [{ path: file.path, language: file.language, source }], removed: [] });
    const started = performance.now();
    const raw = await applyToWorkspace(workspace, payload);
    const ms = performance.now() - started;
    const { evaluation } = workspaceResultSchema.parse(JSON.parse(raw));
    const bytes = Buffer.from(source, "utf8");
    const violations = evaluation.violations.flatMap(violation => {
      if (violation.span === undefined || (violation.path !== undefined && violation.path !== file.path)) return [];
      const start = positionAt(bytes, violation.span.startByte);
      const end = positionAt(bytes, violation.span.endByte);
      return [{ startLine: start.line, startCol: start.col, endLine: end.line, endCol: end.col, clauseId: violation.clauseId }];
    });
    const fixes = evaluation.violations.flatMap(violation =>
      violation.fix === undefined || (violation.path !== undefined && violation.path !== file.path)
        ? []
        : [{ startByte: violation.fix.span.startByte, endByte: violation.fix.span.endByte, text: violation.fix.text }],
    );
    const failures = evaluation.failures
      .filter(failure => !INFORMATIONAL_FAILURE_CODES.has(failure.code))
      .map(({ clauseId, code, message }) => ({ clauseId, code, message }));
    const partial = evaluation.failures.find(failure => INFORMATIONAL_FAILURE_CODES.has(failure.code))?.message;
    return { violations, fixes, failures, partial, ms };
  } finally {
    workspace.close();
  }
}

function compare(expected: ExpectedFailure[], actual: ActualViolation[]): Pick<FileResult, "truePositives" | "endAgreements" | "missed" | "extra" | "endMismatches"> {
  const key = (line: number, col: number): string => `${line}:${col}`;
  const actualByStart = new Map<string, ActualViolation[]>();
  for (const violation of actual) {
    const bucket = actualByStart.get(key(violation.startLine, violation.startCol)) ?? [];
    bucket.push(violation);
    actualByStart.set(key(violation.startLine, violation.startCol), bucket);
  }
  const missed: ExpectedFailure[] = [];
  const endMismatches: FileResult["endMismatches"] = [];
  let truePositives = 0;
  let endAgreements = 0;
  for (const failure of expected) {
    const bucket = actualByStart.get(key(failure.startLine, failure.startCol));
    if (bucket === undefined || bucket.length === 0) {
      missed.push(failure);
      continue;
    }
    truePositives += 1;
    const sameEnd = bucket.findIndex(violation => violation.endLine === failure.endLine && violation.endCol === failure.endCol);
    if (sameEnd !== -1) {
      endAgreements += 1;
      bucket.splice(sameEnd, 1);
    } else {
      endMismatches.push({ expected: failure, actual: bucket.shift()! });
    }
  }
  const extra = [...actualByStart.values()].flat().sort((a, b) => a.startLine - b.startLine || a.startCol - b.startCol);
  return { truePositives, endAgreements, missed, extra, endMismatches };
}

/** Whether any clause of a port document declares `violation.fix`. */
function declaresFix(path: string): boolean {
  const document = z.object({ clauses: z.array(z.object({ violation: z.object({ fix: z.string().optional() }).passthrough() }).passthrough()) }).passthrough();
  return document.parse(JSON.parse(readFileSync(path, "utf8"))).clauses.some(clause => clause.violation.fix !== undefined);
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)] ?? 0;
}

export function loadSidecar(path: string | undefined): { sidecar: VerdictSidecar | undefined; error: string | undefined } {
  if (path === undefined) return { sidecar: undefined, error: undefined };
  try {
    return { sidecar: sidecarSchema.parse(JSON.parse(readFileSync(path, "utf8"))), error: undefined };
  } catch (error) {
    return { sidecar: undefined, error: `invalid verdict sidecar: ${error instanceof Error ? error.message.replace(/\s+/g, " ") : String(error)}` };
  }
}

const ratio = (numerator: number, denominator: number): number | null => (denominator === 0 ? null : numerator / denominator);

export async function evaluateRule(info: RuleInfo, runs: number, options: EvaluateOptions = {}): Promise<RuleResult> {
  const port = findPort(info.name);
  const variants = loadVariants(info.name);
  const { sidecar, error: sidecarError } = loadSidecar(port.sidecar);
  const notes: string[] = [];
  if (sidecarError !== undefined) notes.push(sidecarError);
  const variantIds = new Set(variants.map(variant => variant.id));
  for (const id of port.overrides.keys()) if (!variantIds.has(id)) notes.push(`override rules/${info.name}/${id}.json matches no fixture variant`);
  if (variants.length === 0) notes.push("no fixtures found");
  for (const variant of variants) {
    if (variant.otherRules.length > 0) notes.push(`variant ${variant.id} also enables ${variant.otherRules.join(", ")}`);
    if (variant.files.some(file => file.javascript)) notes.push(`variant ${variant.id} has .js fixtures, evaluated with the JavaScript grammar`);
  }

  const size = portSize(port.document === undefined ? undefined : readFileSync(port.document, "utf8"), [...port.overrides.values()].map(path => readFileSync(path, "utf8")));
  const base = {
    rule: info.name,
    documentBytes: size.totalBytes,
    size,
    requiresTypeInfo: info.requiresTypeInfo,
    hasFixer: info.hasFixer,
    sidecar,
    golden: port.golden,
    goldenEntry: port.golden ? loadGoldenManifest().rules[info.name] : undefined,
    notes,
    variants: variants.length,
    files: variants.reduce((total, variant) => total + variant.files.length, 0),
    variantInfo: variants.map(({ id, config, otherRules, typeChecked }) => ({ id, config, otherRules, typeChecked })),
  };
  const finish = (auto: AutoVerdict, rest: Partial<RuleResult>): RuleResult => {
    const manual = sidecar !== undefined && sidecar.verdict !== "AUTO" ? sidecar.verdict : undefined;
    if (manual !== undefined && auto === "PASS") notes.push(`sidecar says ${manual} but the port passes every fixture`);
    return {
      ...base,
      auto,
      verdict: manual ?? auto,
      compileError: undefined,
      skippedFiles: 0,
      outOfScope: [],
      truePositives: 0,
      falsePositives: 0,
      falseNegatives: 0,
      endAgreements: 0,
      engineFailures: 0,
      partialFiles: 0,
      precision: null,
      recall: null,
      evalMs: null,
      repaired: undefined,
      fixes: undefined,
      results: [],
      ...rest,
    };
  };

  if (port.document === undefined && port.overrides.size === 0) return finish("NO_PORT", {});

  const compiled = new Map<string, NativeCompiledCodeRuleDsl>();
  const compileErrors: string[] = [];
  for (const path of [port.document, ...port.overrides.values()]) {
    if (path === undefined) continue;
    try {
      compiled.set(path, compileDocument(path));
    } catch (error) {
      compileErrors.push(`${toPosix(relative(TSLINT_PACK, path))}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  if (compileErrors.length > 0) return finish("COMPILE_ERROR", { compileError: compileErrors.join("; ") });

  const fixingDocuments = new Set([port.document, ...port.overrides.values()].filter((path): path is string => path !== undefined && declaresFix(path)));
  const results: FileResult[] = [];
  const timings: number[] = Array.from({ length: runs }, () => 0);
  const ruleRepairs = FIXTURE_REPAIRS[info.name] ?? [];
  const usedRepairs = new Set<FixtureRepair>();
  const repairedFiles: Array<{ original: FileResult; repaired: FileResult; dropped: number }> = [];
  const repairErrors: string[] = [];
  for (const variant of variants) {
    const documentPath = port.overrides.get(variant.id) ?? port.document;
    const document = documentPath === undefined ? undefined : toPosix(relative(TSLINT_PACK, documentPath));
    for (const file of variant.files) {
      const common = {
        variant: variant.id,
        lintPath: file.lintPath,
        path: file.path,
        language: file.language,
        javascript: file.javascript,
        config: file.config,
        document,
        truePositives: 0,
        endAgreements: 0,
        missed: [],
        extra: [],
        endMismatches: [],
        engineFailures: [],
      };
      if (file.parsed.skipped) {
        const reason = "parseError" in file.parsed ? `markup error: ${file.parsed.parseError}` : `requires typescript ${file.parsed.requirement}`;
        results.push({ ...common, skipped: reason, expected: [], source: "" });
        continue;
      }
      const { failures: expected } = file.parsed;
      const prepared = prepareSource(info.name, file, file.parsed.source, options);
      const { source } = prepared;
      if (!file.config.enabled) {
        results.push({ ...common, skipped: "rule disabled for this file type in tslint.json", expected, source });
        continue;
      }
      const codeRule = documentPath === undefined ? undefined : compiled.get(documentPath);
      if (codeRule === undefined) {
        results.push({ ...common, expected, missed: expected, source });
        continue;
      }
      if (port.golden && documentPath === port.document && !codeRule.inScope(file.path)) {
        results.push({ ...common, outOfScope: true, expected, source });
        continue;
      }
      let first: FileEvaluation | undefined;
      for (let run = 0; run < runs; run += 1) {
        const evaluation = await evaluateFixture(codeRule, file, source);
        first ??= evaluation;
        timings[run]! += evaluation.ms;
      }
      if (first === undefined) throw new Error("--runs must be at least 1");
      const fixPath = join(FIXTURE_RULES, info.name, variant.id, file.lintPath.replace(/\.lint$/, ".fix"));
      const fixable = options.renameIdentifiers !== true && documentPath !== undefined && fixingDocuments.has(documentPath) && existsSync(fixPath);
      const fix = fixable ? checkFix(source, first.fixes, readFileSync(fixPath, "utf8")) : undefined;
      const original: FileResult = {
        ...common,
        expected,
        ...compare(expected, first.violations),
        engineFailures: first.failures,
        partial: first.partial,
        ...(fix === undefined ? {} : { fix }),
        ...(prepared.issues.length === 0 ? {} : { renameIssues: prepared.issues }),
        source,
      };
      results.push(original);
      const repairs = ruleRepairs.filter(repair => repair.variant === variant.id && repair.file === file.lintPath);
      if (repairs.length === 0) continue;
      for (const repair of repairs) usedRepairs.add(repair);
      const fixed = applyRepairs(file.parsed.source, expected, repairs);
      repairErrors.push(...fixed.errors.map(message => `${variant.id}/${file.lintPath} ${message}`));
      const repairedSource = prepareSource(info.name, file, fixed.source, options).source;
      const evaluation = await evaluateFixture(codeRule, file, repairedSource);
      repairedFiles.push({
        original,
        repaired: { ...common, expected: fixed.expected, ...compare(fixed.expected, evaluation.violations), engineFailures: evaluation.failures, partial: evaluation.partial, source: repairedSource },
        dropped: fixed.dropped,
      });
    }
  }
  for (const repair of ruleRepairs) if (!usedRepairs.has(repair)) repairErrors.push(`repair ${repair.variant}/${repair.file}:${repair.line} matches no evaluated fixture`);
  if (repairErrors.length > 0) notes.push(`repair errors: ${repairErrors.join("; ")}`);

  const evaluated = results.filter(result => result.skipped === undefined && result.outOfScope === undefined);
  const sum = (pick: (result: FileResult) => number): number => evaluated.reduce((total, result) => total + pick(result), 0);
  const truePositives = sum(result => result.truePositives);
  const falsePositives = sum(result => result.extra.length);
  const falseNegatives = sum(result => result.missed.length);
  const engineFailures = sum(result => result.engineFailures.length);
  const auto: AutoVerdict = falsePositives === 0 && falseNegatives === 0 && engineFailures === 0 && evaluated.length > 0 ? "PASS" : "PARTIAL";
  let repaired: RepairedScore | undefined;
  if (ruleRepairs.length > 0) {
    const replaced = new Map(repairedFiles.map(entry => [entry.original, entry.repaired]));
    const scored = evaluated.map(result => replaced.get(result) ?? result);
    const total = (pick: (result: FileResult) => number): number => scored.reduce((sum, result) => sum + pick(result), 0);
    const score = {
      truePositives: total(result => result.truePositives),
      falsePositives: total(result => result.extra.length),
      falseNegatives: total(result => result.missed.length),
      engineFailures: total(result => result.engineFailures.length),
      partialFiles: total(result => (result.partial === undefined ? 0 : 1)),
    };
    repaired = {
      repairedFiles: repairedFiles.length,
      kinds: [...new Set(ruleRepairs.map(repair => repair.kind))].sort(compareDeterministicStrings),
      ...score,
      droppedExpected: repairedFiles.reduce((sum, entry) => sum + entry.dropped, 0),
      auto: score.falsePositives === 0 && score.falseNegatives === 0 && score.engineFailures === 0 && repairErrors.length === 0 ? "PASS" : "PARTIAL",
      errors: repairErrors,
      results: repairedFiles.map(entry => entry.repaired),
    };
  }
  const fixChecks = evaluated.flatMap(result => (result.fix === undefined ? [] : [result.fix]));
  return finish(auto, {
    repaired,
    fixes: fixChecks.length === 0 ? undefined : { files: fixChecks.length, matched: fixChecks.filter(check => check.matched).length },
    results,
    skippedFiles: results.filter(result => result.skipped !== undefined).length,
    outOfScope: results.filter(result => result.outOfScope !== undefined).map(result => `${result.variant}/${result.lintPath}`),
    truePositives,
    falsePositives,
    falseNegatives,
    endAgreements: sum(result => result.endAgreements),
    engineFailures,
    partialFiles: sum(result => (result.partial === undefined ? 0 : 1)),
    precision: ratio(truePositives, truePositives + falsePositives),
    recall: ratio(truePositives, truePositives + falseNegatives),
    evalMs: median(timings),
  });
}
