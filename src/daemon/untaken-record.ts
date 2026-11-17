import { resolve } from "node:path";
import type { AdapterEvent } from "../engine/adapter-contract.js";
import { parseEngineRequest } from "../runtime/engine-service.js";
import { SqliteSessionStore } from "../runtime/session-store.js";
import type { UntakenEvent } from "../runtime/untaken-events.js";

/** Where a daemon records the events hooks left untaken: its database, its own session store on it, and its stores' change mark. */
export interface UntakenRecordTarget {
  databasePath: string;
  sessionStore(): SqliteSessionStore;
  markChanged(): void;
}

/**
 * Records an event a hook left untaken while the daemon did not read (`untaken-events.ts`), as history only: its row in
 * the session's record at the time its hook met it, as the hook kept it (scrubbed of secrets, a result's output
 * withheld), and the session's row from the event's own descriptor where the session has none yet. No runtime sees it:
 * checked late, a call would read its edit's "before" from the file as already edited, and an event at a turn's end
 * would take the steers the session holds for its next live hook, though nobody waits for its answer. One that does not
 * parse or fails is lost, as a request that fails is, and daemon.log says so.
 */
export function recordUntakenEvent(target: UntakenRecordTarget, { body, arrivedAt }: UntakenEvent): void {
  try {
    const { event, configuration } = parseEngineRequest(JSON.parse(body));
    withSessionStore(target, configuration?.databasePath, store => appendLate(store, event, arrivedAt));
    target.markChanged();
  } catch (error) {
    process.stderr.write(`rerail daemon: an event a hook left untaken was not recorded: ${error instanceof Error ? error.message : String(error)}\n`);
  }
}

function appendLate(store: SqliteSessionStore, event: AdapterEvent, arrivedAt: number): void {
  const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
  store.append({ sessionId, eventId: event.eventId, kind: event.kind, payload: event, observedAt: arrivedAt }, event.session, { startedAt: arrivedAt });
}

/** Runs `record` on the session store of `databasePath`: the daemon's own, or one opened for that database and closed. */
function withSessionStore(target: UntakenRecordTarget, databasePath: string | undefined, record: (store: SqliteSessionStore) => void): void {
  if (databasePath === undefined || resolve(databasePath) === target.databasePath) {
    record(target.sessionStore());
    return;
  }
  const store = new SqliteSessionStore(resolve(databasePath));
  try {
    record(store);
  } finally {
    store.close();
  }
}
