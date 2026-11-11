// Full-state parity for the SQL extension. The cold oracle runs outside the capture.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

export const sha256 = bytes => createHash("sha256").update(bytes).digest("hex");
const empty = value => Array.isArray(value) && value.length === 0;
const digest = value => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
const canonical = value => JSON.stringify(normalize(value));
function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize);
  if (value && typeof value === "object") return Object.fromEntries(Object.keys(value).sort().map(key => [key, normalize(value[key])]));
  return value;
}
function cell(value) {
  return value === null || typeof value === "string" && /^-?\d+$/.test(value)
    || value && typeof value === "object" && !Array.isArray(value)
      && (Object.keys(value).length === 1 && (typeof value.symbol === "string" || typeof value.document === "string")
        || Object.keys(value).length === 1 && Array.isArray(value.node) && value.node.length === 2
          && typeof value.node[0] === "string" && typeof value.node[1] === "string" && /^\d+$/.test(value.node[1]));
}
function documents(value) {
  if (!Array.isArray(value) || value.some(f => !f || typeof f.path !== "string" || typeof f.language !== "string" || typeof f.source !== "string")) throw new Error("Missing complete document state");
  if (new Set(value.map(f => f.path)).size !== value.length) throw new Error("Duplicate document path");
  return value.map(f => ({path:f.path,language:f.language,source:f.source})).sort((a,b)=>a.path.localeCompare(b.path));
}
function publishFiles(before, step) {
  if (step.snapshot) return documents(step.snapshot.files);
  if (!step.file_changes) return before;
  const files = new Map(before.map(f=>[f.path,f]));
  for (const file of step.file_changes.upsert ?? []) files.set(file.path,file);
  for (const path of step.file_changes.delete ?? []) files.delete(path);
  return documents([...files.values()]);
}
function state(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Missing complete state");
  return Object.fromEntries(Object.entries(value).map(([name, table]) => {
    if (!Array.isArray(table.columns) || !table.columns.every(c => typeof c === "string") || !Array.isArray(table.rows)
        || !table.rows.every(row => Array.isArray(row) && row.length === table.columns.length
          && row.every(cell))) throw new Error(`Invalid integer relation ${name}`);
    // Preserve multiplicity. Sorting permits differences in SQL scan order.
    return [name, { columns: table.columns, rows: table.rows.map(canonical).sort() }];
  }));
}
const hashState = value => sha256(canonical(state(value)));
function transitions(before, after) {
  const a = new Set(before.rows), b = new Set(after.rows);
  if (a.size !== before.rows.length || b.size !== after.rows.length) throw new Error("Duplicate rows in a set relation");
  return { inserted: [...b].filter(row => !a.has(row)).length, deleted: [...a].filter(row => !b.has(row)).length };
}
export function compareSessions(requestBytes, actual, oracle, execution) {
  const request = JSON.parse(requestBytes), requestSha256 = sha256(requestBytes), problems = [];
  const steps = request.steps ?? (request.changes ?? []).map(changes => ({ changes }));
  const checks = [], failures = [];
  if (typeof request.program !== "string" || Object.keys(request).some(name => !["program", "dialect", "relations", "snapshot", "steps", "changes", "max_waves", "queries"].includes(name))) problems.push("Unsupported verification request fields");
  for (const step of steps) if (!step || Object.keys(step).some(name => !["changes", "snapshot", "file_changes", "limits", "max_waves", "expected_error"].includes(name))) problems.push("Unsupported verification step fields");
  for (const step of steps) if (["changes","snapshot","file_changes"].filter(k=>Object.hasOwn(step,k)).length !== 1) problems.push("Ambiguous publication action");
  for (const q of request.queries ?? []) if (typeof q !== "string" && (!q || Object.keys(q).some(name => !["sql", "parameters"].includes(name)))) problems.push("Unsupported verification query fields");
  if (request.steps && request.changes) problems.push("Ambiguous change sequence");
  if (!Array.isArray(steps)) throw new Error("Missing change sequence");
  for (const [mode, observed] of [["retained", actual], ["oracle", oracle]]) {
    if (observed?.format !== "rerail.sql-session-observation/v1" || observed.mode !== mode
        || observed.requestSha256 !== requestSha256 || observed.runnerSha256 !== execution.runnerSha256) problems.push(`Unpinned ${mode} observation`);
    if (!Array.isArray(observed?.checks) || observed.checks.length !== steps.length + 1) problems.push(`Incomplete ${mode} checks`);
    if (!Array.isArray(observed?.outputs) || !Array.isArray(observed.relations) || !observed.relations.length) problems.push(`Missing ${mode} relation inventory`);
  }
  if (execution.retained?.status !== 0 || execution.oracle?.status !== 0 || execution.retained?.signal || execution.oracle?.signal || execution.retained?.error || execution.oracle?.error) problems.push("Verification process failed");
  if (canonical(actual?.outputs) !== canonical(oracle?.outputs) || canonical(actual?.relations) !== canonical(oracle?.relations)) problems.push("Different compiled relation inventories");
  if (canonical(actual?.inputs) !== canonical(oracle?.inputs)) problems.push("Different bound input inventories");
  let expectedDocuments = request.snapshot ? documents(request.snapshot.files) : null;
  let failureChecks = 0, recoveredFailures = 0, pendingFailure = false;
  for (let id = 0; id <= steps.length; id++) {
    const a = actual?.checks?.[id], o = oracle?.checks?.[id], expectedOutcome = id === 0 ? "ok" : steps[id - 1].expected_error ?? "ok";
    const errors = [];
    if (!a || !o || a.id !== id || o.id !== id) { failures.push({ id, errors: ["Missing checkpoint"] }); continue; }
    if (a.outcome !== expectedOutcome || o.outcome !== expectedOutcome) errors.push("Unexpected execution outcome");
    try {
      if (id > 0 && expectedOutcome === "ok" && (expectedDocuments !== null || steps[id-1].snapshot)) {
        expectedDocuments = publishFiles(expectedDocuments,steps[id-1]);
      }
      if (expectedDocuments !== null) {
        if (canonical(documents(a.documents)) !== canonical(expectedDocuments)
            || canonical(documents(o.documents)) !== canonical(expectedDocuments)) errors.push("Complete document contents differ");
        for (const [name,columns] of Object.entries(actual.inputs ?? {})) if (canonical(a.state[name]?.columns) !== canonical(columns)) errors.push(`Missing native input ${name}`);
      }
      const actualState = state(a.state), oracleState = state(o.state), actualSha256 = hashState(a.state), oracleSha256 = hashState(o.state);
      if (actualSha256 !== oracleSha256) errors.push("Complete relation contents differ");
      if (canonical(Object.keys(actualState).sort()) !== canonical(Object.keys(actual.checks[0].state).sort())) errors.push("Relation inventory changed");
      for (const [name, arity] of actual.relations ?? []) {
        if (!Number.isSafeInteger(arity) || arity < 1 || actualState[name]?.columns.length !== arity) errors.push(`Missing derived relation ${name}`);
      }
      for (const input of request.relations ?? []) if (canonical(actualState[input.name]?.columns) !== canonical(input.columns)) errors.push(`Missing input relation ${input.name}`);
      if (expectedOutcome !== "ok") {
        failureChecks++; pendingFailure = true;
        if (id === 0 || hashState(actual.checks[id - 1].state) !== actualSha256 || hashState(oracle.checks[id - 1].state) !== oracleSha256) errors.push("Failed operation changed state");
      } else {
        if (pendingFailure) { recoveredFailures++; pendingFailure = false; }
        if (id > 0) {
          const reports = a.report?.programs;
          if (!Array.isArray(reports) || reports.length > 1) errors.push("Missing retained update report");
          const outputReport = reports?.[0]?.outputs ?? {};
          for (const name of actual.outputs ?? []) {
            if (!actualState[name] || !oracleState[name]) { errors.push(`Missing output ${name}`); continue; }
            const delta = transitions(state(actual.checks[id - 1].state)[name], actualState[name]);
            const reported = outputReport[name] ?? { inserted: 0, deleted: 0 };
            if (canonical(delta) !== canonical(reported)) errors.push(`Wrong output delta for ${name}`);
          }
          if (Object.keys(outputReport).some(name => !actual.outputs.includes(name))) errors.push("Unexpected output in update report");
        }
      }
      checks.push({ id, outcome: expectedOutcome, relations: Object.keys(actualState).length, actualSha256, oracleSha256, errors });
    } catch (error) { errors.push(error.message); }
    if (errors.length) failures.push({ id, errors });
  }
  const queryCount = (request.queries ?? []).length;
  for (const [name, observed] of [["retained", actual], ["oracle", oracle]]) {
    if (!Array.isArray(observed?.queries) || observed.queries.length !== queryCount
        || observed.queries.some(q => q.truncated !== false || !Array.isArray(q.rows))) problems.push(`Incomplete ${name} query results`);
  }
  if (canonical(actual?.queries) !== canonical(oracle?.queries)) problems.push("Query results differ");
  if (!failureChecks || !recoveredFailures || pendingFailure) problems.push("Failure rollback and subsequent recovery require coverage");
  return { format: "rerail.sql-session-parity/v1", requestSha256, runnerSha256: execution.runnerSha256,
    publications: [...new Set([...(request.snapshot ? ["bindSnapshot"] : []),...steps.map(s=>s.snapshot?"applySnapshot":s.file_changes?"applyFileChanges":"applyChanges")])],
    expectedChecks: steps.length + 1, checks, failureChecks, recoveredFailures, queries: queryCount, problems, failures };
}

