#!/usr/bin/env node
// The tree CI's "Lint and type-check" job checks: a pull request merged into main. A branch behind the local
// `origin/main` can pass every check on its own and fail in CI, where main's newer files count too (a folder over its
// file limit, a migration list both sides added to). `prepare` prints a folder holding the working tree (staged,
// unstaged and untracked work) merged into `origin/main`, checked out as a throwaway detached worktree with this
// checkout's node_modules linked in; or the checkout itself when it already holds `origin/main`, when there is none,
// or when the merge would conflict (said on stderr: CI runs on neither until the branch is merged). `remove <folder>`
// removes what `prepare` made. scripts/check/pre-commit.sh runs its checks there. Nothing touches the network, the
// branch, the index or the working tree; the commits it writes are unreachable and Git collects them.
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const BASE = "origin/main";
const SCRATCH_PREFIX = "rerail-pre-commit-";

/** Who the throwaway commits name, so they need none of the person's Git configuration. */
const IDENTITY = {
  GIT_AUTHOR_NAME: "rerail pre-commit", GIT_AUTHOR_EMAIL: "pre-commit@rerail.invalid",
  GIT_COMMITTER_NAME: "rerail pre-commit", GIT_COMMITTER_EMAIL: "pre-commit@rerail.invalid",
};

/** `env` without the index Git hands a hook (`GIT_INDEX_FILE`), which every other Git command here must not use. */
function withoutHookIndex(env) {
  const { GIT_INDEX_FILE: _index, ...rest } = env;
  return rest;
}

function git(cwd, args, env) {
  return execFileSync("git", args, { cwd, env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function succeeds(cwd, args, env) {
  return spawnSync("git", args, { cwd, env, stdio: "ignore" }).status === 0;
}

/** The working tree as a commit on HEAD: the index being committed, plus every change and untracked file Git keeps. */
function workingTreeCommit(root, scratch, env) {
  const plain = withoutHookIndex(env);
  const index = join(scratch, "index");
  const committing = env.GIT_INDEX_FILE ?? git(root, ["rev-parse", "--git-path", "index"], plain);
  if (existsSync(resolve(root, committing))) copyFileSync(resolve(root, committing), index);
  const own = { ...plain, GIT_INDEX_FILE: index };
  git(root, ["add", "--all"], own);
  const tree = git(root, ["write-tree"], own);
  return git(root, ["commit-tree", tree, "-p", "HEAD", "-m", "pre-commit: the working tree"], { ...plain, ...IDENTITY });
}

/** `work` merged into `base` as a commit, or the files they conflict in. */
function mergeWithBase(root, work, base, env) {
  const merge = spawnSync("git", ["merge-tree", "--write-tree", "--name-only", work, base], { cwd: root, env, encoding: "utf8" });
  const [tree = "", ...rest] = merge.stdout.split("\n");
  if (merge.status !== 0) return { conflicts: rest.slice(0, rest.indexOf("") === -1 ? rest.length : rest.indexOf("")) };
  return { commit: git(root, ["commit-tree", tree, "-p", work, "-p", base, "-m", `pre-commit: merged with ${base}`], { ...env, ...IDENTITY }) };
}

/**
 * The folder the pre-commit checks run in, from the checkout at `root`, with a `note` for the person when there is one,
 * and whether it is a throwaway worktree that {@link removeMergedCheckout} removes.
 */
export function prepareMergedCheckout(root, { base = BASE, env = process.env } = {}) {
  const plain = withoutHookIndex(env);
  if (!succeeds(root, ["rev-parse", "--verify", "--quiet", `${base}^{commit}`], plain)) {
    return { checkout: root, throwaway: false, note: `there is no ${base}, so your branch is checked alone` };
  }
  if (succeeds(root, ["merge-base", "--is-ancestor", base, "HEAD"], plain)) return { checkout: root, throwaway: false };
  const scratch = mkdtempSync(join(tmpdir(), SCRATCH_PREFIX));
  try {
    const merged = mergeWithBase(root, workingTreeCommit(root, scratch, env), base, plain);
    if (merged.commit === undefined) {
      rmSync(scratch, { recursive: true, force: true });
      return {
        checkout: root, throwaway: false,
        note: `your work conflicts with ${base} in ${merged.conflicts.join(", ")}, and CI runs on neither until you merge it; your branch is checked alone`,
      };
    }
    const checkout = join(scratch, "checkout");
    const noHooks = join(scratch, "no-hooks");
    mkdirSync(noHooks);
    // No hook of the repository's (Git LFS's post-checkout) and no LFS download: CI's checkout holds pointers too.
    git(root, ["-c", `core.hooksPath=${noHooks}`, "worktree", "add", "--quiet", "--detach", checkout, merged.commit], { ...plain, GIT_LFS_SKIP_SMUDGE: "1" });
    if (existsSync(join(root, "node_modules"))) symlinkSync(join(root, "node_modules"), join(checkout, "node_modules"), "junction");
    return { checkout, throwaway: true, note: `checking your work merged with ${base}, as CI does` };
  } catch (error) {
    rmSync(scratch, { recursive: true, force: true });
    throw error;
  }
}

/** Removes a worktree {@link prepareMergedCheckout} made, from the repository at `root`; anything else is left alone. */
export function removeMergedCheckout(root, checkout, { env = process.env } = {}) {
  const scratch = dirname(checkout);
  if (basename(checkout) !== "checkout" || !basename(scratch).startsWith(SCRATCH_PREFIX)) return;
  const modules = join(checkout, "node_modules");
  if (isLink(modules)) unlinkSync(modules);
  spawnSync("git", ["worktree", "remove", "--force", checkout], { cwd: root, env: withoutHookIndex(env), stdio: "ignore" });
  rmSync(scratch, { recursive: true, force: true });
  spawnSync("git", ["worktree", "prune"], { cwd: root, env: withoutHookIndex(env), stdio: "ignore" });
}

/** Whether `path` is a symbolic link (or, on Windows, a junction), whatever it points at. */
function isLink(path) {
  try {
    return lstatSync(path).isSymbolicLink();
  } catch {
    return false;
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = git(process.cwd(), ["rev-parse", "--show-toplevel"], withoutHookIndex(process.env));
  const [command, folder] = process.argv.slice(2);
  if (command === "prepare") {
    const { checkout, note } = prepareMergedCheckout(root);
    if (note !== undefined) process.stderr.write(`pre-commit: ${note}\n`);
    process.stdout.write(`${checkout}\n`);
  } else if (command === "remove" && folder !== undefined) {
    removeMergedCheckout(root, folder);
  } else {
    process.stderr.write("usage: merged-checkout.mjs prepare | remove <folder>\n");
    process.exit(2);
  }
}
