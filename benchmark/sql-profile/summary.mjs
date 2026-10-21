// Priorities use measured exclusive CPU. A flag count is not a cost or a speedup estimate.
const number = value => Number.isFinite(value) ? value : 0;
const total = (items, key) => items.reduce((sum, item) => sum + number(item[key]), 0);
const ms = ns => ns / 1e6;
const percent = (part, whole) => whole ? 100 * part / whole : 0;
const names = query => new Set((query.flags ?? []).map(flag => flag.code));
const rootInsert = sql => /^\s*INSERT(?:\s+OR\s+IGNORE)?\s+INTO\s+roots\s*\(\s*run\s*,\s*file\s*,\s*node\s*\)\s*VALUES\s*\(\s*\?\d*\s*,\s*\?\d*\s*,\s*\?\d*\s*\)\s*;?\s*$/i.test(sql);
const rootPublication = query => query.kind === "operator" && query.sql.startsWith("/*ROOT_INPUT*/");
const setupCreate = query => query.kind === "sql" && /^\s*CREATE\s+(?:TEMP\s+)?TABLE\b/i.test(query.sql);

/** Structural grouping only: preserve quoted text and repeated-table identity. */
export function sqlShape(sql) {
  const generated = new Map();
  const tokens = sql.match(/'(?:''|[^'])*'|"(?:""|[^"])*"|`(?:``|[^`])*`|\[[^\]]*\]|--[^\n]*|\/\*[\s\S]*?\*\/|[a-zA-Z_][a-zA-Z_0-9]*|(?:\d+\.?\d*(?:[eE][+-]?\d+)?)|[^\s]/g) ?? [];
  return tokens.map(token => {
    if (/^(?:f\d+_\d+|s\d+_f\d+_\d+|c\d+_[a-zA-Z0-9_]+|e\d+_[a-zA-Z0-9_]+)$/.test(token)) {
      if (!generated.has(token)) generated.set(token, `table_${generated.size}`);
      return generated.get(token);
    }
    if (/^\d/.test(token)) return "?";
    return token;
  }).join(" ");
}

