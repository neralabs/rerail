import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, extname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { languageFor } from "../src/code-rule-enforcer.js";
import { loadNativeBinding, resolveNativeBindingPath, type NativeBinding, type NativeCompiledCodeRuleDsl } from "../src/native.js";

import { isSemanticWorkload, semanticWorkloads, type SemanticWorkload } from "./query-parity-semantic.js";

const script = fileURLToPath(import.meta.url);
const rerailRoot = resolve(dirname(script), "..");
const repositoryRoot = resolve(rerailRoot, "..");
const catalogRoot = resolve(rerailRoot, "code-rule-dsl");
const mainRoot = process.env.RERAIL_QUERY_PARITY_MAIN_ROOT ?? repositoryRoot;
const defaultManifest = resolve(mainRoot, "docs/design/rerail-phase-a/nera-data-layer-python-manifest.json");
const defaultCorpus = resolve(mainRoot, "nera-data-layer");
const referenceAddon = process.env.RERAIL_QUERY_PARITY_REFERENCE_ADDON;
const workloads = ["corpus", ...semanticWorkloads, "catalog-web", "catalog-python", "edits", "relation-order", "select-when"] as const;
type Workload = Exclude<typeof workloads[number], SemanticWorkload> | SemanticWorkload;
type Evaluator = "workspace" | "stateless" | "stateless-per-file";
type SourceFile = { path: string; language: string; source: string };
type Side = { name: string; addon: string; environment: Record<string, string> };

const rangeSchema = z.strictObject({ startByte: z.number().int(), endByte: z.number().int(), startLine: z.number().int(), startColumn: z.number().int(), endLine: z.number().int(), endColumn: z.number().int() });
const violationSchema = z.object({ id: z.string(), ruleId: z.string(), clauseId: z.string(), code: z.string(), mode: z.string(), message: z.string(), path: z.string().optional(), span: rangeSchema.optional(), captures: z.record(z.string(), z.array(rangeSchema)).default({}), variables: z.record(z.string(), z.union([z.string(), z.number(), z.boolean()])).default({}) }).passthrough();
const failureSchema = z.object({ ruleId: z.string().optional(), clauseId: z.string(), code: z.string(), message: z.string(), path: z.string().optional() }).passthrough();
const evaluationSchema = z.object({ status: z.string(), violations: z.array(violationSchema), failures: z.array(failureSchema) }).passthrough();
const workspaceSchema = z.object({ revision: z.number().int(), evaluation: evaluationSchema }).passthrough();

