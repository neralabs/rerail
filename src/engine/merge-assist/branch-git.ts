/**
 * What Merge Assist asks Git about a checkout's branches: the current and base branch, merge bases and ancestry, and
 * the GitHub repository a key names; and the read every Merge Assist Git call shares ({@link gitReadAsync}). Nothing
 * here blocks the daemon: every call goes through `runGitAsync`, so a repository's own config cannot run hooks or an
 * fsmonitor either; nothing here writes a ref or the index.
 */
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { runGitAsync, type RunGitOptions } from "../git.js";

/**
 * Stdout of a successful read-only `git -C root <args>`, paths unquoted (`core.quotePath=false`) and untrimmed so
 * NUL-separated output stays whole; undefined when Git fails, times out or overflows. `input` goes to Git's stdin.
 */
export async function gitReadAsync(root: string, args: readonly string[], options?: RunGitOptions & { input?: string }): Promise<string | undefined> {
  const result = await runGitAsync(root, ["-c", "core.quotePath=false", ...args], options);
  return result.status === 0 && result.error === undefined ? result.stdout : undefined;
}

/** A revision a caller passes: never read as an option, never empty, and no `<rev>:<path>` naming a file in a tree. */
export function revisionArgument(revision: string): boolean {
  return revision.length > 0 && !revision.startsWith("-") && !/[\0\n:]/u.test(revision);
}

/** The branch checked out in `root`, or null when HEAD is detached. */
export async function currentBranchAsync(root: string): Promise<string | null> {
  const ref = (await gitReadAsync(root, ["symbolic-ref", "--quiet", "HEAD"]))?.trim();
  return ref?.startsWith("refs/heads/") === true ? ref.slice("refs/heads/".length) : null;
}

/** The branches {@link baseBranchAsync} tries after `origin/HEAD`'s target, in order. */
const BASE_CANDIDATES = ["refs/remotes/origin/main", "refs/remotes/origin/master", "refs/heads/main", "refs/heads/master"];

/**
 * The branch work merges into: the target of `refs/remotes/origin/HEAD`, else the first of `origin/main`,
 * `origin/master`, `main` and `master` that is a commit. `ref` is the full ref name; `name` the short one. One
 * `for-each-ref` over `origin`'s remote-tracking branches, `main` and `master`.
 */
export async function baseBranchAsync(root: string): Promise<{ ref: string; name: string } | null> {
  const listed = await gitReadAsync(root, [
    "for-each-ref", "--format=%(refname)%00%(symref)%00%(objecttype)", "refs/remotes/origin", "refs/heads/main", "refs/heads/master",
  ], { maxBuffer: 16 * 1024 * 1024 });
  const commits = new Set<string>();
  let originHead: string | undefined;
  for (const line of (listed ?? "").split("\n")) {
    const [ref = "", symref = "", type = ""] = line.split("\0");
    if (ref === "refs/remotes/origin/HEAD") originHead = symref;
    else if (symref === "" && type === "commit") commits.add(ref);
  }
  const refs = [...(originHead?.startsWith("refs/remotes/") === true ? [originHead] : []), ...BASE_CANDIDATES];
  const ref = refs.find(name => commits.has(name));
  if (ref === undefined) return null;
  return { ref, name: ref.startsWith("refs/heads/") ? ref.slice("refs/heads/".length) : ref.slice("refs/remotes/".length) };
}

const firstLine = (output: string | undefined): string | null => output?.trim().split("\n")[0] || null;

/** The best common ancestor of two commits, or null when they share none or either is unknown. */
export async function mergeBaseAsync(root: string, a: string, b: string): Promise<string | null> {
  if (!revisionArgument(a) || !revisionArgument(b)) return null;
  return firstLine(await gitReadAsync(root, ["merge-base", "--end-of-options", a, b]));
}

/** Whether `ancestor` is `descendant` or one of its ancestors; false when either is unknown. */
export async function isAncestorAsync(root: string, ancestor: string, descendant: string): Promise<boolean> {
  if (!revisionArgument(ancestor) || !revisionArgument(descendant)) return false;
  return (await runGitAsync(root, ["merge-base", "--is-ancestor", "--end-of-options", ancestor, descendant])).status === 0;
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

const GITHUB_HOST_LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/u;
const GITHUB_OWNER = /^[a-z0-9](?:[a-z0-9-]{0,38})$/u;
const GITHUB_NAME = /^[a-z0-9._-]{1,100}$/u;

/**
 * The GitHub repository a repository key (`remote:<host>/<owner>/<name>`, see `normalizeRemoteAddress`) names, for
 * github.com (`ssh.github.com`, GitHub's SSH over port 443, is github.com) and GitHub Enterprise hosts (a host whose
 * first label is `github`, as `github.example.com`); null for any other key.
 */
export function githubRepositoryOf(repositoryKey: string): { host: string; owner: string; name: string } | null {
  if (!repositoryKey.startsWith("remote:")) return null;
  const [remoteHost, owner, name, ...rest] = repositoryKey.slice("remote:".length).toLowerCase().split("/");
  if (remoteHost === undefined || owner === undefined || name === undefined || rest.length > 0) return null;
  const host = remoteHost === "ssh.github.com" ? "github.com" : remoteHost;
  const labels = host.split(".");
  if (labels.length < 2 || !labels.every(label => GITHUB_HOST_LABEL.test(label))) return null;
  if (host !== "github.com" && labels[0] !== "github") return null;
  if (!GITHUB_OWNER.test(owner) || !GITHUB_NAME.test(name) || name === "." || name === "..") return null;
  return { host, owner, name };
}
