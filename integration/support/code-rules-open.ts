import { expect } from "vitest";
import { defaultRerailDatabasePath, type AdapterSession } from "../../src/engine/index.js";
import { ingestEngineEvent, SqliteSessionReader } from "../../src/runtime/index.js";

/**
 * Waits until the engine service's evaluator for `session` has opened: a turn end reached while it is still opening
 * goes on without code rules and records a `code-rules-pending` event, and one after it has opened records none.
 */
export async function awaitCodeRulesOpen(socketPath: string, session: AdapterSession): Promise<void> {
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  const pending = () => reader.events(session.sessionId).filter(event => event.kind === "code-rules-pending").length;
  let probe = 0;
  try {
    await expect.poll(async () => {
      const before = pending();
      await ingestEngineEvent(socketPath, { kind: "turn-end", eventId: `open-probe:${++probe}`, sessionId: session.sessionId, session, text: "" });
      return pending() === before;
    }, { timeout: 10_000, interval: 20 }).toBe(true);
  } finally {
    reader.close();
  }
}
