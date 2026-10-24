import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Script } from "node:vm";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
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
    assert.equal(report.actionableSummary.budget.frameIndexCpuMs, null);
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
    const child = spawnSync(process.execPath, [fileURLToPath(new URL("../../benchmark/sql-profile/summarize.mjs", import.meta.url)), directory], { encoding: "utf8" });
    assert.equal(child.status, 1); // Historical captures lack complete coverage and parity proof.
    assert.match(child.stderr, /invalid-capture/);
    assert.ok(child.stdout.includes("Recorded CPU"));
    assert.deepEqual(readFileSync(join(directory, "trace.json")), originalTrace);
    // Mixed captures retain coverage instead of treating untimed legacy builds as free.
    const events = readFileSync(join(raw, "profile-9-0.jsonl"), "utf8").trim().split("\n").map(JSON.parse);
    events.push({ ...events[0], frame: [1,2,3,4,5,6,7,11,13] });
    writeFileSync(join(raw, "profile-9-0.jsonl"), events.map(JSON.stringify).join("\n") + "\n");
    writeFileSync(join(raw, "profile-9-0.json"), JSON.stringify({ ...catalogue, events: 3 }));
    const timed = await buildReport(raw, directory);
    const timedQuery = timed.queries.find(q => q.kind === "sql");
    assert.equal(timedQuery.frameIndexBuilds, 12);
    assert.equal(timedQuery.frameIndexTimedBuilds, 6);
    assert.equal(timedQuery.frameIndexCpuNs, 11);
    assert.equal(timedQuery.frameIndexWallNs, 13);
    assert.equal(timed.summary.selfCpuNs, 280); // Index CPU is a subcost, not additional CPU.
    assert.equal(timed.actionableSummary.budget.frameIndexCpuMs, 11 / 1e6);
    assert.equal(timed.actionableSummary.budget.frameIndexTimedBuilds, 6);
    writeFileSync(join(raw, "profile-9-0.json"), JSON.stringify({ ...catalogue, events: 4 }));
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

