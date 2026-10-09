// `just rerail-query-parity`: run the same code-rule workloads through two (or more) native query-engine addons,
// each in its own process, and report every section where their findings, failures or coverage differ.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import {
  languageFor,
  loadBundledRules,
  loadNativeBinding,
  resolveNativeBindingPath,
  type NativeBinding,
  type NativeCompiledCodeRuleDsl,
} from "../src/engine/index.js";

import { isSemanticWorkload, semanticWorkloads, type SemanticWorkload } from "./query-parity-semantic.js";
import { fixturesPath } from "../test/support/oracles.js";

const script = fileURLToPath(import.meta.url);
const rerailRoot = resolve(dirname(script), "..");
const catalogRoot = resolve(rerailRoot, "native", "src", "code_rule_dsl");
const workloads = ["corpus", ...semanticWorkloads, "edits", "relation-order", "select-when"] as const;
type Workload = typeof workloads[number];
type Evaluator = "workspace" | "stateless" | "stateless-per-file";
type SourceFile = { path: string; language: string; source: string };
type Side = { name: string; addon: string; environment: Record<string, string> };
type RuleDocument = { path: string; source: string; clauses: Array<{ id: string }> };
/** The repository files a corpus workload reads: a directory and a JSON array of paths relative to it. */
type Corpus = { root: string; manifest: string };

const rangeSchema = z.strictObject({ startByte: z.number().int(), endByte: z.number().int(), startLine: z.number().int(), startColumn: z.number().int(), endLine: z.number().int(), endColumn: z.number().int() });
const violationSchema = z.object({ id: z.string(), ruleId: z.string(), clauseId: z.string(), code: z.string(), mode: z.string(), message: z.string(), path: z.string().optional(), span: rangeSchema.optional(), captures: z.record(z.string(), z.array(rangeSchema)).default({}), variables: z.record(z.string(), z.union([z.string(), z.number(), z.boolean()])).default({}) }).passthrough();
const failureSchema = z.object({ ruleId: z.string().optional(), clauseId: z.string(), code: z.string(), message: z.string(), path: z.string().optional() }).passthrough();
const evaluationSchema = z.object({ status: z.string(), violations: z.array(violationSchema), failures: z.array(failureSchema) }).passthrough();
const workspaceSchema = z.object({ revision: z.number().int(), evaluation: evaluationSchema }).passthrough();
const clausesSchema = z.object({ clauses: z.array(z.object({ id: z.string() }).passthrough()) });

interface Options { sides: Side[]; selected: Workload[]; corpus?: Corpus; output?: string; child: boolean; nativeEvaluate: "workspace" | "stateless" }

/** Raised for invalid invocations; main prints it with the usage text. */
class UsageError extends Error {}

