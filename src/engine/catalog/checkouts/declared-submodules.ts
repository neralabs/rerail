/**
 * The submodules a checkout declares: each path its `.gitmodules` names that its index holds as a gitlink, as Git reads
 * them. Every lookup of a file in a checkout nested in another asks it, and so does every walk of a checkout; Git's
 * answer is kept per checkout while what Git read for it reads the same: where Git places the checkout
 * ({@link PlacementInputs.stamp}), the `.gitmodules` text, and the index file, which Git replaces whole at every write,
 * so a new file stands under its name. A checkout with no `.gitmodules` declares none, and Git is not started to say so.
 * Where Git ran past its time limit for that read, the checkout is kept as Git gave no answer ({@link PastLimit}): each
 * call after it answers at once, with no Git on its thread, and Git is asked again off it until Git answers.
 */
import { existsSync, lstatSync } from "node:fs";
import { join } from "node:path";
import { fileText, placementInputs, type PlacementInputs } from "./git-placement.js";
import { GitReads, runGit, runGitAsync, type GitReadOptions, type GitResult } from "../../git.js";

/** Git's answer for a checkout: its declared submodule paths, relative to its top level, and the facts it rested on. */
interface Answered {
  readonly facts: string;
  readonly paths: readonly string[];
  /** The real path of the checkout's common Git folder, by which the answer leaves once the checkout is gone. */
  readonly common: string;
}

/**
 * A checkout whose read Git ran past its time limit for, whatever its facts read since: each call answers that it
 * declares none, as for a read Git gave no answer to, and asks Git again off the thread when no ask is in flight
 * ({@link askedAgain}). Git's answer replaces it; a Git that runs past its limit again leaves it.
 */
interface PastLimit {
  readonly common: string;
  /** The ask off the thread in flight, settling once its Git has exited; undefined while none runs. */
  asking: Promise<void> | undefined;
}

/**
 * Each checkout's kept answer, by its top level, which leaves with the checkout once it is gone
 * ({@link forgetGoneSubmodules}).
 */
const kept = new Map<string, Answered | PastLimit>();

/** The owner of the asks off the thread: {@link stopAskingSubmodules} stops it and puts a new one in its place. */
let asks = new GitReads();

/** What Git's answer for a checkout rests on, and the real path of the checkout's common Git folder. */
interface Facts {
  readonly text: string;
  readonly common: string;
}

/** Drops the answer kept for each checkout of the repository whose common Git folder is `common` once it is gone. */
export function forgetGoneSubmodules(common: string): void {
  for (const [root, answer] of kept) if (answer.common === common && !existsSync(root)) kept.delete(root);
}

/** How many checkouts' submodules are kept: for tests, which count what a removed worktree leaves behind. */
export function keptSubmoduleCount(): number {
  return kept.size;
}

/**
 * The paths, relative to `root` (a checkout's top level), of the submodules it declares: kept while its facts read the
 * same, else asked of Git. Empty when Git cannot answer, and at once while Git's last answer ran past its time limit.
 */
export function declaredSubmodulePaths(root: string): readonly string[] {
  const facts = submoduleFacts(root);
  if (facts === NONE) return [];
  const known = keptPaths(root, facts);
  if (known !== undefined) return known;
  const answers: GitResult[] = [];
  for (let step = submoduleStep(answers); "ask" in step; step = submoduleStep(answers)) answers.push(runGit(root, step.ask));
  return keep(root, facts, answers);
}

/**
 * {@link declaredSubmodulePaths} of `root` learnt without blocking the caller's thread: Git asked as processes no thread
 * waits on, unless a kept answer stands, and kept for the calls after it.
 * The paths learnt, or none where Git's answer is left to the calls after it.
 */
export async function learnDeclaredSubmodules(root: string, options: GitReadOptions = {}): Promise<readonly string[]> {
  const facts = submoduleFacts(root);
  if (facts === undefined || facts === NONE) return [];
  return keptPaths(root, facts) ?? keep(root, facts, await askedWithoutBlocking(root, options));
}

