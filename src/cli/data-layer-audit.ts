/** Production host boundary for the opt-in data-layer pack and its live observations. */
import { execFile } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { z } from "zod";
import { matchingApplications } from "./data-layer/applications.js";
import { schemaApprovalsSchema, schemaApprovalFacts, type SchemaApprovals } from "./data-layer/approvals.js";
import { codeRuleSchema, loadNativeBinding, rulePackFileSchema, parseJsonWithoutDuplicateKeys, RESULT_PAGE_BYTES, PACKAGE_ROOT } from "../engine/index.js";

const execute = promisify(execFile);
const pythonExecutable = (value: string | undefined, root: string): string => value === undefined
  ? existsSync(join(root, ".venv/bin/python")) ? join(root, ".venv/bin/python") : "python3.13"
  : value.includes("/") || value.includes("\\") ? resolve(value) : value;
const observations = z.object({ fingerprints: z.tuple([z.string(), z.string()]), firstRefs: z.array(z.string()), secondRefs: z.array(z.string()) }).strict();
export interface DataLayerAuditOptions {
  repositoryRoot: string;
  packDirectory: string;
  ruleIds?: readonly string[];
  python?: string;
  mode?: "snapshot" | "workspace";
  applicationRoots?: readonly string[];
  schemaApprovals?: SchemaApprovals;
  matchingPhase?: 1 | 2 | 3 | 4;
  snapshot: { format: string; revision: number; files: { path: string; source: string; language: string }[]; facts?: unknown[] };
}
export interface DataLayerAuditResult {
  violations: { code: string; message: string; path: string; mode: "log" | "steer" | "block"; span?: { startLine: number; startByte: number; endByte: number } }[];
  failures: { code: string; message: string }[];
}

export async function configureDataLayerAudit(options: DataLayerAuditOptions) {
  if (options.matchingPhase !== undefined && ![1, 2, 3, 4].includes(options.matchingPhase)) throw new Error("Matching phase must be 1, 2, 3 or 4");
  const root = resolve(options.repositoryRoot), pack = resolve(options.packDirectory);
  const manifestPath = join(pack, "pack.json");
  const manifest = rulePackFileSchema.parse(parseJsonWithoutDuplicateKeys(readFileSync(manifestPath, "utf8"), manifestPath));
  const available = new Map(manifest.rules.map(rule => [rule.id, rule]));
  const ids = options.ruleIds ?? [...available.keys()];
  if (!ids.length || new Set(ids).size !== ids.length) throw new Error("Select distinct data-layer rules");
  for (const id of ids) if (!available.has(id)) throw new Error(`Unknown data-layer rule: ${id}`);
  const applications = ids.includes("data-layer-matching-architecture")
    ? matchingApplications(root, options.snapshot.files, options.applicationRoots ?? [])
    : { roots: [], files: options.snapshot.files };
  const snapshot = { ...options.snapshot, files: applications.files };
  let approvals: ReturnType<typeof schemaApprovalFacts> | undefined;
  if (ids.includes("data-layer-schema-introspection-data-paths")) {
    let configuration = options.schemaApprovals ?? { entries: [], signature: "NEEDS_SIGNING" };
    if (options.schemaApprovals === undefined && existsSync(join(root, "scripts/lint_schema_introspection_data_paths.py"))) {
      const python = pythonExecutable(options.python, root);
      const response = await execute(python, [join(pack, "runtime/approvals.py"), root], { timeout: 110_000, maxBuffer: 4 * 1024 * 1024 });
      configuration = schemaApprovalsSchema.parse(JSON.parse(response.stdout));
    }
    approvals = schemaApprovalFacts(configuration, pack);
  }
  let batch: z.infer<typeof observations> | undefined;
  if (ids.includes("data-layer-plan-composition-architecture")) {
    for (const file of options.snapshot.files) {
      const path = resolve(root, file.path);
      if (!path.startsWith(root + "/") || readFileSync(path, "utf8") !== file.source)
        throw new Error(`Composition probe snapshot does not match the application source: ${file.path}`);
    }
    const python = pythonExecutable(options.python, root);
    // A fresh process observes the actual application on every run, including edits.
    const response = await execute(python, [join(pack, "runtime/observations.py"), root], {
      cwd: root, env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" }, timeout: 110_000, maxBuffer: 4 * 1024 * 1024,
    });
    batch = observations.parse(JSON.parse(response.stdout));
  }
  const programs = ids.map(id => {
    const entry = manifest.rules.find(rule => rule.id === id);
    if (!entry) throw new Error(`Rule is absent from the data-layer manifest: ${id}`);
    // Configuration must precede schema normalization, which resolves option expressions.
    const path = join(pack, entry.document);
    const authored = parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path) as { id: string; scope: { include: string[]; exclude?: string[] }; clauses: { define?: { options?: { parameters?: unknown }; config?: Record<string, unknown> } }[] };
    if (authored.id !== id) throw new Error(`Rule document id mismatch: ${id}`);
    if (id === "data-layer-plan-composition-architecture") {
      for (const clause of authored.clauses) if (clause.define?.options?.parameters)
        clause.define.config = { ...clause.define.config, parameters: JSON.stringify(batch) };
    }
    if (id === "data-layer-matching-architecture" && applications.roots.length) {
      authored.scope.include = ["**/*.py"];
      for (const clause of authored.clauses) if (clause.define?.options?.parameters)
        clause.define.config = { ...clause.define.config, parameters: JSON.stringify({ applicationRoots: applications.roots }) };
    }
    if (id === "data-layer-schema-introspection-data-paths") {
      for (const clause of authored.clauses) if (clause.define?.options?.parameters)
        clause.define.config = { ...clause.define.config, parameters: JSON.stringify({ schemaApprovals: approvals }) };
    }
    return { id, program: JSON.stringify(codeRuleSchema.parse(authored)) };
  });
  return { programs, snapshot };
}

