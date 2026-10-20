/** Bounded SQL access experiments over real parser tapes. One backend per process. */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { performance } from "node:perf_hooks";
import { loadNativeBinding, resolveNativeBindingPath } from "../../src/engine/index.js";

type Spike = { query(sql: string): string };
type Reply = { rows: unknown[][]; queryMs: number; buildMs: number; nodes: number; stats: unknown; ownerLookupWork: number; ownerIndexEntries: number };
const queries = {
  kinds: "SELECT kind,count(*) FROM nodes GROUP BY kind ORDER BY kind",
  calls: "SELECT count(*) FROM nodes WHERE kind IN ('call','attribute')",
  parentJoin: "SELECT count(*) FROM nodes c JOIN nodes p ON p.id=c.parent WHERE c.kind='call'",
  ownerJoin: "SELECT f.id,count(n.id) FROM nodes f JOIN nodes n ON n.fn=f.id WHERE f.kind='function_definition' GROUP BY f.id ORDER BY f.id",
  lazyText: "SELECT sum(length(node_text(id))) FROM nodes WHERE kind='call'",
  descendants: "SELECT root,count(*) FROM reachability WHERE graph=tree_handle() AND root IN (SELECT id FROM nodes WHERE kind='function_definition') GROUP BY root ORDER BY root",
  bindings: "SELECT count(*) FROM native_bindings",
  resolvedCalls: "SELECT count(*) FROM nodes c JOIN nodes n ON n.parent=c.id AND n.kind='identifier' AND n.start_byte=c.start_byte JOIN native_bindings b ON b.reference_start=n.start_byte AND b.reference_end=n.end_byte WHERE c.kind='call'",
};
const { values } = parseArgs({ options: { recording: { type: "string" }, backend: { type: "string" }, output: { type: "string" }, limit: { type: "string", default: "1" }, repeat: { type: "string", default: "1" } } });
if (!values.recording || !values.backend || !values.output || !["vtab", "materialized", "narrow"].includes(values.backend)) throw new Error("Required: --recording --backend vtab|materialized|narrow --output");
const limit = Number(values.limit), repeat = Number(values.repeat);
if (!Number.isSafeInteger(limit) || limit < 1 || limit > 5 || !Number.isSafeInteger(repeat) || repeat < 1 || repeat > 20) throw new Error("Invalid limits");
const native = loadNativeBinding() as ReturnType<typeof loadNativeBinding> & { NativeSqlSpike: new(path: string, source: string, backend: string) => Spike };
const files = (JSON.parse(readFileSync(values.recording, "utf8")) as { files: { path: string; source: string }[] }).files.slice(0, limit);
const started = performance.now(), cpu = process.cpuUsage();
const rows = [];
for (let iteration = 0; iteration < repeat; iteration++) {
  for (const file of files) {
    const begin = performance.now();
    const db = new native.NativeSqlSpike(file.path, file.source, values.backend);
    const metrics = [];
    for (let pass = 0; pass < 2; pass++) {
      for (const [name, sql] of Object.entries(queries)) {
        const reply = JSON.parse(db.query(sql)) as Reply;
        const { rows: result, ...stats } = reply;
        metrics.push({ name, pass, rows: result.length, digest: createHash("sha256").update(JSON.stringify(result)).digest("hex"), ...stats });
      }
    }
    rows.push({ path: file.path, sourceBytes: Buffer.byteLength(file.source), iteration, wallMs: performance.now() - begin, metrics });
  }
  globalThis.gc?.();
}
const usage = process.cpuUsage(cpu);
const report = { format: "rerail.native-tape-spike/v1", backend: values.backend, nativeSha256: createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex"),
  wallMs: performance.now() - started, cpuMs: (usage.user + usage.system) / 1000, peakRssBytes: process.resourceUsage().maxRSS * 1024, rows,
  note: "Fresh tapes per file; parsing, metadata and optional SQL materialization included. SQL suite is an access microbenchmark, not complete linter execution." };
writeFileSync(values.output, JSON.stringify(report, null, 2) + "\n");
console.log(JSON.stringify({ ...report, rows: rows.length }));