interface Options { sides: Side[]; selected: Workload[]; output?: string; child: boolean; nativeEvaluate: "workspace" | "stateless" }

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
function section(value: unknown) { const json = encoded(value); return { sha256: sha256(json), count: Array.isArray(value) ? value.length : undefined, value }; }
function normalizedPath(path: string): string { return path.split(sep).join("/"); }
function language(path: string): string {
  const selected = languageFor(path);
  if (selected === undefined) throw new Error(`No Rerail language for ${path}`);
  return selected;
}
function walk(root: string, predicate: (relativePath: string) => boolean): string[] {
  const output: string[] = [];
  function visit(directory: string) {
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => compare(a.name, b.name))) {
      const absolute = resolve(directory, entry.name);
      if (entry.isDirectory()) visit(absolute);
      else if (entry.isFile()) { const path = normalizedPath(relative(root, absolute)); if (predicate(path)) output.push(path); }
    }
  }
  visit(root); return output;
}
function filesFromPaths(root: string, paths: readonly string[]): SourceFile[] {
  return paths.map(path => ({ path, language: language(path), source: readFileSync(resolve(root, path), "utf8") }));
}
function corpusFiles(): SourceFile[] {
  const paths = z.array(z.string()).parse(JSON.parse(readFileSync(defaultManifest, "utf8"))).slice().sort(compare);
  return filesFromPaths(defaultCorpus, paths);
}
function trackedPythonFiles(): SourceFile[] {
  const result = spawnSync("git", ["-C", mainRoot, "ls-files", "--", "*.py"], { encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ls-files failed: ${result.stderr}`);
  return filesFromPaths(mainRoot, result.stdout.split("\n").filter(path => path.length > 0 && !path.startsWith("nera-data-layer/")).sort(compare));
}
function webFiles(): SourceFile[] {
  const root = resolve(mainRoot, "web/src");
  return filesFromPaths(mainRoot, walk(root, path => [".ts", ".tsx"].includes(extname(path))).map(path => `web/src/${path}`));
}
function readDocuments(directory: string, prefix = ""): Array<{ path: string; source: string; clauses: Array<{ id: string }> }> {
  const documents: Array<{ path: string; source: string; clauses: Array<{ id: string }> }> = [];
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => compare(a.name, b.name))) {
    const child = resolve(directory, entry.name); const childPath = prefix === "" ? entry.name : `${prefix}/${entry.name}`;
    if (entry.isDirectory()) documents.push(...readDocuments(child, childPath));
    else if (entry.isFile() && entry.name.endsWith(".json")) { const source = readFileSync(child, "utf8"); documents.push({ path: childPath, source, clauses: z.object({ clauses: z.array(z.object({ id: z.string() }).passthrough()) }).parse(JSON.parse(source)).clauses }); }
  }
  return documents;
}
/** Small hand-built workloads for behaviour the repository corpora never exercise. */
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
function isSynthetic(kind: string): kind is SyntheticWorkload { return Object.hasOwn(synthetic, kind); }
function catalogDocuments(kind: "bundled" | "full" | SemanticWorkload | SyntheticWorkload, root: string, directDocuments: Array<{ path: string; source: string; clauses: Array<{ id: string }> }>) {
  if (kind === "full") return [...directDocuments, ...readDocuments(resolve(root, "opt-in/tslint"), "opt-in/tslint")];
  if (kind === "bundled") return directDocuments;
  return readDocuments(root);
}

function catalog(kind: "bundled" | "full" | SemanticWorkload | SyntheticWorkload): { capabilities: string; documents: Array<{ path: string; source: string; clauses: Array<{ id: string }> }> } {
  if (isSynthetic(kind)) {
    const { clauses } = synthetic[kind];
    return {
      capabilities: catalog("bundled").capabilities,
      documents: [{ path: `${kind}.json`, clauses, source: JSON.stringify({
        format: "rerail.policy/v1", id: kind, revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] }, clauses,
      }) }],
    };
  }
  const root = kind === "bundled" || kind === "full" ? resolve(catalogRoot, "rules") : resolve(rerailRoot, "benchmark", kind, "with");
  const directDocuments = readdirSync(root, { withFileTypes: true }).filter(entry => entry.isFile() && entry.name.endsWith(".json")).sort((a, b) => compare(a.name, b.name)).map(entry => {
    const source = readFileSync(resolve(root, entry.name), "utf8");
    return { path: entry.name, source, clauses: z.object({ clauses: z.array(z.object({ id: z.string() }).passthrough()) }).parse(JSON.parse(source)).clauses };
  });
  const documents = catalogDocuments(kind, root, directDocuments);
  // The reference engine still requires `maxGraphDepth` (its host-fact `reachable` predicate, which
  // this branch deleted with the fact plane); the candidate ignores it.
  const bundled = JSON.parse(readFileSync(resolve(catalogRoot, "capabilities.json"), "utf8")) as { limits: Record<string, number> };
  const capabilities = JSON.stringify({ ...bundled, limits: { maxGraphDepth: 16, ...bundled.limits } });
  return { capabilities, documents: documents.sort((a, b) => compare(a.path, b.path)) };
}
export function coverage(documents: Array<{ path: string; source: string; clauses: Array<{ id: string }> }>, failures: readonly z.infer<typeof failureSchema>[]) {
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
async function statelessEvaluation(files: SourceFile[], applicable: Array<{ source: string; codeRule: { inScope(path: string): boolean; evaluate(input: string, mode: string): string } }>, mode: Evaluator, resultFrom: (raw: unknown) => Promise<unknown>, edits: Array<{ name: string; files: SourceFile[]; removed: string[] }> | undefined) {
  if (edits !== undefined) throw new Error("--native-evaluate stateless does not support edits; stateless has no incremental workspace");
  const evaluations: Array<{ policyId: string; evaluation: z.infer<typeof evaluationSchema> }> = [];
  const inputs = mode === "stateless-per-file" ? files.map(file => [file]) : [files];
  for (const snapshotFiles of inputs) {
    const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: snapshotFiles });
    for (const entry of applicable) {
      if (mode === "stateless-per-file" && !entry.codeRule.inScope(snapshotFiles[0]!.path)) continue;
      evaluations.push({ policyId: z.object({ id: z.string() }).parse(JSON.parse(entry.source)).id, evaluation: evaluationSchema.parse(JSON.parse(entry.codeRule.evaluate(input, "report"))) });
    }
  }
  return resultFrom({
    status: evaluations.some(entry => entry.evaluation.status !== "completed") ? "failed" : "completed",
    violations: evaluations.flatMap(entry => entry.evaluation.violations),
    failures: evaluations.flatMap(entry => entry.evaluation.failures.map(failure => ({ ...failure, ruleId: failure.ruleId ?? entry.policyId }))),
  });
}

async function workspaceEvaluation(binding: NativeBinding, files: SourceFile[], applicable: Array<{ codeRule: NativeCompiledCodeRuleDsl }>, resultFrom: (raw: unknown) => Promise<unknown>, edits: Array<{ name: string; files: SourceFile[]; removed: string[] }> | undefined) {
  const workspace = new binding.CodeRuleDslWorkspace();
  try {
    for (const entry of applicable) workspace.addCodeRule(entry.codeRule);
    const results = [];
    let revision = 1;
    results.push(await resultFrom(workspaceSchema.parse(JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: revision++, files, removed: [] })))).evaluation));
    for (const edit of edits ?? []) results.push(await resultFrom(workspaceSchema.parse(JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: revision++, files: edit.files, removed: edit.removed })))).evaluation));
    return edits === undefined ? results[0]! : results;
  } finally { workspace.close(); }
}

async function evaluate(files: SourceFile[], catalogKind: "bundled" | "full" | SemanticWorkload | SyntheticWorkload, mode: Evaluator, edits?: Array<{ name: string; files: SourceFile[]; removed: string[] }>) {
  const loaded = catalog(catalogKind); const binding = loadNativeBinding();
  const compiled = loaded.documents.map(document => ({ ...document, codeRule: binding.CodeRuleDslRuntime.compile(document.source, "json", loaded.capabilities) }));
  const applicable = compiled.filter(entry => files.some(file => entry.codeRule.inScope(file.path)) || (edits?.some(edit => edit.files.some(file => entry.codeRule.inScope(file.path))) ?? false));
  async function resultFrom(raw: unknown) {
    const evaluation = evaluationSchema.parse(raw);
    const violations = canonicalViolations(evaluation.violations); const failures = evaluation.failures.map(failure => canonical(failure)).sort((a, b) => compare(encoded(a), encoded(b)));
    return {
      status: evaluation.status,
      findings: section(violations),
      failures: section(failures),
      coverage: section(coverage(applicable, evaluation.failures)),
      phaseAFindingSha256: sha256(JSON.stringify(evaluation.violations)),
    };
  }
  if (mode === "stateless" || mode === "stateless-per-file") return statelessEvaluation(files, applicable, mode, resultFrom, edits);
  return workspaceEvaluation(binding, files, applicable, resultFrom, edits);
}
function editScript(files: SourceFile[]) {
  const real = files[0]; if (real === undefined) throw new Error("Empty corpus");
  const synthetic = "impl/__rerail_query_parity__.py";
  return [
    { name: "comment-add", files: [{ ...real, source: `# parity comment\n${real.source}` }], removed: [] },
    { name: "comment-remove", files: [real], removed: [] },
    { name: "violation-add", files: [{ path: synthetic, language: "python", source: "def run(conn):\n    conn.session('parity').write_async()\n" }], removed: [] },
    { name: "violation-remove", files: [{ path: synthetic, language: "python", source: "def run(conn):\n    pass\n" }], removed: [] },
    { name: "syntax-break", files: [{ path: synthetic, language: "python", source: "def run(:\n    pass\n" }], removed: [] },
    { name: "syntax-repair", files: [{ path: synthetic, language: "python", source: "def run(conn):\n    pass\n" }], removed: [] },
    { name: "rename", files: [{ path: "impl/__rerail_query_parity_renamed__.py", language: "python", source: "def run(conn):\n    pass\n" }], removed: [synthetic] },
    { name: "delete", files: [], removed: ["impl/__rerail_query_parity_renamed__.py"] },
  ];
}
async function runWorkload(workload: Workload, mode: Evaluator) {
  if (isSynthetic(workload)) {
    const { files } = synthetic[workload];
    return { files: files.length, ...await evaluate(files, workload, mode) as object };
  }
  if (workload === "corpus") {
    const files = corpusFiles(); const result = await evaluate(files, "bundled", mode);
    const baselineResult = result as { findings: { value: z.infer<typeof violationSchema>[] } };
    const baseline = baselineProjection(baselineResult.findings.value);
    return { files: files.length, ...result as object, phaseABaseline: { sha256: sha256(`${JSON.stringify(baseline, null, 2)}\n`), count: baseline.length, value: baseline } };
  }
  if (isSemanticWorkload(workload)) { const files = corpusFiles(); return { files: files.length, ...await evaluate(files, workload, mode) as object }; }
  if (workload === "catalog-web") { const files = webFiles(); return { files: files.length, ...await evaluate(files, "full", mode) as object }; }
  if (workload === "catalog-python") { const files = trackedPythonFiles(); return { files: files.length, ...await evaluate(files, "full", mode) as object }; }
  const files = corpusFiles(); const results = await evaluate(files, "bundled", mode, editScript(files));
  return { files: files.length, applies: results };
}
function parseSide(raw: string): Side {
  const [first, ...assignments] = raw.split(",");
  const split = first?.indexOf("=") ?? -1;
  if (split <= 0) {
    throw new Error(`Invalid --side ${raw}; expected name=/absolute/addon.node[,ENV=VALUE...]`);
  }
  const name = first!.slice(0, split);
  const addon = first!.slice(split + 1);
  if (!isAbsolute(addon)) {
    throw new Error(`Addon for ${name} must be absolute`);
  }
  const environment: Record<string, string> = {};
  for (const assignment of assignments) {
    const index = assignment.indexOf("=");
    if (index <= 0) {
      throw new Error(`Invalid side environment ${assignment}`);
    }
    environment[assignment.slice(0, index)] = assignment.slice(index + 1);
  }
  return { name, addon, environment };
}

function parseOption(key: string, value: string, state: { sides: Side[]; selected: Workload[]; output?: string; nativeEvaluate: Options["nativeEvaluate"] }) {
  if (key === "--side") {
    state.sides.push(parseSide(value));
  } else if (key === "--workload") {
    if (!(workloads as readonly string[]).includes(value)) throw new Error(`Unknown workload: ${value}`);
    state.selected.push(value as Workload);
  } else if (key === "--output") {
    state.output = resolve(value);
  } else if (key === "--native-evaluate") {
    if (value !== "workspace" && value !== "stateless") throw new Error("--native-evaluate must be workspace or stateless");
    state.nativeEvaluate = value;
  } else {
    throw new Error(`Unknown argument: ${key}`);
  }
}

function options(argv: string[]): Options {
  const state: { sides: Side[]; selected: Workload[]; output?: string; nativeEvaluate: Options["nativeEvaluate"] } = {
    sides: [], selected: [], nativeEvaluate: "workspace",
  };
  let child = false;
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index]!;
    if (key === "--child") {
      child = true;
      continue;
    }
    const value = argv[++index];
    if (value === undefined || value.startsWith("--")) throw new Error(`Missing value for ${key}`);
    parseOption(key, value, state);
  }
  if (new Set(state.sides.map(side => side.name)).size !== state.sides.length) throw new Error("Side names must be unique");
  if (child && state.sides.length !== 1) throw new Error("Child needs exactly one side");
  if (state.nativeEvaluate === "stateless" && (state.selected.length === 0 || state.selected.includes("edits"))) throw new Error("Stateless mode requires explicit --workload values excluding edits");
  if (state.sides.length === 0 && (referenceAddon === undefined || referenceAddon.length === 0)) {
    throw new Error("A reference addon is required: set RERAIL_QUERY_PARITY_REFERENCE_ADDON=/absolute/reference.node or pass explicit --side values");
  }
  const defaultSides = [{ name: "reference", addon: referenceAddon!, environment: {} }, { name: "candidate", addon: resolveNativeBindingPath(), environment: {} }];
  return { sides: state.sides.length === 0 ? defaultSides : state.sides, selected: state.selected.length === 0 ? [...workloads] : [...new Set(state.selected)], ...(state.output === undefined ? {} : { output: state.output }), child, nativeEvaluate: state.nativeEvaluate };
}

