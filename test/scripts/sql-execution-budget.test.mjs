import assert from "node:assert/strict";
import { test } from "node:test";
import { parse } from "yaml";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { executionBudget, requireExecutionBudget, sqlCategory } from "../../benchmark/sql-profile/execution-budget.mjs";
import { buildReport } from "../../benchmark/sql-profile/report.mjs";

const hash = "a".repeat(64);
const parity = () => ({ format: "rerail.data-layer-parity/v1", commit: "b".repeat(40), nativeSha256: hash,
  nativeCalls: 2, hostGuardCalls: 0, replayedCalls: 2, positiveCalls: 1, cleanCalls: 1,
  falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [],
  nativeCoverage: [{ check: "rule", positive: 1, clean: 1 }], rows: [
    { execution: "native", positive: true, expected: 1, actual: 1, falsePositives: [], falseNegatives: [], engineFailures: [], statelessFailures: [], executionPathDifferences: [] },
    { execution: "native", positive: false, expected: 0, actual: 0, falsePositives: [], falseNegatives: [], engineFailures: [], statelessFailures: [], executionPathDifferences: [] },
  ] });
const job = (counts = [25, 25, 25, 25]) => ({
  sessions: counts.length, captureCoverage: { complete: true, problems: [] },
  queries: [{ kind: "sql", sql: "SELECT 1", runs: counts.reduce((sum, n) => sum + n, 0), originRuns: { engine: counts.reduce((sum, n) => sum + n, 0), sqliteInternal: 0, unknown: 0 } }],
  executionCoverage: counts.map((n, i) => ({ session: `worker-${i}`, pid: 42, source: "code-rule-dsl",
    fromSessionStart: true, started: n, completed: n, running: 0, observed: n,
    observedOrigins: { engine: n, sqliteInternal: 0, unknown: 0 },
    origins: { engine: { started: n, completed: n, running: 0 }, sqliteInternal: { started: 0, completed: 0, running: 0 }, unknown: { started: 0, completed: 0, running: 0 } } })),
  workload: { executor: "relational", nativeSha256: hash, results: { parity: true, exitCode: 0, timedOut: false, conformance: parity() } },
});

test("100 covers the complete job, across workers and batches, without a configurable reset", () => {
  assert.equal(requireExecutionBudget(job()).observed, 100);
  assert.throws(() => requireExecutionBudget(job([25, 25, 25, 26])), /101\/100; budget-exceeded/);
  assert.throws(() => requireExecutionBudget(job([100, 100, 100, 100])), /400\/100/);
  const report = job([101]); report.limit = 1000; report.workload.config = { executionBudget: 1000, batchSize: 1 };
  assert.equal(executionBudget(report).limit, 100);
  assert.equal(executionBudget(report).accepted, false);
});

test("setup, DDL, transactions, helper SQL and cleanup count; enclosing operators do not count twice", () => {
  const report = job([100]);
  report.queries = [
    { kind: "sql", sql: "CREATE TEMP TABLE f(k)", runs: 40 },
    { kind: "sql", sql: "PRAGMA temp_store=MEMORY", runs: 1 },
    { kind: "sql", sql: "BEGIN", runs: 1 }, { kind: "sql", sql: "COMMIT", runs: 1 },
    { kind: "sql", sql: "DELETE FROM f", runs: 57 },
    { kind: "operator", sql: "native publication", runs: 50000 },
  ];
  report.queries.filter(q => q.kind === "sql").forEach(q => { q.originRuns = { engine: q.runs, sqliteInternal: 0, unknown: 0 }; });
  assert.deepEqual(requireExecutionBudget(report).categories, { setup: 41, transaction: 2, matchingAndCleanup: 57 });
  assert.equal(sqlCategory("-- comment\n/* another */ CREATE INDEX keys ON f(k)"), "setup");
});

