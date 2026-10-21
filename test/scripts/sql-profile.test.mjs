import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Script } from "node:vm";
import { buildReport, flags } from "../../benchmark/sql-profile/report.mjs";

const query = overrides => ({ kind: "sql", plan: [], bytecode: [], rows: 0, changes: 0, runs: 1, vmSteps: 0,
  cpuNs: 0, fullscanSteps: 0, autoIndexRows: 0, frameIndexBuilds: 0, frameScans: 0, frameNext: 0, ...overrides });
test("hazards distinguish observed counters from plan hypotheses", () => {
  const result = flags(query({ plan: [[0, 0, 0, "CORRELATED SCALAR SUBQUERY 1"], [1, 0, 0, "USE TEMP B-TREE FOR ORDER BY"]],
    vmSteps: 200_000, fullscanSteps: 45, autoIndexRows: 7, frameIndexBuilds: 12 }));
  assert.equal(result.find(f => f.code === "correlated-subquery").certainty, "potential");
  assert.equal(result.find(f => f.code === "temporary-btree").certainty, "potential");
  assert.equal(result.find(f => f.code === "full-scan").certainty, "observed");
  assert.ok(result.some(f => f.code === "high-vm-per-row"));
  assert.ok(result.some(f => f.code === "empty-result-work"));
  assert.ok(result.some(f => f.code === "automatic-index"));
  assert.ok(result.some(f => f.code === "frame-index-rebuilds"));
  assert.deepEqual(flags(query()), []);
});

test("report preserves execution trace, counter order, plans, and exclusive CPU", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-profile-report-"));
  const raw = join(directory, "raw"); mkdirSync(raw);
  const sql = "SELECT '</script><script>alert(1)</script>' FROM frame";
  const catalogue = { version: 1, pid: 9, events: 2, queries: [
    { id: 0, kind: "sql", sql, plan: [[0,0,0,"SCAN frame VIRTUAL TABLE INDEX 0:"]], bytecode: [] },
    { id: 1, kind: "operator", sql: "INSERT INTO output SELECT * FROM frame" },
  ] };
  writeFileSync(join(raw, "profile-9-0.json"), JSON.stringify(catalogue));
  const context = { scope: "clause", statement: 2, inputs: [["frame", 10]] };
  writeFileSync(join(raw, "profile-9-0.jsonl"), [
    { kind: "sql", query: 0, startNs: 1000, wallNs: 100, cpuNs: 80, selfWallNs: 100, selfCpuNs: 80,
      rows: 2000, changes: 0, counters: [1000, 20, 3, 4], frame: [1,2,3,4,5,6,7], context, thread: "ThreadId(1)" },
    { kind: "operator", query: 1, startNs: 900, wallNs: 300, cpuNs: 200, selfWallNs: 200, selfCpuNs: 120,
      rows: 2000, completed: true, context, thread: "ThreadId(1)" },
  ].map(JSON.stringify).join("\n") + "\n");
  try {
    const report = await buildReport(raw, directory);
    assert.equal(report.summary.selfCpuNs, 200);
    assert.equal(report.contexts[0].selfCpuNs, 200);
    const q = report.queries.find(q => q.kind === "sql");
    assert.equal(q.frameScans, 1); assert.equal(q.frameIndexedRows, 7);
    assert.equal(q.vmSteps, 1000); assert.equal(q.autoIndexRows, 4);
    assert.ok(report.queries.find(q => q.kind === "operator").flags.some(f => f.code === "row-expansion"));
    const trace = JSON.parse(readFileSync(join(directory, "trace.json")));
    assert.equal(trace.traceEvents.length, 2); assert.equal(trace.traceEvents[0].ts, 1);
    const html = readFileSync(join(directory, "profile.html"), "utf8");
    assert.doesNotThrow(() => new Script(html.match(/<script>([\s\S]*)<\/script>/)[1]));
    assert.ok(!html.includes(sql)); assert.ok(html.includes("\\u003c/script>"));
    assert.ok(readFileSync(join(directory, "profile.md"), "utf8").includes(sql));
    writeFileSync(join(raw, "profile-9-0.json"), JSON.stringify({ ...catalogue, events: 3 }));
    await assert.rejects(buildReport(raw, directory), /Incomplete trace/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
