// Acceptance budget for one complete workload, across all workers and cold session setup.
// This is intentionally fixed. Changing batch size or execute_batch calls cannot reset it.
export const JOB_SQL_EXECUTION_LIMIT = 100;

export function sqlCategory(sql) {
  const text = sql.replace(/^(?:\s|--[^\n]*(?:\n|$)|\/\*[\s\S]*?\*\/)+/, "");
  if (/^(?:CREATE|DROP|ALTER|PRAGMA|ANALYZE|REINDEX)\b/i.test(text)) return "setup";
  if (/^(?:BEGIN|COMMIT|END|ROLLBACK|SAVEPOINT|RELEASE)\b/i.test(text)) return "transaction";
  return "matchingAndCleanup";
}

/** Fail closed: low counts cannot pass without a complete trace and native output parity. */
export function executionBudget(report) {
  const queries = (report.queries ?? []).filter(query => query.kind === "sql");
  const observed = queries.reduce((sum, query) => sum + query.runs, 0);
  const categories = { setup: 0, transaction: 0, matchingAndCleanup: 0 };
  for (const query of queries) categories[sqlCategory(query.sql)] += query.runs;
  const sessions = report.executionCoverage ?? [];
  const results = report.workload?.results;
  const problems = [];
  const integer = n => Number.isSafeInteger(n) && n >= 0;
  if (results?.parity !== true || results?.sql?.engine !== "sql"
      || results?.native?.engine !== "native" || results.sql?.counts?.failures !== 0
      || results.native?.counts?.failures !== 0 || results.sql?.digest !== results.native?.digest
      || results.sql?.files !== results.native?.files || !(results.sql?.files > 0)) {
    problems.push("Complete native output parity is required.");
  }
  if (!sessions.length || sessions.length !== report.sessions
      || sessions.filter(session => session.source === "ast-session").length !== results?.sql?.report?.sessions?.count
      || new Set(sessions.map(session => session.pid)).size !== 1) {
    problems.push("Every SQL session from this job must have a trace in one fresh capture directory.");
  }
  for (const session of sessions) {
    if (!["ast-session", "select-when", "crossfile"].includes(session.source)
        || session.fromSessionStart !== true || !integer(session.started) || !integer(session.completed)
        || !integer(session.running) || !integer(session.observed)
        || session.started !== session.completed || session.running !== 0
        || session.completed !== session.observed) {
      problems.push(`Incomplete SQL execution coverage: ${session.session}.`);
    }
  }
  if (!queries.length || queries.some(query => !integer(query.runs))
      || sessions.reduce((sum, session) => sum + (session.observed ?? 0), 0) !== observed) {
    problems.push("SQL execution counts must agree with the complete session traces.");
  }
  if ((report.queries ?? []).some(query => query.incomplete > 0)) {
    problems.push("The workload contains incomplete operations.");
  }
  const exceeded = observed > JOB_SQL_EXECUTION_LIMIT;
  return {
    limit: JOB_SQL_EXECUTION_LIMIT, observed, excess: Math.max(0, observed - JOB_SQL_EXECUTION_LIMIT),
    accepted: !exceeded && problems.length === 0,
    status: exceeded ? "budget-exceeded" : problems.length ? "invalid-capture" : "passed",
    scope: "One complete cold workload across all workers, including setup, DDL, transactions, matching, and cleanup. Profiler-generated EXPLAIN is diagnostic work outside the job.",
    enforcement: "Acceptance gate after the complete run; this does not interrupt the workload at execution 101.",
    categories, sessions, problems,
    firstOverBudget: exceeded ? report.firstOverBudget ?? null : null,
  };
}

export function requireExecutionBudget(report) {
  const budget = executionBudget(report);
  if (!budget.accepted) {
    throw new Error(`SQL job execution budget FAILED: ${budget.observed}/${budget.limit}; ${budget.status}. ${budget.problems.join(" ")}`.trim());
  }
  return budget;
}
