import { spawnSync } from "node:child_process";
import { join, resolve } from "node:path";

import type { RepositoryIdentityLocation } from "../../src/engine/catalog/repository-identity.js";
import { gitInherits } from "../../src/engine/git.js";

/** `given` as Rerail runs Git in it: without the variables through which a parent names a repository of its own. */
export function asRerailRunsGit(given: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  return Object.fromEntries(Object.entries(given).filter(([name]) => gitInherits(name)));
}

/**
 * Git's own answer for `folder`, in `env` (this process's environment by default) as Rerail runs Git in it, without the
 * variables through which a parent names a repository of its own ({@link gitInherits}), read from `git rev-parse`.
 * Where Git finds no work tree but places the folder in a bare repository, the repository is its common folder, whose
 * identity every linked checkout of it reads.
 */
export function gitsAnswer(folder: string, given: NodeJS.ProcessEnv = process.env): RepositoryIdentityLocation {
  const requested = resolve(folder);
  const env = asRerailRunsGit(given);
  const result = spawnSync("git", ["-C", requested, "rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"], { encoding: "utf8", env });
  if (result.status !== 0) {
    const bare = spawnSync("git", ["-C", requested, "rev-parse", "--is-bare-repository"], { encoding: "utf8", env });
    const common = spawnSync("git", ["-C", requested, "rev-parse", "--path-format=absolute", "--git-common-dir"], { encoding: "utf8", env });
    if (bare.status === 0 && bare.stdout.trim() === "true" && common.status === 0) {
      const root = resolve(common.stdout.replace(/\n$/u, ""));
      return { git: true, workspaceRoot: root, identityPath: join(root, "rerail-repository-id"), main: true };
    }
    return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id"), main: true };
  }
  const [top, common, gitDirectory] = result.stdout.trim().split("\n").map(line => resolve(line.trim()));
  return { git: true, workspaceRoot: top!, identityPath: join(common!, "rerail-repository-id"), main: common === gitDirectory };
}