function recommendation(query) {
  const flags = names(query);
  if (rootInsert(query.sql)) return {
    key: "root-input-loading", title: "Remove per-root SQLite input calls", certainty: "observed",
    reason: "The input loader submits one statement for each selected (clause, file, root) tuple.",
    change: "Expose the existing selected-root slices through a virtual input table. Preserve run/file filters and duplicate-root semantics.",
    source: "native/src/code_rule_dsl/sql/exec.rs: Session::run_batch, INSERT_ROOT",
    verify: "Confirm that per-root INSERT executions disappear, then compare unprofiled input and apply CPU with native output parity.",
  };
  if (query.kind === "sql" && /SELECT\s+atom\s+FROM\s+node/i.test(query.sql)
      && /\b(?:INSERT\s+INTO\s+c\d+_(?:d\d+|ex)|NOT\s+EXISTS)\b/i.test(query.sql)) return {
    key: "demand-key-lookups", title: "Compute recursive demand keys once", certainty: "observed query shape",
    reason: "Demand insertion or expansion repeats node-to-atom scalar lookups while joining or deduplicating keys.",
    change: "Project the canonical integer demand key once and reuse it in demand insertion, anti-joins, and expansion joins.",
    source: "native/src/code_rule_dsl/sql/emit/keyed.rs; native/src/code_rule_dsl/sql/emit/demand.rs",
    verify: "Compare VM instructions and node lookups for the same recursive clauses. Check captures and dependency invalidation.",
  };
  if (query.kind === "sql" && /JOIN\s+node\b/i.test(query.sql) && query.frameNext >= 100_000
      && query.frameNext / Math.max(1, query.rows + query.changes) >= 100) return {
    key: "candidate-navigation", title: "Restrict candidates before relation joins", certainty: "observed work; proposed cause",
    reason: "Relation queries advance frame cursors at least 100 times per returned or changed row.",
    change: "Inspect the representative plan. Push kind, ancestry, and sibling bounds into the candidate source before frame expansion.",
    source: "native/src/code_rule_dsl/sql/emit.rs; native/src/code_rule_dsl/sql/emit/guards.rs; native/src/code_rule_dsl/sql/nodes.rs",
    verify: "Reduce cursor advances and VM instructions on these queries, then compare unprofiled CPU with output parity.",
  };
  if (query.kind === "sql" && flags.has("empty-result-work")) return {
    key: "empty-result-work", title: "Avoid repeatedly producing empty intermediates", certainty: "observed result; proposed avoidance",
    reason: "These executions return or change no rows despite measured SQL work. Empty results alone do not prove that the work is avoidable.",
    change: "Inspect input emptiness and terminal demand before materialization. Extend skip rules only where an empty result is provable.",
    source: "native/src/code_rule_dsl/sql/skip.rs; native/src/code_rule_dsl/sql/emit.rs",
    verify: "Identify a proof for each skipped operation and compare native findings, unknown outcomes, and recursive reads.",
  };
  if (query.kind === "sql" && flags.has("frame-index-rebuilds")) return {
    key: "frame-index-rebuilds", title: "Investigate repeated frame index construction", certainty: "observed work; cost not isolated",
    reason: "Frame lookup indexes are built repeatedly. The recorded query time also includes matching and other work.",
    change: "Inspect writes between probes and whether consumers can share an immutable indexed input. Measure index-build time before changing ownership.",
    source: "native/src/code_rule_dsl/sql/frames.rs: Frame::lookup",
    verify: "Compare index builds, indexed rows, memory, and unprofiled CPU. Do not equate the entire query cost with index construction.",
  };
  if (query.kind === "operator" && flags.has("many-small-executions")) return {
    key: "small-native-operations", title: "Reduce repeated native frame operations", certainty: "observed calls; profiling-sensitive time",
    reason: "Many short operations pay dispatch and frame-management costs. Instrumentation can dominate short spans.",
    change: "Inspect the representative steps for fusion or redundant clears, then measure the same workload without profiling.",
    source: "native/src/code_rule_dsl/sql/exec.rs: Runner; native/src/code_rule_dsl/sql/frames.rs",
    verify: "Require lower unprofiled CPU and unchanged frame lifetime, captures, and findings.",
  };
  if (setupCreate(query)) return null;
  return {
    key: `${query.kind}:${sqlShape(query.sql)}`, title: "Inspect a costly query shape", certainty: "measured cost; cause not established",
    reason: "This structural query family has measurable CPU cost. Its flags identify investigation points, not proven causes.",
    change: query.kind === "sql" ? "Inspect its plans and cardinalities. Test one restriction, join, or materialization change against the recorded counters."
      : "Inspect frame publication and dispatch costs. Compare unprofiled CPU before changing the implementation.",
    source: "Use the linked query SQL and clause names to locate its emitter.",
    verify: "Keep native output parity and compare unprofiled CPU on both corpus sizes.",
  };
}

function bucket(query) {
  if (rootInsert(query.sql)) return "Selected-root input inserts";
  if (/^\s*(?:INSERT(?:\s+OR\s+IGNORE)?\s+INTO\s+(?:roots|runs)|DELETE\s+FROM\s+(?:roots|runs))\b/i.test(query.sql)) return "Other input-table SQL";
  if (rootPublication(query)) return "Selected-root slice publication";
  if (query.kind === "operator") return "Native operations and SQL submission overhead";
  if (/^\s*(?:CREATE|PRAGMA|BEGIN|COMMIT|ROLLBACK)\b/i.test(query.sql)) return "Setup and transaction SQL";
  return "Rule, stage, result, and control SQL";
}

function metrics(queries, measuredCpuNs) {
  return {
    cpuMs: ms(total(queries, "selfCpuNs")), measuredCpuPercent: percent(total(queries, "selfCpuNs"), measuredCpuNs),
    executions: total(queries, "runs"), vmSteps: total(queries, "vmSteps"),
    returnedRows: total(queries, "rows"), changedRows: total(queries, "changes"),
    frameAdvances: total(queries, "frameNext"), frameColumnReads: total(queries, "frameColumns"),
    frameIndexBuilds: total(queries, "frameIndexBuilds"), frameIndexedRows: total(queries, "frameIndexedRows"),
    queryIds: queries.map(query => query.id),
    representativeQueryIds: [...queries].sort((a, b) => b.selfCpuNs - a.selfCpuNs).slice(0, 3).map(query => query.id),
    contexts: [...new Set(queries.flatMap(query => query.contexts ?? []))],
  };
}

