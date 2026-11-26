/**
 * What Git answers when asked where a folder's repository is ({@link LOCATE_ARGS}). Git exits 0 and prints the top
 * level, common and Git folders. Git answers that it places the folder in no repository it works in by dying: exit
 * 128, its `die()`, whatever the reason it gives (none found, a `.git` file that leads to none, another account's, a
 * bare one, a config it cannot read) and in whatever language it gives it. Every other ending is no answer from Git,
 * and the ask fails with a {@link GitCouldNotAnswerError} that names it: a Git Windows could not start (0xC0000142),
 * one ended at its time limit or by a signal, one a wrapper ran that failed.
 */
import { gitLivesOnHere, type GitResult } from "../../git.js";

/** What Rerail asks Git for a folder: its top level, then its common and Git folders, as absolute paths. */
export const LOCATE_ARGS: readonly string[] = ["rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"];

/** Git's exit status when it dies (`die()` in Git's usage.c), as it does for every folder it places in no repository. */
const DIED = 128;

/** Git could not answer where a folder's repository is: the ask failed, and its message names how Git ended. */
export class GitCouldNotAnswerError extends Error {
  constructor(readonly folder: string, readonly ending: string) {
    super(`Git could not say where the repository of ${folder} is: ${ending}`);
    this.name = "GitCouldNotAnswerError";
  }
}

/** For each folder whose ask failed in this process, the ending last said; it goes once an ask there is answered. */
const said = new Map<string, string>();

/**
 * Git's answer for `requested` in `result`: what Git printed when it placed the folder, undefined when Git says the
 * folder is outside any repository. Throws {@link GitCouldNotAnswerError} for every other ending; in the daemon, whose
 * error output is `daemon.log`, it is written there too, once for each folder and ending until an ask there is answered.
 */
export function locateAnswer(requested: string, result: GitResult): string | undefined {
  if (result.status === 0 || result.status === DIED) {
    said.delete(requested);
    return result.status === 0 ? result.stdout : undefined;
  }
  const error = new GitCouldNotAnswerError(requested, endingOf(result));
  if (gitLivesOnHere() && said.get(requested) !== error.ending) {
    said.set(requested, error.ending);
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
