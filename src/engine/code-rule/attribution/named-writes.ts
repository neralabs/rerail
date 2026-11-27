/**
 * The edit-tool calls of every session of one repository, by the files their tools named: so a session whose shell
 * command ran as another session's edit tool wrote a file does not take that write for its own. A walk finds that a
 * file changed and when (`ToolCallWindows`); two sessions' calls that ran at once both cover that stamp, and only a tool
 * that named the file says which of them wrote it.
 *
 * Only a call that finished counts, and only one that did not fail: its edit was checked at its own result, so the
 * session that yields leaves someone steered. A call still running, or interrupted, may never be checked, and the
 * sessions whose calls ran with it keep taking the write as theirs.
 *
 * Paths are absolute, as each session names its files from its own folder.
 */
export class NamedWrites {
  /** Each session's edit-tool calls still running, by session, then by call id. */
  readonly #running = new Map<string, Map<string, RunningNamedCall>>();
  /**
   * By file, the last finished call of each session whose tool named it. A walk asks only about a file's latest stamp,
   * so an earlier call of the same session tells it nothing more. Oldest file first, at most `MAX_NAMED_FILES`.
   */
  readonly #finished = new Map<string, Map<string, CallRun>>();

  /** A session's edit tool begins a call that names `paths` (absolute). */
  begin(session: string, toolCallId: string, at: number, paths: readonly string[]): void {
    if (paths.length === 0) return;
    let calls = this.#running.get(session);
    if (calls === undefined) this.#running.set(session, (calls = new Map()));
    if (!calls.has(toolCallId)) calls.set(toolCallId, { start: at, paths });
  }

  /** The call ends; one that `failed` wrote nothing the session was checked for, so it names no write. */
  finish(session: string, toolCallId: string, at: number, failed: boolean): void {
    const calls = this.#running.get(session);
    const call = calls?.get(toolCallId);
    if (calls === undefined || call === undefined) return;
    calls.delete(toolCallId);
    if (calls.size === 0) this.#running.delete(session);
    if (!failed) this.#remember(session, call.paths, { start: call.start, end: at });
  }

  /** Notes `run` as the last call of `session` that named each of `paths`. */
  #remember(session: string, paths: readonly string[], run: CallRun): void {
    for (const path of paths) {
      const sessions = this.#finished.get(path) ?? new Map<string, CallRun>();
      // Moved to the end: the file most recently named is the last to go.
      this.#finished.delete(path);
      this.#finished.set(path, sessions.set(session, run));
    }
    while (this.#finished.size > MAX_NAMED_FILES) this.#finished.delete(this.#finished.keys().next().value as string);
  }

  /**
   * A session's calls still running never finish (its turn ended, or it closed), so they name nothing; what it finished
   * stays for the others.
   */
  drop(session: string): void {
    this.#running.delete(session);
  }

  /** The sessions whose finished, successful edit-tool call named `path` (absolute) and ran across `stamp`. */
  wroteAt(path: string, stamp: number): ReadonlySet<string> {
    const writers = new Set<string>();
    for (const [session, run] of this.#finished.get(path) ?? []) if (run.start <= stamp && stamp <= run.end) writers.add(session);
    return writers;
  }
}

/** At most this many files are remembered; past it the file least recently named goes, and a write to it is judged by time alone. */
export const MAX_NAMED_FILES = 10_000;

interface CallRun {
  readonly start: number;
  readonly end: number;
}

interface RunningNamedCall {
  readonly start: number;
  readonly paths: readonly string[];
}
