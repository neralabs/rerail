/**
 * The guard on each of Rerail's Git calls that reads worktree files, written once for every way Rerail starts a Git.
 *
 * Before such a call it asks, in this order: whether the call reads worktree files at all ({@link worktreeRead});
 * whether the Git this process runs is one the guard works with, asking `git version` once for each Git; whether the
 * repository's config, read in process, may define a filter driver ({@link definesNoDriver}); and, where it may, the
 * config's listing, from which each driver the repository defines is set as the person's own config sets it, or to no
 * program (`filter-drivers.ts`). That sequence is one generator of questions ({@link guardSteps}). A caller says only
 * how each question reaches Git, blocking ({@link CallGuard.now}) or not ({@link CallGuard.later}), within its own time
 * limit, and what a failed question makes of its call.
 */
import { constants as bufferConstants } from "node:buffer";
import {
  CONFIG_LISTING, configEntryVariables, driverOverrides, listedEntries, oldGitNotice, readDriverConfig, worktreeRead, type ListedEntry,
} from "./filter-drivers.js";
import { definesNoDriver, looksOutsideFolder, worktreeGitFolder, type GitFailure } from "./submodule-state.js";

/** The variables a Git call sets over the inherited environment; an undefined value removes one. */
export type GitVariables = Readonly<Record<string, string | undefined>>;

/**
 * The most bytes the guard's own questions (a repository's config, its index's submodules) take: past it, the call it
 * guards is not made. It is the longest string Node makes (`MAX_STRING_LENGTH`, some 512 Mi on 64-bit Node), past which
 * making the output a string would throw, less 64 KiB: a blocking Git's run, through `spawnSync` or the engine, stops
 * only after the 64 KiB read that passed its `maxBuffer`, and keeps that read. UTF-8 decodes to no more UTF-16 units
 * than it has bytes, so output within it becomes a string.
 */
export const MAX_LISTING_BYTES = bufferConstants.MAX_STRING_LENGTH - 64 * 1024;

/** What one of the guard's questions sets beyond its caller's start: the variables it runs with, the most stdout it takes. */
export interface QuestionOptions {
  readonly env: GitVariables;
  readonly maxBuffer: number;
}

/**
 * Asks `git <args>` in the guarded call's folder: its stdout where Git answered ({@link stdoutOrFailure}), else how it
 * failed, as the caller ends a failed call (`F`). Without `options` (Git's version) it starts Git as the caller starts
 * any Git of its own.
 */
export type AskNow<F extends object> = (args: readonly string[], options?: QuestionOptions) => string | F;

/** {@link AskNow} for a call that does not block. */
export type AskLater<F extends object> = (args: readonly string[], options?: QuestionOptions) => Promise<string | F>;

/** A Git call the guard is asked about: its folder, its arguments after `git -C <folder>`, and the variables it sets. */
export interface GuardedCall {
  readonly cwd: string;
  readonly args: readonly string[];
  readonly env: GitVariables;
}

/** A repository's config as the guard listed it, and the variables its calls run with: each driver it defines set. */
export interface Listed {
  readonly entries: readonly ListedEntry[];
  readonly env: GitVariables;
}

/** What the guard decides for a call it covers. */
export interface Guard {
  /** The variables the call runs with: its own, and each driver its repository's config defines set. */
  readonly env: GitVariables;
  /** Whether the Git the call runs looks outside the folder of a `--relative` diff against a commit ({@link looksOutsideFolder}). */
  readonly looksOutside: boolean;
  /** The config, every scope, as the guard listed it; undefined where the config read in process defines no driver. */
  readonly entries: readonly ListedEntry[] | undefined;
}

/**
 * What the guard decides for a call that reads no worktree file, having asked nothing: the call runs as it ran before
 * the guard, `runGit`'s as one Git with the call's own time limit.
 */
export const READS_NO_WORKTREE = "reads-no-worktree";

/** What the guard decides for a call made with a Git older than 2.31: it runs as it is, within the call's limit. */
export const OLDER_GIT = "older-git";

/** What the guard decides for a call it does not cover, which runs as it is. */
export type Unguarded = typeof READS_NO_WORKTREE | typeof OLDER_GIT;

/** Stdout of a Git that answered, exiting 0 with no error; else the run itself, as how it failed. */
export function stdoutOrFailure<R extends GitFailure>(run: R): string | R {
  return run.status === 0 && run.error === undefined ? run.stdout : run;
}

