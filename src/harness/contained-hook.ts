/**
 * How long a tool hook or a stop hold may wait on Rerail. OMP and Pi fail a `tool_call` handler that throws, or that
 * overruns OMP's 30 s budget, closed: the tool is blocked with `Extension … failed` / `timed out` (pi-coding-agent
 * 18.0.4 `ExtensionRunner.emitToolCall`, Pi 0.87 `agent-session`). The daemon client waits up to 30 s for an answer,
 * so a wedged daemon would otherwise cost the tool call itself.
 */
export const HOOK_BUDGET_MS = 25_000;

/**
 * Runs Rerail's side of a native host hook so that it can never fail or stall the host: a fault costs a steer, never a
 * tool call or the end of a turn. A fault is not the person's to act on, so it leaves one stderr line and no notice.
 * Work still running when the budget runs out carries on in the background.
 */
export async function contained(hook: string, work: Promise<void>): Promise<void> {
  let budget: NodeJS.Timeout | undefined;
  const overdue = new Promise<void>(resolve => {
    budget = setTimeout(resolve, HOOK_BUDGET_MS);
    budget.unref();
  });
  try {
    await Promise.race([work, overdue]);
  } catch (error) {
    process.stderr.write(`Rerail ${hook} hook failed: ${error instanceof Error ? error.message : String(error)}\n`);
  } finally {
    clearTimeout(budget);
  }
}