export function actionableSummary(report) {
  const queries = report.queries;
  const measuredCpuNs = total(queries, "selfCpuNs");
  const sqlQueries = queries.filter(query => query.kind === "sql");
  const sqlExecutions = total(sqlQueries, "runs");
  const roots = sqlQueries.filter(query => rootInsert(query.sql));
  const rootMetrics = metrics(roots, measuredCpuNs);
  const publication = metrics(queries.filter(rootPublication), measuredCpuNs);
  const costBuckets = new Map(), candidates = new Map(), shapes = new Map();
  for (const query of queries) {
    const label = bucket(query);
    if (!costBuckets.has(label)) costBuckets.set(label, []);
    costBuckets.get(label).push(query);
    const shape = `${query.kind}:${sqlShape(query.sql)}`;
    if (!shapes.has(shape)) shapes.set(shape, { kind: query.kind, shape: sqlShape(query.sql), queries: [] });
    shapes.get(shape).queries.push(query);
    // One primary recommendation per query prevents overlapping action cost claims.
    const recommendationForQuery = recommendation(query);
    if (!recommendationForQuery) continue;
    if (!candidates.has(recommendationForQuery.key)) candidates.set(recommendationForQuery.key, { ...recommendationForQuery, queries: [] });
    candidates.get(recommendationForQuery.key).queries.push(query);
  }
  const minimumShare = 1, minimumCpuMs = 10;
  const ranked = [...candidates.values()].map(({ queries, ...action }) => ({ ...action, ...metrics(queries, measuredCpuNs) }))
    .filter(action => action.cpuMs >= minimumCpuMs && action.measuredCpuPercent >= minimumShare)
    .sort((a, b) => b.cpuMs - a.cpuMs || a.key.localeCompare(b.key));
  const explanationFailures = sqlQueries.filter(query => names(query).has("explain-unavailable"));
  const expectedSetupFailures = explanationFailures.filter(query => setupCreate(query)
    && query.plans?.length > 0 && query.plans.every(plan => /already exists/i.test(plan.plan?.error ?? plan.bytecode?.error ?? "")));
  const parity = report.workload?.results?.parity;
  const result = {
    version: 1,
    status: parity === true ? "parity-passed" : parity === false ? "parity-failed" : "parity-unavailable",
    nextStep: parity === false ? "Resolve the output mismatch or workload failure before accepting a performance change."
      : measuredCpuNs === 0 ? "No thread CPU time was captured. Inspect the wall-time trace before assigning CPU priorities."
      : "Start with the highest-cost action. Confirm improvements with an unprofiled run and native output parity.",
    budget: {
      cpuTimingAvailable: measuredCpuNs > 0, measuredCpuMs: ms(measuredCpuNs), sqlExecutionCpuMs: ms(total(sqlQueries, "selfCpuNs")),
      operatorExclusiveCpuMs: ms(total(queries.filter(query => query.kind === "operator"), "selfCpuNs")),
      profiledApplyCpuMs: report.workload?.results?.sql?.apply?.cpuMs ?? null,
      nativeApplyCpuMs: report.workload?.results?.native?.apply?.cpuMs ?? null,
      interpretation: "Recorded spans do not partition apply CPU. Profiling, parsing, preparation outside operators, and report work contribute to the difference. Some setup spans precede apply. These costs are not an unprofiled speedup estimate.",
    },
    rootInput: { ...rootMetrics, publication, mode: publication.executions && !rootMetrics.executions ? "virtual-slices" : rootMetrics.executions ? "per-row-inserts" : "not-observed", prioritized: rootMetrics.cpuMs >= minimumCpuMs && rootMetrics.measuredCpuPercent >= minimumShare, sqlExecutionPercent: percent(rootMetrics.executions, sqlExecutions),
      explanation: publication.executions && !rootMetrics.executions
        ? "Selected roots are published as immutable slices once per batch. SQLite reads the roots virtual table directly, without per-root INSERT calls or a B-tree copy."
        : "The loader iterates selected roots for each clause and file, then calls INSERT_ROOT once per tuple. A surrounding transaction batches commits, but not statement execution. Broad or overlapping clause selections repeat roots across runs. The old generated-statement profile omitted these input calls.",
      source: "native/src/code_rule_dsl/sql/exec.rs: Session::run_batch; native/src/code_rule_dsl/sql/inputs.rs: RootInputs",
      nextChange: publication.executions && !rootMetrics.executions
        ? "Per-root insertion is gone. Compare unprofiled input and apply CPU, then investigate the remaining highest-cost actions."
        : rootMetrics.cpuMs >= minimumCpuMs && rootMetrics.measuredCpuPercent >= minimumShare
        ? "Expose selected root slices as a virtual input relation. Preserve clause/file isolation, uniqueness, and the distinction between all roots and an empty selection."
        : "Root loading is below the action threshold in this capture. Use a larger representative capture before changing this path." },
    quality: { expectedSetupExplainFailures: expectedSetupFailures.length,
      unexpectedExplainFailures: explanationFailures.length - expectedSetupFailures.length,
      incompleteOperations: total(queries, "incomplete") },
    costBreakdown: [...costBuckets].map(([label, queries]) => ({ label, ...metrics(queries, measuredCpuNs) })).sort((a, b) => b.cpuMs - a.cpuMs),
    actions: ranked.slice(0, 5).map((action, index) => ({ rank: index + 1, ...action })),
    omittedActionGroups: Math.max(0, ranked.length - 5), minimumActionCpuPercent: minimumShare, minimumActionCpuMs: minimumCpuMs,
    hotShapes: [...shapes.values()].map(({ queries, ...shape }) => ({ ...shape, ...metrics(queries, measuredCpuNs) }))
      .sort((a, b) => b.cpuMs - a.cpuMs).slice(0, 5),
    interpretation: "Action costs describe disjoint sets of queries to investigate, not time caused by a flag or guaranteed savings. Structural shape grouping does not establish semantic equivalence. Flag counts do not determine priority.",
  };
  return result;
}