function validSessionEvidence(workload) {
  const p = workload?.results?.conformance;
  return workload?.executor === "sql-extension" && workload.results?.exitCode === 0
    && p?.format === "rerail.sql-session-parity/v1" && digest(p.requestSha256) && p.requestSha256 === workload.requestSha256
    && digest(p.runnerSha256) && p.runnerSha256 === workload.nativeSha256
    && digest(p.captureSha256) && p.captureSha256 === workload.captureSha256
    && Number.isSafeInteger(p.expectedChecks) && p.expectedChecks > 2 && Array.isArray(p.checks) && p.checks.length === p.expectedChecks
    && Number.isSafeInteger(p.failureChecks) && p.failureChecks > 0
    && Number.isSafeInteger(p.recoveredFailures) && p.recoveredFailures > 0 && p.recoveredFailures <= p.failureChecks
    && Number.isSafeInteger(p.queries) && p.queries >= 0 && empty(p.problems) && empty(p.failures)
    && p.checks.filter(c => c.outcome !== "ok").length === p.failureChecks
    && p.checks.every((c, id) => c.id === id && Number.isSafeInteger(c.relations) && c.relations > 0 && digest(c.actualSha256) && c.actualSha256 === c.oracleSha256 && empty(c.errors));
}

export function completeSessionParity(workload) {
  return workload?.results?.parity === true && validSessionEvidence(workload);
}