export async function prepareDataLayerAudit(options: DataLayerAuditOptions) {
  const { programs, snapshot } = await configureDataLayerAudit(options);
  const native = loadNativeBinding();
  const capabilities = readFileSync(join(PACKAGE_ROOT, "native/src/code_rule_dsl/capabilities.json"), "utf8");
  return { snapshot, compiled: programs.map(({ program }) => native.CodeRuleDslRuntime.compile(program, "json", capabilities)) };
}

export function filterMatchingPhase(result: DataLayerAuditResult, phase: DataLayerAuditOptions["matchingPhase"]): DataLayerAuditResult {
  const codes = phase === 1 ? [...Array.from({ length: 30 }, (_, i) => `MA${String(i + 1).padStart(3, "0")}`), "MA036", "MA041"]
    : phase === 2 ? ["MA031", "MA032", "MA037"] : phase === 3 ? ["MA033", "MA034", "MA036", "MA038"] : ["MA035", "MA039", "MA040"];
  return phase === undefined ? result : { ...result, violations: result.violations.filter(row => !/^MA\d{3}$/.test(row.code) || codes.includes(row.code)) };
}

export async function runDataLayerAudit(options: DataLayerAuditOptions): Promise<DataLayerAuditResult> {
  const { compiled, snapshot } = await prepareDataLayerAudit(options);
  const native = loadNativeBinding();
  const result: DataLayerAuditResult = { violations: [], failures: [] };
  if (options.mode === "snapshot") {
    for (const rule of compiled) {
      const page = JSON.parse(rule.evaluate(JSON.stringify(snapshot), "report")) as DataLayerAuditResult;
      result.violations.push(...page.violations); result.failures.push(...page.failures);
    }
  } else {
    const workspace = new native.CodeRuleDslWorkspace();
    try {
      for (const rule of compiled) workspace.addCodeRule(rule);
      // A fresh workspace starts at zero; the first application must advance it.
      await workspace.applyAsync(JSON.stringify({ revision: Math.max(1, snapshot.revision), files: snapshot.files, removed: [], quiet: true }));
      for (let cursor = 0; ;) {
        const page = JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES));
        result.violations.push(...page.violations); result.failures.push(...page.failures);
        if (page.done) break;
        if (page.cursor <= cursor) throw new Error("Result cursor did not advance");
        cursor = page.cursor;
      }
    } finally { workspace.close(); }
  }
  return filterMatchingPhase(result, options.matchingPhase);
}

