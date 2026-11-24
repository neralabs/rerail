/**
 * Where a tool call ran: the folder its host's hook named (session-relative; undefined where the host did not say),
 * and the files its tool named, as an edit tool names the file it writes (session-relative).
 */
export interface CallPlace {
  readonly folder: string | undefined;
  readonly paths: readonly string[];
}

/**
 * A place a call's writes are found in, as a walk reads it: the session folder (`undefined`), another worktree, or the
 * rest of the session's checkout. Each is answered for by the walk that reads it.
 */
export type CallWalkPlace = string | undefined;

/**
 * A call still running: when it started, on the filesystem's clock, whether it is a shell command's, whether it was
 * never begun on the evaluator (`unwatched`), and where.
 */
interface RunningCall extends CallPlace {
  readonly start: number;
  readonly command: boolean;
  readonly unwatched: boolean;
}

/**
 * A finished tool call's run on the filesystem's clock, from just before it started to just after it ended, its place,
 * and the places other than the session folder whose walks have read past its end (`answeredIn`).
 */
interface ToolCallWindow extends RunningCall {
  readonly end: number;
  /** `ToolCallWindows.ends` once the call ended. */
  readonly ended: number;
  readonly answered: Set<CallWalkPlace>;
}

/** The calls that ended once `after` calls had ended, up to the `through`th (`ToolCallWindows.ends`). */
export interface EndedCalls {
  readonly after: number;
  readonly through: number;
}

/** Which calls a question counts, by where each ran; every call by default. */
export type CallsCounted = (call: CallPlace) => boolean;

/** The places of a call's writes (`CallWalkPlace`), each answered for by the walk that reads it. */
export type CallPlacesOf = (call: CallPlace) => readonly CallWalkPlace[];

/**
 * What a `ranDuring` question asks about: a write in `place` (the session folder by default), by the calls `counted`
 * holds; for the session folder, the calls kept for the file asked about count too (`held`).
 */
export interface CallsAsked {
  readonly place?: CallWalkPlace;
  readonly counted?: CallsCounted;
  readonly held?: EndedCalls | undefined;
}

/** A call begun: whether it is a shell command's, whether it was answered while the evaluator opened, and where it runs. */
export interface CallBegun {
  readonly command?: boolean;
  readonly unwatched?: boolean;
  readonly place?: CallPlace;
}

const EVERY_CALL: CallsCounted = () => true;
const NOWHERE: CallPlace = { folder: undefined, paths: [] };
const IN_THE_SESSION_FOLDER: CallPlacesOf = () => [undefined];

/**
 * The most finished calls a session keeps. Its walks drop the calls they have answered for (`answered`); one whose walks
 * never finish cleanly would otherwise keep every call for the session's life. Past this the oldest go, and a change
 * made in one is taken as another writer's.
 */
export const MAX_FINISHED_CALLS = 1_000;

/**
 * When one session's tool calls ran, on the filesystem's clock, and where. A walk finds that a file changed, not who
 * changed it; the file's stamp says when. An agent writes only through its tool calls, so a change stamped while none of
 * them ran is another writer's: another session, an editor, a process a command left running.
 */
export class ToolCallWindows {
  /** Each call still running, by its id. */
  readonly #running = new Map<string, RunningCall>();
  /** The calls that ended, oldest first. */
  #finished: ToolCallWindow[] = [];
  #ends = 0;
  /**
   * `ends` as the last session folder walk that answered for the calls (`answered`) began: a call that ended by then
   * claims no write there.
   */
  #answered = 0;
  /** The calls that walk kept for the files it could not read (`answered`'s `held`). */
  #held: readonly EndedCalls[] = [];

  /**
   * How many calls have ended so far. A walk notes it as it starts (`commandSince`, `answered`): unlike the filesystem's
   * clock, it orders a call's end and a walk's start that fall within one tick, or that the walk reads the clock after.
   */
  get ends(): number {
    return this.#ends;
  }

