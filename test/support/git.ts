/**
 * Git for tests that build real repositories: a fixed author and committer, and none of the machine's system or global
 * configuration, so no signing, hook or default branch of the person running the tests changes what a commit does.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const GIT_ENV: NodeJS.ProcessEnv = {
  ...process.env,
  GIT_AUTHOR_NAME: "Tim Belanger",
  GIT_AUTHOR_EMAIL: "tim@example.com",
  GIT_COMMITTER_NAME: "Tim Belanger",
  GIT_COMMITTER_EMAIL: "tim@example.com",
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_CONFIG_GLOBAL: "/dev/null",
};

/** Run `git args` in `cwd`; returns its stdout, trimmed, and throws when it exits non-zero. */
export function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", args, { cwd, env: GIT_ENV, encoding: "utf8" }).trim();
}

/** Write each file, by its path under `root`, creating the folders it needs. */
export function writeFiles(root: string, files: Readonly<Record<string, string>>): void {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
}

/** Write `files` and commit every change in the checkout at `root`; returns the new commit. */
export function commit(root: string, files: Readonly<Record<string, string>>, message = "change"): string {
  writeFiles(root, files);
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", message);
  return git(root, "rev-parse", "HEAD");
}
