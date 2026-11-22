/**
 * A file that exists and cannot be opened, as a person's own settings or another program can make one, for the tests of
 * what Rerail does with a file it may stat and not read.
 */
import { execFileSync } from "node:child_process";
import { chmodSync, closeSync, openSync } from "node:fs";

/** Everyone, by its SID, so that the entry reads the same in every Windows language. */
const EVERYONE = "*S-1-1-0";

/** A file made one that cannot be opened: whether an open of it to read now fails for this process, and how to undo it. */
export interface Unopenable {
  readonly held: boolean;
  readonly giveBack: () => void;
}

/**
 * Makes the file at `path` one that cannot be opened to read until `giveBack` runs, while its stat still answers: mode
 * 000 on Linux and macOS, which root opens all the same; on Windows an entry that denies everyone the reading of its
 * data, so every open of it to read fails, a Git's and this process's own. `held` says whether an open of it now fails.
 */
export function unopenable(path: string): Unopenable {
  if (process.platform !== "win32") {
    chmodSync(path, 0o000);
    return { held: openFails(path), giveBack: () => chmodSync(path, 0o644) };
  }
  execFileSync("icacls", [path, "/deny", `${EVERYONE}:(RD)`], { stdio: "ignore" });
  return { held: openFails(path), giveBack: () => execFileSync("icacls", [path, "/remove:d", EVERYONE], { stdio: "ignore" }) };
}

function openFails(path: string): boolean {
  try {
    closeSync(openSync(path, "r"));
    return false;
  } catch {
    return true;
  }
}
