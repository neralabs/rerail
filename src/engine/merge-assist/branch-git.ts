/**
 * What Merge Assist asks Git about a checkout's branches: the current and base branch, local branches and their tips,
 * ancestry, the paths a branch changed, and the files two branches would conflict on. Every call goes through
 * `runGit`, so a repository's own config cannot run hooks or an fsmonitor; nothing here writes a ref or the index.
 */
import { runGit, type RunGitOptions } from "../git.js";

/** Listing paths can be long on a large branch; this bounds what one call reads. */
const LIST_OUTPUT_BYTES = 16 * 1024 * 1024;
const LIST_OPTIONS: RunGitOptions = { maxBuffer: LIST_OUTPUT_BYTES };
/** `merge-tree` merges whole trees; it gets longer than the default 10 s. */
const MERGE_TREE_OPTIONS: RunGitOptions = { maxBuffer: LIST_OUTPUT_BYTES, timeout: 60_000 };

export interface LocalBranch {
  readonly name: string;
  readonly commit: string;
  /** The checkout that has the branch checked out, or null. */
  readonly worktree: string | null;
  /**
   * The remote the branch tracks, as `branch.<name>.remote` holds it: a remote's name such as `origin`, `.` for a local
   * branch, or a URL; null when the branch tracks nothing.
   */
  readonly upstream: string | null;
}

/**
 * Stdout of a successful `git -C root <args>`, untrimmed so NUL-separated output stays whole; undefined when Git
 * fails, times out or overflows.
 */
function gitOutput(root: string, args: readonly string[], options?: RunGitOptions): string | undefined {
  const result = runGit(root, args, options);
  return result.status === 0 && result.error === undefined ? result.stdout : undefined;
}

/** A revision a caller passes: never read as an option, never empty. */
function revisionArgument(revision: string): boolean {
  return revision.length > 0 && !revision.startsWith("-") && !/[\0\n]/u.test(revision);
}

/** The full name of a ref that resolves to a commit, or undefined. */
function verifiedCommit(root: string, revision: string): string | undefined {
  if (!revisionArgument(revision)) return undefined;
  const output = gitOutput(root, ["rev-parse", "--verify", "--quiet", "--end-of-options", `${revision}^{commit}`]);
  return output?.trim() || undefined;
}

/** The branch checked out in `root`, or null when HEAD is detached. */
export function currentBranch(root: string): string | null {
  const ref = gitOutput(root, ["symbolic-ref", "--quiet", "HEAD"])?.trim();
  return ref?.startsWith("refs/heads/") === true ? ref.slice("refs/heads/".length) : null;
}

/**
 * The branch work merges into: the target of `refs/remotes/origin/HEAD`, else the first of `origin/main`,
 * `origin/master`, `main` and `master` that exists. `ref` is the full ref name; `name` the short one.
 */
export function baseBranch(root: string): { ref: string; name: string } | null {
  const originHead = gitOutput(root, ["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"])?.trim();
  const refs = [
    ...(originHead?.startsWith("refs/remotes/") === true ? [originHead] : []),
    "refs/remotes/origin/main",
    "refs/remotes/origin/master",
    "refs/heads/main",
    "refs/heads/master",
  ];
  const ref = refs.find(candidate => verifiedCommit(root, candidate) !== undefined);
  if (ref === undefined) return null;
  return { ref, name: ref.startsWith("refs/heads/") ? ref.slice("refs/heads/".length) : ref.slice("refs/remotes/".length) };
}

/**
 * Each branch's `branch.<name>.remote`. Read from config rather than `%(upstream:remotename)`, which is empty when the
 * remote is a URL, as `gh pr checkout` sets it for a pull request from a fork.
 */
function branchRemotes(root: string): Map<string, string> {
  // `-z` prints `<key>\n<value>\0`; Git exits 1 when no key matches.
  const output = gitOutput(root, ["config", "-z", "--get-regexp", String.raw`^branch\..*\.remote$`]) ?? "";
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
  const output = gitOutput(root, ["for-each-ref", "--format=%(refname)%00%(objectname)%00%(worktreepath)", "refs/heads"], LIST_OPTIONS);
  if (output === undefined) return [];
  const remotes = branchRemotes(root);
  return output.split("\n").flatMap(line => {
    const [refname, commit, worktree] = line.split("\0");
    if (refname === undefined || commit === undefined || !refname.startsWith("refs/heads/") || commit === "") return [];
    const name = refname.slice("refs/heads/".length);
    return [{ name, commit, worktree: worktree === undefined || worktree === "" ? null : worktree, upstream: remotes.get(name) ?? null }];
  });
}