function childArguments(side: Side, options: Options) {
  const environment = Object.entries(side.environment).map(([key, value]) => `,${key}=${value}`).join("");
  return ["--child", "--side", `${side.name}=${side.addon}${environment}`, "--native-evaluate", options.nativeEvaluate, ...options.selected.flatMap(workload => ["--workload", workload])];
}

async function child(options: Options, side: Side) {
  const childOptions = childArguments(side, options);
  const stdout = await new Promise<string>((resolveResult, reject) => {
    const worker = spawn(process.execPath, [...process.execArgv, script, ...childOptions], { env: { ...process.env, ...side.environment, RERAIL_NATIVE_KERNEL_PATH: side.addon }, stdio: ["ignore", "pipe", "inherit"] });
    const chunks: Buffer[] = []; worker.stdout.on("data", chunk => chunks.push(chunk)); worker.on("error", reject); worker.on("close", code => code === 0 ? resolveResult(Buffer.concat(chunks).toString("utf8")) : reject(new Error(`${side.name} exited ${code}`)));
  });
  return JSON.parse(stdout) as { side: Side; workloads: Record<string, unknown> };
}
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
    "Usage: just rerail-query-parity [--side name=/absolute/addon.node] [--workload NAME] [--output DIR]",
    "Without explicit --side values, set RERAIL_QUERY_PARITY_REFERENCE_ADDON=/absolute/reference.node.",
    `Repository defaults are resolved from ${repositoryRoot}.`,
  ].join("\n");
}
async function main() {
  if (process.argv.slice(2).includes("--help")) {
    process.stdout.write(`${usage()}\n`);
    return;
  }
  const config = options(process.argv.slice(2));
  if (config.child) {
    const side = config.sides[0]!; const result: Record<string, unknown> = {};
    // A side can pick its evaluator (`RERAIL_PARITY_EVALUATE=stateless[-per-file]`) independently.
    const requestedEvaluator = process.env.RERAIL_PARITY_EVALUATE;
    const mode: Evaluator = requestedEvaluator === "stateless" || requestedEvaluator === "stateless-per-file" ? requestedEvaluator : config.nativeEvaluate;
    for (const workload of config.selected) result[workload] = await runWorkload(workload, mode);
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
