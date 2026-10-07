import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

export const GIT_EXECUTABLE = process.platform === "win32" ? String.raw`C:\Program Files\Git\cmd\git.exe` : "/usr/bin/git";

/** Where a checkout keeps its repository's identity, and which checkout it is. */
export interface RepositoryIdentityLocation {
  git: boolean;
  /** The checkout's top folder: the main checkout's, or a linked worktree's own. */
  workspaceRoot: string;
  identityPath: string;
  /** False only for a linked Git worktree; a folder outside Git is its own main checkout. */
  main: boolean;
}

/**
 * Where `workspace`'s stable repository id lives: `rerail-repository-id` in Git's common
 * metadata directory, shared by every linked worktree, or `.rerail/repository-id` outside Git.
 */
export function repositoryIdentityLocation(workspace: string): RepositoryIdentityLocation {
  const requested = resolve(workspace);
  const rootResult = spawnSync(GIT_EXECUTABLE, ["-C", requested, "rev-parse", "--show-toplevel"], { encoding: "utf8" });
  if (rootResult.status !== 0) {
    return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id"), main: true };
  }
  const workspaceRoot = resolve(rootResult.stdout.trim());
  const directories = spawnSync(GIT_EXECUTABLE, ["-C", workspaceRoot, "rev-parse", "--path-format=absolute", "--git-common-dir", "--git-dir"], { encoding: "utf8" });
  if (directories.status !== 0) throw new Error(`could not resolve Git common directory for ${workspaceRoot}`);
  const [commonDirectory, gitDirectory] = directories.stdout.trim().split("\n").map(line => resolve(line.trim()));
  if (commonDirectory === undefined || gitDirectory === undefined) throw new Error(`could not resolve Git common directory for ${workspaceRoot}`);
  return { git: true, workspaceRoot, identityPath: join(commonDirectory, "rerail-repository-id"), main: commonDirectory === gitDirectory };
}

/** The repository id `workspace` already has, without creating one; passive readers use it. */
export function readRepositoryId(workspace: string): string | undefined {
  return readRepositoryIdAt(repositoryIdentityLocation(workspace).identityPath);
}

/** The repository id stored at `identityPath`, or undefined before one is created. */
export function readRepositoryIdAt(identityPath: string): string | undefined {
  try {
    const repositoryId = readFileSync(identityPath, "utf8").trim();
    return repositoryId.length === 0 ? undefined : repositoryId;
  } catch (error) {
    if (typeof error === "object" && error !== null && "code" in error && error.code === "ENOENT") return undefined;
    throw error;
  }
}
