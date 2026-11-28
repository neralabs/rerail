/**
 * What Git answers when asked where a folder's repository is ({@link LOCATE_ARGS}). Git exits 0 and prints the top
 * level, common and Git folders. Git answers that it places the folder in no repository it works in by dying: exit
 * 128, its `die()`, whatever the reason it gives (none found, a `.git` file that leads to none, another account's, a
 * bare one, a config it cannot read) and in whatever language it gives it. Every other ending is no answer from Git:
 * a Git Windows could not start (0xC0000142), one ended at its time limit or by a signal, one a wrapper ran that
 * failed. Such an ask is named by a {@link GitCouldNotAnswerError}, which the location carries, so whatever would keep
 * or record from it refuses to. The read of a clone's legacy id from Git's config ({@link LEGACY_ID_ARGS}) takes the
 * same rule with Git's answers for it, exit 0 the value and exit 1 no such key, and fails on every other ending.
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

/**
 * Git could not answer a question about a folder's repository, and the message names how Git ended; `cause` is the
 * error of Git's start, when it has one (could not start, past its time limit, ended by a signal).
 */
export class GitCouldNotAnswerError extends Error {
  constructor(readonly folder: string, readonly ending: string, question = LOCATE.question(folder), cause?: Error) {
    super(`Git could not say ${question}: ${ending}`, { cause });
    this.name = "GitCouldNotAnswerError";
  }
}

/** Git's answer to where a folder's repository is: placed, with what Git printed; outside any; or none at all. */
export type LocateAnswer =
  | { readonly kind: "placed"; readonly printed: string }
  | { readonly kind: "outside" }
  | { readonly kind: "no-answer"; readonly error: GitCouldNotAnswerError };

/**
 * For each question of a folder whose ask failed in this process (the ask and the folder), the lines said; they go
 * once that question is answered there. It holds no more lines than the daemon wrote since those answers.
 */
const said = new Map<string, Set<string>>();

/**
 * Git's answer for `requested` in `result` ({@link LocateAnswer}). For an ask Git gave no answer to, the daemon, whose
 * error output is `daemon.log`, writes the error's line there, once for each folder and line until that ask there is
 * answered.
 */
export function locateAnswer(requested: string, result: GitResult): LocateAnswer {
  const error = noAnswer(requested, result, LOCATE);
  if (error !== undefined) return { kind: "no-answer", error };
  return result.status === 0 ? { kind: "placed", printed: result.stdout } : { kind: "outside" };
}

/**
 * The legacy clone id Git's config holds for `requested`'s repository in `result` ({@link LEGACY_ID_ARGS}), "" when
 * the key is not set. Throws {@link GitCouldNotAnswerError} for every other ending, so the clone gets no new id in
 * place of the one its config holds; the daemon says it as {@link locateAnswer} does.
 */
export function legacyIdAnswer(requested: string, result: GitResult): string {
  const error = noAnswer(requested, result, LEGACY_ID);
  if (error !== undefined) throw error;
  return result.status === 0 ? result.stdout.trim() : "";
}

/**
 * Undefined when Git answered `ask`: exit 0, or `ask.none`, its answer that there is nothing. Else the error naming
 * how Git ended, its line written by the daemon once for each folder, ask and line until that ask there is answered.
 */
function noAnswer(requested: string, result: GitResult, ask: Ask): GitCouldNotAnswerError | undefined {
  const question = ask.question(requested);
  if (result.status === 0 || result.status === ask.none) {
    said.delete(question);
    return undefined;
  }
  const error = new GitCouldNotAnswerError(requested, endingOf(result), question, result.error);
  const lines = said.get(question) ?? new Set<string>();
  if (gitLivesOnHere() && !lines.has(error.message)) {
    said.set(question, lines.add(error.message));
    process.stderr.write(`rerail: ${error.message}\n`);
  }
  return error;
}

/** How `result` ended: its exit status in decimal and hex, or why it has none; its signal; the first line of its stderr. */
function endingOf({ status, signal, error, stderr }: GitResult): string {
  const exit = status === null ? `no exit status (${error?.message ?? "no error either"})` : `exit status ${status} (0x${(status >>> 0).toString(16).toUpperCase().padStart(8, "0")})`;
  const first = stderr.split("\n").find(line => line.trim() !== "")?.trim();
  return `${exit}, signal ${signal ?? "none"}, stderr: ${first ?? "nothing"}`;
}
