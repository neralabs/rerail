/**
 * The start checks one-shot hooks gave up on, as the daemon's queue holds them: at most one for each folder waits there.
 *
 * A hook that repeats the daemon's kept answer of stopped (`repeatsStoppedAnswer`) ends its request, but the request
 * stays queued at a busy daemon, which serves it later though nobody waits for it. Beside a daemon that admits one
 * connection per turn of its loop (Linux), a burst of such hooks queued ahead of every other hook of every checkout.
 * So the hook that gives up marks its check as waiting, and the hooks after it in that folder that find the mark repeat
 * the kept answer without asking, where it holds; the daemon removes the mark as it serves a start check for the folder.
 *
 * The mark is a fact about the daemon's queue, never a clock. A hook makes `<name>.asking` (exclusively) before it sends,
 * which claims nothing and which no hook honours, and only once its request was handed to the OS (`sent`) renames it
 * to `<name>.waiting` as it ends its request (`abandoned`). The daemon removes `.asking` before `.waiting` as it serves
 * a start check for the folder (`served`). So where the daemon served one since the hook made `.asking`, the rename
 * finds nothing and no `.waiting` is made; where the rename came first, the daemon removes `.waiting`. No `.waiting`
 * outlives a served start check of its folder but where the system refuses its removal, which the daemon tries again at
 * every start check it serves after.
 *
 * The marks live in `asks` in the daemon's own folder of kept answers, which only a daemon that removes them makes, as it
 * starts: a new daemon has a new folder, and hooks read only the folder of the daemon `daemon.json` names, once it is
 * known to be alive. `<name>` is the folder's kept answer's name: the digest of the repository string the start check
 * sends, which is the daemon's name for it too. A link in place of a folder on the way is never followed.
 */
import { lstatSync, mkdirSync, rmSync, writeFileSync, type Stats } from "node:fs";
import { dirname, join } from "node:path";
import { renameWithRetrySync } from "../../engine/file-replace.js";
import { answerName, daemonAnswersFolder, hasKeptAnswer, type DaemonIdentity } from "./stopped-answers.js";

/** The folder of marks in a daemon's folder of kept answers. */
const ASKS_FOLDER = "asks";
/** A hook's start check that may be sent: made before it is, honoured by no hook. */
const ASKING = ".asking";
/** A start check for the folder that nobody waits for is queued at the daemon. */
const WAITING = ".waiting";

/** What a start check (`readStartAnswer`) tells the one hook's ask it carries. */
export interface StartAsk {
  /**
   * Once the live `daemon` is known and before it is asked about `workspace` (the repository string the check sends):
   * false where it is not to be asked at all.
   */
  beforeAsking(daemon: DaemonIdentity, workspace: string): boolean;
  /** The request was handed to the OS on a connection to the daemon. */
  sent(): void;
}

/** One hook's start check in one folder, as the daemon's queue of start checks holds it. */
export class HookAsk implements StartAsk {
  readonly #repeats: () => boolean;
  #asking: string | undefined;
  #sent = false;
  #unasked = false;

  /** `repeats` says whether the hook may repeat the daemon's kept answer of stopped in the folder now. */
  constructor(repeats: () => boolean) {
    this.#repeats = repeats;
  }

  /** Whether the hook did not ask, since a check it may repeat the answer of waits at the daemon already. */
  get unasked(): boolean {
    return this.#unasked;
  }

  /**
   * Where the daemon keeps marks and a kept answer for `workspace` is there: false, asking nothing, where a check of
   * the folder waits at the daemon and `repeats` holds; else true, with this hook's `.asking` made where none is there.
   */
  beforeAsking(daemon: DaemonIdentity, workspace: string): boolean {
    const folder = asksFolderOf(daemon);
    if (folder === undefined || !hasKeptAnswer(daemon, workspace)) return true;
    const name = join(folder, answerName(workspace));
    if (plainFile(name + WAITING) && this.#repeats()) {
      this.#unasked = true;
      return false;
    }
    this.#asking = madeAlone(name + ASKING);
    return true;
  }

  sent(): void {
    this.#sent = true;
  }

