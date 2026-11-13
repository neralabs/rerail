/**
 * What Merge Assist asks Git about a checkout's branches: the current and base branch, local and remote-tracking
 * branches and their tips, merge bases and ancestry, and the GitHub repository a key names; and the read every Merge
 * Assist Git call shares ({@link gitRead}, and {@link gitReadAsync} for a detection pass, which must not block). Every
 * call goes through `runGit` or `runGitAsync`, so a repository's own config cannot run hooks or an fsmonitor; nothing
 * here writes a ref or the index.
 */
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { runGit, runGitAsync, type RunGitOptions } from "../git.js";

/** Listing paths can be long on a large branch; this bounds what one call reads. */
const LIST_OUTPUT_BYTES = 16 * 1024 * 1024;
const LIST_OPTIONS: RunGitOptions = { maxBuffer: LIST_OUTPUT_BYTES };

export interface LocalBranch {
  readonly name: string;
  readonly commit: string;
  /** The checkout that has the branch checked out, in this platform's form (Git prints `D:/a/b` on Windows), or null. */
  readonly worktree: string | null;
  /**
   * The remote the branch tracks, as `branch.<name>.remote` holds it: a remote's name such as `origin`, `.` for a local
   * branch, or a URL; null when the branch tracks nothing.
   */
  readonly upstream: string | null;
}

/**
 * Stdout of a successful read-only `git -C root <args>`, paths unquoted (`core.quotePath=false`) and untrimmed so
 * NUL-separated output stays whole; undefined when Git fails, times out or overflows.
 */
export function gitRead(root: string, args: readonly string[], options?: RunGitOptions): string | undefined {
  const result = runGit(root, ["-c", "core.quotePath=false", ...args], options);
  return result.status === 0 && result.error === undefined ? result.stdout : undefined;
}

/** {@link gitRead} without blocking; `input` goes to Git's stdin. */
export async function gitReadAsync(root: string, args: readonly string[], options?: RunGitOptions & { input?: string }): Promise<string | undefined> {
  const result = await runGitAsync(root, ["-c", "core.quotePath=false", ...args], options);
  return result.status === 0 && result.error === undefined ? result.stdout : undefined;
}

/** A revision a caller passes: never read as an option, never empty, and no `<rev>:<path>` naming a file in a tree. */
export function revisionArgument(revision: string): boolean {
  return revision.length > 0 && !revision.startsWith("-") && !/[\0\n:]/u.test(revision);
}

/** The full name of a ref that resolves to a commit, or undefined. */
function verifiedCommit(root: string, revision: string): string | undefined {
  if (!revisionArgument(revision)) return undefined;
  const output = gitRead(root, ["rev-parse", "--verify", "--quiet", "--end-of-options", `${revision}^{commit}`]);
  return output?.trim() || undefined;
}

/** The branch checked out in `root`, or null when HEAD is detached. */
export function currentBranch(root: string): string | null {
  const ref = gitRead(root, ["symbolic-ref", "--quiet", "HEAD"])?.trim();
  return ref?.startsWith("refs/heads/") === true ? ref.slice("refs/heads/".length) : null;
}

/**
 * The branch work merges into: the target of `refs/remotes/origin/HEAD`, else the first of `origin/main`,
 * `origin/master`, `main` and `master` that exists. `ref` is the full ref name; `name` the short one.
 */
export function baseBranch(root: string): { ref: string; name: string } | null {
  const originHead = gitRead(root, ["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"])?.trim();
  const refs = [
    ...(originHead?.startsWith("refs/remotes/") === true ? [originHead] : []),
    "refs/remotes/origin/main",
    "refs/remotes/origin/master",
    "refs/heads/main",
    "refs/heads/master",
  ];
  const ref = refs.find(name => verifiedCommit(root, name) !== undefined);
  if (ref === undefined) return null;
  return { ref, name: ref.startsWith("refs/heads/") ? ref.slice("refs/heads/".length) : ref.slice("refs/remotes/".length) };
}

/**
 * Each branch's `branch.<name>.remote`. Read from config rather than `%(upstream:remotename)`, which is empty when the
 * remote is a URL, as `gh pr checkout` sets it for a pull request from a fork.
 */
function branchRemotes(root: string): Map<string, string> {
  // `-z` prints `<key>\n<value>\0`; Git exits 1 when no key matches.
  const output = gitRead(root, ["config", "-z", "--get-regexp", String.raw`^branch\..*\.remote$`]) ?? "";
  const remotes = new Map<string, string>();
  for (const entry of output.split("\0")) {
    const newline = entry.indexOf("\n");
    if (newline === -1) continue;
    const key = entry.slice(0, newline);
    const value = entry.slice(newline + 1);
    if (key.startsWith("branch.") && key.endsWith(".remote") && value !== "") remotes.set(key.slice("branch.".length, -".remote".length), value);
  }
  return remotes;
}

/** Every branch under `refs/heads`, with its tip, the checkout that has it checked out, and the remote it tracks. */
export function localBranches(root: string): LocalBranch[] {
  const output = gitRead(root, ["for-each-ref", "--format=%(refname)%00%(objectname)%00%(worktreepath)", "refs/heads"], LIST_OPTIONS);
  if (output === undefined) return [];
  const remotes = branchRemotes(root);
  return output.split("\n").flatMap(line => {
    const [refname, commit, worktree] = line.split("\0");
    if (refname === undefined || commit === undefined || !refname.startsWith("refs/heads/") || commit === "") return [];
    const name = refname.slice("refs/heads/".length);
    // In this platform's form: Git prints `D:/a/b` on Windows.
    return [{ name, commit, worktree: worktree ? resolve(worktree) : null, upstream: remotes.get(name) ?? null }];
  });
}

