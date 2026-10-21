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
import { executionBudget, requireExecutionBudget, sqlCategory } from "../../benchmark/sql-profile/execution-budget.mjs";
import { controlStages } from "../../benchmark/sql-profile/cte-steps.mjs";

test("CTE set-step times stay inside SQL and preserve unfinished steps", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-cte-profile-"));
  try {
    writeFileSync(join(directory,"cte-program-9-4.json"),JSON.stringify([{pc:0,sql:"WITH payload(k) AS (SELECT 1)",next:"1",source:{scope:"clause",statement:7}},{pc:1,sql:"0",next:"-1"}]));
    writeFileSync(join(directory,"cte-control-9-0.jsonl"),[
      {phase:4,pc:0,tick:0,started:true},{phase:4,pc:0,tick:0,cpuNs:50,wallNs:100},
      {phase:4,pc:1,tick:1,started:true},
    ].map(JSON.stringify).join("\n")+"\n");
    const stages=await controlStages(directory,new Map([[7,{ruleId:"example",clauseId:"clause"}]]));
    assert.deepEqual(stages.map(s=>[s.cpuNs,s.runs,s.incomplete]),[[50,1,0],[0,0,1]]);
    assert.deepEqual(stages[0].columns,["k"]);
    assert.equal(stages[0].kind,"cte-set-step");
    assert.deepEqual(stages[0].source,{scope:"clause",statement:7,ruleId:"example",clauseId:"clause"});
  } finally { rmSync(directory,{recursive:true,force:true}); }
});

test("plan variants ignore connection addresses and retain every raw capture", async () => {
  const directory=mkdtempSync(join(tmpdir(),"rerail-plan-variants-")),raw=join(directory,"raw");mkdirSync(raw);
  try {
    for (const [i,address,index] of [[0,"AA",1],[1,"BB",1],[2,"CC",2]]) {
      const catalogue={version:1,pid:9,events:1,queries:[{id:0,kind:"sql",sql:"SELECT k FROM data",
        plan:[[0,0,0,`SCAN data VIRTUAL TABLE INDEX ${index}:`]],
        bytecode:[[0,"VOpen",0,0,0,`vtab:${address}`,0,null],[1,"VFilter",0,index,0,null,0,null]]}],
        executionCoverage:{source:"ast-session",fromSessionStart:true,started:1,completed:1,running:0}};
      writeFileSync(join(raw,`profile-9-${i}.json`),JSON.stringify(catalogue));
      writeFileSync(join(raw,`profile-9-${i}.jsonl`),JSON.stringify({kind:"sql",query:0,startNs:i*10,cpuNs:1,wallNs:1,selfCpuNs:1,selfWallNs:1,rows:1,changes:0,counters:[1,0,0,0],frame:[],context:{scope:"test",statement:0,inputs:[]},thread:"test"})+"\n");
    }
    const report=await buildReport(raw,directory);
    assert.equal(report.queries.length,1);
    assert.deepEqual(report.queries[0].plans.map(p=>p.captures.length),[2,1]);
    assert.equal(report.queries[0].plans[0].bytecode[0][5],"vtab:AA");
    assert.equal(report.queries[0].bytecode,undefined);
    const saved=JSON.parse(readFileSync(join(directory,"profile.json"),"utf8"));
    assert.equal(saved.queries[0].plans[1].captures[0].catalogue,"raw/profile-9-2.json");
  } finally {rmSync(directory,{recursive:true,force:true});}
});

const capturedJob = (counts = [25, 25, 25, 25]) => ({
  sessions: counts.length,
  queries: [{ kind: "sql", sql: "SELECT 1", runs: counts.reduce((sum, n) => sum + n, 0) }],
  executionCoverage: counts.map((n, i) => ({session:`worker-${i}`,pid:42,source:"ast-session",
    fromSessionStart:true,started:n,completed:n,running:0,observed:n})),
  workload: { results: { parity: true,
    sql: { engine:"sql",digest:"complete-output",files:32,counts:{failures:0},report:{sessions:{count:counts.length}} },
    native: { engine:"native",digest:"complete-output",files:32,counts:{failures:0} },
  } },
});

