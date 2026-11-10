import { symlinkSync } from "node:fs";
import { dirname, resolve } from "node:path";
import type { TestContext } from "vitest";

/** Why a test that needs a symbolic link does not run on a Windows machine that refuses to make one. */
export const NO_SYMLINKS = "Windows makes symbolic links only in Developer Mode or for an elevated process";

/**
 * Makes `path` a symbolic link to `target`, or skips the running test, naming why, where Windows refuses to make one
 * (`EPERM`). For a link to a folder, `linkFolder` makes a junction instead, which every Windows machine allows.
 */
export function symlinkOrSkip(context: Pick<TestContext, "skip">, target: string, path: string, type?: "file" | "dir"): void {
  try {
    symlinkSync(target, path, type);
  } catch (error) {
    if (process.platform === "win32" && error instanceof Error && "code" in error && error.code === "EPERM") context.skip(NO_SYMLINKS);
    throw error;
  }
}

/**
 * Links `path` to the folder `target`: a symbolic link, or on Windows a junction, which needs no privilege and which
 * Node's `lstat`, `readlink` and `realpath` and Rust's `symlink_metadata` read as a link. A junction holds an absolute
 * path, so a relative `target` is resolved against the link's own folder, as a symbolic link's would be.
 */
export function linkFolder(target: string, path: string): void {
  symlinkSync(process.platform === "win32" ? resolve(dirname(path), target) : target, path, "junction");
}
