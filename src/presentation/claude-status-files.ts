/**
 * Claude Code's status line, drawn ahead of time. Claude Code only runs a command for its status line, so every
 * refresh is a process launch; the daemon therefore keeps each Claude Code session's line in a file,
 * `<Rerail home>/status/<session id>`, and the registered command prints it with shell built-ins
 * (`src/hosts/claude/claude-status.sh`), starting nothing.
 *
 * A file holds the line three times, one per colour depth the script picks from (`STATUS_FILE_DEPTHS`), or nothing
 * where the session draws no line. The lines are what `readPresentation` gives the same session over HTTP: this asks
 * the daemon's own `presentation_get` and builds the footer from its answer with the same code.
 */
import { mkdirSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { invokeOperation, type RerailApiStores } from "../api/operation.js";
import { operations } from "../api/operations.js";
import { presentationReadSchema } from "../api/presentation-contracts.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import { replaceFile } from "../engine/file-replace.js";
import { keepFolderPrivate } from "../engine/storage/private-files.js";
import type { DrawnSession, SqliteSessionReader } from "../runtime/session-store.js";
import type { TerminalEnv } from "./rerail-mark.js";
import {
  answeredSessionFooter, fallbackPresentation, FOOTER_DAEMON_HOSTS, formatTerminalStatus, stoppedDaemonPresentation, type SessionPresentation,
} from "./session-presentation.js";

const HOST = "claude-code";

/**
 * A session id that names its status file. The script reads the id out of Claude Code's JSON with shell built-ins and
 * accepts exactly these, so an id cannot name a file outside the folder; a session with any other id gets no file.
 */
const STATUS_FILE_NAME = /^[A-Za-z0-9_-]+$/u;

/** The terminals a status file holds a line for, in the file's line order: 24-bit colour, 256 colours, no colour. */
const STATUS_FILE_DEPTHS: readonly TerminalEnv[] = [{ COLORTERM: "truecolor" }, {}, { NO_COLOR: "1" }];

/** How often the daemon looks for a change to draw; one read of the database's change counter while there is none. */
const CHECK_EVERY_MS = 1_000;
/**
 * Every session is drawn at least this often, with or without a change in the database: a worktree's own rule files
 * change without one, and so does what a quiet session shows (`QUIET_AFTER_MS`).
 */
const DRAW_ANYWAY_MS = 60_000;
/** After a pass, the daemon waits this many times as long as it took, so drawing takes a bounded share of its time. */
const PACE = 10;
/**
 * A session with no event for this long is quiet: Claude Code runs its status line only around its own events, so its
 * line is drawn with every session's (`DRAW_ANYWAY_MS`), not on every change. Its next event makes it active again.
 */
const QUIET_AFTER_MS = 30 * 60 * 1_000;
/**
 * A session with no event for this long gets no file: its Claude Code is taken to be gone. The database cannot say.
 * Claude Code gives a session's end hook little time and often exits before it has run, so such a session stays open
 * there; and a daemon's stop ends every open session, of which the ones still running open again only at their next
 * event. A session that does go on is drawn again at that event.
 */
const UNREAD_AFTER_MS = 7 * 24 * 60 * 60 * 1_000;

/** The text of a session's status file: its line at each depth, or nothing where it draws none (a silent footer, a repository Rerail is not on in). */
export function statusFileText(presentation: SessionPresentation): string {
  if (presentation.presence === "silent" || presentation.parts.length === 0) return "";
  return STATUS_FILE_DEPTHS.map(env => `${formatTerminalStatus(presentation, env)}\n`).join("");
}

/** The session ids with a status file in `folder`; none when the folder is missing. */
function drawnSessionIds(folder: string): string[] {
  try {
    return readdirSync(folder).filter(name => STATUS_FILE_NAME.test(name));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
}

/**
 * Removes from `folder` the status file of every session the database no longer records, as after `rerail reset`
 * deleted it. Returns the sessions recorded that the folder draws, and those active at or after `activeSince`; none
 * but the folder's by default.
 */
export function removeUnrecordedStatusFiles(
  folder: string, reader: Pick<SqliteSessionReader, "drawnSessions">, activeSince = Number.MAX_SAFE_INTEGER,
): DrawnSession[] {
  const drawn = drawnSessionIds(folder);
  const sessions = reader.drawnSessions(HOST, activeSince, drawn);
  const recorded = new Set(sessions.map(session => session.sessionId));
  for (const sessionId of drawn) if (!recorded.has(sessionId)) rmSync(join(folder, sessionId), { force: true });
  return sessions;
}

export interface ClaudeStatusFilesOptions {
  /** `<Rerail home>/status`; made, in a home that exists, when the first line is written. */
  folder: string;
  /** The daemon's stores: its sessions, its answer for each (`presentation_get`) and the dashboard's origin the lines link into. */
  stores: RerailApiStores;
  /** How often to look for a change (`CHECK_EVERY_MS`); `false` only draws when asked (`refresh`). */
  checkEveryMs?: number | false;
}

export interface ClaudeStatusFiles {
  /** Draws every session's line now, after any pass in flight. Resolves once the files are written; never rejects. */
  refresh(): Promise<void>;
  /**
   * The daemon is stopping: ends the checks and leaves every file saying what a session shows with no daemon, `not
   * running` where Rerail is started. A daemon that is killed leaves the last lines it drew. Never rejects.
   */
  stop(): Promise<void>;
}

const PRESENTATION_GET = operations.find(operation => operation.id === "presentation_get")!;

class StatusFileKeeper implements ClaudeStatusFiles {
  readonly #folder: string;
  readonly #stores: RerailApiStores;
  /** What each file holds, and the start state its session's repository had when it was drawn. */
  readonly #written = new Map<string, { text: string; state: RepositoryStartState | undefined }>();
  /**
   * When each session had ended, as the last pass read it; null for one still open. A session that has ended since, or
   * ended again, ended while this daemon ran, so it ended for good and its file goes. One that was ended already when
   * this daemon first read it may be a session the last daemon's stop ended, which still runs.
   */
  readonly #endedAt = new Map<string, number | null>();
  readonly #timer: NodeJS.Timeout | undefined;
  #done: Promise<void> = Promise.resolve();
  #queued = false;
  #stopped = false;
  #folderMade = false;
  #swept = false;
  /** The database's change counter at the last pass, and when the next pass may and must run. */
  #version: string | undefined;
  #notBefore = 0;
  #drawAnywayAt = 0;
  #reported: string | undefined;

  constructor({ folder, stores, checkEveryMs = CHECK_EVERY_MS }: ClaudeStatusFilesOptions) {
    this.#folder = folder;
    this.#stores = stores;
    if (checkEveryMs === false) return;
    this.#timer = setInterval(() => {
      if (this.#due()) void this.refresh();
    }, checkEveryMs);
    this.#timer.unref();
    void this.refresh();
  }

  /** Whether something may have changed since the last pass, which has had its rest. */
  #due(): boolean {
    const now = Date.now();
    if (this.#queued || now < this.#notBefore) return false;
    try {
      return now >= this.#drawAnywayAt || this.#stores.historyVersion() !== this.#version;
    } catch {
      // A database that cannot be read now is read by the pass, which reports why.
      return true;
    }
  }

  refresh(): Promise<void> {
    this.#queued = true;
    this.#done = this.#done.then(async () => {
      this.#queued = false;
      if (this.#stopped) return;
      const started = Date.now();
      const everySession = started >= this.#drawAnywayAt;
      try {
        const version = this.#stores.historyVersion();
        await this.#drawAll(started, everySession);
        this.#version = version;
        this.#reported = undefined;
      } catch (error) {
        this.#report(error);
      }
      const finished = Date.now();
      this.#notBefore = finished + PACE * (finished - started);
      if (everySession) this.#drawAnywayAt = finished + DRAW_ANYWAY_MS;
    });
    return this.#done;
  }

  async stop(): Promise<void> {
    this.#stopped = true;
    clearInterval(this.#timer);
    await this.#done;
    try {
      for (const sessionId of drawnSessionIds(this.#folder)) {
        const state = this.#written.get(sessionId)?.state;
        await this.#write(sessionId, statusFileText(stoppedDaemonPresentation(state)), state);
      }
    } catch (error) {
      this.#report(error);
    }
  }

  /**
   * One pass. A session gets a file while it is open, or holds one from an earlier daemon, and had an event within
   * `UNREAD_AFTER_MS`; one that ended while this daemon ran, or is no longer recorded, gets none. With `everySession`
   * each of them is drawn; otherwise only the active ones (`QUIET_AFTER_MS`) and those this daemon has not drawn yet.
   */
  async #drawAll(now: number, everySession: boolean): Promise<void> {
    this.#sweepCutShortWrites();
    const sessions = removeUnrecordedStatusFiles(this.#folder, this.#stores.reader, now - UNREAD_AFTER_MS);
    const drawn = new Set(drawnSessionIds(this.#folder));
    // A file removed behind the daemon's back is written again.
    for (const sessionId of this.#written.keys()) if (!drawn.has(sessionId)) this.#written.delete(sessionId);
    for (const session of sessions) {
      const { sessionId, endedAt, lastActiveAt } = session;
      if (!STATUS_FILE_NAME.test(sessionId)) continue;
      const endedHere = endedAt !== null && this.#endedAt.has(sessionId) && this.#endedAt.get(sessionId) !== endedAt;
      this.#endedAt.set(sessionId, endedAt);
      if (endedHere || now - lastActiveAt > UNREAD_AFTER_MS) {
        rmSync(join(this.#folder, sessionId), { force: true });
        this.#written.delete(sessionId);
        continue;
      }
      // An ended session whose file is gone is read by nothing, unless its next event opens it again.
      if (endedAt !== null && !drawn.has(sessionId)) continue;
      if (everySession || !this.#written.has(sessionId) || now - lastActiveAt <= QUIET_AFTER_MS) await this.#draw(session);
    }
    // What no pass reads any more: sessions past `UNREAD_AFTER_MS` whose file is gone.
    const read = new Set(sessions.map(session => session.sessionId));
    for (const sessionId of this.#endedAt.keys()) if (!read.has(sessionId)) this.#endedAt.delete(sessionId);
  }

  /** The session's line as the daemon answers for it now, written when it differs from its file. */
  async #draw({ sessionId, cwd }: { sessionId: string; cwd: string }): Promise<void> {
    let text: string;
    let state: RepositoryStartState | undefined;
    try {
      const answer = await invokeOperation(PRESENTATION_GET, this.#stores, { repository: cwd, host: HOST, sessionId }, { kind: "person" });
      const response = presentationReadSchema.parse(answer);
      state = response.repositoryState;
      const origin = new URL(this.#stores.dashboardOrigin());
      // A reader takes no answer from a daemon bound off loopback (`verifyDaemon`), and draws as if none ran.
      const footer = FOOTER_DAEMON_HOSTS.includes(origin.hostname)
        ? await answeredSessionFooter(origin, response, { sessionId, host: HOST, cwd })
        : stoppedDaemonPresentation(state);
      text = statusFileText(footer);
    } catch {
      // As the status line that asked over HTTP drew a read that failed (`claude-status.ts`).
      text = statusFileText(fallbackPresentation("unavailable"));
    }
    await this.#write(sessionId, text, state);
  }

  async #write(sessionId: string, text: string, state: RepositoryStartState | undefined): Promise<void> {
    const path = join(this.#folder, sessionId);
    const held = this.#written.get(sessionId)?.text ?? readText(path);
    if (held !== text) {
      this.#makeFolder();
      try {
        await replaceFile(path, text, { mode: 0o600 });
      } catch (error) {
        // The folder may have gone with the home; the next pass makes it again.
        this.#folderMade = false;
        throw error;
      }
    }
    this.#written.set(sessionId, { text, state });
  }

  /** In a home that exists only: a daemon whose home was moved away makes no new one at its path. */
  #makeFolder(): void {
    if (this.#folderMade) return;
    let made = true;
    try {
      mkdirSync(this.#folder, { mode: 0o700 });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      made = false;
    }
    keepFolderPrivate(this.#folder, { made });
    this.#folderMade = true;
  }

  /** Once per daemon: the `<file>.<pid>.<uuid>.tmp` a write cut short left, which no session id can name. */
  #sweepCutShortWrites(): void {
    if (this.#swept) return;
    this.#swept = true;
    let names: string[];
    try {
      names = readdirSync(this.#folder);
    } catch {
      return;
    }
    for (const name of names) if (name.endsWith(".tmp")) rmSync(join(this.#folder, name), { force: true });
  }

  /** To the daemon's log, once for each different failure in a row. */
  #report(error: unknown): void {
    const message = error instanceof Error ? error.message : String(error);
    if (message === this.#reported) return;
    this.#reported = message;
    process.stderr.write(`rerail daemon: could not keep the Claude Code status lines in ${this.#folder}: ${message}\n`);
  }
}

function readText(path: string): string | undefined {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return undefined;
  }
}

/**
 * Keeps the status file of every Claude Code session current while the daemon runs: written when what the session's
 * line shows changes, removed when the session ends, and left saying `not running` by a clean stop.
 */
export function keepClaudeStatusFiles(options: ClaudeStatusFilesOptions): ClaudeStatusFiles {
  return new StatusFileKeeper(options);
}