test("100 is a fixed whole-job budget, not a per-worker or per-batch allowance", () => {
  assert.equal(requireExecutionBudget(capturedJob()).observed, 100);
  assert.throws(() => requireExecutionBudget(capturedJob([25,25,25,26])), /101\/100; budget-exceeded/);
  assert.throws(() => requireExecutionBudget(capturedJob([100,100,100,100])), /400\/100/);
  const report = capturedJob([101]);
  report.limit = 1000; report.workload.config = { executionBudget: 1000, batchSize: 1 };
  assert.equal(executionBudget(report).accepted, false);
});

test("setup, cleanup and helper SQL consume the budget; native spans do not count twice", () => {
  const report = capturedJob([99]);
  report.queries = [
    {kind:"sql",sql:"CREATE TEMP TABLE f(k)",runs:40},
    {kind:"sql",sql:"PRAGMA temp_store=MEMORY",runs:1},
    {kind:"sql",sql:"BEGIN",runs:1},
    {kind:"sql",sql:"COMMIT",runs:1},
    {kind:"sql",sql:"DELETE FROM f",runs:56},
    {kind:"operator",sql:"native publication",runs:50000},
  ];
  assert.deepEqual(requireExecutionBudget(report).categories, {setup:41,transaction:2,matchingAndCleanup:56});
  report.sessions++;
  report.executionCoverage.push({session:"select-when",pid:42,source:"select-when",
    fromSessionStart:true,started:2,completed:2,running:0,observed:2});
  report.queries.push({kind:"sql",sql:"INSERT INTO sw_row VALUES (?)",runs:2});
  assert.throws(() => requireExecutionBudget(report), /101\/100/);
  assert.equal(sqlCategory("-- comment\n/* another */ CREATE TABLE f(k)"), "setup");
});

test("small or empty counts fail without complete trace coverage and complete native parity", () => {
  const cases = [
    report => { delete report.executionCoverage; },
    report => { report.executionCoverage[0].started++; },
    report => { report.executionCoverage[0].observed--; },
    report => { report.executionCoverage[0].running = 1; },
    report => { report.executionCoverage[0].fromSessionStart = false; },
    report => { report.executionCoverage[0].source = "untraced"; },
    report => { report.executionCoverage[0].pid = 999; },
    report => { report.workload.results.parity = false; },
    report => { report.workload.results.sql.digest = "missing-captures"; },
    report => { report.workload.results.sql.files = 0; },
    report => { report.workload.results.sql.counts.failures = 1; },
    report => { report.queries.push({kind:"operator",incomplete:1}); },
    report => { report.queries[0].runs = -1; },
    report => { report.queries = []; },
  ];
  for (const mutate of cases) {
    const report = capturedJob([1,1]); mutate(report);
    assert.throws(() => requireExecutionBudget(report), /invalid-capture/, mutate.toString());
  }
});

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
  const catalogue = { version: 1, pid: 9, events: 2,
    relationalBatches: [{stagesByCpu:[{sql:"SELECT work FROM q1",columns:["k","s0"],cpuNs:50,wallNs:75}]},
      {stagesByCpu:[{sql:"SELECT work FROM q1",columns:["k","s0"],cpuNs:25,wallNs:40,incomplete:true}]}], queries: [
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
    assert.equal(report.relationalStages[0].cpuNs,75);
    assert.equal(report.relationalStages[0].runs,2);
    assert.equal(report.relationalStages[0].incomplete,1);
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
    const stdout = execFileSync(process.execPath, [fileURLToPath(new URL("../../benchmark/sql-profile/summarize.mjs", import.meta.url)), directory], { encoding: "utf8" });
    assert.ok(stdout.includes("Recorded CPU"));
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

test("fused operator priorities separate preparation candidates from child SQL CPU", () => {
  const summary = actionableSummary({ queries: [
    query({ id: 1, kind: "operator", sql: "WITH RECURSIVE schedule(step,token) AS MATERIALIZED (...) SELECT result", selfCpuNs: 300_000_000, cpuNs: 400_000_000 }),
    query({ id: 2, sql: "WITH RECURSIVE schedule(step,token) AS MATERIALIZED (...) SELECT result", selfCpuNs: 100_000_000, cpuNs: 100_000_000 }),
  ] });
  const action = summary.actions.find(action => action.key === "fused-operator-overhead");
  assert.equal(action.cpuMs, 300);
  assert.equal(summary.budget.measuredCpuMs, 400);
  assert.match(action.certainty, /candidate/);
});
