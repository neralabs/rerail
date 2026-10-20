// Aggregate diagnostic batches without mixing sampled diagnostics with latency measurements.
// Usage: node benchmark/sql-set-shapes.mjs /path/to/trace-directory /path/to/report.json
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [directory, output] = process.argv.slice(2);
const statements = new Map();
const metadata = [];
const fields = ["runs", "rows", "wallNs", "cpuNs", "vmSteps", "fullscanSteps", "sorts", "autoIndexRows"];
for (const name of readdirSync(directory).filter(name => name.endsWith(".json"))) {
  const document = JSON.parse(readFileSync(join(directory, name), "utf8"));
  if (name.startsWith("metadata-")) {
    metadata.push(document);
    continue;
  }
  for (const record of Object.values(document)) {
    let aggregate = statements.get(record.sql);
    if (aggregate === undefined) {
      aggregate = { sql: record.sql, inputPeakRows: {}, plans: new Map(), ...Object.fromEntries(fields.map(key => [key, 0])) };
      statements.set(record.sql, aggregate);
    }
    for (const key of fields) aggregate[key] += record[key];
    for (const [name, count] of Object.entries(record.inputPeakRows)) {
      aggregate.inputPeakRows[name] = Math.max(count, aggregate.inputPeakRows[name] ?? 0);
    }
    aggregate.plans.set(JSON.stringify(record.plan), record.plan);
  }
}
const family = sql => {
  if (/SET dirty = 1/.test(sql)) return "dirty-propagation";
  if (/SET dirty = 0/.test(sql)) return "dirty-reset";
  if (/INSERT.*INTO c\d+_e\d+ /.test(sql)) return "dependency-edges";
  if (/INSERT.*INTO c\d+_d\d+/.test(sql)) return "demand-insertion";
  if (/INSERT.*INTO c\d+_c\d+/.test(sql)) return "answer-difference";
  if (/JOIN c\d+_a\d+ r/.test(sql)) return "answer-lookup";
  if (/DELETE FROM c\d+_e\d+/.test(sql)) return "dependency-deletion";
  return "other";
};
const families = {};
const ranked = [...statements.values()].sort((a, b) => b.cpuNs - a.cpuNs).map(record => {
  const kind = family(record.sql);
  const total = families[kind] ??= Object.fromEntries(fields.map(key => [key, 0]));
  for (const key of fields) total[key] += record[key];
  return { ...record, family: kind, plans: [...record.plans.values()], fullscanPerOutputRow: record.fullscanSteps / Math.max(record.rows, 1) };
});
const totals = Object.fromEntries(fields.map(key => [key, ranked.reduce((sum, record) => sum + record[key], 0)]));
writeFileSync(output, JSON.stringify({ directory, metadata, totals, families, statements: ranked }, null, 2) + "\n");
console.log(JSON.stringify({ totals, families }, null, 2));
