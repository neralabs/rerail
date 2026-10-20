// Standalone first-match scaling reproduction. It needs an unbounded-work addon, not an external corpus.
// Usage: node benchmark/sql-set-first-match.mjs --addon /path/engine.node --output /tmp/first-match
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(name);
  return at < 0 ? fallback : args[at + 1];
};
if (!option("--addon")) throw new Error("Provide --addon /absolute/path/unbounded-work.node");
const output = resolve(option("--output", "/tmp/rerail-first-match"));
const depths = option("--depths", "128,256,512,1024,2048,4096").split(",").map(Number);
if (depths.some(depth => !Number.isInteger(depth) || depth < 1 || depth > 4096)) {
  throw new Error("Depths must be integers from 1 through 4096");
}
mkdirSync(output, { recursive: true });
const policy = {
  format: "rerail.policy/v1", id: "first-match-scaling", revision: 1, mode: "steer",
  description: "Measure first-match descendant work.", scope: { include: ["**/*"], exclude: [] },
  clauses: [{ id: "probe", forbid: { ast: { language: "typescript", rule: {
    kind: "if_statement", has: { stopBy: "end", kind: "parenthesized_expression", pattern: "$A" },
  } } }, violation: { anchor: "witness.span", code: "PROBE", message: "Found" } }],
};
const end = resolve(output, "has-end.policy.json");
const neighbor = resolve(output, "has-neighbor.policy.json");
writeFileSync(end, JSON.stringify(policy, null, 2));
policy.clauses[0].forbid.ast.rule.has.stopBy = "neighbor";
writeFileSync(neighbor, JSON.stringify(policy, null, 2));
const summary = [];
for (const depth of depths) {
  const path = resolve(output, `nested-${depth}.ts`);
  const source = Array.from({ length: depth }, (_, i) => `if (a${i}) {\n`).join("") + "f();\n" + "}\n".repeat(depth);
  writeFileSync(path, source);
  const baseConfig = resolve(output, `scale-${depth}.base.json`);
  writeFileSync(baseConfig, JSON.stringify({
    addon: resolve(option("--addon")),
    capabilities: fileURLToPath(new URL("../native/src/code_rule_dsl/capabilities.json", import.meta.url)),
    corpus: output, language: "typescript", files: [path], policies: [end], threads: 4, batchSize: 1,
  }, null, 2));
  const sweep = {
    baseConfig, output: resolve(output, `scale-${depth}.json`), rounds: Number(option("--rounds", "1")),
    variants: [
      { name: "native-end", engine: "native" },
      { name: "native-neighbor", engine: "native", policies: [neighbor] },
      { name: "sql-end" },
      { name: "sql-neighbor", policies: [neighbor] },
    ],
  };
  const config = resolve(output, `scale-${depth}.config.json`);
  writeFileSync(config, JSON.stringify(sweep, null, 2));
  const run = spawnSync(process.execPath, [fileURLToPath(new URL("./sql-set-tuning.mjs", import.meta.url)), config], { stdio: "inherit" });
  if (run.status !== 0) throw new Error(`Depth ${depth} failed; inspect ${sweep.output}`);
  const result = JSON.parse(readFileSync(sweep.output, "utf8"));
  summary.push({ depth, bytes: Buffer.byteLength(source), summary: result.summary,
    work: result.records.map(record => ({ name: record.variant.name, round: record.round,
      findings: record.counts.violations, rows: record.report.statements?.rows, statements: record.report.statements?.count })) });
  writeFileSync(resolve(output, "summary.json"), JSON.stringify(summary, null, 2));
}
