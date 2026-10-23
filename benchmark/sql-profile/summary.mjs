// Priorities use measured exclusive CPU. A flag count is not a cost or a speedup estimate.
const number = value => Number.isFinite(value) ? value : 0;
const total = (items, key) => items.reduce((sum, item) => sum + number(item[key]), 0);
const ms = ns => ns / 1e6;
const percent = (part, whole) => whole ? 100 * part / whole : 0;
const names = query => new Set((query.flags ?? []).map(flag => flag.code));
const rootInsert = sql => /^\s*INSERT(?:\s+OR\s+IGNORE)?\s+INTO\s+roots\s*\(\s*run\s*,\s*file\s*,\s*node\s*\)\s*VALUES\s*\(\s*\?\d*\s*,\s*\?\d*\s*,\s*\?\d*\s*\)\s*;?\s*$/i.test(sql);
const rootPublication = query => query.kind === "operator" && query.sql.startsWith("/*ROOT_INPUT*/");
const setupCreate = query => query.kind === "sql" && /^\s*CREATE\s+(?:TEMP\s+)?TABLE\b/i.test(query.sql);

function expectedSchemaExplanation(query) {
  if (query.kind !== "sql" || !query.plans?.length) return false;
  return query.plans.every(plan => {
    const error = plan.plan?.error ?? plan.bytecode?.error ?? "";
    return /^\s*CREATE\b/i.test(query.sql) && /already exists/i.test(error)
      || /^\s*DROP\b/i.test(query.sql) && /no such (?:table|view|index)/i.test(error)
      || /^\s*ALTER\s+TABLE\b/i.test(query.sql) && /duplicate column name/i.test(error);
  });
}

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
  if (query.contextDetails?.some(context => context.scope === "relational")) {
    const source = "native/src/code_rule_dsl/relational.rs: Session::advance/publish; use the recorded clause and stage to locate the authored policy";
    const verify = "Compare the same workload without SQL profiling. Preserve complete diagnostic parity, source gates, cancellation, and work limits.";
    if (/^\s*CREATE\s+(?:TEMP\s+)?TABLE\b[\s\S]*\bAS\s+(?:SELECT|WITH)\b/i.test(query.sql)) return {
      key: `relational-materialize:${sqlShape(query.sql)}`, title: "Reduce costly relation materialization", certainty: "measured query cost; cause not established",
      reason: "This stage executes a SELECT and stores its results in a SQLite table. Its CPU includes expression evaluation and table construction.",
      change: "Inspect the SELECT plan, VM instructions, and virtual-table visits. Test narrower columns, earlier restrictions, or shared derivation where semantics permit.", source, verify,
    };
    if (/^\s*CREATE\s+(?:UNIQUE\s+)?INDEX\b/i.test(query.sql)) return {
      key: "relational-indexes", title: "Reduce repeated SQL index construction", certainty: "measured index statements",
      reason: "These statements build SQLite indexes over materialized results. Their CPU is measured separately from the producer algorithm.",
      change: "Inspect which index permutations consumers use. Compare index reuse or an indexed read-only relation over completed numeric output.", source, verify,
    };
    if (query.kind === "operator") return {
      key: query.sql.startsWith("/*RELATIONAL_PUBLISH*/") ? "relational-publication" : "relational-submission",
      title: query.sql.startsWith("/*RELATIONAL_PUBLISH*/") ? "Reduce result publication overhead" : "Inspect SQL preparation and submission overhead",
      certainty: "exclusive operation CPU; profiling-sensitive",
      reason: "This cost excludes child SQL execution. It includes host preparation, conversions, and instrumentation within the operation.",
      change: "Inspect repeated preparation and row conversion. Confirm their unprofiled cost before changing the completed-output publication contract.", source, verify,
    };
    return { key: `relational-query:${sqlShape(query.sql)}`, title: "Inspect a costly relational query", certainty: "measured cost; cause not established",
      reason: "The recorded counters and plan identify this query as an investigation target.",
      change: "Test one join, restriction, scan, or materialization change using the observed clause and stage.", source, verify };
  }
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
    change: "Inspect anchor-first join order and planner cardinality estimates. Apply kind, ancestry, and sibling bounds before frame expansion.",
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
  if (query.kind === "sql" && /\bAS\s+MATERIALIZED\b/i.test(query.sql)
      && /\bFROM\s+(?:pattern|fact)\s+WHERE\b/i.test(query.sql)) return {
    key: "terminal-materialization", title: "Inspect terminal evaluation and materialization", certainty: "observed query shape; subcosts not isolated",
    reason: "Batched terminal queries gather demand keys, materialize matches, and join results back to frames. Query CPU includes terminal evaluation.",
    change: "Separate terminal evaluation from SQLite bookkeeping. Inspect repeated input scans, temporary key sets, and result rejoining before changing the batched terminal contract.",
    source: "native/src/code_rule_dsl/sql/emit.rs: Emitter::pattern; native/src/code_rule_dsl/sql/emit/three.rs; native/src/code_rule_dsl/sql/emit/leaves.rs",
    verify: "Compare VM instructions, frame visits, and unprofiled CPU. Preserve capture-dependent matching and complete native output parity.",
  };
  if (query.kind === "sql" && flags.has("frame-index-rebuilds")
      && (number(query.frameIndexTimedBuilds) < number(query.frameIndexBuilds)
        || number(query.frameIndexCpuNs) >= 0.1 * number(query.selfCpuNs))) return {
    key: "frame-index-rebuilds", title: "Investigate repeated frame index construction", certainty: "observed work; cost not isolated",
    reason: "Frame lookup indexes are built repeatedly. The recorded query time also includes matching and other work.",
    change: "Compare the measured index subcost with total query CPU before changing ownership. Older captures need a new run to measure this subcost.",
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
  if (/^\s*CREATE\s+(?:TEMP\s+)?TABLE\b[\s\S]*\bAS\s+(?:SELECT|WITH)\b/i.test(query.sql)) return "Relation materialization SQL";
  if (/^\s*CREATE\s+(?:UNIQUE\s+)?INDEX\b/i.test(query.sql)) return "SQL index construction";
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
    frameIndexCpuMs: total(queries, "frameIndexTimedBuilds") ? ms(total(queries, "frameIndexCpuNs")) : null,
    frameIndexTimedBuilds: total(queries, "frameIndexTimedBuilds"),
    queryIds: queries.map(query => query.id),
    representativeQueryIds: [...queries].sort((a, b) => b.selfCpuNs - a.selfCpuNs).slice(0, 3).map(query => query.id),
    contexts: [...new Set(queries.flatMap(query => query.contexts ?? []))],
  };
}