export async function runDataLayerCommand(args: readonly string[], stdout: Pick<NodeJS.WriteStream, "write"> = process.stdout, signingEnvironment?: NodeJS.ProcessEnv): Promise<number> {
  const usage = "Usage: rerail data-layer --repo <root> --pack <pack> --snapshot <json> [--rules <ids>] [--python <executable>] [--application-root <dir>] [--phase <1..4>] [--schema-approvals <json>] [--summary | --summary-only] [--check] [--json]\n       rerail data-layer --repo <root> --sign-allowlist [--python <executable>]\n";
  if (args.includes("--help") || args.includes("-h")) { stdout.write(usage); return 0; }
  const values = new Map<string, string>();
  const flags = new Set<string>();
  const applicationRoots: string[] = [];
  for (let i = 0; i < args.length; i++) {
    const key = args[i]!;
    if (["--check", "--json", "--summary", "--summary-only", "--sign-allowlist"].includes(key)) {
      flags.add(key); continue;
    }
    const value = args[++i];
    if (key === "--application-root" && value && !value.startsWith("--")) { applicationRoots.push(value); continue; }
    if (!["--repo", "--pack", "--snapshot", "--rules", "--python", "--phase", "--schema-approvals"].includes(key) || !value || value.startsWith("--") || values.has(key))
      throw new Error(`Invalid data-layer option: ${key}\n${usage}`);
    values.set(key, value);
  }
  if (flags.has("--sign-allowlist")) {
    if (!values.has("--repo") || values.has("--schema-approvals") || values.has("--snapshot") || flags.has("--json"))
      throw new Error("--sign-allowlist requires --repo and signs only the original inline source catalog");
    const root = resolve(values.get("--repo")!);
    const python = pythonExecutable(values.get("--python"), root);
    const environment = { ...process.env, ...signingEnvironment, PYTHONPATH: [root, join(root, "scripts")].join(process.platform === "win32" ? ";" : ":") };
    try {
      const response = await execute(python, [join(root, "scripts/lint_schema_introspection_data_paths.py"), "--sign-allowlist"], { cwd: root, env: environment, timeout: 110_000, maxBuffer: 4 * 1024 * 1024 });
      stdout.write(response.stdout); stdout.write(response.stderr); return 0;
    } catch (error) {
      const response = error as { stdout?: string; stderr?: string; code?: number | string };
      if (typeof response.code !== "number") throw error;
      stdout.write(response.stdout ?? ""); stdout.write(response.stderr ?? ""); return response.code;
    }
  }
  const phase = values.has("--phase") ? Number(values.get("--phase")) : undefined;
  if (phase !== undefined && ![1, 2, 3, 4].includes(phase)) throw new Error("Matching phase must be 1, 2, 3 or 4");
  if (!["--repo", "--pack", "--snapshot"].every(key => values.has(key))) throw new Error("data-layer requires --repo, --pack and --snapshot");
  const result = await runDataLayerAudit({ repositoryRoot: values.get("--repo")!, packDirectory: values.get("--pack")!,
    snapshot: JSON.parse(readFileSync(values.get("--snapshot")!, "utf8")), applicationRoots,
    ...(phase === undefined ? {} : { matchingPhase: phase as 1 | 2 | 3 | 4 }),
    ...(values.has("--schema-approvals") ? { schemaApprovals: schemaApprovalsSchema.parse(JSON.parse(readFileSync(values.get("--schema-approvals")!, "utf8"))) } : {}),
    ...(values.has("--rules") ? { ruleIds: values.get("--rules")!.split(",") } : {}), ...(values.has("--python") ? { python: values.get("--python")! } : {}) });
  if (flags.has("--json")) stdout.write(JSON.stringify(result) + "\n");
  else if (flags.has("--summary")) {
    const label = values.get("--rules") === "data-layer-schema-introspection-data-paths" ? "schema introspection data-path" : "data-layer";
    stdout.write(`${result.violations.length} ${label} violation(s)\n`);
    for (const failure of result.failures) stdout.write(`ERROR ${failure.code}: ${failure.message}\n`);
  } else if (flags.has("--summary-only")) {
    const counts = new Map<string, number>();
    for (const row of result.violations) counts.set(row.code, (counts.get(row.code) ?? 0) + 1);
    stdout.write(`Found ${result.violations.length} violation(s) across ${counts.size} rule(s):\n`);
    for (const [code, count] of [...counts].sort(([a], [b]) => a.localeCompare(b))) stdout.write(`  ${code}: ${count}\n`);
    for (const failure of result.failures) stdout.write(`ERROR ${failure.code}: ${failure.message}\n`);
  } else {
    for (const row of result.violations) {
      if (!row.span) throw new Error("Data-layer diagnostic has no source span");
      stdout.write(`${row.code} ${row.path}:${row.span.startLine + 1}: ${row.message}\n`);
    }
    if (result.violations.length) stdout.write(`\nFound ${result.violations.length} data-layer violation(s).\n`);
    else if (!result.failures.length) stdout.write("Data-layer linter: clean.\n");
    for (const failure of result.failures) stdout.write(`ERROR ${failure.code}: ${failure.message}\n`);
  }
  return result.failures.length ? 2 : result.violations.some(row => row.code === "SIDP000")
    || flags.has("--check") && result.violations.some(row => row.mode !== "log") ? 1 : 0;
}
