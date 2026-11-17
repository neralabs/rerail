/**
 * Where a tool call ran: the folder its host's hook named (session-relative; undefined where the host did not say),
 * and the files it named, as an edit tool names the file it writes (session-relative).
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
 * A tool call's run on the filesystem's clock, from just before it started to just after it ended, its place, and the
 * places whose walks have read past its end.
 */
interface ToolCallWindow extends CallPlace {
  readonly start: number;
  readonly end: number;
  readonly answered: Set<CallWalkPlace>;
}

/** Which calls a question counts, by where each ran; every call by default. */
export type CallsCounted = (call: CallPlace) => boolean;

const EVERY_CALL: CallsCounted = () => true;
const NOWHERE: CallPlace = { folder: undefined, paths: [] };

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
  /** Each call still running, by its id, with when it started and where. */
  readonly #running = new Map<string, Omit<ToolCallWindow, "end" | "answered">>();
  /** The calls that ended, oldest first. */
  #finished: ToolCallWindow[] = [];

  /** A call starts; one begun again (restored after a restart, or begun after it was answered unwatched) keeps its first start. */
  begin(toolCallId: string, at: number, place: CallPlace = NOWHERE): void {
    if (!this.#running.has(toolCallId)) this.#running.set(toolCallId, { start: at, folder: place.folder, paths: place.paths });
  }

  /** A call whose start was not seen opened no window, and adds none. */
  finish(toolCallId: string, at: number): void {
    const running = this.#running.get(toolCallId);
    if (running === undefined) return;
    this.#running.delete(toolCallId);
    this.#finished.push({ ...running, end: at, answered: new Set() });
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
   * A walk of `place` that began at `at` has seen every write there of the calls that ended before it. A call goes once
   * the walks of all its places (`placesOf`) have; without a place, every call that ended before `at` goes.
   */
  answered(at: number, place?: { readonly place: CallWalkPlace; readonly placesOf: (call: CallPlace) => readonly CallWalkPlace[] }): void {
    this.#finished = this.#finished.filter(window => {
      if (window.end >= at) return true;
      if (place === undefined) return false;
      window.answered.add(place.place);
      return !place.placesOf(window).every(each => window.answered.has(each));
    });
  }

  /** Whether a call `counted` holds ran at some point from `from` to `to`; `from` equal to `to` asks about one stamp. */
  ranDuring(from: number, to: number, counted: CallsCounted = EVERY_CALL): boolean {
    for (const running of this.#running.values()) if (running.start <= to && counted(running)) return true;
    return this.#finished.some(window => window.start <= to && window.end >= from && counted(window));
  }
}