function sha256(value: string | Buffer): string { return `sha256:${createHash("sha256").update(value).digest("hex")}`; }
function compare(left: string, right: string): number {
  if (left < right) return -1;
  return left > right ? 1 : 0;
}
function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value !== null && typeof value === "object") return Object.fromEntries(Object.entries(value).sort(([a], [b]) => compare(a, b)).map(([key, child]) => [key, canonical(child)]));
  return value;
}
function encoded(value: unknown): string { return JSON.stringify(canonical(value)); }
function section<T>(value: T): Section<T> { const json = encoded(value); return { sha256: sha256(json), count: Array.isArray(value) ? value.length : undefined, value }; }
function language(path: string): string {
  const selected = languageFor(path);
  if (selected === undefined) throw new Error(`No Rerail language for ${path}`);
  return selected;
}
function corpusFiles(corpus: Corpus | undefined): SourceFile[] {
  if (corpus === undefined) throw new UsageError("this workload needs a corpus: pass --corpus <dir> --manifest <paths.json>");
  const paths = z.array(z.string()).parse(JSON.parse(readFileSync(corpus.manifest, "utf8"))).slice().sort(compare);
  if (paths.length === 0) throw new Error(`${corpus.manifest} lists no files`);
  return paths.map(path => ({ path, language: language(path), source: readFileSync(resolve(corpus.root, path), "utf8") }));
}
function readDocument(path: string, file: string): RuleDocument {
  const source = readFileSync(file, "utf8");
  return { path, source, clauses: clausesSchema.parse(JSON.parse(source)).clauses };
}
function readDocuments(directory: string, prefix = ""): RuleDocument[] {
  const documents: RuleDocument[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => compare(a.name, b.name))) {
    const child = resolve(directory, entry.name); const childPath = prefix === "" ? entry.name : `${prefix}/${entry.name}`;
    if (entry.isDirectory()) documents.push(...readDocuments(child, childPath));
    else if (entry.isFile() && entry.name.endsWith(".json")) documents.push(readDocument(childPath, child));
  }
  return documents;
}
/** Small hand-built workloads for behaviour repository corpora rarely exercise. */
const synthetic = (() => {
  const violation = (code: string, message: string) => ({ anchor: "witness.span", code, message });
  const regex = (pattern: string) => ({ regex: { source: "file.content", pattern } });
  const ast = (rule: object) => ({ ast: { language: "typescript", rule } });
  const debuggerStatement = ast({ kind: "debugger_statement" });
  const gate = (id: string, exists: object, select: object) => ({ id, forbid: { select_when: { exists, select } }, violation: violation("GATE", id) });
  return {
    // A relation binds its first candidate, and a later comparison reads that capture.
    "relation-order": {
      clauses: [{
        id: "first-argument",
        forbid: ast({
          kind: "call_expression",
          all: [
            { has: { field: "function", pattern: "$FN" } },
            { has: { field: "arguments", has: { kind: "identifier", pattern: "$ARG" } } },
            { compare: { left: "FN", op: "eq", right: "ARG", text: "source" } },
          ],
        }),
        violation: violation("FIRST", "$ARG"),
      }],
      files: [{ path: "order.ts", language: "typescript", source: "f(f, g); f(g, f);" }],
    },
    // `select_when` gates: regex and AST sides, a condition nobody meets, an AST condition read
    // around syntax errors (partial files before and after the deciding witness), a condition
    // unsound under syntax errors whose broken files precede its witness in snapshot order but
    // not in path order, and gates nested on both sides. Files are given out of path order.
    "select-when": {
      clauses: [
        gate("regex-gates-ast", regex("danger\\("), debuggerStatement),
        gate("never-met", regex("never_present\\("), debuggerStatement),
        gate("ast-gates-regex", debuggerStatement, regex("danger\\(")),
        gate("unsound-condition", ast({ kind: "debugger_statement", not: { follows: { kind: "import_statement", stopBy: "end" } } }), regex("danger\\(")),
        gate("nested", { select_when: { exists: debuggerStatement, select: regex("danger\\(") } }, { select_when: { exists: regex("debugger"), select: debuggerStatement } }),
      ],
      files: [
        { path: "z-broken.ts", language: "typescript", source: "debugger;\nfunction h( {\n" },
        { path: "c.ts", language: "typescript", source: "debugger;\nclass C {}\n" },
        { path: "a.ts", language: "typescript", source: "danger();\n" },
        { path: "0-broken.ts", language: "typescript", source: "function f( {\n" },
        { path: "b-broken.ts", language: "typescript", source: "debugger;\nfunction g( {\n" },
      ],
    },
  } satisfies Record<string, { clauses: Array<{ id: string; [key: string]: unknown }>; files: SourceFile[] }>;
})();
type SyntheticWorkload = keyof typeof synthetic;
type CatalogKind = "bundled" | SemanticWorkload | SyntheticWorkload;
function isSynthetic(kind: string): kind is SyntheticWorkload { return Object.hasOwn(synthetic, kind); }

/** The bundled capabilities. The reference engine may still require `maxGraphDepth`, which the candidate ignores. */
function capabilities(): string {
  const bundled = z.object({ limits: z.record(z.string(), z.number()) }).passthrough().parse(JSON.parse(readFileSync(resolve(catalogRoot, "capabilities.json"), "utf8")));
  return JSON.stringify({ ...bundled, limits: { maxGraphDepth: 16, ...bundled.limits } });
}

