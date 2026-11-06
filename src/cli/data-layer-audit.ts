/** Production host boundary for the opt-in data-layer pack and its live observations. */
import { execFile } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { z } from "zod";
import { codeRuleSchema, loadNativeBinding, rulePackFileSchema, parseJsonWithoutDuplicateKeys, RESULT_PAGE_BYTES, PACKAGE_ROOT } from "../engine/index.js";

const execute = promisify(execFile);
const observations = z.object({ fingerprints: z.tuple([z.string(), z.string()]), firstRefs: z.array(z.string()), secondRefs: z.array(z.string()) }).strict();
export interface DataLayerAuditOptions {
  repositoryRoot: string;
  packDirectory: string;
  ruleIds?: readonly string[];
  python?: string;
  mode?: "snapshot" | "workspace";
  snapshot: { format: string; revision: number; files: { path: string; source: string; language: string }[]; facts?: unknown[] };
}
export interface DataLayerAuditResult {
  violations: { code: string; message: string; path: string; mode: "log" | "steer" | "block"; span?: { startLine: number; startByte: number; endByte: number } }[];
  failures: { code: string; message: string }[];
}

export async function runDataLayerAudit(options: DataLayerAuditOptions): Promise<DataLayerAuditResult> {
  const root = resolve(options.repositoryRoot), pack = resolve(options.packDirectory);
  const manifestPath = join(pack, "pack.json");
  const manifest = rulePackFileSchema.parse(parseJsonWithoutDuplicateKeys(readFileSync(manifestPath, "utf8"), manifestPath));
  const available = new Map(manifest.rules.map(rule => [rule.id, rule]));
  const ids = options.ruleIds ?? [...available.keys()];
  if (!ids.length || new Set(ids).size !== ids.length) throw new Error("Select distinct data-layer rules");
  for (const id of ids) if (!available.has(id)) throw new Error(`Unknown data-layer rule: ${id}`);
  let batch: z.infer<typeof observations> | undefined;
  if (ids.includes("data-layer-plan-composition-architecture")) {
    for (const file of options.snapshot.files) {
      const path = resolve(root, file.path);
      if (!path.startsWith(root + "/") || readFileSync(path, "utf8") !== file.source)
        throw new Error(`Composition probe snapshot does not match the application source: ${file.path}`);
    }
    const python = options.python ?? (existsSync(join(root, ".venv/bin/python")) ? join(root, ".venv/bin/python") : "python3.13");
    // A fresh process observes the actual application on every run, including edits.
    const response = await execute(python, [join(pack, "runtime/observations.py"), root], {
      cwd: root, env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" }, timeout: 110_000, maxBuffer: 4 * 1024 * 1024,
    });
    batch = observations.parse(JSON.parse(response.stdout));
  }
  const native = loadNativeBinding();
  const capabilities = readFileSync(join(PACKAGE_ROOT, "native/src/code_rule_dsl/capabilities.json"), "utf8");
  const compiled = ids.map(id => {
    const entry = manifest.rules.find(rule => rule.id === id);
    if (!entry) throw new Error(`Rule is absent from the data-layer manifest: ${id}`);
    // Configuration must precede schema normalization, which resolves option expressions.
    const path = join(pack, entry.document);
    const authored = parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path) as { id: string; clauses: { define?: { options?: { parameters?: unknown }; config?: Record<string, unknown> } }[] };
    if (authored.id !== id) throw new Error(`Rule document id mismatch: ${id}`);
    if (id === "data-layer-plan-composition-architecture") {
      for (const clause of authored.clauses) if (clause.define?.options?.parameters)
        clause.define.config = { ...clause.define.config, parameters: JSON.stringify(batch) };
    }
    return native.CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", capabilities);
  });
  const result: DataLayerAuditResult = { violations: [], failures: [] };
  if (options.mode === "snapshot") {
    for (const rule of compiled) {
      const page = JSON.parse(rule.evaluate(JSON.stringify(options.snapshot), "report")) as DataLayerAuditResult;
      result.violations.push(...page.violations); result.failures.push(...page.failures);
    }
  } else {
    const workspace = new native.CodeRuleDslWorkspace();
    try {
      for (const rule of compiled) workspace.addCodeRule(rule);
      // A fresh workspace starts at zero; the first application must advance it.
      await workspace.applyAsync(JSON.stringify({ revision: Math.max(1, options.snapshot.revision), files: options.snapshot.files, removed: [], quiet: true }));
      for (let cursor = 0; ;) {
        const page = JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES));
        result.violations.push(...page.violations); result.failures.push(...page.failures);
        if (page.done) break;
        if (page.cursor <= cursor) throw new Error("Result cursor did not advance");
        cursor = page.cursor;
      }
    } finally { workspace.close(); }
  }
  return result;
}

export async function runDataLayerCommand(args: readonly string[], stdout: Pick<NodeJS.WriteStream, "write"> = process.stdout): Promise<number> {
  const usage = "Usage: rerail data-layer --repo <root> --pack <pack> --snapshot <json> [--rules <ids>] [--python <executable>] [--check] [--json]\n";
  if (args.includes("--help") || args.includes("-h")) { stdout.write(usage); return 0; }
  const values = new Map<string, string>();
  const flags = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const key = args[i]!;
    if (["--check", "--json"].includes(key)) {
      flags.add(key); continue;
    }
    const value = args[++i];
    if (!["--repo", "--pack", "--snapshot", "--rules", "--python"].includes(key) || !value || value.startsWith("--") || values.has(key))
      throw new Error(`Invalid data-layer option: ${key}\n${usage}`);
    values.set(key, value);
  }
  if (!["--repo", "--pack", "--snapshot"].every(key => values.has(key))) throw new Error("data-layer requires --repo, --pack and --snapshot");
  const result = await runDataLayerAudit({ repositoryRoot: values.get("--repo")!, packDirectory: values.get("--pack")!,
    snapshot: JSON.parse(readFileSync(values.get("--snapshot")!, "utf8")), ...(values.has("--rules") ? { ruleIds: values.get("--rules")!.split(",") } : {}), ...(values.has("--python") ? { python: values.get("--python")! } : {}) });
  if (flags.has("--json")) stdout.write(JSON.stringify(result) + "\n");
  else {
    for (const row of result.violations) {
      if (!row.span) throw new Error("Data-layer diagnostic has no source span");
      stdout.write(`${row.code} ${row.path}:${row.span.startLine + 1}: ${row.message}\n`);
    }
    if (result.violations.length) stdout.write(`\nFound ${result.violations.length} data-layer violation(s).\n`);
    else if (!result.failures.length) stdout.write("Data-layer linter: clean.\n");
    for (const failure of result.failures) stdout.write(`ERROR ${failure.code}: ${failure.message}\n`);
  }
  return result.failures.length ? 2 : flags.has("--check") && result.violations.some(row => row.mode !== "log") ? 1 : 0;
}
