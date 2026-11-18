import { watch as watchFolder, type FSWatcher } from "node:fs";
import { basename, dirname } from "node:path";

/**
 * What a listener of the {@link ChangeFeed} hears. `data`: the database may hold a commit it did not (any connection's,
 * in this process or another), or the API's own stores wrote. `conflicts`: Merge Assist's conflicts may have changed.
 * `beat`: every {@link BEAT_MS} while anyone listens, so a listener can say it is alive and look again at what no
 * notice covers (installed packs, a checkout's own rules, a notice the OS dropped).
 */
export type ChangeNotice = "data" | "conflicts" | "beat";

/**
 * When what the dashboard reads may have changed, for the daemon's change stream (`GET /api/changes`). A notice says
 * only that something may have moved: each listener reads its own answer and sends what differs.
 */
export interface ChangeFeed {
  /** Hear each notice, several in one turn of the event loop as one; returns the call that stops it. */
  listen(listener: (notice: ChangeNotice) => void): () => void;
  /** A change this process knows of without a file notice: a write through the API's stores, a conflict event. */
  notify(notice: "data" | "conflicts"): void;
  /** Stop watching and beating; nobody hears anything after. */
  close(): void;
}

/**
 * The beat, the poll's old 5 s. Two missed beats and the page stops saying Live (`LIVE_MS` in `ui/frame/topbar.js`): a
 * daemon whose event loop is stuck closes no connection, so only a beat that does not come can tell it.
 */
export const BEAT_MS = 5_000;

export interface ChangeFeedOptions {
  /** `node:fs`'s `watch` by default; tests pass one that reports what they say. */
  readonly watch?: typeof watchFolder;
  readonly beatMs?: number;
}

/**
 * The feed over the database at `databasePath`. Every commit, from any connection, writes the database's `-wal` file
 * (the database runs in WAL mode with `synchronous = FULL`, so the write reaches the disk at the commit), and a watch
 * of the folder hears it; a notice for the main file covers a database not in WAL mode yet, and a name the OS does not
 * report is taken as either. The folder is watched, not the file, so a database created, removed or replaced keeps
 * its watch. A watch that could not start or failed is tried again at the next beat; until then the beat alone looks.
 */
export function databaseChangeFeed(databasePath: string, options: ChangeFeedOptions = {}): ChangeFeed {
  const names = new Set([basename(databasePath), `${basename(databasePath)}-wal`]);
  const watch = options.watch ?? watchFolder;
  const listeners = new Set<(notice: ChangeNotice) => void>();
  const pending = new Set<ChangeNotice>();
  let watcher: FSWatcher | undefined;
  let beat: NodeJS.Timeout | undefined;
  let flushing = false;

  const flush = (): void => {
    flushing = false;
    const notices = [...pending];
    pending.clear();
    for (const notice of notices) for (const listener of [...listeners]) listener(notice);
  };
  const hear = (notice: ChangeNotice): void => {
    if (listeners.size === 0) return;
    pending.add(notice);
    if (!flushing) {
      flushing = true;
      setImmediate(flush);
    }
  };
  const unwatch = (): void => {
    watcher?.close();
    watcher = undefined;
  };
  const startWatch = (): void => {
    if (watcher !== undefined) return;
    try {
      watcher = watch(dirname(databasePath), { persistent: false }, (_event, name) => {
        if (name === null || names.has(String(name))) hear("data");
      });
      watcher.on("error", unwatch);
    } catch {
      // No folder yet, or no watch left (Linux's inotify budget): the beat looks, and the next beat tries again.
      watcher = undefined;
    }
  };
  const start = (): void => {
    startWatch();
    beat = setInterval(() => {
      startWatch();
      hear("beat");
    }, options.beatMs ?? BEAT_MS);
    beat.unref();
  };
  const stop = (): void => {
    clearInterval(beat);
    beat = undefined;
    unwatch();
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