/** `bundled` is every code rule a project can add (the packs core ships; core has no rules of its own); semantic kinds read `fixtures/host/benchmark/<kind>`. */
function catalog(kind: CatalogKind): { capabilities: string; documents: RuleDocument[] } {
  if (isSynthetic(kind)) {
    const { clauses } = synthetic[kind];
    return {
      capabilities: capabilities(),
      documents: [{ path: `${kind}.json`, clauses, source: JSON.stringify({
        format: "rerail.policy/v1", id: kind, revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] }, clauses,
      }) }],
    };
  }
  if (kind === "bundled") {
    const documents = loadBundledRules().flatMap(document => document.kind === "code" ? [document.rule] : []).map(rule => {
      const source = JSON.stringify(rule);
      return { path: `${rule.id}.json`, source, clauses: clausesSchema.parse(rule).clauses };
    });
    return { capabilities: capabilities(), documents: documents.sort((a, b) => compare(a.path, b.path)) };
  }
  return { capabilities: capabilities(), documents: readDocuments(fixturesPath("host", "benchmark", kind)).sort((a, b) => compare(a.path, b.path)) };
}
export function coverage(documents: RuleDocument[], failures: readonly z.infer<typeof failureSchema>[]) {
  const byClause = new Map<string, z.infer<typeof failureSchema>[]>();
  for (const failure of failures) {
    if (failure.ruleId === undefined) continue;
    const key = `${failure.ruleId}\u0000${failure.clauseId}`;
    const grouped = byClause.get(key);
    if (grouped === undefined) byClause.set(key, [failure]);
    else grouped.push(failure);
  }
  return documents.flatMap(document => {
    const policyId = z.object({ id: z.string() }).parse(JSON.parse(document.source)).id;
    return document.clauses.map(clause => {
      const listed = byClause.get(`${policyId}\u0000${clause.id}`) ?? [];
      let status: "complete" | "failed" | "partial" = "complete";
      if (listed.some(failure => failure.code === "source-partial")) status = "partial";
      else if (listed.length > 0) status = "failed";
      return { policyId, checkId: clause.id, status, failureCount: listed.length };
    });
  }).sort((a, b) => compare(`${a.policyId}\u0000${a.checkId}`, `${b.policyId}\u0000${b.checkId}`));
}
function canonicalViolations(violations: readonly z.infer<typeof violationSchema>[]) {
  return violations.map(violation => canonical(violation)).sort((a, b) => compare(encoded(a), encoded(b)));
}
function baselineProjection(violations: readonly z.infer<typeof violationSchema>[]) {
  return violations.map(violation => ({
    id: violation.id, policyId: violation.ruleId, checkId: violation.clauseId, path: violation.path,
    range: violation.span === undefined ? undefined : {
      endByte: violation.span.endByte, endColumn: violation.span.endColumn, endLine: violation.span.endLine,
      startByte: violation.span.startByte, startColumn: violation.span.startColumn, startLine: violation.span.startLine,
    },
    message: violation.message,
  })).sort((a, b) => compare(a.id, b.id));
}
type Edit = { name: string; files: SourceFile[]; removed: string[] };
interface Section<T> { sha256: string; count: number | undefined; value: T }
/** One evaluation in comparable form: canonical, order-independent findings, failures and per-clause coverage. */
interface Summary { status: string; findings: Section<unknown[]>; failures: Section<unknown[]>; coverage: Section<unknown[]>; rawFindingSha256: string }
function summarize(applicable: readonly RuleDocument[], raw: unknown): Summary {
  const evaluation = evaluationSchema.parse(raw);
  const failures = evaluation.failures.map(failure => canonical(failure)).sort((a, b) => compare(encoded(a), encoded(b)));
  return {
    status: evaluation.status,
    findings: section(canonicalViolations(evaluation.violations)),
    failures: section(failures),
    coverage: section(coverage([...applicable], evaluation.failures)),
    rawFindingSha256: sha256(JSON.stringify(evaluation.violations)),
  };
}
function statelessEvaluation(files: SourceFile[], applicable: Array<{ source: string; codeRule: { inScope(path: string): boolean; evaluate(input: string, mode: string): string } }>, mode: "stateless" | "stateless-per-file"): unknown {
  const evaluations: Array<{ policyId: string; evaluation: z.infer<typeof evaluationSchema> }> = [];
  const inputs = mode === "stateless-per-file" ? files.map(file => [file]) : [files];
  for (const snapshotFiles of inputs) {
    const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: snapshotFiles });
    for (const entry of applicable) {
      if (mode === "stateless-per-file" && !entry.codeRule.inScope(snapshotFiles[0]!.path)) continue;
      evaluations.push({ policyId: z.object({ id: z.string() }).parse(JSON.parse(entry.source)).id, evaluation: evaluationSchema.parse(JSON.parse(entry.codeRule.evaluate(input, "report"))) });
    }
  }
  return {
    status: evaluations.some(entry => entry.evaluation.status !== "completed") ? "failed" : "completed",
    violations: evaluations.flatMap(entry => entry.evaluation.violations),
    failures: evaluations.flatMap(entry => entry.evaluation.failures.map(failure => ({ ...failure, ruleId: failure.ruleId ?? entry.policyId }))),
  };
}