/** Remote-tracking branches with their tips, without the `<remote>/HEAD` symbolic refs. */
export function remoteBranches(root: string): Array<{ readonly ref: string; readonly commit: string }> {
  const output = gitOutput(root, ["for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)", "refs/remotes"], LIST_OPTIONS) ?? "";
  return output.split("\n").flatMap(line => {
    const [ref, commit, symref] = line.split("\0");
    return ref === undefined || ref === "" || commit === undefined || commit === "" || symref !== "" ? [] : [{ ref, commit }];
  });
}

/** The best common ancestor of two commits, or null when they share none or either is unknown. */
export function mergeBase(root: string, a: string, b: string): string | null {
  if (!revisionArgument(a) || !revisionArgument(b)) return null;
  return gitOutput(root, ["merge-base", "--end-of-options", a, b])?.trim().split("\n")[0] || null;
}

/** Whether `ancestor` is `descendant` or one of its ancestors; false when either is unknown. */
export function isAncestor(root: string, ancestor: string, descendant: string): boolean {
  if (!revisionArgument(ancestor) || !revisionArgument(descendant)) return false;
  return runGit(root, ["merge-base", "--is-ancestor", "--end-of-options", ancestor, descendant]).status === 0;
}

/** Whether `commit` names a commit this repository holds. */
export function commitExists(root: string, commit: string): boolean {
  return verifiedCommit(root, commit) !== undefined;
}

function splitNul(output: string): string[] {
  return output.split("\0").filter(path => path !== "");
}

/**
 * Paths changed from the merge base of `base` and `head` to `head` (from `base` itself when they share no history);
 * with `workingTree`, plus the checkout's staged, unstaged and untracked (not ignored) paths. Renames count as a
 * deletion and an addition, so both paths are listed. Throws when Git cannot list them: no paths would read as a
 * branch that changed nothing.
 */
export function changedPaths(root: string, base: string, head: string, options: { workingTree?: boolean } = {}): string[] {
  const from = mergeBase(root, base, head) ?? verifiedCommit(root, base);
  const to = verifiedCommit(root, head);
  if (from === undefined || to === undefined) throw new Error(`Git cannot compare ${base} and ${head} in ${root}: one of them is not a commit here`);
  const committed = gitOutput(root, ["diff", "--name-only", "-z", "--no-renames", "--no-ext-diff", "--no-textconv", from, to, "--"], LIST_OPTIONS);
  if (committed === undefined) throw new Error(`Git could not list the paths changed between ${base} and ${head} in ${root}`);
  const paths = new Set(splitNul(committed));
  if (options.workingTree === true) {
    const status = gitOutput(root, ["status", "--porcelain=v1", "-z", "--untracked-files=all", "--no-renames"], LIST_OPTIONS);
    if (status === undefined) throw new Error(`Git could not list the working tree's changes in ${root}`);
    // Each entry is `XY <path>`; paths are relative to the top of the working tree.
    for (const entry of splitNul(status)) if (entry.length > 3) paths.add(entry.slice(3));
  }
  return [...paths].sort();
}

/**
 * The files a merge of `theirs` into `ours` would conflict on, from `git merge-tree --write-tree` (Git 2.38 or later);
 * an empty list for a clean merge. Null when either commit is missing or Git cannot merge the two trees (no shared
 * history, an older Git). It writes only unreachable objects, never a ref, the index or the working tree.
 */
export function mergeTreeConflicts(root: string, ours: string, theirs: string): string[] | null {
  const oursCommit = verifiedCommit(root, ours);
  const theirsCommit = verifiedCommit(root, theirs);
  if (oursCommit === undefined || theirsCommit === undefined) return null;
  const result = runGit(root, ["merge-tree", "--write-tree", "--name-only", "--no-messages", "-z", oursCommit, theirsCommit], MERGE_TREE_OPTIONS);
  if (result.error !== undefined || (result.status !== 0 && result.status !== 1)) return null;
  // The merged tree's id comes first; the conflicted paths follow, each ended by NUL.
  const [, ...conflicted] = splitNul(result.stdout);
  return result.status === 0 ? [] : [...new Set(conflicted)].sort();
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