/**
 * The paths kept for `root`: Git's answer while `facts` read as it rested on them, or none while its read ran past its
 * time limit, Git asked again off the thread meanwhile ({@link PastLimit}); undefined where Git is to be asked.
 */
function keptPaths(root: string, facts: Facts | undefined): readonly string[] | undefined {
  const known = kept.get(root);
  if (known === undefined) return undefined;
  if (pastLimit(known)) return answeredMeanwhile(root, known);
  return known.facts === facts?.text ? known.paths : undefined;
}

function pastLimit(known: Answered | PastLimit): known is PastLimit {
  return "asking" in known;
}

/** What a checkout kept past its time limit answers: none, as Git gave none; Git asked again where no ask is in flight. */
function answeredMeanwhile(root: string, known: PastLimit): readonly string[] {
  known.asking ??= askedAgain(root, known);
  return [];
}

/**
 * Asks Git off the thread for the submodules `root` declares, for the checkout `known` keeps past its time limit. An
 * answer is kept as any answer is ({@link keep}): Git's paths, or nothing where Git could not start or failed, which
 * leaves the checkout to the next call's Git as before. A Git that ran past its limit again leaves `known` as it is, and
 * so does a stopped ask ({@link stopAskingSubmodules}). It starts no ask after it.
 */
async function askedAgain(root: string, known: PastLimit): Promise<void> {
  const owner = asks;
  // Read before Git runs: an index written while it runs reads as changed when the answer is next taken.
  const facts = submoduleFacts(root);
  const answers = await owner.run(async signal => await askedWithoutBlocking(root, { signal })).catch(() => [NOT_STARTED]);
  known.asking = undefined;
  if (owner.stopped || kept.get(root) !== known || answers.some(ranPastLimit)) return;
  keep(root, facts === NONE ? undefined : facts, answers);
}

/** A read that threw before Git answered, as `runGitAsync` does when no Git is found: Git gave no answer, and not past its limit. */
const NOT_STARTED: GitResult = { status: null, stdout: "", stderr: "" };

/** Git's answer to each question on the way to `root`'s declared submodules, asked as processes no thread waits on. */
async function askedWithoutBlocking(root: string, options: GitReadOptions): Promise<GitResult[]> {
  const answers: GitResult[] = [];
  for (let step = submoduleStep(answers); "ask" in step; step = submoduleStep(answers)) answers.push(await runGitAsync(root, step.ask, options));
  return answers;
}

/**
 * Ends every ask off the thread ({@link askedAgain}), settling once each Git it started has exited: a daemon's close
 * waits for it. What each checkout keeps stays as it is; a call after it asks under a new owner, as in a later daemon
 * of the same process.
 */
export async function stopAskingSubmodules(): Promise<void> {
  const stopping = asks;
  asks = new GitReads();
  await stopping.stop();
}

/** Settles once the ask off the thread for `root`'s submodules has ended; at once where none runs. */
export async function submodulesAskedAgain(root: string): Promise<void> {
  const known = kept.get(root);
  if (known !== undefined && pastLimit(known)) await known.asking;
}

const NONE = "none";

/**
 * What Git's answer for `root` rests on, read now: {@link NONE} when it holds no `.gitmodules`; undefined when a fact
 * cannot be read, or Git may read another index (`GIT_INDEX_FILE`) or place the folder otherwise ({@link placementInputs}),
 * or reads the index of a folder another account owns only while a `safe.directory` entry, in a file not read here,
 * trusts it.
 */
function submoduleFacts(root: string): Facts | typeof NONE | undefined {
  const modules = fileText(join(root, ".gitmodules"));
  if (modules === "none") return NONE;
  const inputs = process.env["GIT_INDEX_FILE"] === undefined ? placementInputs(root) : undefined;
  return modules === undefined || inputs?.owned !== true ? undefined : factsOf(inputs, modules.text);
}