/** The initial apply of `files`, then one apply per edit, in one incremental workspace. */
async function workspaceEvaluation(binding: NativeBinding, files: SourceFile[], applicable: Array<{ codeRule: NativeCompiledCodeRuleDsl }>, edits: readonly Edit[]): Promise<unknown[]> {
  const workspace = new binding.CodeRuleDslWorkspace();
  try {
    for (const entry of applicable) workspace.addCodeRule(entry.codeRule);
    const results: unknown[] = [];
    let revision = 1;
    const initial: Edit = { name: "initial", files, removed: [] };
    for (const apply of [initial, ...edits]) {
      const response = await workspace.applyAsync(JSON.stringify({ revision: revision++, files: apply.files, removed: apply.removed }));
      results.push(workspaceSchema.parse(JSON.parse(response)).evaluation);
    }
    return results;
  } finally { workspace.close(); }
}

/** One summary per apply: the initial evaluation, then one per edit (workspace evaluator only). */
async function evaluate(files: SourceFile[], catalogKind: CatalogKind, mode: Evaluator, edits: readonly Edit[] = []): Promise<Summary[]> {
  const loaded = catalog(catalogKind); const binding = loadNativeBinding();
  const compiled = loaded.documents.map(document => ({ ...document, codeRule: binding.CodeRuleDslRuntime.compile(document.source, "json", loaded.capabilities) }));
  const applicable = compiled.filter(entry => files.some(file => entry.codeRule.inScope(file.path)) || edits.some(edit => edit.files.some(file => entry.codeRule.inScope(file.path))));
  if (mode === "workspace") return (await workspaceEvaluation(binding, files, applicable, edits)).map(raw => summarize(applicable, raw));
  if (edits.length > 0) throw new Error("--native-evaluate stateless does not support edits; stateless has no incremental workspace");
  return [summarize(applicable, statelessEvaluation(files, applicable, mode))];
}
async function evaluateOnce(files: SourceFile[], catalogKind: CatalogKind, mode: Evaluator): Promise<Summary> {
  return (await evaluate(files, catalogKind, mode))[0]!;
}
/** A function of cyclomatic complexity 7, above the bundled complexity rules' limit of 6. */
const COMPLEX_FUNCTION = [
  "export function run(a: number): number {",
  ...[1, 2, 3, 4, 5, 6].map(limit => `  if (a > ${limit}) return ${limit};`),
  "  return 0;",
  "}",
  "",
].join("\n");
/** Incremental applies over the corpus: a comment edit to its first file, then a TypeScript function the bundled complexity rule flags. */
function editScript(files: SourceFile[]): Edit[] {
  const real = files[0]; if (real === undefined) throw new Error("Empty corpus");
  const comment = real.language === "python" ? "# parity comment\n" : "// parity comment\n";
  const added = "src/__rerail_query_parity__.ts";
  const renamed = "src/__rerail_query_parity_renamed__.ts";
  const file = (path: string, source: string): SourceFile => ({ path, language: "typescript", source });
  return [
    { name: "comment-add", files: [{ ...real, source: `${comment}${real.source}` }], removed: [] },
    { name: "comment-remove", files: [real], removed: [] },
    { name: "violation-add", files: [file(added, COMPLEX_FUNCTION)], removed: [] },
    { name: "violation-remove", files: [file(added, "export function run(): void {}\n")], removed: [] },
    { name: "syntax-break", files: [file(added, "export function run( {\n")], removed: [] },
    { name: "syntax-repair", files: [file(added, "export function run(): void {}\n")], removed: [] },
    { name: "rename", files: [file(renamed, "export function run(): void {}\n")], removed: [added] },
    { name: "delete", files: [], removed: [renamed] },
  ];
}
async function runWorkload(workload: Workload, mode: Evaluator, corpus: Corpus | undefined) {
  if (isSynthetic(workload)) {
    const { files } = synthetic[workload];
    return { files: files.length, ...await evaluateOnce(files, workload, mode) };
  }
  const files = corpusFiles(corpus);
  if (workload === "corpus") {
    const result = await evaluateOnce(files, "bundled", mode);
    const baseline = baselineProjection(z.array(violationSchema).parse(result.findings.value));
    return { files: files.length, ...result, baseline: { sha256: sha256(`${JSON.stringify(baseline, null, 2)}\n`), count: baseline.length, value: baseline } };
  }
  if (isSemanticWorkload(workload)) return { files: files.length, ...await evaluateOnce(files, workload, mode) };
  return { files: files.length, applies: await evaluate(files, "bundled", mode, editScript(files)) };
}
function parseSide(raw: string): Side {
  const [first, ...assignments] = raw.split(",");
  const split = first?.indexOf("=") ?? -1;
  if (split <= 0) {
    throw new UsageError(`Invalid --side ${raw}; expected name=/absolute/addon.node[,ENV=VALUE...]`);
  }
  const name = first!.slice(0, split);
  const addon = first!.slice(split + 1);
  if (!isAbsolute(addon)) {
    throw new UsageError(`Addon for ${name} must be absolute`);
  }
  const environment: Record<string, string> = {};
  for (const assignment of assignments) {
    const index = assignment.indexOf("=");
    if (index <= 0) {
      throw new UsageError(`Invalid side environment ${assignment}`);
    }
    environment[assignment.slice(0, index)] = assignment.slice(index + 1);
  }
  return { name, addon, environment };
}

