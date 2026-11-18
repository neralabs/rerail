/**
 * Where `gh` keeps its sign-in, and a stamp of it read with one stat and no process. `gh auth login`, `logout`,
 * `switch` and `refresh` each end by rewriting `hosts.yml` in `gh`'s configuration folder, in place, so its size or
 * modification time moves; a token in `GH_TOKEN` or `GITHUB_TOKEN` signs `gh` in ahead of that file. The folder is
 * found as `gh`'s own configuration package finds it (go-gh `config.ConfigDir`).
 */
import { createHash } from "node:crypto";
import { statSync } from "node:fs";
import { homedir } from "node:os";
import { posix, win32 } from "node:path";

/** Token variables `gh` signs in with ahead of `hosts.yml` (the Enterprise ones are kept from `gh`, see github.ts). */
const TOKEN_VARIABLES = ["GH_TOKEN", "GITHUB_TOKEN"] as const;

/** A variable's value, unset when empty; on Windows its name in any case, as Windows and `gh` read it. */
function envValue(env: NodeJS.ProcessEnv, name: string, windows: boolean): string | undefined {
  const key = windows ? Object.keys(env).find(candidate => candidate.toUpperCase() === name) : name;
  const value = key === undefined ? undefined : env[key];
  return value === "" ? undefined : value;
}

/** The variables `gh` takes its configuration folder from, in its order, each with the folder under it. */
function folderVariables(windows: boolean): readonly (readonly [string, readonly string[]])[] {
  const home = windows ? "USERPROFILE" : "HOME";
  const own = [["GH_CONFIG_DIR", []], ["XDG_CONFIG_HOME", ["gh"]]] as const;
  return windows ? [...own, ["APPDATA", ["GitHub CLI"]], [home, [".config", "gh"]]] : [...own, [home, [".config", "gh"]]];
}

/**
 * `gh`'s configuration folder: `GH_CONFIG_DIR`, else `XDG_CONFIG_HOME/gh`, else `%AppData%\GitHub CLI` on Windows,
 * else `.config/gh` in the home folder (on macOS too).
 */
export function ghConfigDir(env: NodeJS.ProcessEnv, platform: NodeJS.Platform = process.platform): string {
  const windows = platform === "win32";
  const path = windows ? win32 : posix;
  for (const [variable, under] of folderVariables(windows)) {
    const base = envValue(env, variable, windows);
    if (base !== undefined) return path.join(base, ...under);
  }
  return path.join(homedir(), ".config", "gh");
}

/** The file's identity, size and times, or why it has none: absent, or unreadable (a folder it may not enter). */
function fileStamp(file: string): string {
  try {
    const stat = statSync(file, { bigint: true, throwIfNoEntry: false });
    return stat === undefined ? "absent" : `${stat.ino}:${stat.size}:${stat.mtimeNs}:${stat.ctimeNs}`;
  } catch (error) {
    // Unreadable is a state of its own, so the stamp moves again once it can be read.
    return `unreadable:${(error as NodeJS.ErrnoException).code ?? "error"}`;
  }
}

/**
 * A stamp of `gh`'s sign-in that moves whenever `gh` changes it: `hosts.yml`'s path, identity, size and times, and
 * which token variables are set. Hashed, so what keeps it holds no path.
 */
export function ghSignInStamp(env: NodeJS.ProcessEnv, platform: NodeJS.Platform = process.platform): string {
  const windows = platform === "win32";
  const file = (windows ? win32 : posix).join(ghConfigDir(env, platform), "hosts.yml");
  const tokens = TOKEN_VARIABLES.filter(name => envValue(env, name, windows) !== undefined);
  return createHash("sha256").update([file, fileStamp(file), ...tokens].join("\0")).digest("hex").slice(0, 32);
}
