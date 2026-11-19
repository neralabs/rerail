/** A call still running: when it started, on the filesystem's clock, and whether it is a shell command's. */
interface RunningCall {
  readonly start: number;
  readonly command: boolean;
}

/** A finished tool call's run on the filesystem's clock: from just before it started to just after it ended. */
interface ToolCallWindow extends RunningCall {
  readonly end: number;
  /** `ToolCallWindows.ends` once the call ended. */
  readonly ended: number;
}

/** The calls that ended once `after` calls had ended, up to the `through`th (`ToolCallWindows.ends`). */
export interface EndedCalls {
  readonly after: number;
  readonly through: number;
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
  /** Each call still running, by its id. */
  readonly #running = new Map<string, RunningCall>();
  /** The calls that ended, oldest first. */
  #finished: ToolCallWindow[] = [];
  #ends = 0;

  /**
   * How many calls have ended so far. A walk notes it as it starts (`commandSince`, `answered`): unlike the filesystem's
   * clock, it orders a call's end and a walk's start that fall within one tick, or that the walk reads the clock after.
   */
  get ends(): number {
    return this.#ends;
  }

  /**
   * A call starts; one begun again (restored after a restart, or begun after it was answered unwatched) keeps its first
   * start. `command` marks a shell command's call, whose writes no tool input names.
   */
  begin(toolCallId: string, at: number, command = false): void {
    if (!this.#running.has(toolCallId)) this.#running.set(toolCallId, { start: at, command });
  }

  /** A call whose start was not seen opened no window, and adds none. */
  finish(toolCallId: string, at: number): void {
    const call = this.#running.get(toolCallId);
    if (call === undefined) return;
    this.#running.delete(toolCallId);
    this.#ends += 1;
    this.#finished.push({ ...call, end: at, ended: this.#ends });
    if (this.#finished.length > MAX_FINISHED_CALLS) this.#finished.shift();
  }

  /**
   * Ends every call still running: no call runs at a turn's end, so one still open was interrupted, and left open it
   * would claim every later write in the folder.
   */
  finishAll(at: number): void {
    for (const toolCallId of [...this.#running.keys()]) this.finish(toolCallId, at);
  }

  /**
   * A walk that began once `ends` calls had ended (`ends` as it began) has seen every write of those calls: they go. A
   * call that ended since stays, though it ended before the walk read the clock: the walk may have left what it was
   * writing to the walk after it (`commandSince`), which takes that write as the call's. So do the calls in `held`: they
   * may have written a file the walk could not read.
   */
  answered(ends: number, held: readonly EndedCalls[] = []): void {
    const kept = (ended: number): boolean => ended > ends || held.some(calls => ended > calls.after && ended <= calls.through);
    this.#finished = this.#finished.filter(window => kept(window.ended));
  }

  /** Whether a call ran at some point from `from` to `to`; `from` equal to `to` asks about one stamp. */
  ranDuring(from: number, to: number): boolean {
    for (const { start } of this.#running.values()) if (start <= to) return true;
    return this.#finished.some(window => window.start <= to && window.end >= from);
  }

  /**
   * When the earliest of the shell commands still running as a walk began started, given what `ends` was then: those
   * that still run, and those that ended since. Undefined for none. A source such a command had begun by its stamp may
   * have been read part way through its writing.
   */
  commandSince(ends: number): number | undefined {
    let since: number | undefined;
    for (const call of [...this.#running.values(), ...this.#finished.filter(window => window.ended > ends)]) {
      if (call.command && (since === undefined || call.start < since)) since = call.start;
    }
    return since;
  }
}