export function captureDigests(raw) {
  return Object.fromEntries(readdirSync(raw).filter(name => /^(?:profile-.*\.jsonl?|coverage-.*\.jsonl)$/.test(name)).sort().map(name => [name, sha256(readFileSync(join(raw, name)))]));
}

export function sessionWorkload(raw, bundle) {
  const read = name => readFileSync(join(bundle, name));
  const artifacts = JSON.parse(read("artifacts.json"));
  for (const name of ["request.json", "retained.json", "oracle.json", "execution.json"]) {
    if (artifacts.files?.[name] !== sha256(read(name))) throw new Error(`Changed parity artifact: ${name}`);
  }
  const capture = captureDigests(raw);
  if (!Object.keys(capture).length || canonical(capture) !== canonical(artifacts.capture)) throw new Error("SQL capture does not match the parity run");
  const actual = JSON.parse(read("retained.json")), oracle = JSON.parse(read("oracle.json")), execution = JSON.parse(read("execution.json"));
  const p = compareSessions(read("request.json"), actual, oracle, execution);
  p.captureSha256 = sha256(canonical(capture));
  // Each full-state observation must have exactly one successful SQL receipt.
  const receipts = new Map();
  for (const name of Object.keys(capture).filter(n => /^profile-.*\.json$/.test(n))) {
    const catalog = JSON.parse(readFileSync(join(raw, name)));
    for (const line of readFileSync(join(raw, name.replace(/\.json$/, ".jsonl")), "utf8").split("\n").filter(Boolean)) {
      const event = JSON.parse(line), query = catalog.queries.find(q => q.id === event.query);
      const match = query?.sql.match(/\/\* rerail-verification:([a-f0-9]{64}):(\d+) \*\//);
      if (event.kind !== "sql" || !match) continue;
      if (match[1] !== p.requestSha256 || event.origin !== "engine" || (event.outcome !== undefined && event.outcome !== "ok") || event.completed === false) p.problems.push("Invalid verification receipt");
      const id = Number(match[2]);
      const expectedRows = Object.values(actual.checks?.[id]?.state ?? {}).reduce((n,t) => n + (t.rows?.length ?? 0),0);
      if (event.rows !== expectedRows) p.problems.push("Verification row receipt does not match complete state");
      receipts.set(id, (receipts.get(id) ?? 0) + 1);
    }
  }
  if (receipts.size !== p.expectedChecks || [...Array(p.expectedChecks).keys()].some(id => receipts.get(id) !== 1)) p.problems.push("Incomplete verification receipts");
  const workload = { executor: "sql-extension", nativeSha256: p.runnerSha256, requestSha256: p.requestSha256, captureSha256: p.captureSha256,
    source: execution.source, results: { exitCode: 0, parity: false, conformance: p, oracle: "Independent cold SQL kernel; complete facts, derived relations, widening counters, edit deltas, failure rollback, recovery, and queries." } };
  workload.results.parity = validSessionEvidence(workload);
  return workload;
}
