import { realpathSync } from "node:fs";
import { join } from "node:path";

function resolved(path: string): string | undefined {
  try {
    return realpathSync.native(path);
  } catch {
    return undefined;
  }
}

/**
 * The node set-up records for the hooks, the status line and the launcher: the one running, by a path its next
 * upgrade keeps. Homebrew runs node from its versioned keg (`<prefix>/Cellar/node/26.5.0/bin/node`), which
 * `brew upgrade` and its cleanup delete, so there the link Homebrew keeps to the same binary is recorded: `<prefix>/bin/node`,
 * or `<prefix>/opt/<formula>/bin/node` for a formula it does not link there, such as `node@24`. Both follow the formula
 * to its next version. Any other path is recorded as it is: a version manager (nvm, fnm, Volta) keeps each version in
 * a folder of its own, and a link it keeps follows whichever version a shell last chose, so once that folder goes the
 * recorded-first fallback runs `node` from PATH.
 */
export function pinnedNodePath(execPath: string = process.execPath): string {
  const keg = /^(.+)\/Cellar\/([^/]+)\/[^/]+\/bin\/node$/u.exec(execPath);
  const binary = keg === null ? undefined : resolved(execPath);
  if (keg === null || binary === undefined) return execPath;
  const links = [join(keg[1]!, "bin", "node"), join(keg[1]!, "opt", keg[2]!, "bin", "node")];
  return links.find(link => resolved(link) === binary) ?? execPath;
}
