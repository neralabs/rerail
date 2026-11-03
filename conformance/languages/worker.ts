/**
 * A worker of the conformance check: a child process `shards.ts` forks, running one request at a time and answering
 * each. It plans each pack (`planPack`) and keeps the plan, so the jobs it is asked to run are the ones every other
 * process making the same plan has. A plan made to run jobs is given the rules that compile, compiles each only when a
 * job needs it, and keeps one language's copies (the jobs come grouped by language); one made to plan or report
 * compiles them all. A worker keeps one plan per pack, of the kind its last request needed.
 */
import { judgeDocuments, judgeToolPack } from "./gates.js";
import { loadEngine } from "./checks/engine.js";
import { loadPack, RERAIL_ROOT } from "./pack.js";
import { planPack, type PackPlan } from "./runner.js";
import { readResults, type PlanOptions, type WorkerRequest, type WorkerResponse } from "./shards.js";

const plans = new Map<string, { plan: PackPlan; runsJobs: boolean }>();

/** The pack's plan: one compiling every rule, or, given `compiles`, one to run jobs. */
function planOf(directory: string, options: PlanOptions, compiles?: string[]): PackPlan {
  const runsJobs = compiles !== undefined;
  const known = plans.get(directory);
  if (known?.runsJobs === runsJobs) return known.plan;
  const plan = planPack(loadPack(directory), options, compiles);
  plans.set(directory, { plan, runsJobs });
  return plan;
}

async function answer(request: WorkerRequest): Promise<WorkerResponse> {
  switch (request.kind) {
    case "jobs": {
      const plan = planOf(request.directory, request.options);
      return { kind: "jobs", jobs: plan.jobs.map(({ source, weight, group }) => ({ source, weight, group })), compiles: [...plan.compiles] };
    }
    case "evaluate": {
      const { jobs } = planOf(request.directory, request.options, request.compiles);
      const results = [];
      for (const index of request.jobs) results.push(await jobs[index]!.run());
      return { kind: "evaluate", results };
    }
    case "report": {
      const plan = planOf(request.directory, request.options);
      return { kind: "report", report: plan.finish(readResults(request.results, loadPack(request.directory).pack.pack, plan.jobs.length)) };
    }
    case "documents":
      return { kind: "documents", documents: judgeDocuments(RERAIL_ROOT, loadEngine(), loadPack(request.directory), request.tool) };
    case "judge":
      return { kind: "judge", verdict: judgeToolPack(RERAIL_ROOT, loadEngine(), loadPack(request.directory), request.report, request.tool, request.documents) };
  }
}

// A compiled code rule's native memory goes only when the JavaScript object holding it is collected, and the
// JavaScript heap stays small, so a worker collects after each request: the probes and the workspaces' rules it compiled
// for the request go with it (`--expose-gc`, shards.ts).
process.on("message", (request: WorkerRequest) => {
  answer(request).then(
    response => {
      process.send!(response);
      globalThis.gc?.();
    },
    (error: unknown) => process.send!({ kind: "error", error: error instanceof Error ? error.message : String(error) } satisfies WorkerResponse),
  );
});
