import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { compareSessions, completeSessionParity, sessionWorkload, captureDigests, sha256 } from "../../../benchmark/sql-profile/session-parity.mjs";
import { executionBudget } from "../../../benchmark/sql-profile/execution-budget.mjs";

function fixture() {
  const request = Buffer.from(JSON.stringify({ program: ".decl seed(n:number)\n.input seed\n.decl reach(n:number)\nreach(n) :- seed(n).\n.output reach\n", relations: [{ name: "seed", columns: ["n"], rows: [[1]] }],
    steps: [{ changes: [{ name: "seed", delete: [[1]] }], max_waves: 1, expected_error: "wave-limit" },
      { changes: [{ name: "seed", delete: [[1]] }] }] }));
  const full = { seed: { columns: ["n"], rows: [["1"]] }, reach: { columns: ["c0"], rows: [["1"]] } };
  const empty = { seed: { columns: ["n"], rows: [] }, reach: { columns: ["c0"], rows: [] } };
  const base = { format: "rerail.sql-session-observation/v1", requestSha256: sha256(request), runnerSha256: "a".repeat(64),
    relations: [["reach", 1]], outputs: ["reach"], queries: [], checks: [
      { id: 0, outcome: "ok", state: full }, { id: 1, outcome: "wave-limit", state: full },
      { id: 2, outcome: "ok", state: empty, report: { programs: [{ outputs: { reach: { inserted: 0, deleted: 1 } } }] } },
    ] };
  return { request, actual: JSON.parse(JSON.stringify({ ...base, mode: "retained" })), oracle: JSON.parse(JSON.stringify({ ...base, mode: "oracle" })),
    execution: { runnerSha256: base.runnerSha256, retained: { status: 0 }, oracle: { status: 0 } } };
}
function compare(f) { return compareSessions(f.request, f.actual, f.oracle, f.execution); }
function workload(p) {
  p.captureSha256 = "c".repeat(64);
  return { executor: "sql-extension", nativeSha256: p.runnerSha256, requestSha256: p.requestSha256, captureSha256: p.captureSha256,
    results: { exitCode: 0, parity: true, conformance: p } };
}
test("SQL parity compares complete state, exact integers, rollback, recovery, and deltas", () => {
  const f = fixture();
  for (const observation of [f.actual, f.oracle]) {
    for (const check of observation.checks.slice(0, 2)) check.state.reach.rows.push(["9007199254740993"]);
    observation.checks[2].report.programs[0].outputs.reach.deleted = 2;
  }
  const p = compare(f);
  assert.deepEqual(p.failures, []); assert.deepEqual(p.problems, []);
  assert.equal(completeSessionParity(workload(p)), true);
});
test("SQL parity rejects dropped rows, relation inventories, and truncated query results", () => {
  for (const corrupt of [
    f => f.actual.checks[0].state.reach.rows.push(["2"]),
    f => delete f.actual.checks[2].state.reach,
    f => f.actual.checks.pop(),
    f => f.actual.relations.push(["omitted", 1]),
    f => { f.actual.queries = [{ rows: [], truncated: true }]; },
  ]) {
    const f = fixture(); corrupt(f);
    assert.equal(completeSessionParity(workload(compare(f))), false);
  }
});
test("SQL parity rejects wrong reported deltas and mutation after an expected failure", () => {
  for (const corrupt of [
    f => f.actual.checks[2].report.programs[0].outputs.reach.deleted = 0,
    f => f.actual.checks[1].state.seed.rows.push(["2"]),
    f => f.actual.checks[1].outcome = "ok",
    f => f.execution.oracle.status = 1,
    f => f.oracle.runnerSha256 = "b".repeat(64),
  ]) {
    const f = fixture(); corrupt(f);
    assert.equal(completeSessionParity(workload(compare(f))), false);
  }
});
test("SQL parity requires recovery after a failed operation", () => {
  const f = fixture();
  const request = JSON.parse(f.request); request.steps.pop(); f.request = Buffer.from(JSON.stringify(request));
  for (const observed of [f.actual, f.oracle]) { observed.checks.pop(); observed.requestSha256 = sha256(f.request); }
  assert.ok(compare(f).problems.some(p => p.includes("recovery")));
});
test("SQL parity checks the retained session cap without limiting the oracle", () => {
  const f = fixture();
  const request = { ...JSON.parse(f.request), max_submissions: 100 };
  f.request = Buffer.from(JSON.stringify(request));
  for (const observation of [f.actual, f.oracle]) observation.requestSha256 = sha256(f.request);
  f.actual.sql_budget = { limit: 100, executed: 98, reserved: 0 };
  f.oracle.sql_budget = { limit: null, executed: 120, reserved: 0 };
  assert.deepEqual(compare(f).problems, []);
  for (const invalid of [{ executed: 101 }, { limit: null }, { reserved: 2 }]) {
    const original = f.actual.sql_budget;
    f.actual.sql_budget = { ...original, ...invalid };
    assert.ok(compare(f).problems.some(p => p.includes("SQL admission")));
    f.actual.sql_budget = original;
  }
});
test("valid SQL parity leaves the fixed complete-job allowance at 100", () => {
  const p = workload(compare(fixture()));
  function report(n) { return { workload: p, sessions: 1, captureCoverage: { complete: true, problems: [] },
    queries: [{ kind: "sql", sql: "SELECT * FROM reach", runs: n, originRuns: { engine: n, sqliteInternal: 0, unknown: 0 } }],
    executionCoverage: [{ session: "profile-1-0.json", pid: 1, source: "code-rule-dsl", observedOrigins: { engine: n, sqliteInternal: 0, unknown: 0 }, fromSessionStart: true, started: n, completed: n, running: 0, observed: n,
      origins: { engine: { started: n, completed: n, running: 0 }, sqliteInternal: { started: 0, completed: 0, running: 0 }, unknown: { started: 0, completed: 0, running: 0 } } }] }; }
  assert.equal(executionBudget(report(100)).accepted, true);
  assert.equal(executionBudget(report(101)).status, "budget-exceeded");
  p.results.parity = false;
  assert.equal(executionBudget(report(99)).accepted, false);
});