test("low counts fail closed on incomplete coverage, malformed counts, missing sessions or failed parity", () => {
  const cases = [
    r => { delete r.captureCoverage; }, r => { r.captureCoverage.complete = false; },
    r => { delete r.executionCoverage; }, r => { r.executionCoverage[0].started++; },
    r => { r.executionCoverage[0].observed--; }, r => { r.executionCoverage[0].running = 1; },
    r => { r.executionCoverage[0].fromSessionStart = false; }, r => { r.executionCoverage[0].pid = 999; },
    r => { r.sessions++; }, r => { r.queries[0].runs = -1; }, r => { r.queries[0].runs = 1.5; },
    r => { r.queries = []; }, r => { r.workload.results.parity = false; },
    r => { r.workload.results.timedOut = true; }, r => { r.workload.results.exitCode = 1; },
    r => { r.workload.results.conformance.nativeSha256 = "wrong"; },
    r => { r.workload.results.conformance.nativeCalls = 0; },
    r => { r.workload.results.conformance.missingCoverage = [{ check: "missing" }]; },
    r => { r.workload.results.conformance.rows[0].statelessFailures = [{}]; },
    r => { r.workload.results.conformance.nativeCoverage[0].positive = 0; },
    r => { r.queries.push({ kind: "operator", incomplete: 1 }); },
  ];
  for (const mutate of cases) {
    const report = job([1, 1]); mutate(report);
    assert.throws(() => requireExecutionBudget(report), /invalid-capture/, mutate.toString());
  }
});

function rawJob(directory, counts) {
  const raw = join(directory, "sql", "raw"); mkdirSync(raw, { recursive: true });
  const lifecycle = [];
  counts.forEach((count, i) => {
    const session = `profile-42-${i}`;
    const coverage = { source: "code-rule-dsl", fromSessionStart: true, started: count, completed: count, running: 0,
      origins: { engine: { started: count, completed: count, running: 0 }, sqliteInternal: { started: 0, completed: 0, running: 0 }, unknown: { started: 0, completed: 0, running: 0 } } };
    writeFileSync(join(raw, `${session}.json`), JSON.stringify({ version: 1, pid: 42, events: count,
      executionCoverage: coverage, queries: [{ id: 0, kind: "sql", sql: "SELECT 1", plan: [], bytecode: [] }] }));
    writeFileSync(join(raw, `${session}.jsonl`), Array.from({ length: count }, (_, n) => JSON.stringify({
      kind: "sql", origin: "engine", query: 0, startNs: n * counts.length + i, wallNs: 1, cpuNs: 1, selfCpuNs: 1, selfWallNs: 1,
      rows: 1, changes: 0, context: {}, thread: `worker-${i}`,
    })).join("\n") + "\n");
    for (const event of ["opened", "closed"]) lifecycle.push({ version: 1, pid: 42, session, event });
  });
  writeFileSync(join(raw, "coverage-42.jsonl"), lifecycle.map(JSON.stringify).join("\n") + "\n");
  writeFileSync(join(directory, "profile.json"), JSON.stringify({ exitCode: 0, timedOut: false, targetExitCode: 0, targetTimedOut: false,
    source: { nativeSha256: hash }, parity: parity(), native: { phaseMarkers: [] } }));
  return raw;
}