/** Remote-tracking branches with their tips, without the `<remote>/HEAD` symbolic refs. */
export function remoteBranches(root: string): Array<{ readonly ref: string; readonly commit: string }> {
  const output = gitRead(root, ["for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)", "refs/remotes"], LIST_OPTIONS) ?? "";
  return output.split("\n").flatMap(line => {
    const [ref, commit, symref] = line.split("\0");
    return ref === undefined || ref === "" || commit === undefined || commit === "" || symref !== "" ? [] : [{ ref, commit }];
  });
}

const mergeBaseArgs = (a: string, b: string): string[] => ["merge-base", "--end-of-options", a, b];
const firstLine = (output: string | undefined): string | null => output?.trim().split("\n")[0] || null;
const ancestorArgs = (ancestor: string, descendant: string): string[] => ["merge-base", "--is-ancestor", "--end-of-options", ancestor, descendant];

/** The best common ancestor of two commits, or null when they share none or either is unknown. */
export function mergeBase(root: string, a: string, b: string): string | null {
  if (!revisionArgument(a) || !revisionArgument(b)) return null;
  return firstLine(gitRead(root, mergeBaseArgs(a, b)));
}

/** {@link mergeBase} without blocking. */
export async function mergeBaseAsync(root: string, a: string, b: string): Promise<string | null> {
  if (!revisionArgument(a) || !revisionArgument(b)) return null;
  return firstLine(await gitReadAsync(root, mergeBaseArgs(a, b)));
}

/** Whether `ancestor` is `descendant` or one of its ancestors; false when either is unknown. */
export function isAncestor(root: string, ancestor: string, descendant: string): boolean {
  if (!revisionArgument(ancestor) || !revisionArgument(descendant)) return false;
  return runGit(root, ancestorArgs(ancestor, descendant)).status === 0;
}

/** {@link isAncestor} without blocking. */
export async function isAncestorAsync(root: string, ancestor: string, descendant: string): Promise<boolean> {
  if (!revisionArgument(ancestor) || !revisionArgument(descendant)) return false;
  return (await runGitAsync(root, ancestorArgs(ancestor, descendant))).status === 0;
}

/** The commit `revision` resolves to, without blocking; null when it names none. */
export async function resolveCommit(root: string, revision: string): Promise<string | null> {
  if (!revisionArgument(revision)) return null;
  return firstLine(await gitReadAsync(root, ["rev-parse", "--verify", "--quiet", "--end-of-options", `${revision}^{commit}`]));
}

/**
 * The branch a rebase in progress is rebasing (HEAD is detached meanwhile), from `rebase-merge/head-name` or
 * `rebase-apply/head-name`; null when no rebase of a branch is in progress.
 */
export async function branchBeingRebased(root: string): Promise<string | null> {
  const paths = await gitReadAsync(root, ["rev-parse", "--git-path", "rebase-merge/head-name", "--git-path", "rebase-apply/head-name"]);
  for (const path of (paths ?? "").split("\n").filter(line => line !== "")) {
    const name = await readFile(resolve(root, path), "utf8").then(text => text.trim(), () => "");
    if (name.startsWith("refs/heads/")) return name.slice("refs/heads/".length);
  }
  return null;
}

/** The full name of the ref `branch` tracks (`refs/remotes/origin/x`, or `refs/heads/y` for a local upstream), or null. */
export async function upstreamOf(root: string, branch: string): Promise<string | null> {
  // No `--end-of-options`: `rev-parse --symbolic-full-name` prints it back; the check keeps an option out.
  if (!revisionArgument(branch)) return null;
  return firstLine(await gitReadAsync(root, ["rev-parse", "--symbolic-full-name", `${branch}@{upstream}`]));
}

/** Whether `commit` names a commit this repository holds. */
export function commitExists(root: string, commit: string): boolean {
  return verifiedCommit(root, commit) !== undefined;
}

const GITHUB_HOST_LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/u;
const GITHUB_OWNER = /^[a-z0-9](?:[a-z0-9-]{0,38})$/u;
const GITHUB_NAME = /^[a-z0-9._-]{1,100}$/u;

/**
 * The GitHub repository a repository key (`remote:<host>/<owner>/<name>`, see `normalizeRemoteAddress`) names, for
 * github.com and GitHub Enterprise hosts (a host whose first label is `github`, as `github.example.com`); null for any
 * other key.
 */
export function githubRepositoryOf(repositoryKey: string): { host: string; owner: string; name: string } | null {
  if (!repositoryKey.startsWith("remote:")) return null;
  const [host, owner, name, ...rest] = repositoryKey.slice("remote:".length).toLowerCase().split("/");
  if (host === undefined || owner === undefined || name === undefined || rest.length > 0) return null;
  const labels = host.split(".");
  if (labels.length < 2 || !labels.every(label => GITHUB_HOST_LABEL.test(label))) return null;
  if (host !== "github.com" && labels[0] !== "github") return null;
  if (!GITHUB_OWNER.test(owner) || !GITHUB_NAME.test(name) || name === "." || name === "..") return null;
  return { host, owner, name };
}
