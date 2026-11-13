/** Bounded canonical-column kernel; no data-layer repository scan. */
import { performance } from "node:perf_hooks";
import { codeRuleSchema, loadNativeBinding, resolveNativeBindingPath } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";

const source = Array.from({ length: 20 }, (_, i) => `@decorate\ndef method_${i}(arg: int, *args: str, **kwargs: bool):\n    return {first(): value(), second(): other()}\n`).join("\n");
const files = Array.from({ length: 20 }, (_, i) => ({ path: `kernel_${i}.py`, language: "python", source }));
const authored = codeRuleSchema.parse({ format: "rerail.policy/v1", id: "batch-nine-canonical-kernel", revision: 1, mode: "log", clauses: [{
  id: "canonical", relational: {
    inputs: { gate: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
    syntax: { name: "nodes", gate: "gate", columns: ["__path", "id", "kind", "__start", "__end"] },
    stages: [{ name: "canonical", pythonAst: { syntax: "nodes", roots: "SELECT __path,id FROM nodes WHERE kind='module'", fields: ["ast_kind", "ast_name", "ast_preorder", "ast_parent", "ast_field", "ast_start", "ast_stop"], expand: "nodes" } }],
    query: "SELECT __path path,CAST(ast_start AS INTEGER) start_byte,CAST(ast_stop AS INTEGER) end_byte,ast_preorder identity FROM canonical WHERE ast_kind='arg' AND ast_field='args'", key: ["identity"],
  }, violation: { code: "ARG", message: "Canonical argument", anchor: "witness.span" },
}] });
const native = loadNativeBinding();
let cpu = process.cpuUsage(), start = performance.now();
const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", capabilitiesJson());
const compile = { wallMs: performance.now() - start, nodeCpuMs: totalCpu(cpu) };
const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] });
function totalCpu(previous: NodeJS.CpuUsage): number { const diff = process.cpuUsage(previous); return (diff.user + diff.system) / 1000; }
function evaluate() {
  cpu = process.cpuUsage(); start = performance.now();
  const result = JSON.parse(rule.evaluate(snapshot, "report"));
  const timing = { wallMs: performance.now() - start, nodeCpuMs: totalCpu(cpu), findings: result.violations.length };
  if (result.failures.length || timing.findings !== 400) throw new Error(JSON.stringify({ failures: result.failures, ...timing }));
  return timing;
}
const cold = evaluate(), warm = evaluate();
console.log(JSON.stringify({ kernel: "canonical-columns", addon: resolveNativeBindingPath(), files: files.length, sourceBytes: files.reduce((n,f) => n + Buffer.byteLength(f.source),0), compile, cold, warm }));