  /**
   * A call starts; one begun again (restored after a restart, or begun after it was answered unwatched) keeps its first
   * start. `command` marks a shell command's call, whose writes no tool input names. `unwatched` marks a call answered
   * while the evaluator opened, never begun on it: the files its tool names (its place's `paths`) count for `namedAt`.
   */
  begin(toolCallId: string, at: number, { command = false, unwatched = false, place = NOWHERE }: CallBegun = {}): void {
    if (!this.#running.has(toolCallId)) this.#running.set(toolCallId, { start: at, command, unwatched, folder: place.folder, paths: place.paths });
  }

  /** A call whose start was not seen opened no window, and adds none. */
  finish(toolCallId: string, at: number): void {
    const call = this.#running.get(toolCallId);
    if (call === undefined) return;
    this.#running.delete(toolCallId);
    this.#ends += 1;
    this.#finished.push({ ...call, end: at, ended: this.#ends, answered: new Set() });
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
   * A session folder walk that began once `ends` calls had ended (`ends` as it began) has seen every write there of
   * those calls. A call that ended since stays, though it ended before the walk read the clock: the walk may have left
   * what it was writing to the walk after it (`commandSince`), which takes that write as the call's. The calls in `held`
   * stay too, though only for the file the walk could not read that holds them (`ranDuring`'s `held`). A call goes once
   * the walks of all its places (`placesOf`) have answered for it (`answeredIn`).
   */
  answered(ends: number, held: readonly EndedCalls[] = [], placesOf: CallPlacesOf = IN_THE_SESSION_FOLDER): void {
    this.#answered = Math.max(this.#answered, ends);
    this.#held = held;
    this.#prune(placesOf);
  }

  /**
   * A read of `place`, another worktree or the rest of the session's checkout, that began at `read.at` has seen every
   * write there of the calls that ended before it. With `read.ends` (`ends` as the session's part in that walk began), a
   * call that ended since stays, as `answered` keeps it: the walk may have left what it was writing to the walk after it.
   */
  answeredIn(place: string, read: { readonly at: number; readonly ends?: number }, placesOf: CallPlacesOf): void {
    const ends = read.ends ?? this.#ends;
    for (const window of this.#finished) if (window.end < read.at && window.ended <= ends) window.answered.add(place);
    this.#prune(placesOf);
  }

  /** Drops the calls the walks of all their places have answered for, and no file the session folder's walk missed holds. */
  #prune(placesOf: CallPlacesOf): void {
    this.#finished = this.#finished.filter(window => {
      if (window.ended > this.#answered || this.#held.some(calls => within(calls, window.ended))) return true;
      return !placesOf(window).every(place => place === undefined || window.answered.has(place));
    });
  }

  /**
   * Whether a call `asked.counted` holds ran at some point from `from` to `to`; `from` equal to `to` asks about one
   * stamp. A call the walk of `asked.place` answered for counts only, in the session folder, when it is in `held`, the
   * calls kept for the file asked about (`answered`).
   */
  ranDuring(from: number, to: number, { place, counted = EVERY_CALL, held }: CallsAsked = {}): boolean {
    for (const running of this.#running.values()) if (running.start <= to && counted(running)) return true;
    const claims = (window: ToolCallWindow): boolean => place === undefined
      ? window.ended > this.#answered || (held !== undefined && within(held, window.ended))
      : !window.answered.has(place);
    return this.#finished.some(window => window.start <= to && window.end >= from && counted(window) && claims(window));
  }

  /**
   * Whether an `unwatched` call whose tool named `path` (`begin`'s place's `paths`) ran at `stamp`: a write then is
   * taken as that call's edit, as the session's walks take a write stamped while one of its calls ran.
   */
  namedAt(path: string, stamp: number): boolean {
    const named = (call: RunningCall): boolean => call.unwatched && call.paths.includes(path);
    for (const call of this.#running.values()) if (call.start <= stamp && named(call)) return true;
    return this.#finished.some(window => window.start <= stamp && window.end >= stamp && named(window));
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