function bundle(f, directory) {
  const raw = join(directory, "raw"), parity = join(directory, "parity"); mkdirSync(raw); mkdirSync(parity);
  const queries = f.actual.checks.map(c => ({ id: c.id, kind: "sql", sql: `SELECT 1 /* rerail-verification:${sha256(f.request)}:${c.id} */` }));
  writeFileSync(join(raw, "profile-1-0.json"), JSON.stringify({ queries }));
  writeFileSync(join(raw, "profile-1-0.jsonl"), queries.map(q => JSON.stringify({ query: q.id, kind: "sql", origin: "engine", rows: Object.values(f.actual.checks[q.id].state).reduce((n,t) => n+t.rows.length,0) })).join("\n"));
  writeFileSync(join(parity, "request.json"), f.request);
  for (const [name, value] of [["retained", f.actual], ["oracle", f.oracle], ["execution", f.execution]]) writeFileSync(join(parity, `${name}.json`), JSON.stringify(value));
  const seal = () => writeFileSync(join(parity, "artifacts.json"), JSON.stringify({
    files: Object.fromEntries(["request.json", "retained.json", "oracle.json", "execution.json"].map(name => [name, sha256(readFileSync(join(parity, name)))])), capture: captureDigests(raw),
  }));
  seal(); return { raw, parity, seal };
}
test("SQL acceptance binds observations to the captured SQL receipts", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-sql-parity-test-"));
  try {
    const b = bundle(fixture(), directory);
    assert.equal(sessionWorkload(b.raw, b.parity).results.parity, true);
    const trace = join(b.raw, "profile-1-0.jsonl");
    writeFileSync(trace, readFileSync(trace, "utf8").split("\n").slice(1).join("\n"));
    assert.throws(() => sessionWorkload(b.raw, b.parity), /capture does not match/);
    b.seal();
    const changed = sessionWorkload(b.raw, b.parity);
    assert.equal(changed.results.parity, false);
    assert.ok(changed.results.conformance.problems.includes("Incomplete verification receipts"));
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
test("SQL acceptance rejects changed observation artifacts", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-sql-parity-test-"));
  try {
    const b = bundle(fixture(), directory);
    writeFileSync(join(b.parity, "oracle.json"), "{}");
    assert.throws(() => sessionWorkload(b.raw, b.parity), /Changed parity artifact/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("SQL parity refuses unsupported work instead of accepting a partial replay", () => {
  for (const mutate of [r => r.repeat = true, r => r.file_changes = [], r => r.steps[0].sql = "DELETE FROM seed", r => r.queries = [{sql:"SELECT 1",max_rows:1}]]) {
    const f=fixture();const request=JSON.parse(f.request);mutate(request);f.request=Buffer.from(JSON.stringify(request));
    for (const observation of [f.actual,f.oracle]) observation.requestSha256=sha256(f.request);
    assert.equal(completeSessionParity(workload(compare(f))),false);
  }
});

test("SQL acceptance rejects malformed conformance metadata", () => {
  for (const mutate of [p => p.problems = "", p => p.failures = "", p => p.checks[0].errors = "", p => p.failureChecks = "1", p => p.checks[0].actualSha256 = "unverified"]) {
    const w=workload(compare(fixture()));mutate(w.results.conformance);
    assert.equal(completeSessionParity(w),false);
  }
});

test("snapshot and file publication compare semantic cells and complete documents", () => {
  const f=fixture(), request=JSON.parse(f.request);
  const a={path:"a.py",language:"python",source:"x = 1\n"}, b={path:"b.py",language:"python",source:"y = 2\n"};
  const c={path:"c.py",language:"python",source:"z = 3\n"};
  request.snapshot={files:[a,b]};
  request.steps[0]={snapshot:{files:[c]},limits:{max_steps:0},expected_error:"execution-limit"};
  request.steps[1]={file_changes:{delete:["b.py"],upsert:[c]}};
  f.request=Buffer.from(JSON.stringify(request));
  for (const observed of [f.actual,f.oracle]) {
    observed.requestSha256=sha256(f.request);
    observed.inputs={native:["n","s","k"]};
    for (const [id,check] of observed.checks.entries()) {
      check.documents=id<2?[a,b]:[a,c];
      check.state.native={columns:["n","s","k"],rows:[[{node:["a.py","0"]},{symbol:"x"},{nodeOrder:["a.py","1"]}]]};
    }
    observed.checks[1].outcome="execution-limit";
  }
  const p=compare(f);
  assert.deepEqual(p.problems,[]);assert.deepEqual(p.failures,[]);
  assert.deepEqual(p.publications,["bindSnapshot","applySnapshot","applyFileChanges"]);
  assert.equal(completeSessionParity(workload(p)),true);
  f.actual.checks[2].documents=[a,b];
  assert.ok(compare(f).failures.some(f=>f.errors.includes("Complete document contents differ")));
});

test("document verification rejects missing native inputs, malformed cells and ambiguous actions", () => {
  for (const corrupt of [
    r=>r.steps[0].snapshot={files:[]},
    r=>r.steps[0].file_changes={delete:[]},
  ]) {
    const f=fixture(),request=JSON.parse(f.request);corrupt(request);f.request=Buffer.from(JSON.stringify(request));
    for(const o of [f.actual,f.oracle])o.requestSha256=sha256(f.request);
    assert.equal(completeSessionParity(workload(compare(f))),false);
  }
  const f=fixture();f.actual.checks[0].state.seed.rows=[[{symbol:"x",extra:1}]];
  assert.ok(compare(f).failures.length>0);
});


test("document comparison starts when a later step introduces the first snapshot", () => {
  const f=fixture(),request=JSON.parse(f.request);
  const a={path:"a.py",language:"python",source:"x = 1\n"};
  request.steps[1]={snapshot:{files:[a]}};
  f.request=Buffer.from(JSON.stringify(request));
  for (const o of [f.actual,f.oracle]) {
    o.requestSha256=sha256(f.request);
    o.checks[2].documents=[a];
  }
  assert.deepEqual(compare(f).failures,[]);
  f.actual.checks[2].documents=[];
  assert.ok(compare(f).failures.some(f=>f.errors.includes("Complete document contents differ")));
});