interface ParseState { sides: Side[]; selected: Workload[]; corpusRoot?: string; manifest?: string; output?: string; nativeEvaluate: Options["nativeEvaluate"] }

function parseOption(key: string, value: string, state: ParseState) {
  if (key === "--side") {
    state.sides.push(parseSide(value));
  } else if (key === "--workload") {
    if (!(workloads as readonly string[]).includes(value)) throw new UsageError(`Unknown workload: ${value} (expected one of ${workloads.join(", ")})`);
    state.selected.push(value as Workload);
  } else if (key === "--corpus") {
    state.corpusRoot = resolve(value);
  } else if (key === "--manifest") {
    state.manifest = resolve(value);
  } else if (key === "--output") {
    state.output = resolve(value);
  } else if (key === "--native-evaluate") {
    if (value !== "workspace" && value !== "stateless") throw new UsageError("--native-evaluate must be workspace or stateless");
    state.nativeEvaluate = value;
  } else {
    throw new UsageError(`Unknown argument: ${key}`);
  }
}

/** `--corpus`/`--manifest`, else `RERAIL_QUERY_PARITY_CORPUS`/`RERAIL_QUERY_PARITY_MANIFEST`; both or neither. */
function resolveCorpus(state: ParseState, env: NodeJS.ProcessEnv): Corpus | undefined {
  const fromEnv = (name: string) => (env[name] === undefined || env[name] === "" ? undefined : resolve(env[name]));
  const root = state.corpusRoot ?? fromEnv("RERAIL_QUERY_PARITY_CORPUS");
  const manifest = state.manifest ?? fromEnv("RERAIL_QUERY_PARITY_MANIFEST");
  if (root === undefined && manifest === undefined) return undefined;
  if (root === undefined || manifest === undefined) {
    throw new UsageError("--corpus <dir> and --manifest <paths.json> (or RERAIL_QUERY_PARITY_CORPUS and RERAIL_QUERY_PARITY_MANIFEST) go together");
  }
  return { root, manifest };
}

