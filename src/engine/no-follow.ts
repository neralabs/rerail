import { type BigIntStats, constants } from "node:fs";
import { type FileHandle, lstat, open } from "node:fs/promises";

function symlinkRefusal(path: string): Error {
  return Object.assign(new Error(`ELOOP: too many symbolic links encountered, open '${path}'`), { code: "ELOOP", path });
}

async function lstatOrMissing(path: string): Promise<BigIntStats | undefined> {
  try {
    // As bigints: a Windows file id is 64 bits, more than a number holds exactly.
    return await lstat(path, { bigint: true });
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return undefined;
    throw error;
  }
}

/**
 * Opens `path` with `flags` without following a symbolic link at its name, which is refused with `ELOOP`. Where the OS
 * has `O_NOFOLLOW` the kernel refuses it. Windows has none: `constants.O_NOFOLLOW` is undefined there, and `flags |
 * undefined` adds nothing, so a planted link was opened as if it were Rerail's own file. There the name is read with
 * lstat before the open, and the opened file is checked to be the one lstat saw, so a link swapped in between is
 * refused too. `noFollow` is the flag to use, which tests set to undefined to take the Windows path.
 */
export async function openNoFollow(path: string, flags: number, mode?: number, noFollow: number | undefined = constants.O_NOFOLLOW): Promise<FileHandle> {
  if (noFollow !== undefined) return await open(path, flags | noFollow, mode);
  const before = await lstatOrMissing(path);
  if (before?.isSymbolicLink() === true) throw symlinkRefusal(path);
  const handle = await open(path, flags, mode);
  try {
    const opened = await handle.stat({ bigint: true });
    const now = before ?? await lstatOrMissing(path);
    if (now === undefined || now.isSymbolicLink() || opened.dev !== now.dev || opened.ino !== now.ino) throw symlinkRefusal(path);
    return handle;
  } catch (error) {
    await handle.close();
    throw error;
  }
}
