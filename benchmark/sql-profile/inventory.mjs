// Count executions at their observed origin, before merging identical SQL text.
const phases = ["connectionSetup", "catalogPublication", "filePublication", "candidatePreparation", "relationalEvaluation", "cleanup", "unknown"];

function phase(sql, context) {
  if (context.executionPhase && phases.includes(context.executionPhase)) return context.executionPhase;
  if (context.scope === "compile") return "connectionSetup";
  if (context.component === "ast-index") {
    if (context.stage === "schema") return "connectionSetup";
    if (context.stage === "candidate-preparation") return "candidatePreparation";
    if (context.stage === "ast-index-put") {
      if (/^\s*INSERT\s+INTO\s+(?:lang|kind|field)\b/i.test(sql)) return "catalogPublication";
      // Older captures left this stage active during later candidate queries.
      return context.path === undefined ? "unknown" : "filePublication";
    }
  }
  if (context.scope === "relational") return "relationalEvaluation";
  // Connection PRAGMAs have a known purpose even before an owner sets its context.
  if (/^\s*PRAGMA\s+(?:temp_store|analysis_limit|cache_size|temp\.cache_size)\b/i.test(sql)) return "connectionSetup";
  return "unknown";
}

export class ExecutionInventory {
  #entries = new Map();
  add(event, query, session) {
    if (event.kind !== "sql") return;
    const context = event.context ?? {};
    const identity = {
      phase: phase(query.sql, context), origin: event.origin ?? "unknown", session, thread: event.thread ?? null,
      query: query.id, producer: context.producer ?? null,
      binding: context.binding ?? null, generation: context.generation ?? null,
      fileDomain: context.fileDomain ?? context.path ?? null,
      consumer: context.ruleId === undefined ? null : [context.ruleId, context.clauseId ?? null],
      stage: context.stage ?? null,
    };
    const key = JSON.stringify(identity);
    let entry = this.#entries.get(key);
    if (!entry) { entry = { ...identity, executions: 0, selfCpuNs: 0, vmSteps: 0 }; this.#entries.set(key, entry); }
    entry.executions++;
    entry.selfCpuNs += event.selfCpuNs ?? 0;
    entry.vmSteps += event.counters?.[0] ?? 0;
  }
  finish(expected) {
    const entries = [...this.#entries.values()];
    const counts = Object.fromEntries(phases.map(p => [p, 0]));
    let total = 0, selfCpuNs = 0;
    for (const entry of entries) { total += entry.executions; selfCpuNs += entry.selfCpuNs; counts[entry.phase] += entry.executions; }
    if (total !== expected) throw new Error(`SQL inventory does not reconcile: ${total}/${expected}`);
    return { version: 1, total, selfCpuNs, phases: counts,
      attribution: "Null identities and the unknown phase mean that the trace did not identify an owner. Statement counts include every observed SQL execution; enclosing operators are excluded.",
      entries };
  }
}