export function parseOptions(argv: readonly string[], env: NodeJS.ProcessEnv = process.env): Options {
  const state: ParseState = { sides: [], selected: [], nativeEvaluate: "workspace" };
  let child = false;
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index]!;
    if (key === "--child") {
      child = true;
      continue;
    }
    const value = argv[++index];
    if (value === undefined || value.startsWith("--")) throw new UsageError(`Missing value for ${key}`);
    parseOption(key, value, state);
  }
  if (new Set(state.sides.map(side => side.name)).size !== state.sides.length) throw new UsageError("Side names must be unique");
  if (child && state.sides.length !== 1) throw new UsageError("Child needs exactly one side");
  if (state.nativeEvaluate === "stateless" && (state.selected.length === 0 || state.selected.includes("edits"))) throw new UsageError("Stateless mode requires explicit --workload values excluding edits");
  const selected = state.selected.length === 0 ? [...workloads] : [...new Set(state.selected)];
  const corpus = resolveCorpus(state, env);
  const problems: string[] = [];
  const needsCorpus = selected.filter(workload => !isSynthetic(workload));
  if (corpus === undefined && needsCorpus.length > 0) {
    problems.push(`A corpus is required for ${needsCorpus.join(", ")}: pass --corpus <dir> --manifest <paths.json> `
      + "(or set RERAIL_QUERY_PARITY_CORPUS and RERAIL_QUERY_PARITY_MANIFEST), or select only --workload relation-order/select-when");
  }
  const referenceAddon = env.RERAIL_QUERY_PARITY_REFERENCE_ADDON;
  if (state.sides.length === 0 && (referenceAddon === undefined || referenceAddon === "")) {
    problems.push("A reference addon is required: set RERAIL_QUERY_PARITY_REFERENCE_ADDON=/absolute/reference.node or pass explicit --side values");
  }
  if (problems.length > 0) throw new UsageError(problems.join("\n"));
  const defaultSides = [{ name: "reference", addon: referenceAddon!, environment: {} }, { name: "candidate", addon: resolveNativeBindingPath(), environment: {} }];
  return {
    sides: state.sides.length === 0 ? defaultSides : state.sides,
    selected,
    ...(corpus === undefined ? {} : { corpus }),
    ...(state.output === undefined ? {} : { output: state.output }),
    child,
    nativeEvaluate: state.nativeEvaluate,
  };
}

function childArguments(side: Side, options: Options) {
  const environment = Object.entries(side.environment).map(([key, value]) => `,${key}=${value}`).join("");
  return [
    "--child", "--side", `${side.name}=${side.addon}${environment}`, "--native-evaluate", options.nativeEvaluate,
    ...(options.corpus === undefined ? [] : ["--corpus", options.corpus.root, "--manifest", options.corpus.manifest]),
    ...options.selected.flatMap(workload => ["--workload", workload]),
  ];
}

