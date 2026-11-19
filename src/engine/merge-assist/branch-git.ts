/**
 * What Merge Assist asks Git about a checkout's branches: the current and base branch, merge bases and ancestry, and
 * the GitHub repository a key names; and the read every Merge Assist Git call shares ({@link gitReadAsync}), which a
 * detection pass's owner stops ({@link readsStoppedBy}). Nothing here blocks the daemon: every call goes through
 * `runGitAsync`, so a repository's own config cannot run hooks or an fsmonitor either; nothing here writes a ref or the
 * index.
 */
import { AsyncLocalStorage } from "node:async_hooks";
import { readFile, stat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { type GitReads, runGitAsync, type GitResult, type RunGitOptions } from "../git.js";

/** The owner of the Git reads of the work a read runs in, as {@link readsStoppedBy} set it; none outside such work. */
const readOwner = new AsyncLocalStorage<GitReads | undefined>();

/**
 * Runs `work` with every Git read of this module it starts, however deep, run by `reads`: once `reads` stops, a read
 * running ends its Git with every process that Git started and answers as a failed read once they have exited, and a
 * later read starts no Git. A detection pass is stopped this way at its next read, with no read naming its owner, and
 * the owner's stop waits for those Git processes, not for the pass's other awaits such as a file read.
 */
export async function readsStoppedBy<T>(reads: GitReads | undefined, work: () => Promise<T>): Promise<T> {
  return await readOwner.run(reads, work);
}

/** `runGitAsync` run by the owner of the work it runs in ({@link readsStoppedBy}). */
async function runRead(root: string, args: readonly string[], options: RunGitOptions & { input?: string } = {}): Promise<GitResult> {
  const reads = readOwner.getStore();
  if (reads === undefined) return await runGitAsync(root, args, options);
  return await reads.run(signal => runGitAsync(root, args, { ...options, signal }));
}

/**
 * Stdout of a successful read-only `git -C root <args>`, paths unquoted (`core.quotePath=false`) and untrimmed so
 * NUL-separated output stays whole; undefined when Git fails, times out or overflows. `input` goes to Git's stdin.
 */
export async function gitReadAsync(root: string, args: readonly string[], options?: RunGitOptions & { input?: string }): Promise<string | undefined> {
  const result = await runRead(root, ["-c", "core.quotePath=false", ...args], options);
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

/** A file's text, or empty when it cannot be read. */
const textOr = (path: string): Promise<string> => readFile(path, "utf8").catch(() => "");

/** The checkout's own Git folder (a linked worktree's `.git` file names it) and the common one; undefined when unreadable. */
async function gitFolders(root: string): Promise<{ own: string; common: string } | undefined> {
  const dotGit = join(root, ".git");
  const folder = await stat(dotGit).then(info => info.isDirectory(), () => undefined);
  if (folder === undefined) return undefined;
  // A `.git` file holds `gitdir: <path>`, relative to the checkout or absolute.
  const named = folder ? dotGit : /^gitdir: (.+)/u.exec(await textOr(dotGit))?.[1]?.trim();
  if (named === undefined) return undefined;
  const own = resolve(root, named);
  return { own, common: resolve(own, (await textOr(join(own, "commondir"))).trim() || ".") };
}

/**
 * What {@link currentBranchAsync} and {@link baseBranchAsync} answer from, as Git's files say it, read without blocking
 * and with no Git process: `HEAD`'s text, `origin/HEAD`'s, whether the ref it names and each of the candidates exist as
 * loose refs, and `packed-refs`' time and size (a clone, a fetch or `git pack-refs` rewrites it). Git writes these with
 * nothing Rerail hears, so a caller that keeps this stamp learns when to ask those two again. Empty when the checkout's
 * Git folder cannot be read.
 */
export async function branchFilesStamp(root: string): Promise<string> {
  const folders = await gitFolders(root);
  if (folders === undefined) return "";
  const { own, common } = folders;
  const originHead = await textOr(join(common, "refs", "remotes", "origin", "HEAD"));
  const target = /^ref: (refs\/remotes\/\S+)/u.exec(originHead)?.[1];
  const refs = [...(target === undefined ? [] : [target]), ...BASE_CANDIDATES];
  const present = await Promise.all(refs.map(ref => stat(join(common, ref)).then(() => ref, () => "")));
  const packed = await stat(join(common, "packed-refs")).then(info => `${info.mtimeMs}:${info.size}`, () => "");
  return [await textOr(join(own, "HEAD")), originHead, ...present, packed].join("\0");
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
  return (await runRead(root, ["merge-base", "--is-ancestor", "--end-of-options", ancestor, descendant])).status === 0;
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