function factsOf(inputs: PlacementInputs, modules: string): Facts | undefined {
  const index = fileIdentity(join(inputs.gitFolder, "index"));
  return index === undefined ? undefined : { text: [inputs.stamp, index, modules].join("\0"), common: inputs.realCommon };
}

/** A file's identity as a fact (device, file id, size and change time), "none" when nothing is there; undefined when unreadable. */
function fileIdentity(path: string): string | undefined {
  try {
    const stats = lstatSync(path, { bigint: true, throwIfNoEntry: false });
    return stats === undefined ? NONE : `${stats.dev}:${stats.ino}:${stats.size}:${stats.mtimeNs}:${stats.ctimeNs}`;
  } catch {
    return undefined;
  }
}

const CONFIG_ARGS: readonly string[] = ["config", "--file", ".gitmodules", "--null", "--get-regexp", String.raw`^submodule\..*\.path$`];

/** The next Git question on the way to a checkout's declared submodules, given the answers so far, or the paths they give. */
type SubmoduleStep = { readonly ask: readonly string[] } | { readonly paths: readonly string[] };

/**
 * Each `.gitmodules` path (`git config` exits 1 when it names none), then which of them the index holds as a gitlink.
 * A Git that failed gives none, as before.
 */
function submoduleStep(answers: readonly GitResult[]): SubmoduleStep {
  const [configured, index] = answers;
  if (configured === undefined) return { ask: CONFIG_ARGS };
  const paths = configured.status === 0 ? configured.stdout.split("\0").filter(Boolean).map(entry => entry.slice(entry.indexOf("\n") + 1)) : [];
  if (paths.length === 0) return { paths: [] };
  if (index === undefined) return { ask: ["--literal-pathspecs", "ls-files", "--stage", "-z", "--", ...paths] };
  return { paths: index.status === 0 ? gitlinks(index.stdout) : [] };
}

function gitlinks(stdout: string): string[] {
  return stdout.split("\0").filter(entry => entry.startsWith("160000 ")).map(entry => entry.slice(entry.indexOf("\t") + 1));
}

/** The paths `answers` give, and what is kept for `root` after them ({@link keptAfter}). */
function keep(root: string, facts: Facts | undefined, answers: readonly GitResult[]): readonly string[] {
  const step = submoduleStep(answers);
  const paths = "paths" in step ? step.paths : [];
  const next = facts === undefined ? undefined : keptAfter(facts, paths, answers, kept.get(root));
  if (next === undefined) kept.delete(root);
  else kept.set(root, next);
  return paths;
}

/**
 * What a checkout keeps after Git's `answers`: its `paths` under `facts` when Git answered each question to its end; the
 * checkout past its time limit (`known`, where it is kept so already) when a Git ran past its limit; nothing when a Git
 * could not start or failed otherwise, so the next call asks Git as before.
 */
function keptAfter(facts: Facts, paths: readonly string[], answers: readonly GitResult[], known: Answered | PastLimit | undefined): Answered | PastLimit | undefined {
  if (answers.every(answeredToItsEnd)) return { facts: facts.text, paths, common: facts.common };
  if (!answers.some(ranPastLimit)) return undefined;
  return known !== undefined && pastLimit(known) ? known : { common: facts.common, asking: undefined };
}

/** Whether Git answered: `git config --get-regexp` exits 1 when nothing matches, and that is an answer too. */
function answeredToItsEnd(answer: GitResult, index: number): boolean {
  return answer.status === 0 || (index === 0 && answer.status === 1);
}

/** Whether Git ran past its time limit, which {@link runGit} and {@link runGitAsync} give as `spawnSync` does: `ETIMEDOUT`. */
function ranPastLimit(answer: GitResult): boolean {
  return (answer.error as NodeJS.ErrnoException | undefined)?.code === "ETIMEDOUT";
}
