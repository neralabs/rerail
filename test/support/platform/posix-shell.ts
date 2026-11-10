import { execFileSync } from "node:child_process";
import { join } from "node:path";

/**
 * The POSIX sh a test runs its shell scripts with: `/bin/sh`, or on Windows, which has none, the `usr\bin\sh.exe` Git
 * for Windows ships three folders above its `git --exec-path` (`<Git>\mingw64\libexec\git-core`). Every machine that
 * runs Rerail's tests has Git.
 */
export function posixShell(): string {
  if (process.platform !== "win32") return "/bin/sh";
  const execPath = execFileSync("git", ["--exec-path"], { encoding: "utf8", windowsHide: true }).trim();
  return join(execPath, "..", "..", "..", "usr", "bin", "sh.exe");
}
