/** Replay complete batch-nine API values. Rule decisions stay in the native engine. */
import { readFileSync } from "node:fs";
import { codeRuleSchema, loadNativeBinding, RESULT_PAGE_BYTES, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
import { replayExecution } from "./execution.js";

export interface NineRecord {
  test: string; check: string; entry: string;
  files: { path: string; sourceSha256: string; source?: string }[];
  parameters: Record<string, unknown>; expected: unknown;
}
export interface NineOracle { cases: Record<string, unknown>; sources: Record<string, string>; records: NineRecord[] }
type Row = Record<string, string | number | null>;
type Finding = { code: string; message: string; path: string; span: { startLine: number }; variables: { row: Row } };
const scalarCompare = (a: unknown, b: unknown): number => typeof a === "number" && typeof b === "number" ? a - b
  : Buffer.compare(Buffer.from(String(a)), Buffer.from(String(b)));

export function nineResult(record: NineRecord, findings: Finding[]): unknown {
  const fileOrder = new Map(record.files.map((file, i) => [file.path, i]));
  const rows = findings.map(f => f.variables.row);
  const order = (r: Row) => {
    const cross = record.check === "connection_service_architecture" && (record.entry === "_lint_duplicate_service_ownership_across_files" || r.api_group === 4);
    const file = fileOrder.get(String(r.path)) ?? 0;
    if (record.check === "bootstrap_cache_health") return [file, r.line, r.issue, r.message];
    return [r.api_group ?? 0, cross ? 0 : file, r.phase ?? 0, r.sort_name ?? "", cross ? file : 0, r.ordering ?? 0, r.issue];
  };
  rows.sort((a, b) => {
    const left = order(a), right = order(b);
    for (let i = 0; i < left.length; i++) { const value = scalarCompare(left[i], right[i]); if (value) return value; }
    return 0;
  });
  return rows.map(r => {
    const path = r.path, line = r.line, code = r.issue, message = r.message;
    switch (record.check) {
      case "connection_mode_split": return { path, line, message };
      case "connection_service_architecture": return { path, line: code === "CONN000" ? null : line, code, detail: message };
      case "schema_architecture": return record.entry === "scan_source"
        ? { linter_name: "schema_architecture", code, path, line, message, severity: "error" }
        : { rule_id: code, path, line, symbol: r.symbol, remediation: message };
      case "schema_bootstrap_contracts": case "bootstrap_lazy_sql": case "bootstrap_eager_resolve": return { path, line, code, detail: message };
      case "plan_composition_architecture": return { path, line, rule: code, message };
      case "bootstrap_cache_health": return record.entry === "collector" ? { path, line, code, message, key: `${code}:${path}:${line}` } : { path, line, code, message };
      default: return { path, line, code, message };
    }
  });
}

export class NineReplay {
  private readonly compiled = new Map<string, NativeCompiledCodeRuleDsl>();
  private readonly execution = replayExecution();
  constructor(private readonly directory = "packs/data-layer/rules", private readonly mutant = false) {}
  close(): void { this.execution.close(); }
  async run(oracle: NineOracle, record: NineRecord, mode: "snapshot" | "workspace"): Promise<unknown> {
    const authored = JSON.parse(readFileSync(`${this.directory}/data-layer-${record.check.replaceAll("_", "-")}.json`, "utf8"));
    for (const clause of authored.clauses) if (clause.define?.options !== undefined)
      clause.define.config = { ...clause.define.config, entryPoint: record.check === "connection_service_architecture" && record.entry === "lint" ? "all" : record.entry,
        parameters: JSON.stringify(record.parameters) };
    if (this.mutant) authored.clauses = authored.clauses.filter((clause: { define?: unknown }) => clause.define !== undefined).concat([{
      id: "disabled", forbid: { regex: { source: "file.content", pattern: "\\b\\B" } }, violation: { code: "CONTROL", message: "Disabled", anchor: "witness.span" },
    }]);
    authored.scope = { include: ["**/*"], exclude: [] };
    const program = JSON.stringify(codeRuleSchema.parse(authored));
    let rule = this.compiled.get(program);
    if (rule === undefined) { rule = loadNativeBinding().CodeRuleDslRuntime.compile(program, "json", capabilitiesJson()); this.compiled.set(program, rule); }
    const files = record.files.map(file => ({ path: file.path, language: "python", source: file.source ?? oracle.sources[file.sourceSha256]! }));
    let result: { violations: Finding[]; failures: { message: string }[] };
    if (mode === "snapshot") result = JSON.parse(this.execution.evaluate(rule, JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report"));
    else {
      const workspace = this.execution.createWorkspace(); result = { violations: [], failures: [] };
      try {
        workspace.addCodeRule(rule); await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [], quiet: true }));
        for (let cursor = 0; ;) {
          const page = JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES));
          result.violations.push(...page.violations); result.failures.push(...page.failures);
          if (page.done) break;
          if (page.cursor <= cursor) throw new Error("Result cursor did not advance");
          cursor = page.cursor;
        }
      } finally { workspace.close(); }
    }
    if (result.failures.length) {
      if (result.failures.every(f => f.message.includes("legacy decorator cardinality"))) return { exception: { type: "ValueError", message: "too many values to unpack (expected 1)" } };
      throw new Error(`${mode}: ${record.test}: ${record.entry}: ${JSON.stringify(result.failures)}`);
    }
    const syntax = result.violations.filter(v => v.code === "NATIVE-SYNTAX");
    if (syntax.length) {
      const file = files.find(f => f.path === syntax[0]!.path)!;
      const response = JSON.parse(loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify([{ source: file.source, requests: [{ start: 0, end: Buffer.byteLength(file.source), kind: "module", fields: ["syntax_error"] }] }])));
      const detail = JSON.parse(response.documents[0][0].syntax_error);
      const filename = ["schema_architecture", "schema_bootstrap_contracts", "bootstrap_lazy_sql", "bootstrap_eager_resolve", "plan_composition_architecture", "bootstrap_cache_health"].includes(record.check) ? file.path.split("/").at(-1) : "<unknown>";
      const message = `${detail.msg} (${filename}, line ${detail.lineno})`;
      return { exception: { type: "SyntaxError", message, ...detail } };
    }
    for (const finding of result.violations) {
      const row = finding.variables.row;
      if (finding.code !== row.issue || finding.message !== row.message || finding.path !== row.path
        || (row.issue !== "CONN000" && finding.span.startLine + 1 !== row.line))
        throw new Error(`Public diagnostic differs from the replay DTO: ${record.test}: ${JSON.stringify(finding)}`);
    }
    return nineResult(record, result.violations);
  }
}
