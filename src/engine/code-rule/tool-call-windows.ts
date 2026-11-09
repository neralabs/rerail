import { sep } from "node:path";

/** A finished tool call's run on the filesystem's clock: from just before it started to just after it ended. */
interface ToolCallWindow {
  readonly start: number;
  readonly end: number;
}

/**
 * The most finished calls a session keeps. A session that never walks again would otherwise keep every other session's
 * calls for the daemon's life; past this, the oldest go, and a change made in one is taken as the walking session's.
 */
const MAX_FINISHED_CALLS = 1_000;

/** One open session's tool calls, from `ToolCallWindows.open` until `close`. */
export interface SessionToolCalls {
  /** The session's folder, as a real path. */
  readonly folder: string;
  /** Each call still running, by its id, with when it started. */
  readonly running: Map<string, number>;
  /** The calls that ended, oldest first. */
  finished: ToolCallWindow[];
  /** Its next walk compares with observations made no earlier than this, so it never asks about a write before it. */
  since: number;
  closed: boolean;
}

/**
 * When each session in this process had a tool call running, on the filesystem's clock. A walk finds that a file changed,
 * not who changed it; the file's stamp says when. A change stamped while a call of another session working on the same
 * files ran, and none of the walking session's, is that session's write. Stamped while none ran, it stays the walking
 * session's, as before.
 */
export class ToolCallWindows {
  readonly #sessions = new Set<SessionToolCalls>();

  /** A session opened in `folder` at `at`: its first walk may find its own writes made since. */
  open(folder: string, at: number): SessionToolCalls {
    const session: SessionToolCalls = { folder, running: new Map(), finished: [], since: at, closed: false };
    this.#sessions.add(session);
    return session;
  }

  /** A call starts; a call begun again (restored after a restart) keeps its first start. */
  begin(session: SessionToolCalls, toolCallId: string, at: number): void {
    if (!session.closed && !session.running.has(toolCallId)) session.running.set(toolCallId, at);
  }

  /** A call whose start was not seen opened no window, and adds none. */
  finish(session: SessionToolCalls, toolCallId: string, at: number): void {
    const start = session.running.get(toolCallId);
    if (start === undefined) return;
    session.running.delete(toolCallId);
    session.finished.push({ start, end: at });
    if (session.finished.length > MAX_FINISHED_CALLS) session.finished.shift();
  }

  /** Ends every call still running: at a turn's end none is, though the host may not have said so. */
  finishAll(session: SessionToolCalls, at: number): void {
    for (const toolCallId of [...session.running.keys()]) this.finish(session, toolCallId, at);
  }

  /** The session's walk that began at `at` is applied: calls that ended before every open session's last walk go. */
  walked(session: SessionToolCalls, at: number): void {
    session.since = at;
    this.#prune();
  }

  /** The session is closed: its calls stay for the other sessions' walks that have yet to pass them. */
  close(session: SessionToolCalls, at: number): void {
    this.finishAll(session, at);
    session.closed = true;
    this.#prune();
  }

  /**
   * Whether a file stamped `modifiedAt` is another session's write: a call of a session whose folder holds `session`'s,
   * or lies in it, ran then, and none of `session`'s did. A session in an unrelated folder never takes it.
   */
  byAnother(session: SessionToolCalls, modifiedAt: number): boolean {
    if (ranAt(session, modifiedAt)) return false;
    for (const other of this.#sessions)
      if (other !== session && sharesFiles(session.folder, other.folder) && ranAt(other, modifiedAt)) return true;
    return false;
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

/** Two session folders share files when one is the other or lies in it. */
function sharesFiles(left: string, right: string): boolean {
  return left === right || left.startsWith(`${right}${sep}`) || right.startsWith(`${left}${sep}`);
}

/** Every session the process runs: Claude Code, Codex, Cursor and OMP sessions all reach the one daemon. */
export const PROCESS_TOOL_CALL_WINDOWS = new ToolCallWindows();