test("cheap measured indexes do not assign whole-query CPU to index construction", () => {
  const summary = actionableSummary({ workload: {}, queries: [
    query({ id: 1, sql: "WITH p AS MATERIALIZED (SELECT file, node FROM pattern WHERE term=1) SELECT * FROM p", selfCpuNs: 500_000_000,
      frameIndexBuilds: 16, frameIndexTimedBuilds: 16, frameIndexCpuNs: 300_000, flags: [{ code: "frame-index-rebuilds" }] }),
    query({ id: 2, sql: "SELECT * FROM f1_0 WHERE x=1", selfCpuNs: 100_000_000,
      frameIndexBuilds: 16, frameIndexTimedBuilds: 16, frameIndexCpuNs: 200_000, flags: [{ code: "frame-index-rebuilds" }] }),
    query({ id: 3, sql: "SELECT * FROM f2_0 WHERE y=1", selfCpuNs: 100_000_000,
      frameIndexBuilds: 16, frameIndexTimedBuilds: 16, frameIndexCpuNs: 50_000_000, flags: [{ code: "frame-index-rebuilds" }] }),
  ] });
  assert.equal(summary.actions[0].key, "terminal-materialization");
  assert.equal(summary.actions[0].frameIndexCpuMs, 0.3);
  assert.deepEqual(summary.actions.find(a => a.key === "frame-index-rebuilds").queryIds, [3]);
  assert.equal(summary.budget.measuredCpuMs, 700);
  assert.equal(summary.budget.frameIndexCpuMs, 50.5);
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


test("virtual root publication reports removal of the insertion path", () => {
  const summary = actionableSummary({ workload: {}, queries: [
    query({ id: 1, kind: "operator", sql: "/*ROOT_INPUT*/ publish selected root slices", selfCpuNs: 1_000_000, runs: 16, rows: 919_765 }),
    query({ id: 2, sql: "SELECT * FROM roots WHERE run=0", selfCpuNs: 500_000_000, runs: 16 }),
  ] });
  assert.equal(summary.rootInput.mode, "virtual-slices");
  assert.equal(summary.rootInput.executions, 0);
  assert.equal(summary.rootInput.publication.executions, 16);
  assert.equal(summary.rootInput.publication.returnedRows, 919_765);
  assert.match(summary.rootInput.explanation, /without per-root INSERT/);
  assert.ok(!summary.actions.some(action => action.key === "root-input-loading"));
});

test("CTAS rows are unobserved, rather than diagnosed as an empty result", () => {
  const result = flags(query({ sql: 'CREATE TEMP TABLE "derived" AS SELECT * FROM inputs', vmSteps: 2_000_000, fullscanSteps: 1000 }));
  assert.ok(result.some(flag => flag.code === "full-scan"));
  assert.ok(!result.some(flag => ["empty-result-work", "high-vm-per-row"].includes(flag.code)));
});

test("relational priorities retain materialization costs and refer to this executor", () => {
  const summary = actionableSummary({ workload: { executor: "relational" }, queries: [
    query({ id: 1, sql: 'CREATE TEMP TABLE "names" AS SELECT * FROM syntax_nodes', selfCpuNs: 400_000_000, contextDetails: [{ scope: "relational", stage: "names" }] }),
    query({ id: 2, sql: 'CREATE INDEX "by_id" ON "names"(id)', selfCpuNs: 100_000_000, contextDetails: [{ scope: "relational", stage: "names" }] }),
  ] });
  assert.match(summary.actions[0].title, /materialization/);
  assert.equal(summary.actions.length, 2);
  assert.ok(summary.actions.every(action => action.source.includes("relational.rs")));
  assert.equal(summary.costBreakdown[0].label, "Relation materialization SQL");
  assert.match(summary.rootInput.explanation, /no per-root/);
});

test("relational contexts retain phase, clause, file and stage; partial captures fail", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-relational-profile-"));
  const raw = join(directory, "raw"); mkdirSync(raw);
  const catalogue = { version: 1, pid: 1, originUnixUs: 1000, events: 1, queries: [
    { id: 0, kind: "sql", sql: 'CREATE TEMP TABLE "derived" AS SELECT * FROM inputs', planSql: 'SELECT * FROM inputs', plan: [], bytecode: [] },
  ] };
  writeFileSync(join(raw, "profile-1-0.json"), JSON.stringify(catalogue));
  writeFileSync(join(raw, "profile-1-0.jsonl"), JSON.stringify({ kind: "sql", query: 0, startNs: 10000, wallNs: 10, cpuNs: 5,
    selfCpuNs: 5, selfWallNs: 10, rows: 0, changes: 0, context: { scope: "relational", ruleId: "rule", clauseId: "clause", path: "a.py", stage: "derived" }, thread: "worker" }) + "\n");
  try {
    const report = await buildReport(raw, directory, { phaseMarkers: [{ phase: "cold", startUnixUs: 1005 }] });
    assert.equal(report.contexts[0].runPhase, "cold");
    assert.equal(report.contexts[0].clauseId, "clause");
    assert.equal(report.contexts[0].path, "a.py");
    assert.equal(report.queries[0].plans[0].planSql, "SELECT * FROM inputs");
    writeFileSync(join(raw, "profile-1-1.jsonl"), "");
    await assert.rejects(buildReport(raw, directory), /missing catalogue/);
    writeFileSync(join(raw, "profile-1-1.json"), JSON.stringify({ ...catalogue, error: "trace callback failed" }));
    await assert.rejects(buildReport(raw, directory), /trace callback failed/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("completed schema operations have expected explanation failures", () => {
  const pairs = [
    ['CREATE INDEX by_id ON data(id)', 'index by_id already exists'],
    ['CREATE TEMP VIEW v AS SELECT * FROM data', 'view v already exists'],
    ['DROP TABLE data', 'no such table: data'],
    ['ALTER TABLE data ADD COLUMN text', 'duplicate column name: text'],
  ];
  const queries = pairs.map(([sql, error], id) => query({ id, sql, selfCpuNs: 1,
    flags: [{ code: 'explain-unavailable' }], plans: [{ plan: { error }, bytecode: { error } }] }));
  queries.push(query({ id: 5, sql: 'SELECT * FROM missing', selfCpuNs: 1,
    flags: [{ code: 'explain-unavailable' }], plans: [{ plan: { error: 'no such table: missing' } }] }));
  const summary = actionableSummary({ workload: {}, queries });
  assert.equal(summary.quality.expectedSetupExplainFailures, 4);
  assert.equal(summary.quality.unexpectedExplainFailures, 1);
});

// An FTS vocabulary cursor can stay open while many outer statements run.
test("overlapping cursor lifetimes cannot become CPU priorities", () => {
  const summary = actionableSummary({ workload: { results: { parity: true, sql: { apply: { cpuMs: 1000 } } } }, queries: [
    query({ id: 1, sql: "SELECT * FROM postings", selfCpuNs: 5_000_000_000 }),
    query({ id: 2, sql: "SELECT * FROM selected", selfCpuNs: 800_000_000 }),
  ] });
  assert.equal(summary.quality.unreconciledCpu, true);
  assert.equal(summary.budget.cpuCostsReconciled, false);
  assert.match(summary.nextStep, /API boundary timers/);
  assert.deepEqual(summary.actions, []);
});


test("execution inventory retains event attribution when identical SQL crosses producers and workers", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-inventory-"));
  const raw = join(directory, "raw"); mkdirSync(raw);
  const events = [
    { context: { scope: "compile", producer: "validation", path: "/rules/0/clauses/0" }, thread: "main" },
    { context: { component: "ast-index", stage: "ast-index-put", path: "a.py", generation: "v1" }, thread: "worker-1" },
    { context: { component: "ast-index", stage: "candidate-preparation", producer: "anchors", binding: "python", fileDomain: ["a.py", "b.py"] }, thread: "worker-2" },
    { context: { scope: "relational", ruleId: "R1", clauseId: "C1", stage: "result" }, thread: "worker-1" },
    { context: {}, thread: "worker-3" },
  ].map((event, i) => ({ ...event, kind: "sql", query: 0, startNs: i, cpuNs: 10, selfCpuNs: 10, wallNs: 10, selfWallNs: 10, rows: 0, changes: 0 }));
  events.push({ ...events[0], kind: "operator", query: 1, selfCpuNs: 1000, completed: true });
  writeFileSync(join(raw, "profile-9-0.json"), JSON.stringify({ version: 1, pid: 9, events: events.length, queries: [
    { id: 0, kind: "sql", sql: "SELECT 1", plan: [], bytecode: [] }, { id: 1, kind: "operator", sql: "enclosing operator" },
  ] }));
  writeFileSync(join(raw, "profile-9-0.jsonl"), events.map(JSON.stringify).join("\n") + "\n");
  try {
    const report = await buildReport(raw, directory);
    assert.equal(report.executionInventory.total, 5);
    assert.equal(report.executionInventory.selfCpuNs, 50);
    assert.deepEqual(report.executionInventory.phases, {
      connectionSetup: 1, catalogPublication: 0, filePublication: 1, candidatePreparation: 1,
      relationalEvaluation: 1, cleanup: 0, unknown: 1,
    });
    assert.equal(report.executionInventory.entries.length, 5);
    assert.ok(report.executionInventory.entries.every(e => !("sql" in e)), "inventory refers to the shared query catalogue without duplicating SQL");
    assert.ok(report.queries.every(q => !("bytecode" in q) && !("plan" in q)), "plans have one physical representation in the query catalogue");
    const candidate = report.executionInventory.entries.find(e => e.phase === "candidatePreparation");
    assert.deepEqual(candidate.fileDomain, ["a.py", "b.py"]);
    assert.equal(candidate.producer, "anchors");
    assert.equal(candidate.binding, "python");
    assert.equal(candidate.generation, null);
    assert.equal(report.executionInventory.entries.find(e => e.phase === "filePublication").generation, "v1");
    assert.equal(report.executionInventory.entries.reduce((n, e) => n + e.executions, 0), report.executionBudget.observed);
    assert.deepEqual(JSON.parse(readFileSync(join(directory, "execution-inventory.json"))), report.executionInventory);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
