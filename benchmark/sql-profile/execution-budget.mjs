// Restored from sql-capture-demand-spikes c109a915. One fixed allowance per complete job.
export const JOB_SQL_EXECUTION_LIMIT = 100;

export function sqlCategory(sql) {
  const text = sql.replace(/^(?:\s|--[^\n]*(?:\n|$)|\/\*[\s\S]*?\*\/)+/, "");
  if (/^(?:CREATE|DROP|ALTER|PRAGMA|ANALYZE|REINDEX)\b/i.test(text)) return "setup";
  if (/^(?:BEGIN|COMMIT|END|ROLLBACK|SAVEPOINT|RELEASE)\b/i.test(text)) return "transaction";
  return "matchingAndCleanup";
}

const integer = n => Number.isSafeInteger(n) && n >= 0;
const empty = n => Array.isArray(n) && n.length === 0;
const digest = n => typeof n === "string" && /^[a-f0-9]{64}$/.test(n);
function completeParity(workload) {
  const result = workload?.results;
  if (result?.parity !== true) return false;
  // Preserve the original sibling-branch native-versus-SQL comparison contract.
  if (result.sql?.engine === "sql" && result.native?.engine === "native") {
    return result.sql.counts?.failures === 0 && result.native.counts?.failures === 0
      && result.sql.digest === result.native.digest && typeof result.sql.digest === "string"
      && result.sql.files > 0 && result.sql.files === result.native.files;
  }
  const p = result.conformance;
  if (workload.executor !== "relational" || result.exitCode !== 0 || result.timedOut !== false
      || !digest(p?.nativeSha256) || p.nativeSha256 !== workload.nativeSha256) return false;
  if (p.format === "rerail.data-layer-parity/v1") {
    return integer(p.nativeCalls) && p.nativeCalls > 0 && p.nativeCalls === p.replayedCalls && p.hostGuardCalls === 0
      && p.positiveCalls > 0 && p.cleanCalls > 0 && empty(p.missingCoverage)
      && [p.falsePositives, p.falseNegatives, p.engineFailures, p.executionPathDifferences].every(n => n === 0)
      && Array.isArray(p.nativeCoverage) && p.nativeCoverage.length > 0
      && p.nativeCoverage.every(c => c.positive > 0 && c.clean > 0)
      && Array.isArray(p.rows) && p.rows.length === p.nativeCalls && p.rows.every(row => row.execution === "native"
        && integer(row.expected) && row.actual === row.expected
        && [row.falsePositives, row.falseNegatives, row.engineFailures, row.statelessFailures, row.executionPathDifferences].every(empty));
  }
  // Whole-repository suite reports compare complete finding multisets and incremental results.
  return integer(p.files) && p.files > 0 && integer(p.expected) && p.expected === p.actual
    && digest(p.workloadSha256) && Object.keys(p.policyDigests ?? {}).length > 0
    && [p.falsePositives, p.falseNegatives, p.failures, p.incrementalDifferences].every(empty);
}

/** Acceptance after complete execution. Never stop matching at execution 101. */
export function executionBudget(report) {
  const queries = (report.queries ?? []).filter(q => q.kind === "sql");
  const totalObserved = queries.reduce((sum, q) => sum + (integer(q.runs) ? q.runs : 0), 0);
  const origins = ["engine", "sqliteInternal", "unknown"];
  const totals = Object.fromEntries(origins.map(origin => [origin, queries.reduce((sum, q) => sum + (integer(q.originRuns?.[origin]) ? q.originRuns[origin] : 0), 0)]));
  const observed = totals.engine;
  const categories = { setup: 0, transaction: 0, matchingAndCleanup: 0 };
  for (const q of queries) if (integer(q.runs)) categories[sqlCategory(q.sql)] += integer(q.originRuns?.engine) ? q.originRuns.engine : 0;
  const sessions = report.executionCoverage ?? [], problems = [];
  if (!completeParity(report.workload)) problems.push("Complete output parity without execution failures is required.");
  if (report.captureCoverage?.complete !== true) problems.push("Complete connection lifecycle coverage is required.");
  problems.push(...(report.captureCoverage?.problems ?? []));
  if (!sessions.length || sessions.length !== report.sessions
      || new Set(sessions.map(s => s.session)).size !== sessions.length
      || new Set(sessions.map(s => s.pid)).size !== 1) problems.push("Every connection must belong to one fresh job capture.");
  if (queries.some(q => !origins.every(origin => integer(q.originRuns?.[origin]))
      || origins.reduce((n, origin) => n + (q.originRuns?.[origin] ?? 0), 0) !== q.runs)) {
    problems.push("Every SQL execution requires a reconciled engine or SQLite-internal origin.");
  }
  if (totals.unknown > 0) problems.push("Unknown SQL execution origins cannot satisfy acceptance.");
  for (const s of sessions) {
    if (!["code-rule-dsl", "ast-session", "select-when", "crossfile"].includes(s.source)
        || s.fromSessionStart !== true || ![s.started, s.completed, s.running, s.observed].every(integer)
        || s.started !== s.completed || s.running !== 0 || s.completed !== s.observed) {
      problems.push(`Incomplete SQL execution coverage: ${s.session}.`);
    }
    if (!origins.every(origin => {
      const count = s.origins?.[origin];
      return count && [count.started, count.completed, count.running, s.observedOrigins?.[origin]].every(integer)
        && count.started === count.completed && count.running === 0 && count.completed === s.observedOrigins[origin];
    }) || origins.reduce((n, origin) => n + (s.origins?.[origin]?.completed ?? 0), 0) !== s.completed) {
      problems.push(`Incomplete SQL origin coverage: ${s.session}.`);
    }
  }
  for (const origin of origins) {
    if (sessions.reduce((n, s) => n + (s.observedOrigins?.[origin] ?? 0), 0) !== totals[origin]) {
      problems.push(`SQL origin totals do not reconcile: ${origin}.`);
    }
  }
  if (!queries.length || queries.some(q => !integer(q.runs)) || !integer(totalObserved)
      || sessions.reduce((sum, s) => sum + (s.observed ?? 0), 0) !== totalObserved) problems.push("Execution counts must agree with complete session traces.");
  if ((report.queries ?? []).some(q => q.incomplete > 0)) problems.push("The workload contains incomplete operations.");
  const exceeded = observed > JOB_SQL_EXECUTION_LIMIT;
  return { limit: JOB_SQL_EXECUTION_LIMIT, observed, totalObserved, sqliteInternal: totals.sqliteInternal, unknownOrigins: totals.unknown, excess: Math.max(0, observed - JOB_SQL_EXECUTION_LIMIT),
    accepted: !exceeded && problems.length === 0,
    status: exceeded ? "budget-exceeded" : problems.length ? "invalid-capture" : "passed",
    scope: "One complete cold job across workers and connections. All engine-submitted publication, setup, DDL, helper statements, transactions, matching and cleanup count. SQLite internal executions are reported separately. Diagnostic EXPLAIN is excluded.",
    enforcement: "Acceptance after the complete run; execution 101 does not truncate findings. No per-file, per-rule, per-batch or per-connection reset.",
    categories, sessions, problems, firstOverBudget: exceeded ? report.firstOverBudget ?? null : null };
}

export function requireExecutionBudget(report) {
  const budget = executionBudget(report);
  if (!budget.accepted) throw new Error(`SQL job execution budget FAILED: ${budget.observed}/${budget.limit}; ${budget.status}. ${budget.problems.join(" ")}`.trim());
  return budget;
}
