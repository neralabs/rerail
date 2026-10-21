/**
 * How long a tool hook or a stop hold may wait on Rerail. OMP and Pi fail a `tool_call` handler that throws, or that
 * overruns OMP's 30 s budget, closed: the tool is blocked with `Extension … failed` / `timed out` (pi-coding-agent
 * 18.0.4 `ExtensionRunner.emitToolCall`, Pi 0.87 `agent-session`). The daemon client waits for as long as a live daemon
 * is still working on the event (a turn's audits can take minutes), so a busy or wedged daemon would otherwise cost the
 * tool call itself.
 */
export const HOOK_BUDGET_MS = 25_000;

/**
 * Runs Rerail's side of a native host hook so that it can never fail or stall the host: a fault costs a steer, never a
 * tool call or the end of a turn. A fault is not the person's to act on, so it leaves one stderr line and no notice,
 * whenever it happens. Work still running when the budget runs out carries on in the background, and the release is
 * logged too.
 */
export async function contained(hook: string, work: Promise<void>): Promise<void> {
  let budget: NodeJS.Timeout | undefined;
  const overdue = new Promise<"overdue">(resolve => {
    budget = setTimeout(() => resolve("overdue"), HOOK_BUDGET_MS);
    budget.unref();
  });
  const logged = work.catch((error: unknown) => {
    process.stderr.write(`Rerail ${hook} hook failed: ${error instanceof Error ? error.message : String(error)}\n`);
  });
  try {
    if (await Promise.race([logged, overdue]) === "overdue") {
      process.stderr.write(`Rerail ${hook} hook released after ${HOOK_BUDGET_MS / 1000}s; Rerail is still working on it\n`);
    }
  } finally {
    clearTimeout(budget);
  }
}
