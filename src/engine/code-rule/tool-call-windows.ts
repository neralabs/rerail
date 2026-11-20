/**
 * A call still running: when it started, on the filesystem's clock, whether it is a shell command's, and the files its
 * tool named, as the session names them, where it was never begun on the evaluator (`begin`'s `paths`).
 */
interface RunningCall {
  readonly start: number;
  readonly command: boolean;
  readonly paths: readonly string[];
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
  /** `ends` as the last walk that answered for the calls (`answered`) began: a call that ended by then claims no write. */
  #answered = 0;

  /**
   * How many calls have ended so far. A walk notes it as it starts (`commandSince`, `answered`): unlike the filesystem's
   * clock, it orders a call's end and a walk's start that fall within one tick, or that the walk reads the clock after.
   */
  get ends(): number {
    return this.#ends;
  }

  /**
   * A call starts; one begun again (restored after a restart, or begun after it was answered unwatched) keeps its first
   * start. `command` marks a shell command's call, whose writes no tool input names. `paths` are the files the tool of a
   * call answered while the evaluator opened names (`namedAt`).
   */
  begin(toolCallId: string, at: number, command = false, paths: readonly string[] = []): void {
    if (!this.#running.has(toolCallId)) this.#running.set(toolCallId, { start: at, command, paths });
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
   * writing to the walk after it (`commandSince`), which takes that write as the call's. The calls in `held` stay too,
   * though only for the file the walk could not read that holds them (`ranDuring`'s `held`).
   */
  answered(ends: number, held: readonly EndedCalls[] = []): void {
    this.#answered = Math.max(this.#answered, ends);
    this.#finished = this.#finished.filter(window => window.ended > ends || held.some(calls => within(calls, window.ended)));
  }

  /**
   * Whether a call ran at some point from `from` to `to`; `from` equal to `to` asks about one stamp. A call a walk
   * answered for counts only when it is in `held`, the calls kept for the file asked about (`answered`).
   */
  ranDuring(from: number, to: number, held?: EndedCalls): boolean {
    for (const { start } of this.#running.values()) if (start <= to) return true;
    const claims = (ended: number): boolean => ended > this.#answered || (held !== undefined && within(held, ended));
    return this.#finished.some(window => window.start <= to && window.end >= from && claims(window.ended));
  }

  /**
   * Whether a call whose tool named `path` (`begin`'s `paths`) ran at `stamp`: a write then is taken as that call's
   * edit, as the session's walks take a write stamped while one of its calls ran.
   */
  namedAt(path: string, stamp: number): boolean {
    for (const call of this.#running.values()) if (call.start <= stamp && call.paths.includes(path)) return true;
    return this.#finished.some(window => window.start <= stamp && window.end >= stamp && window.paths.includes(path));
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

/** Whether the call that ended `ended`th is one of `calls`. */
function within(calls: EndedCalls, ended: number): boolean {
  return ended > calls.after && ended <= calls.through;
}