export function summaryMarkdown(report) {
  const summary = report.actionableSummary;
  const fmt = value => value.toLocaleString("en-US", { maximumFractionDigits: value > 0 && value < 1 ? 3 : 1 });
  const lines = ["# SQL profile: next actions", "", `Output check: **${summary.status}**. ${summary.nextStep}`, "",
    `Recorded CPU: **${fmt(summary.budget.measuredCpuMs)} ms**. Profiled apply CPU: **${summary.budget.profiledApplyCpuMs === null ? "unavailable" : fmt(summary.budget.profiledApplyCpuMs) + " ms"}**.`,
    "", summary.budget.interpretation, "", "## Where recorded CPU goes", "",
    "| Work | CPU ms | Recorded CPU | Executions |", "|---|---:|---:|---:|"];
  for (const cost of summary.costBreakdown) lines.push(`| ${cost.label} | ${fmt(cost.cpuMs)} | ${fmt(cost.measuredCpuPercent)}% | ${fmt(cost.executions)} |`);
  lines.push("", "## Priority actions", "", summary.interpretation);
  if (!summary.actions.length) lines.push("", "No action group reaches both 10 ms and 1% of recorded CPU. Use a larger representative capture before drawing a conclusion.");
  for (const action of summary.actions) {
    lines.push("", `### ${action.rank}. ${action.title}`, "",
      `**${fmt(action.cpuMs)} ms (${fmt(action.measuredCpuPercent)}% of recorded CPU)** across ${fmt(action.executions)} executions.`, "",
      `Evidence: ${action.certainty}. ${action.reason}`, "",
      `Counters: ${fmt(action.vmSteps)} VM instructions, ${fmt(action.frameAdvances)} frame advances, ${fmt(action.frameIndexBuilds)} frame index builds.`, "",
      `Next change: ${action.change}`, "", `Code: ${action.source}`, "",
      `Check: ${action.verify}`, "", `Queries: ${action.representativeQueryIds.map(id => `Q${id}`).join(", ")}.`, "",
      `Contexts: ${action.contexts.slice(0, 6).join(", ") || "session"}${action.contexts.length > 6 ? ` (+${action.contexts.length - 6} more)` : ""}.`);
  }
  lines.push("", "## Selected-root input", "",
    `Published ${fmt(summary.rootInput.publication.returnedRows)} candidate memberships in ${fmt(summary.rootInput.publication.executions)} slice-publication operations (${fmt(summary.rootInput.publication.cpuMs)} ms).`, "",
    `${fmt(summary.rootInput.executions)} individual INSERT executions account for ${fmt(summary.rootInput.sqlExecutionPercent)}% of SQL calls and ${fmt(summary.rootInput.measuredCpuPercent)}% of recorded CPU.`, "",
    summary.rootInput.explanation, "", `Next change: ${summary.rootInput.nextChange}`, "", `Code: ${summary.rootInput.source}`, "",
    "## Measurement quality", "",
    `${summary.quality.expectedSetupExplainFailures} expected setup explanation failures, ${summary.quality.unexpectedExplainFailures} unexpected explanation failures, and ${summary.quality.incompleteOperations} incomplete operations.`, "",
    "Expected setup failures occur because deferred EXPLAIN sees a table that setup already created. They do not outrank expensive runtime queries.", "");
  return lines.join("\n");
}
