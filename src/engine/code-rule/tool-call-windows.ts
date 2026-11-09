/** A finished tool call's run on the filesystem's clock: from just before it started to just after it ended. */
interface ToolCallWindow {
  readonly start: number;
  readonly end: number;
}

/**
 * The most finished calls a session keeps. Its walks drop the calls they have answered for (`answered`); one whose walks
 * never finish cleanly would otherwise keep every call for the session's life. Past this the oldest go, and a change
 * made in one is taken as another writer's.
 */
export const MAX_FINISHED_CALLS = 1_000;

/**
 * When one session's tool calls ran, on the filesystem's clock. A walk finds that a file changed, not who changed it;
 * the file's stamp says when. An agent writes only through its tool calls, so a change stamped while none of them ran
 * is another writer's: another session, an editor, a process a command left running.
 */
export class ToolCallWindows {
  /** Each call still running, by its id, with when it started. */
  readonly #running = new Map<string, number>();
  /** The calls that ended, oldest first. */
  #finished: ToolCallWindow[] = [];

  /** A call starts; one begun again (restored after a restart, or begun after it was answered unwatched) keeps its first start. */
  begin(toolCallId: string, at: number): void {
    if (!this.#running.has(toolCallId)) this.#running.set(toolCallId, at);
  }

  /** A call whose start was not seen opened no window, and adds none. */
  finish(toolCallId: string, at: number): void {
    const start = this.#running.get(toolCallId);
    if (start === undefined) return;
    this.#running.delete(toolCallId);
    this.#finished.push({ start, end: at });
    if (this.#finished.length > MAX_FINISHED_CALLS) this.#finished.shift();
  }

  /**
   * Ends every call still running: no call runs at a turn's end, so one still open was interrupted, and left open it
   * would claim every later write in the folder.
   */
  finishAll(at: number): void {
    for (const toolCallId of [...this.#running.keys()]) this.finish(toolCallId, at);
  }

  /** A walk that began at `at` has seen every write of the calls that ended before it: those go. */
  answered(at: number): void {
    this.#finished = this.#finished.filter(window => window.end >= at);
  }

  /** Whether a call ran at some point from `from` to `to`; `from` equal to `to` asks about one stamp. */
  ranDuring(from: number, to: number): boolean {
    for (const start of this.#running.values()) if (start <= to) return true;
    return this.#finished.some(window => window.start <= to && window.end >= from);
  }
}
