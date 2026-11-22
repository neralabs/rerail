/**
 * A file that exists and cannot be opened, as a person's own settings or another program can make one, for the tests of
 * what Rerail does with a file it may stat and not read.
 */
import { execFileSync } from "node:child_process";
import { chmodSync } from "node:fs";

/** Everyone, by its SID, so that the entry reads the same in every Windows language. */
const EVERYONE = "*S-1-1-0";

/**
 * Makes the file at `path` one that cannot be opened to read until what it returns runs, while its stat still answers:
 * mode 000 on Linux and macOS (root opens it all the same); on Windows an entry that denies everyone the reading of
 * its data, so every open of it to read fails, a Git's and this process's own.
 */
export function unopenable(path: string): () => void {
  if (process.platform !== "win32") {
    chmodSync(path, 0o000);
    return () => chmodSync(path, 0o644);
  }
  execFileSync("icacls", [path, "/deny", `${EVERYONE}:(RD)`], { stdio: "ignore" });
  return () => execFileSync("icacls", [path, "/remove:d", EVERYONE], { stdio: "ignore" });
}
