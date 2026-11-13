import { spawnSync } from "node:child_process";

/**
 * `path` spelt with Windows' 8.3 short names (`C:\Users\RUNNER~1\...`), as cmd's `%~sI` gives it, or undefined when no
 * name on it has a short form: off Windows, or on a volume with short names turned off. `path` must exist and hold no
 * character cmd treats as special (`"`, `%`, `&`, `^`, `|`, `<`, `>`).
 */
export function shortSpelling(path: string): string | undefined {
  if (process.platform !== "win32") return undefined;
  if (/["%&^|<>]/u.test(path)) throw new Error(`cmd cannot spell ${path}`);
  const spelt = spawnSync(process.env.ComSpec ?? "cmd.exe", ["/d", "/c", `for %I in ("${path}") do @echo %~sI`], {
    encoding: "utf8",
    windowsHide: true,
    windowsVerbatimArguments: true,
  });
  if (spelt.status !== 0) throw new Error(`cmd could not spell ${path}: ${spelt.stderr}`);
  const short = spelt.stdout.trim();
  return short.toLowerCase() === path.toLowerCase() ? undefined : short;
}
