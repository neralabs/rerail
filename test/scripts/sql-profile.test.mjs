import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Script } from "node:vm";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { actionableSummary, sqlShape } from "../../benchmark/sql-profile/summary.mjs";
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
    assert.ok(readFileSync(join(directory, "summary.md"), "utf8").includes("Priority actions"));
    const originalTrace = readFileSync(join(directory, "trace.json"));
    const stdout = execFileSync(process.execPath, [fileURLToPath(new URL("../../benchmark/sql-profile/summarize.mjs", import.meta.url)), directory], { encoding: "utf8" });
    assert.ok(stdout.includes("Recorded CPU"));
    assert.deepEqual(readFileSync(join(directory, "trace.json")), originalTrace);
    writeFileSync(join(raw, "profile-9-0.json"), JSON.stringify({ ...catalogue, events: 3 }));
    await assert.rejects(buildReport(raw, directory), /Incomplete trace/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});


test("action priorities use exclusive CPU and never count a query in two actions", () => {
  const queries = [
    query({ id: 1, sql: "INSERT OR IGNORE INTO roots(run, file, node) VALUES (?1,?2,?3)", selfCpuNs: 100_000_000, runs: 100_000, changes: 100_000 }),
    query({ id: 2, sql: "SELECT expensive FROM node", selfCpuNs: 800_000_000, runs: 1 }),
    query({ id: 3, kind: "operator", sql: "frame publication", cpuNs: 900_000_000, selfCpuNs: 100_000_000 }),
    query({ id: 4, sql: "CREATE TABLE tiny(n)", selfCpuNs: 1, flags: [{ code: "explain-unavailable" }], plans: [{ plan: { error: "table tiny already exists" }, bytecode: { error: "table tiny already exists" } }] }),
    query({ id: 5, sql: "SELECT empty FROM frame", selfCpuNs: 200_000_000, flags: [{ code: "empty-result-work" }, { code: "frame-index-rebuilds" }] }),
  ];
  const summary = actionableSummary({ queries, workload: { results: { parity: false, sql: { apply: { cpuMs: 2500 } } } } });
  assert.equal(summary.status, "parity-failed");
  assert.match(summary.nextStep, /Resolve the output mismatch/);
  assert.ok(Math.abs(summary.budget.measuredCpuMs - 1200) < 0.001);
  assert.equal(summary.budget.profiledApplyCpuMs, 2500);
  assert.equal(summary.actions[0].representativeQueryIds[0], 2);
  const assigned = summary.actions.flatMap(action => action.queryIds);
  assert.equal(assigned.length, new Set(assigned).size);
  assert.equal(assigned.filter(id => id === 5).length, 1);
  assert.ok(!assigned.includes(4));
  assert.equal(summary.quality.expectedSetupExplainFailures, 1);
  assert.equal(summary.quality.unexpectedExplainFailures, 0);
  assert.equal(summary.rootInput.executions, 100_000);
  assert.ok(summary.rootInput.explanation.includes("surrounding transaction"));
  assert.ok(Math.abs(summary.costBreakdown.reduce((sum, c) => sum + c.cpuMs, 0) - summary.budget.measuredCpuMs) < 1e-6);
});

test("shape grouping preserves literals and table relationships", () => {
  const shape = sql => sqlShape(sql);
  assert.equal(shape("SELECT a.k FROM f7_0 a JOIN f8_1 b ON a.k=b.k WHERE a.x=5"), shape("SELECT a.k FROM f90_0 a JOIN f1_1 b ON a.k=b.k WHERE a.x=9"));
  assert.notEqual(shape("SELECT a.k FROM f7_0 a JOIN f8_1 b ON a.k=b.k"), shape("SELECT a.k FROM f7_0 a JOIN f7_0 b ON a.k=b.k"));
  assert.notEqual(shape("SELECT 'f7_0,123'"), shape("SELECT 'f8_0,456'"));
  assert.ok(shape("SELECT 'FROM f7_0 WHERE n = 8', \"f7_0\" FROM f7_0").includes("'FROM f7_0 WHERE n = 8'"));
});

test("unexpected explanation failures stay visible and native descriptors require no plan", () => {
  const summary = actionableSummary({ workload: {}, queries: [
    query({ id: 1, kind: "operator", sql: "/*FRONTIER*/roots", selfCpuNs: 100 }),
    query({ id: 2, sql: "SELECT removed FROM stale", selfCpuNs: 1, flags: [{ code: "explain-unavailable" }], plans: [{ plan: { error: "no such table: stale" } }] }),
    query({ id: 3, sql: "CREATE TABLE incomplete(n)", selfCpuNs: 1, flags: [{ code: "explain-unavailable" }], plans: [] }),
  ] });
  assert.equal(summary.quality.expectedSetupExplainFailures, 0);
  assert.equal(summary.quality.unexpectedExplainFailures, 2);
  assert.equal(summary.status, "parity-unavailable");
  assert.equal(summary.budget.profiledApplyCpuMs, null);
});


test("tiny captures do not turn timer noise into optimization priorities", () => {
  const summary = actionableSummary({ workload: {}, queries: [
    query({ id: 1, sql: "SELECT n FROM frame", selfCpuNs: 500_000, flags: [{ code: "full-scan" }] }),
    query({ id: 2, sql: "INSERT OR IGNORE INTO roots(run,file,node) VALUES (?1,?2,?3)", selfCpuNs: 10_000, runs: 3 }),
  ] });
  assert.deepEqual(summary.actions, []);
  assert.equal(summary.rootInput.prioritized, false);
  assert.match(summary.rootInput.nextChange, /below the action threshold/);
});


test("a multi-row input statement is not diagnosed as one call per root", () => {
  const summary = actionableSummary({ workload: {}, queries: [
    query({ id: 1, sql: "INSERT OR IGNORE INTO roots(run,file,node) VALUES (?1,?2,?3),(?4,?5,?6)", selfCpuNs: 100_000_000, runs: 10, changes: 20 }),
  ] });
  assert.equal(summary.rootInput.executions, 0);
  assert.ok(!summary.actions.some(action => action.key === "root-input-loading"));
  assert.equal(summary.costBreakdown[0].label, "Other input-table SQL");
});
