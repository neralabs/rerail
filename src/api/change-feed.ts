import { statSync, watch as watchFolder, type FSWatcher } from "node:fs";
import { basename, dirname } from "node:path";

/**
 * What a listener of the {@link ChangeFeed} hears. `data`: the database holds a commit it did not (any connection's,
 * in this process or another), or the API's own stores wrote. `files`: a folder of {@link ChangeFeedOptions.files}
 * changed (the installed rule packs). `conflicts`: Merge Assist's conflicts may have changed. `beat`: every
 * {@link BEAT_MS} while anyone listens, so a listener can say it is alive and look again at what no notice covers (a
 * checkout's own rules and branches, another process's commit on Windows, a notice the OS dropped).
 */
export type ChangeNotice = "data" | "files" | "conflicts" | "beat";

/**
 * When what the dashboard reads may have changed, for the daemon's change stream (`GET /api/changes`). A notice says
 * only that something may have moved: each listener reads its own answer and sends what differs.
 */
export interface ChangeFeed {
  /** Hear each notice, several in one turn of the event loop as one; returns the call that stops it. */
  listen(listener: (notice: ChangeNotice) => void): () => void;
  /**
   * A change this process knows of without a file notice: a write through the API's stores, a commit of this process's
   * record writer (`onRecordsWritten`), a conflict event.
   */
  notify(notice: "data" | "conflicts"): void;
  /** Stop watching and beating; nobody hears anything after. */
  close(): void;
}

/**
 * The beat, the poll's old 5 s. Two missed beats and the page stops saying Live (`LIVE_MS` in `ui/poll.js`): a
 * daemon whose event loop is stuck closes no connection, so only a beat that does not come can tell it.
 */
export const BEAT_MS = 5_000;

/**
 * The looks again after a file notice that found no new commit, in milliseconds after it. Another process's commit is
 * heard when it writes the `-wal` file, before readers can see it: the writer then publishes the commit in the
 * shared-memory index, which no watch hears, and writes nothing more. Rerail's own opener commits with `synchronous =
 * NORMAL`, so the two follow each other at once; a commit that waits for the disk (`commitDurably`, a writer at `FULL`)
 * syncs the file between them. Nothing else tells when that sync ends, so the version is read again on these steps
 * until it moves; they cover the syncs measured on Linux and Windows 11 with room to spare, and past the last one the
 * next beat reads it, as the poll did.
 */
export const LOOK_AGAIN_MS = [2, 5, 10, 20, 40, 80, 160, 320, 640, 1_280] as const;

export interface ChangeFeedOptions {
  /** `node:fs`'s `watch` by default; tests pass one that reports what they say. */
  readonly watch?: typeof watchFolder;
  readonly beatMs?: number;
  /** Folders of Rerail's own whose every change is a `files` notice: the installed packs' prefix and its packages. */
  readonly files?: readonly string[];
}

/**
 * One folder's watch, which `ensure` starts when it is not running: a folder that does not exist yet, or no watch
 * left (Linux's inotify budget), is tried again at the next `ensure`, and a watch that failed is dropped for it.
 */
function folderWatch(watch: typeof watchFolder, folder: string, onName: (name: string | null) => void): { ensure(): void; close(): void } {
  let watcher: FSWatcher | undefined;
  const close = (): void => {
    watcher?.close();
    watcher = undefined;
  };
  return {
    ensure() {
      if (watcher !== undefined) return;
      try {
        watcher = watch(folder, { persistent: false }, (_event, name) => onName(name === null ? null : String(name)));
        watcher.on("error", close);
      } catch {
        // Not there yet, or no watch left: the beat looks until a later `ensure` starts it.
        watcher = undefined;
      }
    },
    close,
  };
}

/** Whether the version moved since the last look: told once per move. A version that cannot be read counts as moved. */
function versionTracker(version: () => string | undefined): { moved(): boolean; reset(): void } {
  let last: string | undefined;
  let unreadable = 0;
  const read = (): string | undefined => {
    try {
      return version();
    } catch {
      // The database closing: tell the listeners, whose own reads then end what they serve.
      unreadable += 1;
      return `unreadable:${unreadable}`;
    }
  };
  return {
    moved() {
      const now = read();
      if (now === last) return false;
      last = now;
      return true;
    },
    reset() {
      last = read();
    },
  };
}

/**
 * The database's main and `-wal` files by size and time: every write to either moves it. Files it cannot read (their
 * folder's rights changed while the daemon runs, a link loop) give a value never given before, so the looks run as for
 * a new write; it is read in timer callbacks, where a throw would end the daemon.
 */
function filesWritten(databasePath: string): () => string {
  let unreadable = 0;
  const stamp = (path: string): string => {
    const info = statSync(path, { throwIfNoEntry: false });
    return info === undefined ? "" : `${info.mtimeMs}:${info.size}`;
  };
  return () => {
    try {
      return `${stamp(databasePath)}|${stamp(`${databasePath}-wal`)}`;
    } catch {
      unreadable += 1;
      return `unreadable:${unreadable}`;
    }
  };
}

/**
 * Reads the version again on {@link LOOK_AGAIN_MS} after a notice that found it unmoved, and calls `onMoved` once it
 * moves; a new notice starts the steps over, and `stop` ends them.
 */