  /**
   * The hook ends its request without the answer: where the request was handed to the OS, its `.asking` becomes the
   * folder's `.waiting`, unless the daemon has served a start check there since it was made, which removed it. A request
   * never sent leaves nothing.
   */
  abandoned(): void {
    const asking = this.#asking;
    this.#asking = undefined;
    if (asking !== undefined && !(this.#sent && renamed(asking, asking.slice(0, -ASKING.length) + WAITING))) removed(asking);
  }

  /** The hook has its answer, or waits for it no more: its `.asking` goes, where the daemon has not removed it. */
  done(): void {
    if (this.#asking !== undefined) removed(this.#asking);
    this.#asking = undefined;
  }
}

/**
 * The daemon's side: its folder of marks, made as it starts, and their removal as it serves each start check. Before
 * `open`, and where the folder could not be made, it removes nothing and hooks make no marks.
 */
export class ServedAsks {
  #folder: string | undefined;
  /** The names whose marks a removal was refused for, tried again at each start check served. */
  readonly #refused = new Set<string>();

  /**
   * Makes `asks` in `daemon`'s folder of kept answers in the Rerail home `home`, each folder 0700 and only inside the
   * folder of kept answers a daemon made; anything else at a name, a link among them, leaves it unmade.
   */
  open(home: string, daemon: DaemonIdentity): void {
    const own = daemonAnswersFolder(daemon, home);
    const asks = join(own, ASKS_FOLDER);
    this.#folder = isFolder(dirname(own)) && folderMade(own) && folderMade(asks) ? asks : undefined;
  }

  /**
   * As the daemon serves a start check for `workspace` (the repository string it was sent): removes the folder's
   * `.asking`, then its `.waiting`, and tries again each removal refused before.
   */
  served(workspace: string): void {
    const folder = this.#folder;
    if (folder === undefined) return;
    this.#refused.add(answerName(workspace));
    for (const name of [...this.#refused]) {
      if (bothRemoved(join(folder, name))) this.#refused.delete(name);
    }
  }
}

/** Removes `<name>.asking`, then `<name>.waiting`; whether neither is there now. */
function bothRemoved(name: string): boolean {
  const gone = [ASKING, WAITING].map(kind => removed(name + kind));
  return gone.every(Boolean);
}

/** The folder of marks of `daemon`, where it and each folder above it in the Rerail home is a folder, not a link. */
function asksFolderOf(daemon: DaemonIdentity): string | undefined {
  const own = daemonAnswersFolder(daemon);
  const asks = join(own, ASKS_FOLDER);
  return [dirname(own), own, asks].every(isFolder) ? asks : undefined;
}

function isFolder(path: string): boolean {
  return entryAt(path)?.isDirectory() === true;
}

function plainFile(path: string): boolean {
  return entryAt(path)?.isFile() === true;
}

/** What is at `path`, not followed through a link; undefined where nothing is, or the look fails. */
function entryAt(path: string): Stats | undefined {
  try {
    return lstatSync(path, { throwIfNoEntry: false });
  } catch {
    return undefined;
  }
}

/** `path`, made empty where nothing is at its name; undefined where something is, or the system refuses it. */
function madeAlone(path: string): string | undefined {
  try {
    writeFileSync(path, "", { flag: "wx", mode: 0o600 });
    return path;
  } catch {
    return undefined;
  }
}

/**
 * Whether `from` is now at `to`; false where it is gone or the system refuses the rename, at once: a refused rename only
 * leaves no mark, so the hook does not wait out a file Windows holds.
 */
function renamed(from: string, to: string): boolean {
  try {
    renameWithRetrySync(from, to, { retryMs: 0 });
    return true;
  } catch {
    return false;
  }
}

/** Removes the file `path`, a link as a link; whether nothing is at its name now. */
function removed(path: string): boolean {
  try {
    rmSync(path, { force: true });
    return true;
  } catch {
    return false;
  }
}

/** Makes `folder`, 0700, where nothing is at its name; whether a folder, not a link, is there now. */
function folderMade(folder: string): boolean {
  try {
    mkdirSync(folder, { mode: 0o700 });
  } catch {
    // Something is at the name already, or its parent is not there.
  }
  return isFolder(folder);
}