export function actionableSummary(report) {
  const queries = report.queries;
  const measuredCpuNs = total(queries, "selfCpuNs");
  const applyCpuMs = report.workload?.results?.sql?.apply?.cpuMs ?? null;
  // TRACE_PROFILE ends when a cursor finishes, not when each sqlite3_step returns.
  // Suspended internal cursors can therefore include other statements' CPU lifetimes.
  const unreconciledCpu = applyCpuMs !== null && measuredCpuNs / 1e6 > Math.max(2 * applyCpuMs, applyCpuMs + 50);
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
  const expectedSetupFailures = explanationFailures.filter(expectedSchemaExplanation);
  const parity = report.workload?.results?.parity;
  const result = {
    version: 1,
    status: parity === true ? "parity-passed" : parity === false ? "parity-failed" : "parity-unavailable",
    nextStep: parity === false ? "Resolve the output mismatch or workload failure before accepting a performance change."
      : unreconciledCpu ? "Recorded statement lifetimes exceed the apply CPU budget. Use API boundary timers before ranking CPU costs; suspended cursors or setup outside apply can overlap these lifetimes."
      : measuredCpuNs === 0 ? "No thread CPU time was captured. Inspect the wall-time trace before assigning CPU priorities."
      : "Start with the highest-cost action. Confirm improvements with an unprofiled run and native output parity.",
    budget: {
      cpuTimingAvailable: measuredCpuNs > 0, measuredCpuMs: ms(measuredCpuNs), sqlExecutionCpuMs: ms(total(sqlQueries, "selfCpuNs")),
      operatorExclusiveCpuMs: ms(total(queries.filter(query => query.kind === "operator"), "selfCpuNs")),
      profiledApplyCpuMs: applyCpuMs,
      cpuCostsReconciled: !unreconciledCpu,
      nativeApplyCpuMs: report.workload?.results?.native?.apply?.cpuMs ?? null,
      frameIndexCpuMs: total(sqlQueries, "frameIndexTimedBuilds") ? ms(total(sqlQueries, "frameIndexCpuNs")) : null,
      frameIndexBuilds: total(sqlQueries, "frameIndexBuilds"), frameIndexTimedBuilds: total(sqlQueries, "frameIndexTimedBuilds"),
      interpretation: "Recorded spans do not partition apply CPU. Profiling, parsing, preparation outside operators, and report work contribute to the difference. Some setup spans precede apply. These costs are not an unprofiled speedup estimate.",
    },
    rootInput: { ...rootMetrics, publication, mode: publication.executions && !rootMetrics.executions ? "virtual-slices" : rootMetrics.executions ? "per-row-inserts" : "not-observed", prioritized: rootMetrics.cpuMs >= minimumCpuMs && rootMetrics.measuredCpuPercent >= minimumShare, sqlExecutionPercent: percent(rootMetrics.executions, sqlExecutions),
      explanation: publication.executions && !rootMetrics.executions
        ? "Selected roots are published as immutable slices once per batch. SQLite reads the roots virtual table directly, without per-root INSERT calls or a B-tree copy."
        : !rootMetrics.executions ? "This capture contains no per-root INSERT_ROOT path. Use the recorded input and publication statements to assess this executor."
        : "The loader iterates selected roots for each clause and file, then calls INSERT_ROOT once per tuple. A surrounding transaction batches commits, but not statement execution. Broad or overlapping clause selections repeat roots across runs. The old generated-statement profile omitted these input calls.",
      source: report.workload?.executor === "relational" ? "native/src/code_rule_dsl/relational/insert.rs; native/src/code_rule_dsl/relational/producer.rs" : "native/src/code_rule_dsl/sql/exec.rs: Session::run_batch; native/src/code_rule_dsl/sql/inputs.rs: RootInputs",
      nextChange: publication.executions && !rootMetrics.executions
        ? "Per-root insertion is gone. Compare unprofiled input and apply CPU, then investigate the remaining highest-cost actions."
        : rootMetrics.cpuMs >= minimumCpuMs && rootMetrics.measuredCpuPercent >= minimumShare
        ? "Expose selected root slices as a virtual input relation. Preserve clause/file isolation, uniqueness, and the distinction between all roots and an empty selection."
        : "Root loading is below the action threshold in this capture. Use a larger representative capture before changing this path." },
    quality: { expectedSetupExplainFailures: expectedSetupFailures.length,
      unreconciledCpu,
      unexpectedExplainFailures: explanationFailures.length - expectedSetupFailures.length,
      incompleteOperations: total(queries, "incomplete") },
    costBreakdown: [...costBuckets].map(([label, queries]) => ({ label, ...metrics(queries, measuredCpuNs) })).sort((a, b) => b.cpuMs - a.cpuMs),
    actions: (unreconciledCpu ? [] : ranked.slice(0, 5)).map((action, index) => ({ rank: index + 1, ...action })),
    omittedActionGroups: unreconciledCpu ? ranked.length : Math.max(0, ranked.length - 5), minimumActionCpuPercent: minimumShare, minimumActionCpuMs: minimumCpuMs,
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
  lines.push("", summary.budget.frameIndexCpuMs === null
    ? summary.budget.frameIndexBuilds ? "Frame index construction was not timed in this capture. Re-run with the current profiler to isolate this cost." : "No lazy frame index builds were observed inside SQL. Prepared structural indexes and SQL CREATE INDEX statements have separate attribution."
    : `Frame index construction: **${fmt(summary.budget.frameIndexCpuMs)} ms** across ${fmt(summary.budget.frameIndexTimedBuilds)} timed builds (${fmt(summary.budget.frameIndexBuilds)} total). This subcost includes scan-order preparation on lookup cache misses and is already included in SQL CPU.`);
  lines.push("", "## Priority actions", "", summary.interpretation);
  if (!summary.actions.length) lines.push("", summary.quality.unreconciledCpu
    ? "CPU priorities are withheld because statement lifetimes exceed the apply CPU budget. Use API boundary timers."
    : "No action group reaches both 10 ms and 1% of recorded CPU. Use a larger representative capture before drawing a conclusion.");
  for (const action of summary.actions) {
    lines.push("", `### ${action.rank}. ${action.title}`, "",
      `**${fmt(action.cpuMs)} ms (${fmt(action.measuredCpuPercent)}% of recorded CPU)** across ${fmt(action.executions)} executions.`, "",
      `Evidence: ${action.certainty}. ${action.reason}`, "",
      `Counters: ${fmt(action.vmSteps)} VM instructions, ${fmt(action.frameAdvances)} frame advances, ${fmt(action.frameIndexBuilds)} frame index builds.`, "",
      ...(action.frameIndexCpuMs === null ? [] : [`Measured index construction: ${fmt(action.frameIndexCpuMs)} ms across ${fmt(action.frameIndexTimedBuilds)} timed builds. This is included in the query cost above.`, ""]),
      `Next change: ${action.change}`, "", `Code: ${action.source}`, "",
      `Check: ${action.verify}`, "", `Queries: ${action.representativeQueryIds.map(id => `Q${id}`).join(", ")}.`, "",
      `Contexts: ${action.contexts.slice(0, 6).join(", ") || "session"}${action.contexts.length > 6 ? ` (+${action.contexts.length - 6} more)` : ""}.`);
  }
  if (report.workload?.executor !== "relational" || summary.rootInput.mode !== "not-observed") lines.push("", "## Selected-root input", "",
    `Published ${fmt(summary.rootInput.publication.returnedRows)} candidate memberships in ${fmt(summary.rootInput.publication.executions)} slice-publication operations (${fmt(summary.rootInput.publication.cpuMs)} ms).`, "",
    `${fmt(summary.rootInput.executions)} individual INSERT executions account for ${fmt(summary.rootInput.sqlExecutionPercent)}% of SQL calls and ${fmt(summary.rootInput.measuredCpuPercent)}% of recorded CPU.`, "",
    summary.rootInput.explanation, "", `Next change: ${summary.rootInput.nextChange}`, "", `Code: ${summary.rootInput.source}`, "");
  lines.push("## Measurement quality", "",
    `${summary.quality.expectedSetupExplainFailures} expected setup explanation failures, ${summary.quality.unexpectedExplainFailures} unexpected explanation failures, and ${summary.quality.incompleteOperations} incomplete operations.`, "",
    "Expected schema failures occur because deferred EXPLAIN sees completed CREATE, ALTER, or DROP operations. They do not outrank expensive runtime queries.", "");
  return lines.join("\n");
}
