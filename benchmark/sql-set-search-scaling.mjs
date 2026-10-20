// Compare early, late, and missing matches without an external corpus.
// Usage: node benchmark/sql-set-search-scaling.mjs --addon /path/engine.node --output /tmp/search-scaling
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
const output = resolve(option("--output", "/tmp/rerail-search-scaling"));
const depths = option("--depths", "128,512,2048,4096").split(",").map(Number);
const rounds = Number(option("--rounds", "1"));
if (depths.some(depth => !Number.isInteger(depth) || depth < 1 || depth > 4096)
    || !Number.isInteger(rounds) || rounds < 1) {
  throw new Error("Depths must be integers from 1 through 4096; rounds must be a positive integer");
}
mkdirSync(output, { recursive: true });
const summary = [];
for (const [name, kind, regex, pattern] of [
  ["early", "parenthesized_expression", ".", "$X"],
  ["late", "identifier", "^target$", "$X"],
  ["missing", "call_expression", ".", "f($X, $X)"],
]) {
  const policy = resolve(output, `${name}.policy.json`);
  writeFileSync(policy, JSON.stringify({
    format: "rerail.policy/v1", id: `search-scaling-${name}`, revision: 1, mode: "steer",
    description: "Measure first-match composition.", scope: { include: ["**/*"], exclude: [] },
    clauses: [{ id: "search", forbid: { ast: { language: "typescript", rule: {
      kind: "if_statement", has: { stopBy: "end", kind, regex, pattern },
    } } }, violation: { anchor: "witness.span", code: "SEARCH", message: "Found" } }],
  }, null, 2));
  for (const depth of depths) {
    const path = resolve(output, `${name}-${depth}.ts`);
    // Negative candidates have all required tokens and kinds. They fail repeated-variable
    // unification, which the workspace's exact regex planner cannot decide before SQL.
    const source = Array.from({ length: depth }, (_, i) =>
      `if (a${i}) {${name === "missing" ? " f(a, b);" : ""}\n`).join("") + "target;\n" + "}\n".repeat(depth);
    writeFileSync(path, source);
    const baseConfig = resolve(output, `${name}-${depth}.base.json`);
    writeFileSync(baseConfig, JSON.stringify({
      addon: resolve(option("--addon")),
      capabilities: fileURLToPath(new URL("../native/src/code_rule_dsl/capabilities.json", import.meta.url)),
      corpus: output, language: "typescript", files: [path], policies: [policy], threads: 4, batchSize: 1,
    }, null, 2));
    const sweep = {
      baseConfig, output: resolve(output, `${name}-${depth}.json`), rounds,
      variants: [
        { name: "native", engine: "native" },
        { name: "sql-default" },
        { name: "sql-shared", environment: { RERAIL_SQL_SHARED_SEARCH: "1" } },
      ],
    };
    const config = resolve(output, `${name}-${depth}.config.json`);
    writeFileSync(config, JSON.stringify(sweep, null, 2));
    const run = spawnSync(process.execPath, [fileURLToPath(new URL("./sql-set-tuning.mjs", import.meta.url)), config], { stdio: "inherit" });
    if (run.status !== 0) throw new Error(`${name}, depth ${depth} failed; inspect ${sweep.output}`);
    const result = JSON.parse(readFileSync(sweep.output, "utf8"));
    for (const record of result.records) {
      if (record.counts.violations !== (name === "missing" ? 0 : depth)) {
        throw new Error(`${name}, depth ${depth}: unexpected finding count`);
      }
      if (record.engine === "sql" && !(record.report.statements?.count > 0)) {
        throw new Error(`${name}, depth ${depth}: SQL was skipped; this is not a search benchmark`);
      }
    }
    summary.push({ name, depth, bytes: Buffer.byteLength(source), summary: result.summary,
      work: result.records.map(record => ({ variant: record.variant.name, round: record.round,
        findings: record.counts.violations, rows: record.report.statements?.rows, statements: record.report.statements?.count })) });
    writeFileSync(resolve(output, "summary.json"), JSON.stringify(summary, null, 2));
  }
}