function lookAgain(moved: () => boolean, onMoved: () => void): { start(): void; stop(): void } {
  let timer: NodeJS.Timeout | undefined;
  const step = (index: number): void => {
    const previous = index === 0 ? 0 : LOOK_AGAIN_MS[index - 1]!;
    timer = setTimeout(() => {
      timer = undefined;
      if (moved()) onMoved();
      else if (index + 1 < LOOK_AGAIN_MS.length) step(index + 1);
    }, LOOK_AGAIN_MS[index]! - previous);
    timer.unref();
  };
  const stop = (): void => {
    clearTimeout(timer);
    timer = undefined;
  };
  return { start: () => { stop(); step(0); }, stop };
}

/**
 * The feed over the database at `databasePath`, whose committed state `version` reads (`PRAGMA data_version`, which
 * moves at every other connection's commit, with the stores' own writes). Every commit, from any connection, writes the
 * database's `-wal` file. On Linux and macOS a watch of the folder hears each such write. On Windows it hears the log's
 * first write after it is opened or restarted, a write synced to disk and a writer closing the file, and nothing of the
 * other commits of a writer at `synchronous = NORMAL`, which leave the log's directory entry as it was until it is
 * flushed or closed: on Windows 11, 4 notices for 20 such commits, and 20 for 20 at `FULL`, while the log's size and
 * time, read by path or through a handle, and `data_version` moved at every commit. So this process's record writes are
 * told by their writer (`onRecordsWritten`, through {@link ChangeFeed.notify}), as the API stores' own writes are, and
 * another process's commit on Windows shows when that process closes the database (a CLI command, at its end), at the
 * next notice or this process's next commit, and else at the next beat, within {@link BEAT_MS}: no fact of it reaches
 * this process sooner, since SQLite tells no other process of a commit and the system reports no change for it. A
 * `data` notice is told once `version` moved, at once for this process's commits, and after the looks again of
 * {@link LOOK_AGAIN_MS} for another process's. A notice for the main file covers a database not in WAL mode yet, and a
 * name the OS does not report is taken as either. The folder is watched, not the file, so a database created, removed
 * or replaced keeps its watch. Each folder of `options.files` is watched too. A watch that could not start or failed is
 * tried again at the next beat; until then the beat alone looks.
 */
export function databaseChangeFeed(databasePath: string, version: () => string | undefined, options: ChangeFeedOptions = {}): ChangeFeed {
  const names = new Set([basename(databasePath), `${basename(databasePath)}-wal`]);
  const watch = options.watch ?? watchFolder;
  const listeners = new Set<(notice: ChangeNotice) => void>();
  const pending = new Set<ChangeNotice>();
  const committed = versionTracker(version);
  const written = filesWritten(databasePath);
  let beat: NodeJS.Timeout | undefined;
  let flushing = false;

  /** The database's files as they were just before the look that last found a commit showing. */
  let told: string | undefined;
  const tell = (notice: ChangeNotice): void => {
    for (const listener of [...listeners]) listener(notice);
  };
  /** Whether a commit shows since the last look, and the files as they were just before it looked. */
  const look = (): { shows: boolean; files: string } => {
    const files = written();
    const shows = committed.moved();
    if (shows) told = files;
    return { shows, files };
  };
  const looks = lookAgain(() => look().shows, () => tell("data"));
  /**
   * A data notice is told once its commit shows; else the looks again wait for it, unless the files are as they were
   * when the last commit showed: the notice then tells of writes already told (this process's own commit, heard after
   * its `notify`; the last write of a commit readers saw first). The files are read before the version, so a write that
   * lands after that look always differs.
   */
  const tellData = (): void => {
    const { shows, files } = look();
    if (shows) {
      looks.stop();
      tell("data");
    } else if (files !== told) looks.start();
  };
  const flush = (): void => {
    flushing = false;
    const notices = [...pending];
    pending.clear();
    for (const notice of notices) (notice === "data" ? tellData : tell)(notice);
  };
  const hear = (notice: ChangeNotice): void => {
    if (listeners.size === 0) return;
    pending.add(notice);
    if (!flushing) {
      flushing = true;
      setImmediate(flush);
    }
  };
  const watches = [
    folderWatch(watch, dirname(databasePath), name => { if (name === null || names.has(name)) hear("data"); }),
    ...(options.files ?? []).map(folder => folderWatch(watch, folder, () => hear("files"))),
  ];
  const startWatches = (): void => {
    for (const folder of watches) folder.ensure();
  };
  const start = (): void => {
    committed.reset();
    startWatches();
    beat = setInterval(() => {
      startWatches();
      hear("beat");
    }, options.beatMs ?? BEAT_MS);
    beat.unref();
  };
  const stop = (): void => {
    clearInterval(beat);
    beat = undefined;
    looks.stop();
    for (const folder of watches) folder.close();
    pending.clear();
  };

  return {
    listen(listener) {
      if (listeners.size === 0) start();
      listeners.add(listener);
      return () => {
        if (listeners.delete(listener) && listeners.size === 0) stop();
      };
    },
    notify: hear,
    close() {
      listeners.clear();
      stop();
    },
  };
}
