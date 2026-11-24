/**
 * The audit's and the scan's `git diff --name-only -z` under the guard (`call-guard.ts`): run with Git kept out of
 * submodules, then each submodule Git's diff would have looked into asked through the guard, opened as Git opens a
 * submodule (`submodule-state.ts`), all of them at once. A caller says only how a Git of the call starts
 * ({@link StartLater}), within the call's one time limit.
 */
import { existsSync } from "node:fs";
import { join } from "node:path";
import { listedLater, MAX_LISTING_BYTES, stdoutOrFailure, unanswered, type GitVariables, type Guard } from "./call-guard.js";
import {
  definesNoDriver, GITLINK_LISTING, nameDiff, outsideSubmodules, recordsNoSubmodule, repositoryConfig, UNASKED, withChangedSubmodules, worktreeIndex,
  type Ask, type DiffRepository, type GitFailure, type GuardedRepository, type Places, type SubmoduleOpener,
} from "./submodule-state.js";

/** What a Git of a guarded call answered, as `runGit` gives it: its status, output and error. */
export type GitRun = GitFailure;

/** What a Git of a guarded call is started with: the variables it sets, the most it prints to each stream, its limit. */
export interface StartOptions {
  readonly env?: GitVariables;
  readonly maxBuffer?: number;
  /** The most bytes Git may print to stderr; `maxBuffer` by default. */
  readonly maxErrorBuffer?: number;
  readonly timeout?: number;
}

/** Starts `git -C <cwd> <args>`, one Git of a call that does not block, within what is left of the call's time limit. */
export type StartLater = (cwd: string, args: readonly string[], options?: StartOptions) => Promise<GitRun>;

/** A guarded call that does not block: its folder, its arguments and the options it was asked with. */
export interface DiffCall {
  readonly cwd: string;
  readonly args: readonly string[];
  readonly options: StartOptions;
}

/**
 * Runs each `work` given it while fewer than `count` run, the others in the order they came as each one ends: the Gits
 * of one call that does not block take turns, so a checkout's submodules, asked at once, start no more Gits at a time.
 */
export function inTurn(count: number): <T>(work: () => Promise<T>) => Promise<T> {
  let running = 0;
  const waiting: Array<() => void> = [];
  return async work => {
    if (running < count) running += 1;
    else await new Promise<void>(resolve => waiting.push(resolve));
    try {
      return await work();
    } finally {
      // The place passes to the next waiting work, or is given back.
      const next = waiting.shift();
      if (next === undefined) running -= 1;
      else next();
    }
  };
}

/**
 * `call`, which reads worktree files, under `guard`, each Git started with `start`. A `git diff --name-only -z` that
 * would look into submodules is answered as Git answers it with Git looking into none: run with
 * `--ignore-submodules=dirty`, then each submodule it would have looked into asked through the guard. Any other call
 * runs as it is, so one that would look into submodules (Rerail makes none) has Git look into each with that
 * submodule's drivers as they are.
 */
export async function guardedCall(start: StartLater, { cwd, args, options }: DiffCall, guard: Guard): Promise<GitRun> {
  const diff = nameDiff(args, guard.looksOutside);
  if (diff === undefined) return await start(cwd, args, { ...options, env: guard.env });
  const outside = await start(cwd, outsideSubmodules(args), { ...options, env: guard.env });
  if (outside.status !== 0 || outside.error !== undefined) return outside;
  const answer = await withChangedSubmodules(outside.stdout, diff, diffRepository(start, cwd, guard), submoduleOpener(start, options.env));
  return typeof answer === "string" ? { ...outside, stdout: answer } : unanswered(answer);
}

/** Stdout of each question `git -C <cwd> <args>` with `env` set, or how it failed. */
function asker(start: StartLater, cwd: string, env: GitVariables): Ask {
  return async (args, most = MAX_LISTING_BYTES) => {
    // Stderr keeps the default's cap whatever stdout's: what a question prints there does not change its answer.
    return stdoutOrFailure(await start(cwd, args, { env, maxBuffer: most, maxErrorBuffer: MAX_LISTING_BYTES }));
  };
}

/** The guard's questions in `folder`, each with what it sets ({@link listedLater}). */
function guardAsker(start: StartLater, folder: string): (args: readonly string[], options?: StartOptions) => Promise<string | GitRun> {
  return async (args, options) => stdoutOrFailure(await start(folder, args, options));
}

/** The repository a guarded diff runs in, as `submodule-state.ts` asks it: its index found in process. */
function diffRepository(start: StartLater, cwd: string, guard: Guard): DiffRepository {
  const ask = asker(start, cwd, guard.env);
  const { entries } = guard;
  const listed = entries === undefined ? async () => await listedIn(start, cwd, guard.env) : async () => await Promise.resolve(entries);
  return { listed, ask, place: async () => await places(ask), index: worktreeIndex(cwd, { ...process.env, ...guard.env }) };
}

/** The config of the repository in `folder`, whose questions run with `env` set, as the guard lists it; how that failed. */
async function listedIn(start: StartLater, folder: string, env: GitVariables): Promise<GuardedRepository["entries"] | GitRun> {
  const listed = await listedLater(guardAsker(start, folder), env);
  return "entries" in listed ? listed.entries : unanswered(listed);
}