async function child(options: Options, side: Side) {
  const childOptions = childArguments(side, options);
  const stdout = await new Promise<string>((resolveResult, reject) => {
    const worker = spawn(process.execPath, [...process.execArgv, script, ...childOptions], { env: { ...process.env, ...side.environment, RERAIL_ENGINE_PATH: side.addon }, stdio: ["ignore", "pipe", "inherit"] });
    const chunks: Buffer[] = []; worker.stdout.on("data", chunk => chunks.push(chunk)); worker.on("error", reject); worker.on("close", code => code === 0 ? resolveResult(Buffer.concat(chunks).toString("utf8")) : reject(new Error(`${side.name} exited ${code}`)));
  });
  return childResultSchema.parse(JSON.parse(stdout));
}
const childResultSchema = z.object({
  side: z.object({ name: z.string(), addon: z.string(), environment: z.record(z.string(), z.string()) }).passthrough(),
  workloads: z.record(z.string(), z.unknown()),
});
interface DifferenceExample { index: number; reference: unknown; actual: unknown }
function canonicalSection(result: unknown, sectionName: string): unknown {
  const value = valueAt(result, sectionName);
  if (!Array.isArray(value)) return canonical(value);
  return value.map(canonical).sort((left, right) => compare(encoded(left), encoded(right)));
}
function valueAt(result: unknown, sectionName: string): unknown {
  if (result === null || typeof result !== "object") return undefined;
  const section = Reflect.get(result, sectionName);
  if (sectionName === "applies") {
    if (!Array.isArray(section)) return section;
    return section.map(apply => ({
      status: Reflect.get(apply, "status"),
      findings: canonicalSection(apply, "findings"),
      failures: canonicalSection(apply, "failures"),
      coverage: canonicalSection(apply, "coverage"),
    }));
  }
  if (section === null || typeof section !== "object" || !("value" in section)) return undefined;
  return section.value;
}
function firstDifferences(left: unknown, right: unknown): DifferenceExample[] {
  const leftItems = Array.isArray(left) ? left : [left]; const rightItems = Array.isArray(right) ? right : [right];
  const examples: DifferenceExample[] = [];
  for (let index = 0; index < Math.max(leftItems.length, rightItems.length) && examples.length < 20; index += 1) {
    if (encoded(leftItems[index]) !== encoded(rightItems[index])) examples.push({ index, reference: leftItems[index], actual: rightItems[index] });
  }
  return examples;
}
export function compareSides(results: Array<{ side: Side; workloads: Record<string, unknown> }>) {
  const reference = results[0]!;
  const differences: Array<{ side: string; workload: string; section: string; reference: string; actual: string; examples: DifferenceExample[] }> = [];
  for (const result of results.slice(1)) for (const workload of Object.keys(reference.workloads)) for (const sectionName of workload === "edits" ? ["applies"] : ["findings", "failures", "coverage"]) {
    const left = valueAt(reference.workloads[workload], sectionName); const right = valueAt(result.workloads[workload], sectionName);
    if (encoded(left) !== encoded(right)) differences.push({
      side: result.side.name, workload, section: sectionName, reference: sha256(encoded(left)), actual: sha256(encoded(right)), examples: firstDifferences(left, right),
    });
  }
  return differences;
}
function usage() {
  return [
    "Usage: just rerail-query-parity --corpus <dir> --manifest <paths.json> [--side name=/absolute/addon.node[,ENV=VALUE...]]...",
    "                                [--workload NAME]... [--native-evaluate workspace|stateless] [--output DIR]",
    "",
    "  --corpus, --manifest  the files to evaluate: a directory and a JSON array of paths relative to it",
    "                        (or RERAIL_QUERY_PARITY_CORPUS / RERAIL_QUERY_PARITY_MANIFEST); relative paths resolve",
    "                        from the repository root. Only relation-order and select-when run without them.",
    "  --side                one engine per side, the first being the reference. Without --side, compares",
    "                        RERAIL_QUERY_PARITY_REFERENCE_ADDON=/absolute/reference.node with this checkout's addon.",
    `  --workload            any of ${workloads.join(", ")} (default: all)`,
    "  --output              also write DIR/report.json",
    "",
    "A side may pick its own evaluator with RERAIL_QUERY_PARITY_EVALUATE=stateless|stateless-per-file in its ENV list.",
  ].join("\n");
}
async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--help") || argv.includes("-h")) {
    process.stdout.write(`${usage()}\n`);
    return;
  }
  let config: Options;
  try {
    config = parseOptions(argv);
  } catch (error) {
    if (!(error instanceof UsageError)) throw error;
    process.stderr.write(`rerail-query-parity: ${error.message}\n\n${usage()}\n`);
    process.exitCode = 2;
    return;
  }
  if (config.child) {
    const side = config.sides[0]!; const result: Record<string, unknown> = {};
    const requestedEvaluator = process.env.RERAIL_QUERY_PARITY_EVALUATE;
    const mode: Evaluator = requestedEvaluator === "stateless" || requestedEvaluator === "stateless-per-file" ? requestedEvaluator : config.nativeEvaluate;
    for (const workload of config.selected) result[workload] = await runWorkload(workload, mode, config.corpus);
    process.stdout.write(`${JSON.stringify({ side: { ...side, addon: realpathSync(side.addon), addonSha256: sha256(readFileSync(side.addon)) }, workloads: result })}\n`); return;
  }
  const results = [];
  for (const side of config.sides) results.push(await child(config, side));
  const differences = compareSides(results); const report = { format: "rerail.query-parity/v1", nativeEvaluate: config.nativeEvaluate, workloads: config.selected, results, differences, equal: differences.length === 0 };
  if (config.output !== undefined) { mkdirSync(config.output, { recursive: true }); writeFileSync(resolve(config.output, "report.json"), `${JSON.stringify(report, null, 2)}\n`); }
  for (const difference of differences.slice(0, 20)) {
    process.stderr.write(`DIFF ${difference.side} ${difference.workload}/${difference.section}: ${difference.reference} != ${difference.actual}\n`);
    for (const example of difference.examples) process.stderr.write(`  [${example.index}] reference=${JSON.stringify(example.reference)} actual=${JSON.stringify(example.actual)}\n`);
  }
  const differenceSummary = differences.length === 0 ? "equal" : `${differences.length} differing sections`;
  process.stderr.write(`rerail-query-parity: ${differenceSummary} across ${results.length} sides\n`);
  if (differences.length !== 0) process.exitCode = 1;
}

if (resolve(process.argv[1] ?? "") === script) await main();
