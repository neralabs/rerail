/**
 * One pack's conformance report, scored in a process of its own. `cli.ts --check` forks this module once per pack, as
 * many at once as the machine has cores (`scorePacks` there): the engine evaluates one workspace at a time per process,
 * so packs scored side by side need processes. The parent sends `{ directory, options }`; the child answers with the
 * report, or with the error that stopped it, and exits.
 */
import { loadPack } from "./pack.js";
import type { T4Report } from "./report.js";
import { runPack, type RunOptions } from "./runner.js";

/** What the parent sends: the pack's directory and the run's options (no samplesDirectory: a function does not travel). */
export interface ScoreRequest {
  directory: string;
  options: Omit<RunOptions, "samplesDirectory">;
}

/** What the child answers. */
export type ScoreResponse = { report: T4Report } | { error: string };

process.once("message", (request: ScoreRequest) => {
  const answer = (response: ScoreResponse): void => {
    process.send!(response, () => process.disconnect!());
  };
  Promise.resolve()
    .then(() => runPack(loadPack(request.directory), request.options))
    .then(
      report => answer({ report }),
      (error: unknown) => answer({ error: error instanceof Error ? error.message : String(error) }),
    );
});
