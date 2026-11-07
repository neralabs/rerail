/** Focused cold measurements of batch-eight producers; never loads the rule batches. */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { performance } from "node:perf_hooks";
import { codeRuleSchema, loadNativeBinding, resolveNativeBindingPath } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";

const { values } = parseArgs({ options: { kernel: { type: "string", default: "graph" }, seconds: { type: "string", default: "0" } } });
if (!["graph", "graph-state", "python"].includes(values.kernel)) throw new Error("Unknown kernel");
const duration = Number(values.seconds) * 1000;
if (!Number.isFinite(duration) || duration < 0 || duration > 10_000) throw new Error("Duration must be in [0,10] seconds");
const cpuMs = (usage: NodeJS.CpuUsage) => (usage.user + usage.system) / 1000;
const phase = async <T>(fn: () => T | Promise<T>) => {
  const cpu = process.cpuUsage(), wall = performance.now(); const value = await fn();
  return { value, cpuMs: cpuMs(process.cpuUsage(cpu)), wallMs: performance.now() - wall };
};
const native = loadNativeBinding();
const sequence = "WITH RECURSIVE seq(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM seq WHERE i<999) ";
const compiled = await phase(() => {
  if (values.kernel === "python") return undefined;
  const state = values.kernel === "graph-state" ? {
    memo: "SELECT 'batch' __path,'500' id,NULL node,777 location,0 ordinal",
    visiting: "SELECT 'batch' __path,'400' id",
  } : {};
  const policy = { format: "rerail.policy/v1", id: "batch-eight-kernel", revision: 1, mode: "log", clauses: [{ id: "graph", relational: {
    inputs: { unused: { evidence: { as: "file", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
    stages: [{ name: "matches", reachability: { selection: "memoized-first", match: "edge",
      roots: sequence + "SELECT 'batch' __path,CAST(i AS TEXT) id,i ordinal FROM seq",
      edges: sequence + "SELECT 'batch' __path,CAST(i AS TEXT) src,CAST(i+1 AS TEXT) dst,i ordinal,i+1 location FROM seq",
      sinks: "SELECT 'batch' __path,'1000' id,1000 location", ...state,
    } }], locations: ["metadata"], query: "SELECT 'metadata' path,0 start_byte,0 end_byte,root identity,root,location FROM matches", key: ["identity"],
  }, violation: { code: "REACHABLE", message: "Reachable", anchor: "witness.span" } }] };
  return native.CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(policy)), "json", capabilitiesJson());
});
const documents = Array.from({ length: 20 }, () => {
  const source = "def f(value: int) -> int:\n    return value * 2\n";
  return { source, requests: [{ start: 0, end: Buffer.byteLength(source), kind: "function_definition", fields: ["body_sha256", "signature_sha256"] }] };
});
const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [] });
function execute() {
  if (compiled.value === undefined) {
    const result = JSON.parse(native.CodeRuleDslRuntime.pythonAstBatch(JSON.stringify(documents)));
    if (result.documents.length !== 20 || result.documents.some((rows: { body_sha256: string }[]) => rows[0]!.body_sha256 !== "7e783fc807de80e8856ea7d18c4321d18b0ac3c95691c8267538f7c1957c4ced")) throw new Error("Canonical fingerprint mismatch");
    return result.documents.length;
  }
  const result = JSON.parse(compiled.value.evaluate(snapshot, "report"));
  const expected = values.kernel === "graph-state" ? 599 : 1000;
  if (result.failures.length || result.violations.length !== expected) throw new Error(JSON.stringify(result));
  for (const row of result.violations) {
    const id = Number(row.variables.row.root), location = Number(row.variables.row.location);
    if (location !== (values.kernel === "graph-state" && id === 500 ? 777 : id + 1)) throw new Error("Ordered location mismatch");
  }
  return result.violations.length;
}
process.stdout.write(JSON.stringify({ pid: process.pid, kernel: values.kernel }) + "\n");
const cold = await phase(execute);
let iterations = 0;
const warm = await phase(() => {
  const until = performance.now() + duration;
  while (performance.now() < until) { execute(); iterations++; }
});
process.stdout.write(JSON.stringify({ format: "rerail.batch-eight-kernels/v1", kernel: values.kernel,
  engineSha256: createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex"),
  compile: { cpuMs: compiled.cpuMs, wallMs: compiled.wallMs }, cold,
  warm: { iterations, cpuMs: warm.cpuMs, wallMs: warm.wallMs }, maxRssKiB: process.resourceUsage().maxRSS,
  boundary: "Node phase CPU excludes Python children. Use external time for process-family CPU. One kernel only; no rule-batch cold scan.",
}) + "\n");