/** The top of the worktree a question runs in, and its folder below that top; how asking failed instead. */
async function places(ask: Ask): Promise<Places | GitFailure> {
  const answer = await ask(["rev-parse", "--show-toplevel", "--show-prefix"]);
  if (typeof answer !== "string") return answer;
  const [top = "", prefix = ""] = answer.split("\n");
  return { top, prefix };
}

/**
 * The variables Git clears before it starts a Git in a submodule (`local_repo_env` in Git's environment.c, less the
 * command-line config that `prepare_other_repo_env` hands on); it then sets `GIT_DIR=.git` in the submodule's folder.
 */
const SUBMODULE_CLEARED = [
  "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_CONFIG", "GIT_OBJECT_DIRECTORY", "GIT_DIR", "GIT_WORK_TREE", "GIT_IMPLICIT_WORK_TREE",
  "GIT_GRAFT_FILE", "GIT_INDEX_FILE", "GIT_NO_REPLACE_OBJECTS", "GIT_REPLACE_REF_BASE", "GIT_PREFIX", "GIT_SHALLOW_FILE", "GIT_COMMON_DIR",
] as const;

/**
 * Opens each submodule through the guard as Git opens one for the `git status` it runs there: in the submodule's
 * folder with the variables of {@link SUBMODULE_CLEARED} cleared and `GIT_DIR` set, which also skips the ownership
 * check, as Git's does. Git sets `GIT_DIR=.git` and its status reads the `.git` file; Rerail names the Git folder that
 * file named when it looked, so the guard's listing and the status open the same one. `env` is the variables the diff
 * was asked with.
 */
function submoduleOpener(start: StartLater, env: GitVariables = {}): SubmoduleOpener {
  const cleared: GitVariables = { ...env, ...Object.fromEntries(SUBMODULE_CLEARED.map(name => [name, undefined])) };
  return async (folder, gitFolder) => {
    const opened = { ...cleared, GIT_DIR: gitFolder };
    const plain = definesNoDriver(gitFolder) && await recordsNoSubmodule(join(gitFolder, "index"));
    return plain ? statusOnly(asker(start, folder, opened), folder) : await listedSubmodule(start, folder, gitFolder, opened);
  };
}

/**
 * A submodule whose own config defines no driver ({@link definesNoDriver}) and whose index records no submodule
 * ({@link recordsNoSubmodule}), each read in process: its status asked with `ask`, one Git as Git's own diff starts in
 * it, and no config listed. Its index's submodules ({@link GITLINK_LISTING}) are none, as read, so no rule of its
 * config is read.
 */
function statusOnly(ask: Ask, folder: string): GuardedRepository {
  return { entries: [], ask: async (args, most) => args === GITLINK_LISTING ? "" : await ask(args, most), places: { top: folder, prefix: "" } };
}

/**
 * The submodule at `folder`, whose Git folder is `gitFolder`, opened with `opened` set through its config's listing:
 * each question with the drivers it defines set; how listing failed, or {@link UNASKED}.
 */
async function listedSubmodule(start: StartLater, folder: string, gitFolder: string, opened: GitVariables): Promise<GuardedRepository | GitFailure | typeof UNASKED> {
  const listed = await listedLater(guardAsker(start, folder), opened);
  if (!("entries" in listed)) return unanswered(listed);
  if (!listed.entries.some(entry => entry.scope === "local") && !(await holdsNoEntry(start, folder, gitFolder))) return UNASKED;
  const ask = submoduleAsker(asker(start, folder, listed.env), gitFolder);
  return { entries: listed.entries, ask, places: { top: folder, prefix: "" } };
}

/**
 * The questions of the submodule whose Git folder is `gitFolder`, each asked with `ask` but its index's submodules
 * ({@link GITLINK_LISTING}): those are answered as none, with no Git started, when its index records none
 * ({@link recordsNoSubmodule}). Such a submodule costs Rerail two Gits, its config listing and its status, where Git's
 * own diff started one, its status.
 */
function submoduleAsker(ask: Ask, gitFolder: string): Ask {
  return async (args, most) => args === GITLINK_LISTING && await recordsNoSubmodule(join(gitFolder, "index")) ? "" : await ask(args, most);
}

/**
 * Whether the repository at `gitFolder`, whose listing showed no config of its own, is one whose config file Git reads
 * as holding no entry, so it defines no driver: absent, or empty to Git's own listing of that one file. A listing shows
 * no config of the repository's own also for a Git folder a writer took away while Git read it: one put back since,
 * which holds an entry, or one still away is not asked, so a status never opens a config no listing read as empty.
 */
async function holdsNoEntry(start: StartLater, folder: string, gitFolder: string): Promise<boolean> {
  const config = repositoryConfig(gitFolder);
  if (config === undefined) return false;
  if (!existsSync(config)) return true;
  // `--file` reads that one file and no other config, whatever the environment names.
  const listed = await start(folder, ["config", "-z", "--list", "--file", config], { maxBuffer: MAX_LISTING_BYTES });
  return listed.status === 0 && listed.stdout === "";
}