test("the report counts every execution across sessions and locates execution 101", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-budget-"));
  try {
    const report = await buildReport(rawJob(directory, [51, 50]), join(directory, "sql"), job().workload);
    assert.equal(report.executionBudget.observed, 101);
    assert.equal(report.executionBudget.status, "budget-exceeded");
    assert.equal(report.firstOverBudget.startNs, 100);
    assert.equal(report.firstOverBudget.sql, "SELECT 1");
    assert.equal(JSON.parse(readFileSync(join(directory, "sql", "execution-budget.json"))).accepted, false);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("removing an entire session or its close receipt invalidates a small capture", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-budget-missing-"));
  try {
    const raw = rawJob(directory, [1, 1]);
    rmSync(join(raw, "profile-42-1.json")); rmSync(join(raw, "profile-42-1.jsonl"));
    let report = await buildReport(raw, join(directory, "sql"), job().workload);
    assert.equal(report.executionBudget.status, "invalid-capture");
    assert.match(report.executionBudget.problems.join(" "), /lifecycle/i);
    rawJob(directory, [1, 1]);
    const ledger = join(raw, "coverage-42.jsonl");
    const events = readFileSync(ledger, "utf8").trim().split("\n").map(JSON.parse);
    writeFileSync(ledger, events.filter(e => e.session !== "profile-42-1" || e.event !== "closed").map(JSON.stringify).join("\n") + "\n");
    report = await buildReport(raw, join(directory, "sql"), job().workload);
    assert.equal(report.executionBudget.status, "invalid-capture");
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("the ordinary SQL report command exits nonzero at 101 after preserving its evidence", () => {
  for (const count of [100, 101]) {
    const directory = mkdtempSync(join(tmpdir(), "rerail-budget-cli-"));
    try {
      rawJob(directory, [count]);
      const child = spawnSync(process.execPath, ["scripts/profiling/sql-report.mjs", directory], { encoding: "utf8" });
      assert.equal(child.status, count === 100 ? 0 : 1, child.stderr);
      assert.equal(JSON.parse(readFileSync(join(directory, "sql", "execution-budget.json"))).observed, count);
      if (count === 101) assert.match(child.stderr, /101\/100; budget-exceeded/);
      const profile = join(directory, "profile.json");
      writeFileSync(profile, JSON.stringify({ ...JSON.parse(readFileSync(profile)), exitCode: 1 }));
      const again = spawnSync(process.execPath, ["scripts/profiling/sql-report.mjs", directory], { encoding: "utf8" });
      assert.equal(again.status, count === 100 ? 0 : 1, again.stderr);
      assert.equal(JSON.parse(readFileSync(join(directory, "sql", "profile.json"))).workload.results.parity, true);
    } finally { rmSync(directory, { recursive: true, force: true }); }
  }
});

test("the CI workload refuses an existing output directory without rewriting its inputs", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-budget-reuse-"));
  try {
    const input = join(directory, "recording.json"); writeFileSync(input, "preserved");
    const child = spawnSync(process.execPath, ["scripts/profiling/sql-budget.mjs", directory], { encoding: "utf8" });
    assert.equal(child.status, 1);
    assert.match(child.stderr, /EEXIST/);
    assert.equal(readFileSync(input, "utf8"), "preserved");
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("the CI launcher independently rejects a missing or bypassed execution gate", () => {
  const launcher = resolve("scripts/profiling/sql-budget.mjs");
  const fixture = "fixtures/host/data-layer-port/batch-06-recording.json";
  const original = JSON.parse(readFileSync(fixture));
  original.events = original.events.filter(e => [
    "test_rejects_lazy_result_private_step_ref_access",
    "test_collector_uses_generated_return_kinds_without_recomputing_project_map",
  ].some(name => e.test.endsWith(`::${name}`)));
  for (const count of [null, 100, 101]) {
    const sandbox = mkdtempSync(join(tmpdir(), "rerail-budget-bypass-"));
    try {
      const source = join(sandbox, dirname(fixture)); mkdirSync(source, { recursive: true });
      writeFileSync(join(source, "batch-06-recording.json"), JSON.stringify(original));
      const tools = join(sandbox, "scripts", "profiling"); mkdirSync(tools, { recursive: true });
      // Simulate a broken controller that reports success without enforcing the budget.
      const report = count === null ? null : job([count]);
      writeFileSync(join(tools, "run.py"), [
        "import json, pathlib, sys",
        "output = pathlib.Path(sys.argv[sys.argv.index('--output') + 1]) / 'sql'",
        "output.mkdir(parents=True)",
        `report = json.loads(${JSON.stringify(JSON.stringify(report))})`,
        "if report is not None:",
        "    (output / 'profile.json').write_text(json.dumps(report))",
        "    (output / 'execution-budget.json').write_text(json.dumps({'status':'passed','accepted':True,'limit':100,'observed':0,'problems':[]}))",
      ].join("\n"));
      const child = spawnSync(process.execPath, [launcher, join(sandbox, "output")], { cwd: sandbox, encoding: "utf8", timeout: 15000 });
      assert.equal(child.status, count === 100 ? 0 : 1, `count=${count}\n${child.stdout}\n${child.stderr}`);
      if (count === 101) assert.match(child.stderr, /101\/100; budget-exceeded/);
    } finally { rmSync(sandbox, { recursive: true, force: true }); }
  }
});


test("100 engine submissions remain bounded while every SQLite internal execution stays visible", () => {
  const report = job([100]);
  const session = report.executionCoverage[0];
  session.started = session.completed = session.observed = 600;
  session.origins.sqliteInternal = { started: 500, completed: 500, running: 0 };
  session.observedOrigins.sqliteInternal = 500;
  report.queries.push({ kind: "sql", sql: "REPLACE INTO ftsk_docsize VALUES(?,?,?)", runs: 500,
    originRuns: { engine: 0, sqliteInternal: 500, unknown: 0 } });
  const budget = requireExecutionBudget(report);
  assert.equal(budget.observed, 100);
  assert.equal(budget.sqliteInternal, 500);
  assert.equal(budget.totalObserved, 600);
  report.queries[0].runs++; report.queries[0].originRuns.engine++;
  session.started++; session.completed++; session.observed++;
  session.origins.engine.started++; session.origins.engine.completed++; session.observedOrigins.engine++;
  assert.throws(() => requireExecutionBudget(report), /101\/100; budget-exceeded/);
});

test("a small capture without reconciled origin evidence cannot satisfy the engine submission gate", () => {
  for (const mutate of [r => { delete r.queries[0].originRuns; }, r => { delete r.executionCoverage[0].origins; },
    r => { r.queries[0].originRuns.unknown = 1; r.queries[0].originRuns.engine = 0; },
    r => { r.executionCoverage[0].origins.engine.completed = 0; }]) {
    const report = job([1]); mutate(report);
    assert.throws(() => requireExecutionBudget(report), /invalid-capture/);
  }
});


test("CI keeps the 100-submission gate mandatory when its workload or regression tests change", () => {
  const workflow = parse(readFileSync(".github/workflows/ci.yml", "utf8"));
  const steps = Object.values(workflow.jobs).flatMap(job => job.steps ?? []);
  const gate = steps.find(step => step.run?.includes("npm run test:sql-execution-budget"));
  assert.ok(gate, "CI must execute the real complete-job budget workload");
  assert.equal(gate["continue-on-error"] ?? false, false);
  assert.equal(gate.if, "matrix.task == 'test'");
  assert.ok(gate.env.RERAIL_ENGINE_PATH);
  assert.ok(steps.some(step => step.run?.includes("cargo build") && step.run.includes("--features code-rule-profile")));
  const scripts = JSON.parse(readFileSync("package.json")).scripts;
  assert.equal(scripts["test:sql-execution-budget"], "node scripts/profiling/sql-budget.mjs");
  assert.ok(scripts["test:profiling"].includes("test/scripts/sql-execution-budget.test.mjs"));
  const filter = parse(workflow.jobs.changes.steps.find(step => step.with?.filters).with.filters).rust;
  for (const file of ["test/scripts/sql-execution-budget.test.mjs", "test/engine/native/candidate-admission.test.ts", "test/engine/native/candidate-publication.test.ts"]) {
    assert.ok(filter.includes(file), `${file} must trigger the profiling addon and mandatory native gates`);
  }
});
