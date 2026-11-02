import { randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { type FileHandle, lstat, mkdir, open, rename, rm } from "node:fs/promises";
import { resolve, sep } from "node:path";

/**
 * Session records default to `<checkout>/.rerail/sessions`, a path the repository controls: a committed `.rerail` or
 * `sessions` symlink would send prompts, pending steers and preimages to a folder the repository chose, and a planted
 * file would be read back as Rerail's own state. Below the innermost `.rerail` folder on a path, every folder must be
 * a real one and every file is opened without following a symlink. A root outside any `.rerail` folder
 * (`RERAIL_STORAGE_ROOT`) is the person's own choice and is used as it is.
 */
const LEDGER_FOLDER = ".rerail";

/** A session record path Rerail refuses because a symlink, or something that is not a folder, sits on it. */
class LedgerPathRefusedError extends Error {
  readonly path: string;

  constructor(path: string, what: string) {
    super(`Rerail does not keep session records through ${path}: it is ${what}, which could send them outside the checkout. `
      + "Remove it, and Rerail makes its own.");
    this.name = "LedgerPathRefusedError";
    this.path = path;
  }
}

/** Logs the refusal, which a hook's caller may only see as a skipped hook, and throws it. */
function refuse(path: string, what: string): never {
  const error = new LedgerPathRefusedError(path, what);
  process.stderr.write(`rerail: ${error.message}\n`);
  throw error;
}

function errno(error: unknown): string | undefined {
  return error instanceof Error && "code" in error && typeof error.code === "string" ? error.code : undefined;
}

/**
 * Checks the folders from the innermost `.rerail` on `directory` down to it, refusing a symlink or a non-folder among
 * them. With `create`, makes each missing one (`0700`), and the folders above `.rerail` as `mkdir -p` would; without
 * it, stops at the first missing one.
 */
async function ledgerFolder(directory: string, create: boolean): Promise<void> {
  const absolute = resolve(directory);
  const segments = absolute.split(sep);
  const start = segments.lastIndexOf(LEDGER_FOLDER);
  if (start <= 0) {
    if (create) await mkdir(absolute, { recursive: true, mode: 0o700 });
    return;
  }
  for (let end = start + 1; end <= segments.length; end += 1) {
    const folder = segments.slice(0, end).join(sep);
    let stats = await lstat(folder).catch((error: unknown) => {
      if (errno(error) === "ENOENT") return undefined;
      throw error;
    });
    if (stats === undefined) {
      if (!create) return;
      if (end === start + 1) await mkdir(segments.slice(0, start).join(sep) || sep, { recursive: true, mode: 0o700 });
      await mkdir(folder, { mode: 0o700 }).catch((error: unknown) => {
        if (errno(error) !== "EEXIST") throw error;
      });
      stats = await lstat(folder);
    }
    if (stats.isSymbolicLink()) refuse(folder, "a symbolic link");
    if (!stats.isDirectory()) refuse(folder, "not a folder");
  }
}

/** Creates the session record folder `directory`, refusing a symlink below `.rerail` (see {@link LEDGER_FOLDER}). */
export async function makeLedgerFolder(directory: string): Promise<void> {
  await ledgerFolder(directory, true);
}

/**
 * Opens a session record file below a checked folder without following a symlink at its name; a symlink there is
 * refused. `flags` are `fs.constants` open flags; a file it creates gets `0600`.
 */
export async function openLedgerFile(path: string, flags: number): Promise<FileHandle> {
  try {
    return await open(path, flags | constants.O_NOFOLLOW, 0o600);
  } catch (error) {
    if (errno(error) === "ELOOP") refuse(path, "a symbolic link");
    throw error;
  }
}

/** A session record file's text, or undefined when it or its folder is missing. Refuses what `makeLedgerFolder` does. */
export async function readLedgerFile(path: string, directory: string): Promise<string | undefined> {
  await ledgerFolder(directory, false);
  let handle: FileHandle;
  try {
    handle = await openLedgerFile(path, constants.O_RDONLY);
  } catch (error) {
    if (errno(error) === "ENOENT") return undefined;
    throw error;
  }
  try {
    return await handle.readFile("utf8");
  } finally {
    await handle.close();
  }
}

/**
 * Replaces a session record file whole: written to a new file beside it, then renamed over it, so a symlink at its
 * name is replaced, never followed, and a reader never sees half of it. Creates its folder as `makeLedgerFolder` does.
 */
export async function writeLedgerFile(path: string, directory: string, text: string): Promise<void> {
  await makeLedgerFolder(directory);
  const temporary = `${path}.${process.pid}.${randomUUID()}.tmp`;
  try {
    const handle = await openLedgerFile(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL);
    try {
      await handle.writeFile(text, "utf8");
    } finally {
      await handle.close();
    }
    await rename(temporary, path);
  } catch (error) {
    await rm(temporary, { force: true });
    throw error;
  }
}
