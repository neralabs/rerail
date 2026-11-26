/**
 * What Git answers when asked where a folder's repository is ({@link LOCATE_ARGS}). Git exits 0 and prints the top
 * level, common and Git folders. Git answers that it places the folder in no repository it works in by dying: exit
 * 128, its `die()`, whatever the reason it gives (none found, a `.git` file that leads to none, another account's, a
 * bare one, a config it cannot read) and in whatever language it gives it. Every other ending is no answer from Git,
 * and the ask fails with a {@link GitCouldNotAnswerError} that names it: a Git Windows could not start (0xC0000142),
 * one ended at its time limit or by a signal, one a wrapper ran that failed. The read of a clone's legacy id from
 * Git's config ({@link LEGACY_ID_ARGS}) takes the same rule with Git's answers for it: exit 0, the value; exit 1, no
 * such key.
 */
import { gitLivesOnHere, type GitResult } from "../../git.js";

/** What Rerail asks Git for a folder: its top level, then its common and Git folders, as absolute paths. */
export const LOCATE_ARGS: readonly string[] = ["rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"];

/** What Rerail asks Git for a clone's legacy id: the `rerail.repository-id` value in the repository's own config. */
export const LEGACY_ID_ARGS: readonly string[] = ["config", "--local", "--get", "rerail.repository-id"];

/**
 * A question put to Git: `none` is the exit status of Git's answer that there is nothing to give, and `question` says
 * what was asked of a folder. Git answers where a folder's repository is by dying (`die()` in Git's usage.c, exit 128)
 * for every folder it places in no repository; `git config --get` exits 1 for a key that is not set.
 */
interface Ask {
  readonly none: number;
  readonly question: (folder: string) => string;
}

const LOCATE: Ask = { none: 128, question: folder => `where the repository of ${folder} is` };
const LEGACY_ID: Ask = { none: 1, question: folder => `which clone id the config of the repository of ${folder} holds` };

/** Git could not answer a question about a folder's repository: the ask failed, and its message names how Git ended. */
export class GitCouldNotAnswerError extends Error {
  constructor(readonly folder: string, readonly ending: string, question = LOCATE.question(folder)) {
    super(`Git could not say ${question}: ${ending}`);
    this.name = "GitCouldNotAnswerError";
  }
}

/** For each folder whose ask failed in this process, the line last said; it goes once an ask there is answered. */
const said = new Map<string, string>();

/**
 * Git's answer for `requested` in `result`: what Git printed when it placed the folder, undefined when Git says the
 * folder is outside any repository. Throws {@link GitCouldNotAnswerError} for every other ending; in the daemon, whose
 * error output is `daemon.log`, it is written there too, once for each folder and ending until an ask there is answered.
 */
export function locateAnswer(requested: string, result: GitResult): string | undefined {
  return answered(requested, result, LOCATE) ? result.stdout : undefined;
}

/**
 * The legacy clone id Git's config holds for `requested`'s repository in `result` ({@link LEGACY_ID_ARGS}), "" when
 * the key is not set. Throws {@link GitCouldNotAnswerError} for every other ending, as {@link locateAnswer} does, so
 * the clone gets no new id in place of the one its config holds.
 */
export function legacyIdAnswer(requested: string, result: GitResult): string {
  return answered(requested, result, LEGACY_ID) ? result.stdout.trim() : "";
}

/**
 * Whether Git gave what `ask` asks for (exit 0) rather than its answer that there is nothing (`ask.none`). Every other
 * ending throws, and the daemon writes the line once for each folder and line until an ask there is answered.
 */
function answered(requested: string, result: GitResult, ask: Ask): boolean {
  if (result.status === 0 || result.status === ask.none) {
    said.delete(requested);
    return result.status === 0;
  }
  const error = new GitCouldNotAnswerError(requested, endingOf(result), ask.question(requested));
  if (gitLivesOnHere() && said.get(requested) !== error.message) {
    said.set(requested, error.message);
    process.stderr.write(`rerail: ${error.message}\n`);
  }
  throw error;
}

/** How `result` ended: its exit status in decimal and hex, or why it has none; its signal; the first line of its stderr. */
function endingOf({ status, signal, error, stderr }: GitResult): string {
  const exit = status === null ? `no exit status (${error?.message ?? "no error either"})` : `exit status ${status} (0x${(status >>> 0).toString(16).toUpperCase().padStart(8, "0")})`;
  const first = stderr.split("\n").find(line => line.trim() !== "")?.trim();
  return `${exit}, signal ${signal ?? "none"}, stderr: ${first ?? "nothing"}`;
}
