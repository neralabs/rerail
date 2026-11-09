/** A tool call's run on the filesystem's clock: from just before it started to just after it ended, open while it runs. */
interface ToolCallWindow {
  readonly start: number;
  readonly end: number;
}

/** One open session's tool calls, from `ToolCallWindows.open` until `close`. */
export interface SessionToolCalls {
  readonly running: Map<string, number>;
  finished: ToolCallWindow[];
  /** Its next walk compares with observations made no earlier than this, so it never asks about a write before it. */
  since: number;
  closed: boolean;
}

/**
 * When each session in this process had a tool call running, on the filesystem's clock. A walk finds that a file changed,
 * not who changed it; the file's stamp says when. A change stamped while only another session had a call running is
 * that session's write, never this one's. Stamped while none had, it stays the walking session's, as before.
 */
export class ToolCallWindows {
  readonly #sessions = new Set<SessionToolCalls>();

  /** A session opened at `at`: its first walk may find its own writes made since. */
  open(at: number): SessionToolCalls {
    const session: SessionToolCalls = { running: new Map(), finished: [], since: at, closed: false };
    this.#sessions.add(session);
    return session;
  }

  begin(session: SessionToolCalls, toolCallId: string, at: number): void {
    if (!session.closed && !session.running.has(toolCallId)) session.running.set(toolCallId, at);
  }

  /** A call whose start was not seen opened no window, and adds none. */
  finish(session: SessionToolCalls, toolCallId: string, at: number): void {
    const start = session.running.get(toolCallId);
    if (start === undefined) return;
    session.running.delete(toolCallId);
    session.finished.push({ start, end: at });
  }

  /** Ends every call still running: at a turn's end none is, though the host may not have said so. */
  finishAll(session: SessionToolCalls, at: number): void {
    for (const toolCallId of [...session.running.keys()]) this.finish(session, toolCallId, at);
  }

  /** The session's walk that began at `at` is applied: windows that ended before every open session's last walk go. */
  walked(session: SessionToolCalls, at: number): void {
    session.since = at;
    this.#prune();
  }

  /** The session is closed: its windows stay for the other sessions' walks that have yet to pass them. */
  close(session: SessionToolCalls, at: number): void {
    this.finishAll(session, at);
    session.closed = true;
    this.#prune();
  }

  /** Whose tool call ran when a file was stamped `modifiedAt`: `session`'s first; undefined when none ran then. */
  writer(session: SessionToolCalls, modifiedAt: number): "this" | "another" | undefined {
    if (ranAt(session, modifiedAt)) return "this";
    for (const other of this.#sessions) if (other !== session && ranAt(other, modifiedAt)) return "another";
    return undefined;
  }

  #prune(): void {
    let floor = Infinity;
    for (const session of this.#sessions) if (!session.closed) floor = Math.min(floor, session.since);
    for (const session of this.#sessions) {
      session.finished = session.finished.filter(window => window.end >= floor);
      if (session.closed && session.finished.length === 0) this.#sessions.delete(session);
    }
  }
}

function ranAt(session: SessionToolCalls, at: number): boolean {
  for (const start of session.running.values()) if (start <= at) return true;
  return session.finished.some(window => window.start <= at && at <= window.end);
}

/** Every session the process runs: Claude Code, Codex, Cursor and OMP sessions all reach the one daemon. */
export const PROCESS_TOOL_CALL_WINDOWS = new ToolCallWindows();