/** A failed question asked for a call, as that call's result: its status, stderr and error, none of its output. */
export function unanswered(failure: GitFailure): GitFailure {
  return { status: failure.status, stdout: "", stderr: failure.stderr, ...(failure.error === undefined ? {} : { error: failure.error }) };
}

/**
 * What a Git means for the guard: what Rerail says when it is too old ({@link oldGitNotice}), whether it said it, and
 * whether its diff against a commit looks outside the folder of a `--relative` diff ({@link looksOutsideFolder}).
 */
interface GuardGit {
  readonly notice: string | undefined;
  told: boolean;
  readonly looksOutside: boolean;
}

/** A Git whose version was not read: the guard covers its calls, and takes it as one that looks outside no folder. */
const UNREAD_GIT = { looksOutside: false } as const;

/**
 * The Git that answered a question asked while `before` was the Git this process ran. For a blocking call, the one the
 * process runs once Git answered, the one found again for the question included: nothing else ran meanwhile. For one
 * that does not block, `before` while the process still runs it: another call may have found that one gone meanwhile,
 * and then which Git answered is not known.
 */
type AnsweredBy = (before: string | undefined) => string | undefined;

/** What the guard of one process works from: the Git this process runs now, and what each Git it asked answered. */
interface GuardState {
  readonly gits: Map<string, GuardGit>;
  readonly runs: () => string | undefined;
}

/**
 * The guard of one process. It keeps what each Git it asked answered, by its path, as `ProcessGit` keeps each Git's
 * build: a daemon whose Git was found gone and replaced asks the one found again.
 */
export class CallGuard {
  readonly #state: GuardState;

  /** `runs` gives the path of the Git this process runs now, or undefined when none is found. */
  constructor(runs: () => string | undefined) {
    this.#state = { gits: new Map(), runs };
  }

