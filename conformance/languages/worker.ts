/**
 * A worker of the conformance check: a child process `shards.ts` forks, running one request at a time and answering
 * each. It plans each pack once (`planPack`) and keeps the plan, so the jobs it is asked to run are the ones every other
 * process making the same plan has.
 */
import { judgeToolPack } from "./gates.js";
import { loadEngine } from "./checks/engine.js";
import { loadPack, RERAIL_ROOT } from "./pack.js";
import { planPack, type PackPlan } from "./runner.js";
import { readResults, type WorkerRequest, type WorkerResponse } from "./shards.js";

const plans = new Map<string, PackPlan>();

function planOf(directory: string, options: Parameters<typeof planPack>[1]): PackPlan {
  let plan = plans.get(directory);
  if (plan === undefined) plans.set(directory, (plan = planPack(loadPack(directory), options)));
  return plan;
}

async function answer(request: WorkerRequest): Promise<WorkerResponse> {
  switch (request.kind) {
    case "jobs":
      return { kind: "jobs", jobs: planOf(request.directory, request.options).jobs.map(({ source, weight }) => ({ source, weight })) };
    case "evaluate": {
      const { jobs } = planOf(request.directory, request.options);
      const results = [];
      for (const index of request.jobs) results.push(await jobs[index]!.run());
      return { kind: "evaluate", results };
    }
    case "report": {
      const plan = planOf(request.directory, request.options);
      return { kind: "report", report: plan.finish(readResults(request.results, loadPack(request.directory).pack.pack, plan.jobs.length)) };
    }
    case "judge":
      return { kind: "judge", verdict: judgeToolPack(RERAIL_ROOT, loadEngine(), loadPack(request.directory), request.report, request.tool) };
  }
}

process.on("message", (request: WorkerRequest) => {
  answer(request).then(
    response => process.send!(response),
    (error: unknown) => process.send!({ kind: "error", error: error instanceof Error ? error.message : String(error) } satisfies WorkerResponse),
  );
});
