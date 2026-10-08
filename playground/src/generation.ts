import { readFile, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { z } from "zod";
import { createSandbox, managedProcess, spawnProcess, ompExecutable, type ManagedProcess } from "./process.js";
import { prepareReplay } from "./real-agent.js";
import type { PlaygroundRuleSnapshot, PlaygroundRun, PlaygroundScenario } from "./protocol.js";
import { generatedScenariosSchema, generationInputSchema, parseGeneratedScenarios } from "./generation-contract.js";

/** No discovery or paid work happens at import/startup. Only this explicit operation starts a model. */
export async function generateScenarios(input: {
  rule: PlaygroundRuleSnapshot;
  previous?: PlaygroundRun;
  checkoutRoot: string;
  hostHome: string;
  signal: AbortSignal;
}): Promise<PlaygroundScenario[]> {
  const signal = AbortSignal.any([input.signal, AbortSignal.timeout(180_000)]);
  signal.throwIfAborted();
  const request = generationInputSchema.parse({
    rule: input.rule,
    previous: input.previous,
    checkoutRoot: await realpath(input.checkoutRoot),
  });
  const sandbox = await createSandbox();
  let child: ManagedProcess | undefined;
  let stopping: Promise<void> | undefined;
  const stop = () => (stopping ??= child?.stop() ?? Promise.resolve());
  const abort = () => {
    void stop().catch(() => {
      /* The awaited finally reports cleanup failures. */
    });
  };
  try {
    const profile = await prepareReplay(sandbox, input.hostHome, true);
    signal.throwIfAborted();
    const requestPath = join(sandbox.root, "generation-input.json");
    const resultPath = join(sandbox.root, "generation-result.json");
    await writeFile(requestPath, JSON.stringify(request), { mode: 0o600, flag: "wx" });
    signal.throwIfAborted();
    child = managedProcess(
      spawnProcess(
        [
          "/usr/bin/sandbox-exec",
          "-f",
          profile,
          ompExecutable(), "--mode", "rpc", "--no-extensions", "--extension",
          join(input.checkoutRoot, "rerail-playground", "dist", "generation-child.js"),
        ],
        {
          cwd: sandbox.cwd,
          env: { ...sandbox.env, RERAIL_PLAYGROUND_INPUT: requestPath, RERAIL_PLAYGROUND_RESULT: resultPath },
          stdin: "ignore",
          stdout: "ignore",
          stderr: "ignore",
        },
      ),
    );
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) abort();
    const code = await child.child.exited;
    signal.throwIfAborted();
    let result: unknown;
    try {
      result = JSON.parse(await readFile(resultPath, "utf8"));
    } catch {
      throw new Error(`Scenario generation exited ${code} without a valid result; no scenarios were changed`);
    }
    const envelope = z
      .discriminatedUnion("ok", [
        z.object({ ok: z.literal(true), scenarios: generatedScenariosSchema }).strict(),
        z.object({ ok: z.literal(false), error: z.string().max(16_384) }).strict(),
      ])
      .parse(result);
    if (!envelope.ok) throw new Error(envelope.error);
    if (code !== 0) throw new Error(`Scenario generation exited with status ${code}`);
    return parseGeneratedScenarios(envelope.scenarios, request.rule);
  } finally {
    signal.removeEventListener("abort", abort);
    try {
      await stop();
    } finally {
      await sandbox.remove();
    }
  }
}
