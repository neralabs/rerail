import { expect } from "vitest";
import { defaultRerailDatabasePath, type AdapterSession } from "../../src/engine/index.js";
import { type EngineEndpoint, ingestEngineEvent, SqliteSessionReader } from "../../src/runtime/index.js";

/**
 * Waits until the engine service's evaluator for `session` has opened: a turn end reached while it is still opening
 * goes on without code rules and records a `code-rules-pending` event, and one after it has opened records none.
 */
export async function awaitCodeRulesOpen(engine: EngineEndpoint, session: AdapterSession): Promise<void> {
  await awaitCodeRulesOpenThrough(session.sessionId, probe => ingestEngineEvent(engine, {
    kind: "turn-end", eventId: `open-probe:${probe}`, sessionId: session.sessionId, session, text: "",
  }));
}

/**
 * Waits until the evaluator of session `sessionId` has opened, as `awaitCodeRulesOpen` does, for a host whose events
 * the test reaches only through the host: `probe` reaches code-rule boundaries (a call, a result or a turn end) that
 * edit nothing, and returns once they are answered. A probe recording no `code-rules-pending` event found it open.
 */
export async function awaitCodeRulesOpenThrough(sessionId: string, probe: (probe: number) => Promise<unknown>): Promise<void> {
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  const pending = () => reader.events(sessionId).filter(event => event.kind === "code-rules-pending").length;
  let probes = 0;
  try {
    await expect.poll(async () => {
      const before = pending();
      await probe(++probes);
      return pending() === before;
    }, { timeout: 10_000, interval: 20 }).toBe(true);
  } finally {
    reader.close();
  }
}
