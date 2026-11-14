import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";
import { capabilitiesJson } from "../../../conformance/engine.js";
const audit = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-09-audit-20261105.json", "utf8"));
for (const spec of audit.publicDefaultEvaluations.filter((r: { id: string }) => r.id !== "composition-default")) {
  it(`preserves upstream ${spec.id} through the shipped scope and original entry point`, () => {
    const document = JSON.parse(readFileSync(`packs/data-layer/rules/data-layer-${spec.check}.json`, "utf8"));
    // This historical case came from scan_file(), rather than the configured plugin.
    if (spec.check === "bootstrap-cache-health") document.clauses[0].define.config = { entryPoint: "scan_file" };
    const authored = codeRuleSchema.parse(document);
    const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", capabilitiesJson());
    const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: spec.path, source: spec.source, language: "python" }] }), "report"));
    expect(result.failures).toEqual([]);
    expect(result.violations.map((r: { code: string; message: string; span: { startLine: number } }) => ({ code: r.code, message: r.message, line: r.span.startLine + 1 })))
      .toEqual(spec.expected.map((r: { code: string; message: string; line: number }) => ({ code: r.code, message: r.message, line: r.line })));
  });
}

const corpus = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-09-complete-api.json", "utf8"));
const defaults = [
  { check: "connection_mode_split", entry: "_violations_for_files", path: "api/probe.py" },
  { check: "connection_structural_boundaries", entry: "lint" },
  { check: "connection_service_architecture", entry: "lint" },
  { check: "schema_architecture", entry: "scan_source", path: "l2000_connection/probe.py" },
  { check: "schema_bootstrap_contracts", entry: "lint_file" },
  { check: "bootstrap_engine_invariants", entry: "_collect_file_violations", path: "l3200_metadata/bootstrap_coordinator.py" },
  { check: "bootstrap_lazy_sql", entry: "scan_source", path: "l3200_metadata/bootstrap_state.py" },
  { check: "bootstrap_eager_resolve", entry: "scan_source", path: "l3200_metadata/bootstrap_state.py" },
  { check: "bootstrap_cache_health", entry: "scan_file", path: "conform/test_probe.py" },
];
for (const mode of ["snapshot", "workspace"]) it.each(defaults)(`${mode} retains shipped defaults and scope for $check`, async spec => {
  const record = corpus.records.find((r: any) => r.check === spec.check && r.entry === spec.entry && Array.isArray(r.expected) && r.expected.length
    && (!spec.path || spec.check !== "bootstrap_engine_invariants" || r.files.some((f: any) => f.path === spec.path)));
  expect(record, "A production-context original positive is required").toBeDefined();
  const files = record.files.map((f: any) => ({ path: spec.path ?? f.path, source: corpus.sources[f.sourceSha256], language: "python" }));
  const remap = new Map(record.files.map((f: any, i: number) => [f.path, files[i].path]));
  const authored = codeRuleSchema.parse(JSON.parse(readFileSync(`packs/data-layer/rules/data-layer-${spec.check.replaceAll("_", "-")}.json`, "utf8")));
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", capabilitiesJson());
  const expected = record.expected.map((r: any) => ({ path: remap.get(r.path), code: r.code ?? "connection_mode_split", message: r.message ?? r.detail, line: r.line ?? 1 }));
  const sort = (rows: any[]) => rows.sort((a,b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
  async function evaluate(inputFiles: typeof files) {
    if (mode === "snapshot") return JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: inputFiles, facts: [] }), "report"));
    const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
    try {
      workspace.addCodeRule(rule); await workspace.applyAsync(JSON.stringify({ revision: 1, files: inputFiles, removed: [], quiet: true }));
      return JSON.parse(await workspace.resultPageAsync(0, 4 * 1024 * 1024));
    } finally { workspace.close(); }
  }
  const actual = await evaluate(files);
  expect(actual.failures).toEqual([]);
  expect(sort(actual.violations.map((r: any) => ({ path: r.path, code: r.code, message: r.message, line: r.span.startLine + 1 })))).toEqual(sort(expected));
  const excluded = files.map((f: any, i: number) => ({ ...f, path: `unrelated/${i}.py` }));
  expect((await evaluate(excluded)).violations).toEqual([]);
  for (const f of files) expect(rule.inScope(f.path)).toBe(true);
  expect(rule.inScope("unrelated/outside.py")).toBe(false);
});