  /** What the guard decides for `call`, each question asked with `ask`, which blocks. */
  now<F extends object>(call: GuardedCall, ask: AskNow<F>): Guard | F | Unguarded {
    const { runs } = this.#state;
    return answeredNow(guardSteps<F>(call, this.#state, () => runs()), ask);
  }

  /** What the guard decides for `call`, each question asked with `ask`, which does not block. */
  async later<F extends object>(call: GuardedCall, ask: AskLater<F>): Promise<Guard | F | Unguarded> {
    const { runs } = this.#state;
    return await answeredLater(guardSteps<F>(call, this.#state, before => (runs() === before ? before : undefined)), ask);
  }
}

/** A question of the guard's: the arguments after `git -C <folder>`, and what it sets beyond its caller's start. */
interface Question {
  readonly args: readonly string[];
  readonly options?: QuestionOptions;
}

/** The guard's questions, each answered with Git's stdout or how it failed, then what it decided (`T`). */
type Steps<T, F> = Generator<Question, T, string | F>;

/** What `steps` decides, each question asked with `ask` in turn. */
function answeredNow<T, F extends object>(steps: Steps<T, F>, ask: AskNow<F>): T {
  for (let step = steps.next(); ; step = steps.next(ask(step.value.args, step.value.options))) {
    if (step.done === true) return step.value;
  }
}

/** {@link answeredNow} for questions that do not block. */
async function answeredLater<T, F extends object>(steps: Steps<T, F>, ask: AskLater<F>): Promise<T> {
  for (let step = steps.next(); ; step = steps.next(await ask(step.value.args, step.value.options))) {
    if (step.done === true) return step.value;
  }
}

/**
 * The guard's sequence for `call`: no question for a call that reads no worktree file ({@link READS_NO_WORKTREE}); none
 * more for one that runs a Git the guard does not cover ({@link coveringGit}, {@link OLDER_GIT}); the call's own
 * variables, with no config listed, where the repository's config read in process defines no driver
 * ({@link definesNoDriverAt}); else the config listed ({@link listingQuestion}) and each driver it defines set, or how
 * listing it failed, with none of the call made. The listing and the call are two Git
 * processes: a writer that changes the config between them can have its driver started (docs/threat-model.md, T11).
 */
function* guardSteps<F extends object>(call: GuardedCall, state: GuardState, answeredBy: AnsweredBy): Steps<Guard | F | Unguarded, F> {
  if (worktreeRead(call.args) === "none") return READS_NO_WORKTREE;
  const git = yield* coveringGit<F>(state, answeredBy);
  if (git === undefined) return OLDER_GIT;
  if (definesNoDriverAt(call.cwd, call.env)) return { env: call.env, looksOutside: git.looksOutside, entries: undefined };
  const listing = yield listingQuestion(call.env);
  return typeof listing === "string" ? { ...listedBy(listing, call.env), looksOutside: git.looksOutside } : listing;
}

/**
 * The Git the guard covers a call with: the one this process runs, asked its version where no call has asked that
 * Git, or a Git whose version was not read ({@link UNREAD_GIT}) where it did not answer. Undefined for a Git older than
 * 2.31: every call then runs as it ran before the guard, so each answers as that Git answers, and the first call the
 * guard would cover says why on the process's error stream, which for the daemon is its log.
 */
function* coveringGit<F extends object>(state: GuardState, answeredBy: AnsweredBy): Steps<Pick<GuardGit, "looksOutside"> | undefined, F> {
  // Read before the question: while the Git kept answers, a call that does not block may find it gone and keep another.
  const before = state.runs();
  const decided = (before === undefined ? undefined : state.gits.get(before)) ?? (yield* askedGit<F>(state.gits, answeredBy, before));
  return guardCovers(decided) ? decided ?? UNREAD_GIT : undefined;
}

/** What the Git this process runs answers to `git version`, kept for the Git that answered; undefined where none answered. */
function* askedGit<F extends object>(gits: Map<string, GuardGit>, answeredBy: AnsweredBy, before: string | undefined): Steps<GuardGit | undefined, F> {
  const answer = yield { args: ["version"] };
  return kept(gits, typeof answer === "string" ? guardGitFrom(answer) : undefined, answeredBy(before));
}

/**
 * `asked`, kept for the Git that answered it, `by`: none where that Git is not known, so the next call asks the Git it
 * runs. Where another call kept one for that Git while this one asked, that one stays and is returned, so the process
 * says why a Git is too old once.
 */
function kept(gits: Map<string, GuardGit>, asked: GuardGit | undefined, by: string | undefined): GuardGit | undefined {
  if (asked === undefined || by === undefined) return asked;
  const keep = gits.get(by) ?? asked;
  gits.set(by, keep);
  return keep;
}

/** {@link GuardGit} from the `version` a `git version` printed. */
function guardGitFrom(version: string): GuardGit {
  return { notice: oldGitNotice(version), told: false, looksOutside: looksOutsideFolder(version) };
}

/** Whether `git` is one the guard works with, saying once why not when it is older. */
function guardCovers(git: GuardGit | undefined): boolean {
  if (git?.notice === undefined) return true;
  if (!git.told) process.stderr.write(`rerail: ${git.notice}\n`);
  git.told = true;
  return false;
}

/**
 * Whether the repository a Git started in `cwd` with `env` set opens, found in process ({@link worktreeGitFolder}),
 * defines no filter driver in its own config ({@link definesNoDriver}): its calls then run with no driver set and no
 * config listed, starting the Gits they started before the guard. False, so that the guard lists the config, when that
 * repository cannot be found in process.
 */
function definesNoDriverAt(cwd: string, env: GitVariables): boolean {
  const gitFolder = worktreeGitFolder(cwd, { ...process.env, ...env });
  return gitFolder !== undefined && definesNoDriver(gitFolder);
}

/**
 * The question that lists the config of a repository whose calls run with `env` set ({@link CONFIG_LISTING}). It runs
 * without `GIT_CONFIG`, which `git config` alone reads in place of every config file, so a listing in its presence would
 * miss the files the guarded call reads.
 */
function listingQuestion(env: GitVariables): Question {
  return { args: CONFIG_LISTING, options: { env: { ...env, GIT_CONFIG: undefined }, maxBuffer: MAX_LISTING_BYTES } };
}

/**
 * {@link Listed} from the config `listing` of a repository whose calls run with `env` set: each filter driver its own
 * config defines set as the person's own config sets it, or to no program (`filter-drivers.ts`).
 */
function listedBy(listing: string, env: GitVariables): Listed {
  const entries = listedEntries(listing);
  const drivers = configEntryVariables({ ...process.env, ...env }, driverOverrides(readDriverConfig(entries)));
  return { entries, env: { ...env, ...drivers } };
}

/**
 * The config of the repository `ask` asks in, whose calls run with `env` set, listed as {@link guardSteps} lists it,
 * with no question before: for a diff's own repository once its submodules need it, and for a submodule's.
 */
export async function listedLater<F extends object>(ask: AskLater<F>, env: GitVariables): Promise<Listed | F> {
  const { args, options } = listingQuestion(env);
  const listing = await ask(args, options);
  return typeof listing === "string" ? listedBy(listing, env) : listing;
}
