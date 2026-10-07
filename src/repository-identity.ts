import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

export const GIT_EXECUTABLE = process.platform === "win32" ? String.raw`C:\Program Files\Git\cmd\git.exe` : "/usr/bin/git";

/**
 * Where `workspace`'s stable repository id lives: `rerail-repository-id` in Git's common
 * metadata directory, shared by every linked worktree, or `.rerail/repository-id` outside Git.
 */
export function repositoryIdentityLocation(workspace: string): { git: boolean; workspaceRoot: string; identityPath: string } {
  const requested = resolve(workspace);
  const rootResult = spawnSync(GIT_EXECUTABLE, ["-C", requested, "rev-parse", "--show-toplevel"], { encoding: "utf8" });
  if (rootResult.status !== 0) {
    return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id") };
  }
  const workspaceRoot = resolve(rootResult.stdout.trim());
  const common = spawnSync(GIT_EXECUTABLE, ["-C", workspaceRoot, "rev-parse", "--path-format=absolute", "--git-common-dir"], { encoding: "utf8" });
  if (common.status !== 0) throw new Error(`could not resolve Git common directory for ${workspaceRoot}`);
  return { git: true, workspaceRoot, identityPath: join(resolve(common.stdout.trim()), "rerail-repository-id") };
}

/** The repository id `workspace` already has, without creating one; passive readers use it. */
export function readRepositoryId(workspace: string): string | undefined {
  const { identityPath } = repositoryIdentityLocation(workspace);
  try {
    const repositoryId = readFileSync(identityPath, "utf8").trim();
    return repositoryId.length === 0 ? undefined : repositoryId;
  } catch (error) {
    if (typeof error === "object" && error !== null && "code" in error && error.code === "ENOENT") return undefined;
    throw error;
  }
}
