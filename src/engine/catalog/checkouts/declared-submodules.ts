/**
 * The submodules a checkout declares: each path its `.gitmodules` names that its index holds as a gitlink, as Git reads
 * them. Every lookup of a file in a checkout nested in another asks it, and so does every walk of a checkout; Git's
 * answer is kept per checkout while what Git read for it reads the same: where Git places the checkout
 * ({@link PlacementInputs.stamp}), the `.gitmodules` text, and the index file, which Git replaces whole at every write,
 * so a new file stands under its name. A checkout with no `.gitmodules` declares none, and Git is not started to say so.
 */
import { lstatSync } from "node:fs";
import { join } from "node:path";
import { fileText, placementInputs, type PlacementInputs } from "./git-placement.js";
import { runGit, type GitResult } from "../../git.js";

/** Each checkout's declared submodule paths, relative to its top level, with the facts Git's answer rested on. */
const kept = new Map<string, { readonly facts: string; readonly paths: readonly string[] }>();

/**
 * The paths, relative to `root` (a checkout's top level), of the submodules it declares: kept while its facts read the
 * same, else asked of Git. Empty when Git cannot answer.
 */
export function declaredSubmodulePaths(root: string): readonly string[] {
  const facts = submoduleFacts(root);
  if (facts === NONE) return [];
  const known = facts === undefined ? undefined : kept.get(root);
  if (known !== undefined && known.facts === facts) return known.paths;
  const answers: GitResult[] = [];
  for (let step = submoduleStep(answers); "ask" in step; step = submoduleStep(answers)) answers.push(runGit(root, step.ask));
  return keep(root, facts, answers);
}

const NONE = "none";

/**
 * What Git's answer for `root` rests on, read now: {@link NONE} when it holds no `.gitmodules`; undefined when a fact
 * cannot be read, or Git may read another index (`GIT_INDEX_FILE`) or place the folder otherwise ({@link placementInputs}),
 * or reads the index of a folder another account owns only while a `safe.directory` entry, in a file not read here,
 * trusts it.
 */
function submoduleFacts(root: string): string | undefined {
  const modules = fileText(join(root, ".gitmodules"));
  if (modules === "none") return NONE;
  const inputs = process.env["GIT_INDEX_FILE"] === undefined ? placementInputs(root) : undefined;
  return modules === undefined || inputs?.owned !== true ? undefined : factsOf(inputs, modules.text);
}

function factsOf(inputs: PlacementInputs, modules: string): string | undefined {
  const index = fileIdentity(join(inputs.gitFolder, "index"));
  return index === undefined ? undefined : [inputs.stamp, index, modules].join("\0");
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

/** The paths `answers` give, kept under `facts` only when Git answered each question to its end. */
function keep(root: string, facts: string | undefined, answers: readonly GitResult[]): readonly string[] {
  const step = submoduleStep(answers);
  const paths = "paths" in step ? step.paths : [];
  if (facts !== undefined && answers.every(answeredToItsEnd)) kept.set(root, { facts, paths });
  else kept.delete(root);
  return paths;
}

/** Whether Git answered: `git config --get-regexp` exits 1 when nothing matches, and that is an answer too. */
function answeredToItsEnd(answer: GitResult, index: number): boolean {
  return answer.status === 0 || (index === 0 && answer.status === 1);
}
